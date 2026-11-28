//! A model provider on the Google Antigravity plan (design §2.8,
//! Antigravity): Google's official `agy` CLI driven over ACP (`agy acp`,
//! JSON-RPC over stdio), with Stitch's tools served to it over MCP. agy is
//! only the model backend: Stitch's loop, tools, approvals and records stay
//! Stitch's.
//!
//! **The MCP call is the tool step.** When agy calls one of the tools, the
//! provider's stream ends with that call (`StopReason::ToolUse`), its HTTP
//! request held open. The node's loop runs the tool as for any provider;
//! the next stream of the thread brings the result, which answers the held
//! request, and streams on from the same agy turn until the next call or
//! the turn's end. A new user message is a new prompt in the thread's
//! session; anything else (a rewind, a compaction, a turn cut off, a node
//! that never saw the session) starts a fresh session with the earlier
//! conversation as a transcript (`sync`).
//!
//! One agy process per tool set, each in a home of its own
//! (`<data>/antigravity/procs/<key>`): a PreToolUse hook that refuses
//! agy's own tools (`hook`), and the MCP configuration naming exactly that
//! set as eager; its `.gemini/antigravity-cli` is the shared one
//! (`<data>/antigravity/home`: the sign-in token linked from the user's,
//! agy's conversations), so a session moves between processes.

mod acp;
mod errors;
pub mod hook;
mod mcp;
pub mod sync;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stitch_providers::{
  BoxFuture, Content, Error, Event, External, Message, Request, Response, Result, Role, StopReason,
  ToolDef, Usage,
};
use tokio::sync::{OwnedMutexGuard, mpsc, oneshot};

use acp::{AgyProc, Busy, RpcError, Spawn};
use mcp::{Mcp, Router, SessionEvent, ToolAnswer};
use sync::{Block, Plan};

/// The provider's kind and its name in `config.toml`.
pub const KIND: &str = "antigravity";
/// The subcommand of `stitch` that is the hook.
pub const HOOK_SUBCOMMAND: &str = "antigravity-hook";
/// How long agy waits on one of Stitch's tools: a day (a detached thread's
/// call may wait on the user's approval). agy's own default is 3 minutes.
const TOOL_TIMEOUT_SECS: u64 = 86_400;
/// agy processes at most, one per tool set (more only while every one has
/// a turn under way).
const MAX_PROCS: usize = 4;
/// A process with no turn under way is stopped after this long unused.
const IDLE: std::time::Duration = std::time::Duration::from_secs(600);

/// Where the bridge keeps its things and how it starts agy.
#[derive(Clone, Debug)]
pub struct Options {
  /// `<data>/antigravity`.
  pub dir: PathBuf,
  pub program: PathBuf,
  pub args: Vec<String>,
  /// Added to agy's environment (tests).
  pub env: Vec<(String, String)>,
  /// The home agy signs in to (`sign_in_home`): where `stitch auth login
  /// antigravity` runs agy's sign-in.
  pub sign_in_home: PathBuf,
  /// The sign-in token agy wrote there.
  pub token: PathBuf,
  /// The hook's command line, as agy runs it.
  pub hook: String,
}

/// Where agy's sign-in is read from: `STITCH_AGY_HOME` when set (the
/// workspace's `.cargo/config.toml` sets it for everything cargo runs, so
/// no test sees the user's sign-in), else the user's home.
pub fn sign_in_home() -> PathBuf {
  std::env::var_os("STITCH_AGY_HOME")
    .filter(|h| !h.is_empty())
    .or_else(|| std::env::var_os("HOME"))
    .map(PathBuf::from)
    .unwrap_or_default()
}

/// The token agy writes in `home` once signed in.
pub fn token_in(home: &Path) -> PathBuf {
  home.join(".gemini/antigravity-cli/antigravity-oauth-token")
}

impl Options {
  /// The defaults for the data directory `data`: `agy` from `STITCH_AGY`,
  /// the `PATH`, else `~/.local/bin`; the sign-in `sign_in_home`'s; the
  /// hook this executable's.
  pub fn new(data: &Path) -> Options {
    let home = std::env::var_os("HOME")
      .map(PathBuf::from)
      .unwrap_or_default();
    let program = std::env::var_os("STITCH_AGY")
      .map(PathBuf::from)
      .or_else(|| in_path("agy"))
      .unwrap_or_else(|| home.join(".local/bin/agy"));
    let sign_in_home = sign_in_home();
    Options {
      dir: data.join(KIND),
      program,
      args: vec!["acp".into()],
      env: Vec::new(),
      token: token_in(&sign_in_home),
      sign_in_home,
      hook: hook_command(),
    }
  }

  /// How things stand, for `stitch auth status`.
  pub fn status(&self) -> String {
    if !self.program.is_file() {
      format!(
        "not available: Google's Antigravity CLI (`agy`) is not installed (not at {})",
        self.program.display()
      )
    } else if self.token.is_file() {
      format!("signed in through agy ({})", self.token.display())
    } else {
      "not signed in (`stitch auth login antigravity` runs agy's sign-in)".into()
    }
  }

  /// agy's own sign-in, interactive on this terminal (`agy acp --login`:
  /// a URL to open anywhere, then the code pasted back, so it works over
  /// SSH), then `agy models` to confirm. The models it lists.
  pub fn login(&self) -> std::result::Result<Vec<String>, String> {
    if !self.program.is_file() {
      return Err(format!(
        "Google's Antigravity CLI (`agy`) is not installed here (not at {}); install it, then sign in again",
        self.program.display()
      ));
    }
    let cwd = self.dir.join("login");
    std::fs::create_dir_all(&cwd).map_err(|e| format!("{}: {e}", cwd.display()))?;
    let status = std::process::Command::new(&self.program)
      .args(["acp", "--login"])
      .env("HOME", &self.sign_in_home)
      .current_dir(&cwd)
      .status()
      .map_err(|e| format!("could not run {}: {e}", self.program.display()))?;
    if !status.success() {
      return Err(format!("agy's sign-in did not finish ({status})"));
    }
    let models = self.cli_models(&cwd)?;
    if models.is_empty() {
      return Err("signed in, but `agy models` listed no models".into());
    }
    Ok(models)
  }

  /// `agy models`, within a minute.
  fn cli_models(&self, cwd: &Path) -> std::result::Result<Vec<String>, String> {
    let mut child = std::process::Command::new(&self.program)
      .arg("models")
      .env("HOME", &self.sign_in_home)
      .current_dir(cwd)
      .stdin(std::process::Stdio::null())
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .map_err(|e| format!("could not run {}: {e}", self.program.display()))?;
    let start = std::time::Instant::now();
    loop {
      match child.try_wait() {
        Ok(Some(_)) => break,
        Ok(None) if start.elapsed() > std::time::Duration::from_secs(60) => {
          let _ = child.kill();
          return Err("`agy models` did not answer within a minute".into());
        }
        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
        Err(e) => return Err(e.to_string()),
      }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
      return Err(format!(
        "`agy models` failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
      ));
    }
    Ok(cli_model_ids(&String::from_utf8_lossy(&out.stdout)))
  }
}

/// The IDs `agy models` prints, one model a line after its first.
fn cli_model_ids(out: &str) -> Vec<String> {
  out
    .lines()
    .filter(|l| !l.trim().is_empty() && !l.starts_with("Fetching"))
    .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
    .collect()
}

/// What signing out is: agy's own, not Stitch's to delete.
pub const LOGOUT: &str = "Stitch does not remove agy's sign-in: it is agy's own, shared with agy \
  itself. To sign out, run `agy` and type /logout. Until you sign in again, \
  antigravity/… models do not run here.";

fn in_path(name: &str) -> Option<PathBuf> {
  std::env::split_paths(&std::env::var_os("PATH")?)
    .map(|d| d.join(name))
    .find(|p| p.is_file())
}

/// `'<this executable>' antigravity-hook`.
fn hook_command() -> String {
  let exe = std::env::current_exe()
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_else(|_| "stitch".into());
  // Replaced by an update while it runs (Linux names it so).
  let exe = exe.strip_suffix(" (deleted)").unwrap_or(&exe);
  format!("'{}' {HOOK_SUBCOMMAND}", exe.replace('\'', r"'\''"))
}

pub(crate) fn random_hex(bytes: usize) -> String {
  let mut out = String::new();
  while out.len() < bytes * 2 {
    for b in rand::random::<[u8; 16]>() {
      out.push_str(&format!("{b:02x}"));
    }
  }
  out.truncate(bytes * 2);
  out
}

/// The bridge of a data directory: one per directory in this process.
pub fn bridge(opts: Options) -> Arc<Bridge> {
  static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, Arc<Bridge>>>> = OnceLock::new();
  let mut all = BRIDGES
    .get_or_init(Default::default)
    .lock()
    .expect("bridges lock");
  all
    .entry(opts.dir.clone())
    .or_insert_with(|| Arc::new(Bridge::new(opts)))
    .clone()
}

/// The provider as the node hands it to `stitch_providers`.
#[derive(Clone, Debug)]
pub struct Antigravity {
  pub bridge: Arc<Bridge>,
  /// `thought_level`: `low` to `xhigh` (`max` is `xhigh`); `None` leaves
  /// the model's default.
  pub effort: Option<String>,
}

impl External for Antigravity {
  fn kind(&self) -> &'static str {
    KIND
  }
  fn label(&self) -> String {
    "Google (Antigravity plan)".into()
  }
  fn is_subscription(&self) -> bool {
    true
  }
  fn has_key(&self) -> bool {
    self.bridge.ready().is_ok()
  }
  fn models(&self) -> BoxFuture<'_, Result<Vec<String>>> {
    Box::pin(self.bridge.models())
  }
  fn context_window<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Result<Option<u64>>> {
    Box::pin(async move { Ok(self.bridge.window(model)) })
  }
  fn stream<'a>(
    &'a self,
    request: &'a Request,
    on_event: &'a mut (dyn FnMut(Event) + Send),
  ) -> BoxFuture<'a, Result<Response>> {
    let level = thought_level(self.effort.as_deref());
    Box::pin(self.bridge.stream(request, level, on_event))
  }
}

/// agy's `thought_level` for a provider's `effort`.
fn thought_level(effort: Option<&str>) -> Option<String> {
  match effort?.trim() {
    "" => None,
    "max" => Some("xhigh".into()),
    "minimal" | "none" => Some("low".into()),
    e => Some(e.to_owned()),
  }
}

/// What a thread's session is, kept in `sessions.json` so a node that
/// restarts resumes it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Saved {
  session: String,
  synced: Vec<u64>,
  /// No turn was under way: the session holds exactly `synced`.
  clean: bool,
}

/// The models and windows agy told.
#[derive(Default)]
struct Catalog {
  models: Option<Vec<String>>,
  windows: HashMap<String, u64>,
  any_window: Option<u64>,
}

/// A process and the tool set it serves.
struct Slot {
  key: String,
  proc: Arc<AgyProc>,
  used: Instant,
}

/// The key of a tool set: its names and schemas, in name order, so a
/// changed schema is another set.
fn tool_key(tools: &[ToolDef]) -> String {
  let mut sorted: Vec<&ToolDef> = tools.iter().collect();
  sorted.sort_by(|a, b| a.name.cmp(&b.name));
  let mut h = blake3::Hasher::new();
  for t in sorted {
    h.update(t.name.as_bytes());
    h.update(b"\0");
    h.update(t.input_schema.to_string().as_bytes());
    h.update(b"\0");
  }
  h.finalize().to_hex()[..16].to_owned()
}

fn io_error(p: &Path, e: std::io::Error) -> Error {
  protocol(format!("{}: {e}", p.display()))
}

fn write_json(p: PathBuf, v: Value) -> Result<()> {
  std::fs::write(&p, serde_json::to_vec_pretty(&v).expect("json")).map_err(|e| io_error(&p, e))
}

/// The call agy waits on.
struct Pending {
  call: String,
  reply: oneshot::Sender<ToolAnswer>,
}

/// A thread and its session.
#[derive(Default)]
struct ThreadState {
  key: String,
  persist: bool,
  session: Option<String>,
  /// The process the session is open in.
  proc: Option<Arc<AgyProc>>,
  tx: Option<mpsc::UnboundedSender<SessionEvent>>,
  rx: Option<mpsc::UnboundedReceiver<SessionEvent>>,
  /// Fingerprints of the messages the session holds.
  synced: Vec<u64>,
  /// The session no longer matches `synced`: the next request starts
  /// fresh.
  dirty: bool,
  pending: Option<Pending>,
  /// An agy prompt runs (streaming or waiting on a call), counted in its
  /// process.
  in_turn: Option<Busy>,
  /// The model and thought level set on the open session.
  set: Option<(String, Option<String>)>,
}

pub struct Bridge {
  opts: Options,
  router: Arc<Router>,
  mcp: tokio::sync::OnceCell<(Arc<Mcp>, u16)>,
  procs: tokio::sync::Mutex<Vec<Slot>>,
  reaping: OnceLock<()>,
  threads: Mutex<HashMap<String, Arc<tokio::sync::Mutex<ThreadState>>>>,
  catalog: Mutex<Catalog>,
  saved: Mutex<BTreeMap<String, Saved>>,
}

impl std::fmt::Debug for Bridge {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Bridge")
      .field("dir", &self.opts.dir)
      .finish()
  }
}

/// A turn holding its thread; dropped while armed (the loop stopped the
/// model call), it cancels agy's prompt and marks the session to start
/// fresh.
struct Turn {
  st: OwnedMutexGuard<ThreadState>,
  bridge: Arc<Bridge>,
  armed: bool,
}

impl Drop for Turn {
  fn drop(&mut self) {
    if !self.armed {
      return;
    }
    if let (Some(p), Some(s)) = (&self.st.proc, &self.st.session) {
      p.notify("session/cancel", json!({"sessionId": s}));
    }
    self.st.dirty = true;
    self.st.in_turn = None;
    self.st.pending = None;
    self.bridge.save(&self.st);
  }
}

fn protocol(message: impl std::fmt::Display) -> Error {
  Error::Protocol {
    provider: KIND,
    message: message.to_string(),
  }
}

impl Bridge {
  pub fn new(opts: Options) -> Bridge {
    let saved = std::fs::read(opts.dir.join("sessions.json"))
      .ok()
      .and_then(|b| serde_json::from_slice(&b).ok())
      .unwrap_or_default();
    Bridge {
      opts,
      router: Arc::new(Router::default()),
      mcp: tokio::sync::OnceCell::new(),
      procs: tokio::sync::Mutex::new(Vec::new()),
      reaping: OnceLock::new(),
      threads: Mutex::new(HashMap::new()),
      catalog: Mutex::new(Catalog::default()),
      saved: Mutex::new(saved),
    }
  }

  fn home(&self) -> PathBuf {
    self.opts.dir.join("home")
  }

  fn work(&self) -> PathBuf {
    self.opts.dir.join("work")
  }

  /// Whether agy can be asked: installed, and signed in.
  pub fn ready(&self) -> Result<()> {
    if !self.opts.program.is_file() {
      return Err(protocol(format!(
        "Google's Antigravity CLI (`agy`) is not installed here (not at {})",
        self.opts.program.display()
      )));
    }
    if !self.opts.token.is_file() {
      return Err(Error::Api {
        provider: KIND,
        status: 401,
        kind: "not signed in".into(),
        message: "sign in to Google Antigravity by running `agy` on this machine".into(),
      });
    }
    Ok(())
  }

  fn save(&self, st: &ThreadState) {
    if !st.persist {
      return;
    }
    let mut saved = self.saved.lock().expect("saved lock");
    match &st.session {
      Some(s) => {
        saved.insert(
          st.key.clone(),
          Saved {
            session: s.clone(),
            synced: st.synced.clone(),
            clean: !st.dirty && st.in_turn.is_none(),
          },
        );
      }
      None => {
        saved.remove(&st.key);
      }
    }
    let _ = std::fs::create_dir_all(&self.opts.dir);
    let path = self.opts.dir.join("sessions.json");
    let tmp = path.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_vec(&*saved)
      && std::fs::write(&tmp, text).is_ok()
    {
      let _ = std::fs::rename(&tmp, &path);
    }
  }

  fn thread(&self, key: &str, persist: bool) -> Arc<tokio::sync::Mutex<ThreadState>> {
    let mut threads = self.threads.lock().expect("threads lock");
    threads
      .entry(key.to_owned())
      .or_insert_with(|| {
        let saved = self.saved.lock().expect("saved lock").get(key).cloned();
        let mut st = ThreadState {
          key: key.to_owned(),
          persist,
          ..ThreadState::default()
        };
        if let Some(s) = saved {
          st.session = Some(s.session);
          st.synced = s.synced;
          st.dirty = !s.clean;
        }
        Arc::new(tokio::sync::Mutex::new(st))
      })
      .clone()
  }

  /// The MCP endpoint, started once.
  async fn mcp(&self) -> Result<(Arc<Mcp>, u16)> {
    self
      .mcp
      .get_or_try_init(|| async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
          .await
          .map_err(|e| protocol(format!("the MCP endpoint did not start: {e}")))?;
        let port = listener.local_addr().map_err(protocol)?.port();
        let mcp = Arc::new(Mcp::new(random_hex(32), self.router.clone()));
        tokio::spawn(mcp::serve(listener, mcp.clone()));
        Ok((mcp, port))
      })
      .await
      .cloned()
  }

  /// What every process shares: the sign-in token linked, the scratch
  /// workspace trusted, and so agy's conversations (a session made in one
  /// process resumes in another), all in `home/.gemini/antigravity-cli`.
  fn write_shared(&self) -> Result<PathBuf> {
    let cli = self.home().join(".gemini/antigravity-cli");
    for d in [&cli, &self.work()] {
      std::fs::create_dir_all(d).map_err(|e| io_error(d, e))?;
    }
    let link = cli.join("antigravity-oauth-token");
    if std::fs::read_link(&link).ok().as_deref() != Some(self.opts.token.as_path()) {
      let _ = std::fs::remove_file(&link);
      #[cfg(unix)]
      std::os::unix::fs::symlink(&self.opts.token, &link).map_err(|e| io_error(&link, e))?;
    }
    write_json(
      cli.join("settings.json"),
      json!({"trustedWorkspaces": [self.work()]}),
    )?;
    Ok(cli)
  }

  /// The home of the process for the tool set `key`: its own
  /// `.gemini/config` (the hook, and the MCP configuration naming exactly
  /// `tools` as eager, at `/mcp/<key>`), `.gemini/antigravity-cli` the
  /// shared one. agy reads both from `$HOME`, once, when it starts.
  fn write_home(&self, key: &str, tools: &[ToolDef], port: u16, token: &str) -> Result<PathBuf> {
    let shared = self.write_shared()?;
    let home = self.opts.dir.join("procs").join(key);
    let config = home.join(".gemini/config");
    std::fs::create_dir_all(&config).map_err(|e| io_error(&config, e))?;
    let link = home.join(".gemini/antigravity-cli");
    if std::fs::read_link(&link).ok().as_deref() != Some(shared.as_path()) {
      let _ = std::fs::remove_file(&link);
      #[cfg(unix)]
      std::os::unix::fs::symlink(&shared, &link).map_err(|e| io_error(&link, e))?;
    }
    write_json(
      config.join("hooks.json"),
      json!({"stitch-gate": {"PreToolUse": [{"matcher": "*", "hooks": [
        {"type": "command", "command": self.opts.hook, "timeout": 10}
      ]}]}}),
    )?;
    let eager: serde_json::Map<String, Value> = tools
      .iter()
      .map(|t| (t.name.clone(), json!({"eager": true})))
      .collect();
    write_json(
      config.join("mcp_config.json"),
      json!({"mcpServers": {mcp::SERVER: {
        "url": format!("http://127.0.0.1:{port}/mcp/{key}"),
        "headers": {mcp::TOKEN_HEADER: token},
        "timeoutSeconds": TOOL_TIMEOUT_SECS,
        "tools": eager,
      }}}),
    )?;
    Ok(home)
  }

  /// The process for exactly `tools`, started when there is none or it
  /// died. At most `MAX_PROCS`: to start one more, the one used longest
  /// ago that has no turn under way is stopped (none such: one more runs
  /// until the reaper finds some idle).
  async fn proc_for(&self, tools: &[ToolDef]) -> Result<Arc<AgyProc>> {
    self.ready()?;
    let (mcp, port) = self.mcp().await?;
    let key = tool_key(tools);
    mcp.offer(&key, tools);
    let mut procs = self.procs.lock().await;
    procs.retain(|s| s.proc.alive());
    if let Some(s) = procs.iter_mut().find(|s| s.key == key) {
      s.used = Instant::now();
      return Ok(s.proc.clone());
    }
    let mut stopped = Vec::new();
    while procs.len() >= MAX_PROCS {
      let Some(i) = procs
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.proc.busy())
        .min_by_key(|(_, s)| s.used)
        .map(|(i, _)| i)
      else {
        break;
      };
      stopped.push(procs.remove(i));
    }
    let home = self.write_home(&key, tools, port, &mcp.token)?;
    let log = self.opts.dir.join("agy.log");
    let spawned = AgyProc::spawn(
      Spawn {
        program: &self.opts.program,
        args: &self.opts.args,
        home: &home,
        cwd: &self.work(),
        env: &self.opts.env,
        log: &log,
      },
      self.router.clone(),
    )
    .await
    .map_err(|e| protocol(format!("{e}; its log: {}", log.display())))?;
    procs.push(Slot {
      key,
      proc: spawned.clone(),
      used: Instant::now(),
    });
    drop(procs);
    self.let_go(&stopped);
    Ok(spawned)
  }

  /// Idle threads let go of processes stopped (the last holder of one
  /// stops it); a thread in another step keeps its own until it moves.
  fn let_go(&self, stopped: &[Slot]) {
    if stopped.is_empty() {
      return;
    }
    let threads: Vec<_> = self
      .threads
      .lock()
      .expect("threads lock")
      .values()
      .cloned()
      .collect();
    for t in threads {
      if let Ok(mut st) = t.try_lock()
        && st.in_turn.is_none()
        && st
          .proc
          .as_ref()
          .is_some_and(|p| stopped.iter().any(|s| Arc::ptr_eq(p, &s.proc)))
      {
        st.proc = None;
        st.tx = None;
        st.rx = None;
      }
    }
  }

  /// Stops the processes idle for `IDLE`, every minute, while the bridge
  /// lives.
  fn reap(self: &Arc<Self>) {
    if self.reaping.set(()).is_err() {
      return;
    }
    let weak = Arc::downgrade(self);
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let Some(bridge) = weak.upgrade() else { return };
        bridge.reap_now(IDLE).await;
      }
    });
  }

  /// Stops the processes with no turn under way, unused for `idle`.
  pub async fn reap_now(&self, idle: std::time::Duration) {
    let mut procs = self.procs.lock().await;
    let (stopped, kept): (Vec<Slot>, Vec<Slot>) = std::mem::take(&mut *procs)
      .into_iter()
      .partition(|s| !s.proc.alive() || (!s.proc.busy() && s.used.elapsed() >= idle));
    *procs = kept;
    drop(procs);
    self.let_go(&stopped);
  }

  /// How many agy processes run (tests).
  pub async fn processes(&self) -> usize {
    self
      .procs
      .lock()
      .await
      .iter()
      .filter(|s| s.proc.alive())
      .count()
  }

  fn learn(&self, options: Option<&Value>) {
    let Some(options) = options.and_then(Value::as_array) else {
      return;
    };
    let models: Vec<String> = options
      .iter()
      .find(|o| o.get("id").and_then(Value::as_str) == Some("model"))
      .and_then(|o| o.get("options"))
      .and_then(Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(|v| v.get("value").and_then(Value::as_str).map(str::to_owned))
          .collect()
      })
      .unwrap_or_default();
    if !models.is_empty() {
      self.catalog.lock().expect("catalog lock").models = Some(models);
    }
  }

  /// The models agy offers (`configOptions`' `model` values), asked once.
  pub async fn models(&self) -> Result<Vec<String>> {
    if let Err(e) = self.ready() {
      return Err(match e {
        Error::Api { status: 401, .. } => Error::NoKey(KIND),
        e => e,
      });
    }
    if let Some(m) = self.catalog.lock().expect("catalog lock").models.clone() {
      return Ok(m);
    }
    let proc = self.proc_for(&[]).await?;
    let r = proc
      .request("session/new", json!({"cwd": self.work(), "mcpServers": []}))
      .await
      .map_err(rpc_error)?;
    self.learn(r.get("configOptions"));
    if let Some(s) = r.get("sessionId").and_then(Value::as_str) {
      proc.close(s);
    }
    self
      .catalog
      .lock()
      .expect("catalog lock")
      .models
      .clone()
      .ok_or_else(|| protocol("agy listed no models"))
  }

  /// The window agy said for `model` (its `usage_update`'s `size`).
  pub fn window(&self, model: &str) -> Option<u64> {
    let c = self.catalog.lock().expect("catalog lock");
    c.windows.get(model).copied().or(c.any_window)
  }

  async fn stream(
    self: &Arc<Self>,
    req: &Request,
    level: Option<String>,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    if req.messages.is_empty() {
      return Err(protocol("a request with no messages"));
    }
    self.ready()?;
    self.reap();
    let (key, persist) = match &req.cache_key {
      Some(k) => (k.clone(), true),
      None => (anonymous_key(req), false),
    };
    let st = self.thread(&key, persist).lock_owned().await;
    let mut turn = Turn {
      st,
      bridge: self.clone(),
      armed: false,
    };
    let fps = sync::fingerprints(&req.messages);
    let synced = (turn.st.session.is_some() && !turn.st.dirty).then(|| turn.st.synced.clone());
    let waiting = turn.st.pending.as_ref().map(|p| p.call.clone());
    let mut plan = sync::plan(synced.as_deref(), waiting.as_deref(), &req.messages);
    if let Plan::Answer { output, is_error } = &plan {
      let pending = turn.st.pending.take().expect("a waiting call");
      let alive = turn.st.proc.as_ref().is_some_and(|p| p.alive());
      let answer = ToolAnswer {
        text: output.clone(),
        is_error: *is_error,
      };
      if alive && pending.reply.send(answer).is_ok() {
        turn.st.synced = fps.clone();
        turn.armed = true;
      } else {
        plan = Plan::Fresh {
          blocks: sync::fresh_prompt(&req.messages),
          why: "the call agy waited on is gone".into(),
        };
      }
    }
    if let Plan::Prompt(blocks) = &plan {
      if self.open(&mut turn.st, req, &level).await? {
        self.prompt(&mut turn, blocks);
        turn.st.synced = fps.clone();
      } else {
        plan = Plan::Fresh {
          blocks: sync::fresh_prompt(&req.messages),
          why: "the session could not be resumed".into(),
        };
      }
    }
    if let Plan::Fresh { blocks, why } = &plan {
      if turn.st.session.is_some() {
        let why = if turn.st.dirty {
          "its last turn did not finish"
        } else {
          why
        };
        eprintln!("antigravity {key}: a fresh session ({why})");
      }
      self.abandon(&mut turn.st);
      self.new_session(&mut turn.st, req, &level).await?;
      self.prompt(&mut turn, blocks);
      turn.st.synced = fps;
    }
    self.save(&turn.st);
    let names: HashSet<String> = req.tools.iter().map(|t| t.name.clone()).collect();
    let out = self.run(&mut turn, &names, &req.model, on_event).await;
    // A one-off request (no thread) leaves no session open once answered.
    if !turn.st.persist
      && turn.st.in_turn.is_none()
      && let Some(session) = turn.st.session.take()
    {
      if let Some(p) = turn.st.proc.take() {
        p.close(&session);
      }
      self.abandon(&mut turn.st);
      self.router.remove(&session);
      self.threads.lock().expect("threads lock").remove(&key);
    }
    out
  }

  /// Leaves the session as it is: a prompt under way cancelled, a waiting
  /// call let go.
  fn abandon(&self, st: &mut ThreadState) {
    if st.in_turn.is_some()
      && let (Some(p), Some(s)) = (&st.proc, &st.session)
    {
      p.notify("session/cancel", json!({"sessionId": s}));
    }
    if let Some(s) = &st.session {
      self.router.remove(s);
    }
    st.pending = None;
    st.in_turn = None;
    st.tx = None;
    st.rx = None;
  }

  fn listen(&self, st: &mut ThreadState, session: &str) {
    let (tx, rx) = mpsc::unbounded_channel();
    self.router.register(session, tx.clone());
    st.tx = Some(tx);
    st.rx = Some(rx);
  }

  /// The thread's session open in the process of the request's tools
  /// (resumed there if
  /// it is elsewhere or was never open in this node's run), its model set;
  /// false when it cannot be resumed.
  async fn open(
    &self,
    st: &mut ThreadState,
    req: &Request,
    level: &Option<String>,
  ) -> Result<bool> {
    let Some(session) = st.session.clone() else {
      return Ok(false);
    };
    let current = self.proc_for(&req.tools).await?;
    let here = st
      .proc
      .as_ref()
      .is_some_and(|p| Arc::ptr_eq(p, &current) && p.alive())
      && st.rx.is_some();
    if !here {
      if let Some(old) = st.proc.take()
        && old.alive()
      {
        old.close(&session);
      }
      self.router.remove(&session);
      let params = json!({"sessionId": session, "cwd": self.work(), "mcpServers": []});
      let resumed = match current.request("session/resume", params.clone()).await {
        Err(e) if e.code == -32601 => current.request("session/load", params).await,
        r => r,
      };
      match resumed {
        Ok(_) => {}
        Err(e) if e.code == 0 => return Err(rpc_error(e)),
        Err(e) => {
          eprintln!("antigravity: session {session} not resumed: {e}");
          return Ok(false);
        }
      }
      self.listen(st, &session);
      st.proc = Some(current);
      st.set = None;
    }
    self.set_model(st, req, level).await?;
    Ok(true)
  }

  async fn new_session(
    &self,
    st: &mut ThreadState,
    req: &Request,
    level: &Option<String>,
  ) -> Result<()> {
    let proc = self.proc_for(&req.tools).await?;
    let mut params = json!({"cwd": self.work(), "mcpServers": []});
    if let Some(system) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
      params["_meta"] = json!({"antigravity": {"systemPrompt": {"append": system}}});
    }
    let r = proc
      .request("session/new", params)
      .await
      .map_err(rpc_error)?;
    self.learn(r.get("configOptions"));
    let session = r
      .get("sessionId")
      .and_then(Value::as_str)
      .ok_or_else(|| protocol("session/new answered no sessionId"))?
      .to_owned();
    self.listen(st, &session);
    st.session = Some(session);
    st.proc = Some(proc);
    st.set = None;
    st.dirty = false;
    st.synced.clear();
    self.set_model(st, req, level).await
  }

  async fn set_model(
    &self,
    st: &mut ThreadState,
    req: &Request,
    level: &Option<String>,
  ) -> Result<()> {
    let want = (req.model.clone(), level.clone());
    if st.set.as_ref() == Some(&want) {
      return Ok(());
    }
    let (Some(proc), Some(session)) = (&st.proc, &st.session) else {
      return Err(protocol("no session"));
    };
    let set = |id: &str, value: &str| {
      proc.request(
        "session/set_config_option",
        json!({"sessionId": session, "configId": id, "value": value}),
      )
    };
    match set("model", &req.model).await {
      Ok(_) => {}
      Err(e) if e.code == -32602 => {
        let known = self
          .catalog
          .lock()
          .expect("catalog lock")
          .models
          .clone()
          .unwrap_or_default();
        return Err(Error::Api {
          provider: KIND,
          status: 404,
          kind: "model_not_found".into(),
          message: format!(
            "Antigravity has no model `{}` (its models: {})",
            req.model,
            known.join(", ")
          ),
        });
      }
      Err(e) => return Err(rpc_error(e)),
    }
    if let Some(l) = level
      && let Err(e) = set("thought_level", l).await
    {
      eprintln!(
        "antigravity: thought level `{l}` not set on {}: {e}",
        req.model
      );
    }
    st.set = Some(want);
    Ok(())
  }

  /// Sends a prompt; its answer comes to the session's stream as `Done`.
  fn prompt(&self, turn: &mut Turn, blocks: &[Block]) {
    let (Some(proc), Some(session), Some(tx)) = (&turn.st.proc, &turn.st.session, &turn.st.tx)
    else {
      return;
    };
    let prompt: Vec<Value> = blocks.iter().map(Block::to_acp).collect();
    let answer = proc.start(
      "session/prompt",
      json!({"sessionId": session, "prompt": prompt}),
    );
    let tx = tx.clone();
    tokio::spawn(async move {
      let r = answer
        .await
        .unwrap_or_else(|_| Err(RpcError::gone("agy exited")));
      let _ = tx.send(SessionEvent::Done(r));
    });
    turn.st.in_turn = Some(Busy::new(proc.clone()));
    turn.armed = true;
  }

  /// Streams the session's turn until agy calls a tool or ends it.
  async fn run(
    &self,
    turn: &mut Turn,
    tools: &HashSet<String>,
    model: &str,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    let mut text = String::new();
    // A chunk that may be one of agy's error lines, shown only once more
    // follows.
    let mut held = String::new();
    let named = format!("{KIND}/{model}");
    loop {
      let Some(event) = turn
        .st
        .rx
        .as_mut()
        .expect("a listening session")
        .recv()
        .await
      else {
        turn.armed = false;
        turn.st.in_turn = None;
        turn.st.dirty = true;
        self.save(&turn.st);
        return Err(protocol("agy's session closed"));
      };
      match event {
        SessionEvent::Update(u) => match u.get("sessionUpdate").and_then(Value::as_str) {
          Some("agent_message_chunk") => {
            let Some(t) = u.pointer("/content/text").and_then(Value::as_str) else {
              continue;
            };
            if !held.is_empty() {
              let h = std::mem::take(&mut held);
              on_event(Event::Text(h.clone()));
              text.push_str(&h);
            }
            if errors::may_be_error(t) {
              held.push_str(t);
            } else {
              on_event(Event::Text(t.to_owned()));
              text.push_str(t);
            }
          }
          Some("agent_thought_chunk") => {
            if let Some(t) = u.pointer("/content/text").and_then(Value::as_str) {
              on_event(Event::Thinking(t.to_owned()));
            }
          }
          Some("usage_update") => {
            if let Some(size) = u.get("size").and_then(Value::as_u64).filter(|s| *s > 0) {
              let mut c = self.catalog.lock().expect("catalog lock");
              c.windows.insert(model.to_owned(), size);
              c.any_window = Some(size);
            }
          }
          _ => {}
        },
        SessionEvent::Call { name, input, reply } => {
          if !tools.contains(&name) {
            let _ = reply.send(ToolAnswer {
              text: format!("`{name}` is not one of this thread's tools here."),
              is_error: true,
            });
            continue;
          }
          if !held.is_empty() {
            on_event(Event::Text(held.clone()));
            text.push_str(&std::mem::take(&mut held));
          }
          let id = format!("agy_{}", random_hex(12));
          let mut content = Vec::new();
          if !text.is_empty() {
            content.push(Content::Text(text));
          }
          content.push(Content::ToolCall {
            id: id.clone(),
            name,
            input,
          });
          turn.st.pending = Some(Pending { call: id, reply });
          turn.st.synced.push(sync::fingerprint(&Message {
            role: Role::Assistant,
            content: content.clone(),
            native: None,
          }));
          turn.armed = false;
          self.save(&turn.st);
          return Ok(Response {
            model: named,
            content,
            native: None,
            stop: StopReason::ToolUse,
            usage: Usage::default(),
            cost: None,
          });
        }
        SessionEvent::Done(r) => {
          turn.armed = false;
          turn.st.in_turn = None;
          let r = match r {
            Ok(r) => r,
            Err(e) => {
              turn.st.dirty = true;
              self.save(&turn.st);
              return Err(rpc_error(e));
            }
          };
          let usage = r.get("usage");
          if usage.is_none() {
            let said = if held.is_empty() { &text } else { &held };
            if let Some(e) = errors::backend_error(said, now_secs()) {
              turn.st.dirty = true;
              self.save(&turn.st);
              return Err(e);
            }
          }
          if !held.is_empty() {
            on_event(Event::Text(held.clone()));
            text.push_str(&held);
          }
          let stop = match r.get("stopReason").and_then(Value::as_str) {
            Some("end_turn") | None => StopReason::EndTurn,
            Some("max_tokens") | Some("max_turn_requests") => StopReason::MaxTokens,
            Some("refusal") => StopReason::Refusal { category: None },
            Some("cancelled") => {
              turn.st.dirty = true;
              self.save(&turn.st);
              return Err(Error::Cut {
                provider: KIND,
                message: "agy cancelled the turn".into(),
              });
            }
            Some(other) => StopReason::Other(other.to_owned()),
          };
          let content = if text.is_empty() {
            Vec::new()
          } else {
            vec![Content::Text(text)]
          };
          if !content.is_empty() {
            turn.st.synced.push(sync::fingerprint(&Message {
              role: Role::Assistant,
              content: content.clone(),
              native: None,
            }));
          }
          self.save(&turn.st);
          return Ok(Response {
            model: named,
            content,
            native: None,
            stop,
            usage: usage.map(usage_of).unwrap_or_default(),
            cost: None,
          });
        }
      }
    }
  }
}

/// A key for a request that names no thread: its system prompt and first
/// message.
fn anonymous_key(req: &Request) -> String {
  let mut h = blake3::Hasher::new();
  h.update(req.system.as_deref().unwrap_or("").as_bytes());
  if let Some(m) = req.messages.first() {
    h.update(sync::text_of(&m.content).as_bytes());
  }
  format!("anon:{}", &h.finalize().to_hex()[..16])
}

fn now_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

/// A prompt's `usage` in Stitch's sense: `inputTokens` already leaves the
/// cache reads out; thoughts are output.
fn usage_of(u: &Value) -> Usage {
  let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
  Usage {
    input_tokens: n("inputTokens"),
    output_tokens: n("outputTokens") + n("thoughtTokens"),
    cache_read_tokens: n("cachedReadTokens"),
    cache_write_tokens: 0,
    cache_write_1h_tokens: 0,
  }
}

fn rpc_error(e: RpcError) -> Error {
  let lower = e.message.to_lowercase();
  if lower.contains("not logged in")
    || lower.contains("sign in")
    || lower.contains("unauthenticated")
  {
    return Error::Api {
      provider: KIND,
      status: 401,
      kind: "not signed in".into(),
      message: format!("{} (sign in by running `agy` once)", e.message),
    };
  }
  if e.code == 0 {
    return protocol(e.message);
  }
  Error::Api {
    provider: KIND,
    status: 500,
    kind: format!("acp {}", e.code),
    message: e.message,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn efforts_are_thought_levels_and_usage_is_stitchs() {
    assert_eq!(thought_level(Some("max")).as_deref(), Some("xhigh"));
    assert_eq!(thought_level(Some("high")).as_deref(), Some("high"));
    assert_eq!(thought_level(Some("")), None);
    assert_eq!(thought_level(None), None);
    let u = usage_of(
      &json!({"inputTokens": 10, "cachedReadTokens": 5, "outputTokens": 3, "thoughtTokens": 2}),
    );
    assert_eq!(
      (u.input_tokens, u.cache_read_tokens, u.output_tokens),
      (10, 5, 5)
    );
  }

  #[test]
  fn agys_model_list_is_read() {
    let out = "Fetching available models...\ngemini-4-hydrogen-high  Gemini 4 Hydrogen (High)\n\
               claude-sonnet-4-6       Claude Sonnet 4.6 (Thinking)\n\n";
    assert_eq!(
      cli_model_ids(out),
      ["gemini-4-hydrogen-high", "claude-sonnet-4-6"]
    );
  }

  #[test]
  fn the_hook_command_quotes_the_executable() {
    let c = hook_command();
    assert!(
      c.starts_with('\'') && c.ends_with(" antigravity-hook"),
      "{c}"
    );
  }
}
