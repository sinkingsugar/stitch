//! Tools from MCP servers (design §2.13): the node's client of the Model
//! Context Protocol, over `rmcp`.
//!
//! Servers are defined per node in `config.toml` (`[mcp.servers.<name>]`),
//! started lazily the first time a thread that enables them needs their
//! tools, shared by every thread on the node, started again after a crash
//! and stopped with the node. The model sees their tools as
//! `<server>__<tool>`; calls go through the same rules and approvals as
//! shell commands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{
  CallToolRequest, CallToolRequestParams, CancelledNotification, CancelledNotificationParam,
  ClientCapabilities, ClientConfig, ClientNotification, ClientRequest, Implementation,
  ProtocolVersion, RequestId, ServerResult, Tool,
};
use rmcp::service::{NotificationContext, PeerRequestOptions, RunningService};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceError, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::RecordId;

use crate::place::expand_home;
use crate::tools::{ApprovalRequest, Approver, BoxFuture, ToolImage, ToolOutcome};
use crate::toolset::{Entry, decide, matches_tool, may_match_server};

/// `[mcp]` in config.toml.
///
/// ```toml
/// [mcp]
/// enable = ["fetch"]                  # what every thread here gets
///
/// [mcp.servers.fetch]
/// command = "uvx"
/// args = ["mcp-server-fetch"]
/// allow = ["fetch"]                   # runs without asking
///
/// [mcp.servers.github]
/// url = "https://api.githubcopilot.com/mcp/"
/// key = "github"                      # `stitch key set github`
/// tools = ["get_*", "list_*", "search_*", "create_issue"]
///
/// [mcp.projects."~/devel/stitch"]
/// enable = ["github__get_*", "github__list_*", "github__create_issue"]
///
/// [mcp.places."project:p-42ada59c2195414f".allow]
/// github = ["create_issue"]           # added there by "always" answers (or by hand)
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSettings {
  /// Servers (`github`) or patterns over namespaced tool names
  /// (`github__get_*`) that every thread with tools on this node gets.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub enable: Vec<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub servers: BTreeMap<String, ServerSettings>,
  /// By directory (`~` allowed): what threads whose shells start there or
  /// below get, besides `enable`.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub projects: BTreeMap<String, ProjectSettings>,
  /// Tools allowed in one place only (a project, a folder, `global`), by
  /// its key.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub places: BTreeMap<String, PlaceMcp>,
}

/// `[mcp.places."<place>"]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceMcp {
  /// By server: its tools that run without asking there, as its own
  /// `allow` names them.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub allow: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSettings {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub enable: Vec<String>,
}

/// `[mcp.servers.<name>]`: a stdio server (`command`) or a streamable HTTP
/// one (`url`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
  /// The program of a stdio server: a path, or a name found on `PATH`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub args: Vec<String>,
  /// Added to the node's environment.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub env: BTreeMap<String, String>,
  /// Where a stdio server runs (default: the home directory).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cwd: Option<String>,
  /// The endpoint of a streamable HTTP server.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub url: Option<String>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub headers: BTreeMap<String, String>,
  /// A key stored with `stitch key set <key>`: the bearer token over
  /// HTTP, or the value of `key_env` for a stdio server.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub key: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub key_env: Option<String>,
  /// The server's tools offered at all (unprefixed names, `*` patterns);
  /// all when empty.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tools: Vec<String>,
  /// Tools that run without asking (unprefixed names, `*` patterns).
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allow: Vec<String>,
  /// `ask` (default): what `allow` does not cover waits for the user;
  /// `allow`: every call runs.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval: Option<String>,
  /// A call's longest wait (default 120, at most 3600).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout_secs: Option<u64>,
  /// Starting and listing tools (default 30).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub startup_timeout_secs: Option<u64>,
  /// `true`: offered to the threads of the user's other nodes too, its
  /// calls running here, under this node's rules (design §2.14).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub share: Option<bool>,
}

const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 3600;
const DEFAULT_STARTUP: u64 = 30;
/// Inline text, at most; longer text is cut and saved whole.
const MAX_CHARS: usize = 32_000;
const MAX_DESCRIPTION: usize = 4_000;
/// The strictest provider's limit on tool names.
const MAX_NAME: usize = 64;
const SEPARATOR: &str = "__";
/// How long a server has to stop: its session to close, then its process
/// to exit after SIGTERM.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

impl McpSettings {
  /// What a thread whose shells start in `cwd` enables: `enable`, then
  /// the lists of the projects `cwd` is in (entries that do not read are
  /// said in the node's log and left out).
  pub fn enabled(&self, cwd: Option<&Path>) -> Vec<Entry> {
    let mut out = self.enable.clone();
    if let Some(cwd) = cwd {
      let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
      for (dir, project) in &self.projects {
        let dir = expand_home(dir);
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        if cwd.starts_with(&dir) {
          out.extend(project.enable.iter().cloned());
        }
      }
    }
    // These lists only ever named servers: one named like a built-in
    // group (`memory`) is that server, not the group.
    out
      .iter()
      .filter_map(|e| match Entry::parse(e) {
        Ok(mut e) => {
          if crate::toolset::group_named(&e.pattern).is_some() {
            e.pattern = format!("{}{SEPARATOR}*", e.pattern);
          }
          Some(e)
        }
        Err(err) => {
          eprintln!("mcp: enable: {err}");
          None
        }
      })
      .collect()
  }
}

/// Whether `name` may name a server: `[a-z0-9-]`, so `__` splits a tool's
/// name unambiguously.
pub fn valid_server_name(name: &str) -> bool {
  !name.is_empty()
    && name.len() <= 32
    && name
      .bytes()
      .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `pattern` with `*` matching any run of characters.
pub fn glob(pattern: &str, s: &str) -> bool {
  let parts: Vec<&str> = pattern.split('*').collect();
  if parts.len() == 1 {
    return pattern == s;
  }
  let (first, last) = (parts[0], parts[parts.len() - 1]);
  let Some(mut rest) = s.strip_prefix(first) else {
    return false;
  };
  for p in &parts[1..parts.len() - 1] {
    match rest.find(p) {
      Some(i) => rest = &rest[i + p.len()..],
      None => return false,
    }
  }
  rest.len() >= last.len() && rest.ends_with(last)
}

/// A tool's name as the model sees it, when it fits.
pub fn exposed_name(server: &str, tool: &str) -> Option<String> {
  let tool: String = tool
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
        c
      } else {
        '_'
      }
    })
    .collect();
  let name = format!("{server}{SEPARATOR}{tool}");
  (!tool.is_empty() && name.len() <= MAX_NAME).then_some(name)
}

/// The node's MCP servers: one connection each, shared by every thread.
pub struct Mcp {
  data: PathBuf,
  servers: Mutex<BTreeMap<String, Arc<Server>>>,
  /// Set by `stop_all` (the node is stopping): nothing starts again.
  closed: Arc<AtomicBool>,
  /// Notified when a server says its tools changed (what the node
  /// shares is published again).
  pub tools_changed: Arc<tokio::sync::Notify>,
  vault: Arc<crate::secrets::Vault>,
}

impl Mcp {
  pub fn new(data: &Path) -> Arc<Mcp> {
    Mcp::with_vault(data, Default::default())
  }

  /// With the node's secrets and variables, for `secret:` and `var:` in
  /// servers' `env` and `key` (design §2.19).
  pub fn with_vault(data: &Path, vault: Arc<crate::secrets::Vault>) -> Arc<Mcp> {
    Arc::new(Mcp {
      data: data.to_owned(),
      servers: Mutex::new(BTreeMap::new()),
      closed: Arc::new(AtomicBool::new(false)),
      tools_changed: Arc::new(tokio::sync::Notify::new()),
      vault,
    })
  }

  /// Whether the node stopped its servers (it is stopping).
  pub fn is_closed(&self) -> bool {
    self.closed.load(Ordering::SeqCst)
  }

  /// Takes the servers as configured now: a new one is added (started on
  /// first use), a changed or removed one let go. Once the servers were
  /// stopped, nothing.
  pub fn configure(&self, settings: &BTreeMap<String, ServerSettings>) {
    let mut servers = self.servers.lock().expect("mcp lock");
    if self.is_closed() {
      return;
    }
    // A loop using a server keeps it until it ends; the server stops
    // once nothing holds it.
    servers.retain(|name, s| settings.get(name) == Some(&s.settings));
    for (name, cfg) in settings {
      if servers.contains_key(name) {
        continue;
      }
      if !valid_server_name(name) {
        eprintln!("mcp: server name `{name}` left out: use a-z, 0-9 and -");
        continue;
      }
      servers.insert(
        name.clone(),
        Arc::new(Server {
          name: name.clone(),
          settings: cfg.clone(),
          data: self.data.clone(),
          vault: self.vault.clone(),
          closed: self.closed.clone(),
          tools_changed: self.tools_changed.clone(),
          state: tokio::sync::Mutex::new(State::default()),
        }),
      );
    }
  }

  pub fn server(&self, name: &str) -> Option<Arc<Server>> {
    self.servers.lock().expect("mcp lock").get(name).cloned()
  }

  pub fn servers(&self) -> Vec<Arc<Server>> {
    self
      .servers
      .lock()
      .expect("mcp lock")
      .values()
      .cloned()
      .collect()
  }

  /// Stops every server (the node is stopping), and waits until they
  /// have: a stdio server gets its input closed, then its process group
  /// is killed if it has not exited within a few seconds. None starts
  /// again: the hub is closed, so a loop or a call still under way finds
  /// no server.
  pub async fn stop_all(&self) {
    let servers: Vec<Arc<Server>> = {
      let mut servers = self.servers.lock().expect("mcp lock");
      self.closed.store(true, Ordering::SeqCst);
      std::mem::take(&mut *servers).into_values().collect()
    };
    futures_util::future::join_all(servers.iter().map(|s| s.close())).await;
  }
}

/// A configured server, and its connection while it has one.
pub struct Server {
  pub name: String,
  pub settings: ServerSettings,
  data: PathBuf,
  vault: Arc<crate::secrets::Vault>,
  /// The hub's: set, it never starts again.
  closed: Arc<AtomicBool>,
  /// The hub's, notified on `list_changed`.
  tools_changed: Arc<tokio::sync::Notify>,
  state: tokio::sync::Mutex<State>,
}

#[derive(Default)]
struct State {
  conn: Option<Arc<Conn>>,
  /// The tools last listed: kept while the server is down.
  tools: Option<Vec<Tool>>,
  failures: u32,
  retry_at: Option<Instant>,
  error: Option<String>,
  stopped: bool,
  /// The `tools/list` under way, shared by every caller waiting for it
  /// (the state is not locked meanwhile).
  listing: Option<Listing>,
  /// Listings that failed in a row, and when to list again: until then
  /// the last known tools stand.
  list_failures: u32,
  list_retry_at: Option<Instant>,
  list_error: Option<String>,
}

type Listing = futures_util::future::Shared<
  std::pin::Pin<Box<dyn Future<Output = Result<Vec<Tool>, String>> + Send>>,
>;

/// How long to wait before trying again after `failures` failures in a
/// row: 2 s, doubling to 60 s.
fn backoff(failures: u32) -> Duration {
  Duration::from_secs((2u64 << failures.saturating_sub(1).min(5)).min(60))
}

struct Conn {
  peer: Peer<RoleClient>,
  /// Taken out to be closed and waited for.
  service: Mutex<Option<RunningService<RoleClient, Handler>>>,
  /// A stdio server's process, never reaped before `close` signals its
  /// group: while it is unreaped, its pid (the group's ID) is no other
  /// process's.
  child: tokio::sync::Mutex<Option<Group>>,
  /// `notifications/tools/list_changed` came.
  changed: Arc<AtomicBool>,
}

impl Conn {
  fn new(
    service: RunningService<RoleClient, Handler>,
    child: Option<Group>,
    changed: Arc<AtomicBool>,
  ) -> Conn {
    Conn {
      peer: service.peer().clone(),
      service: Mutex::new(Some(service)),
      child: tokio::sync::Mutex::new(child),
      changed,
    }
  }

  fn alive(&self) -> bool {
    !self.peer.is_transport_closed()
      && self
        .service
        .lock()
        .expect("mcp lock")
        .as_ref()
        .is_some_and(|s| !s.is_closed())
  }

  /// Closes the session; a stdio server's process group gets SIGTERM,
  /// then SIGKILL once its leader has exited or `CLOSE_WAIT` has passed, so
  /// what it started (`npx`, `uvx` wrappers and their servers) stops too.
  async fn close(&self) {
    let service = self.service.lock().expect("mcp lock").take();
    if let Some(mut service) = service {
      let _ = service.close_with_timeout(CLOSE_WAIT).await;
    }
    let Some(group) = self.child.lock().await.take() else {
      return;
    };
    let Some(mut child) = group.into_child() else {
      return;
    };
    let Some(pid) = child.id() else {
      return; // reaped already: its group is no longer ours to signal
    };
    kill_group(pid, libc::SIGTERM);
    // The leader is waited for without being reaped, so its pid stays
    // the group's ID while the rest of the group is killed: a wrapper that
    // exits on SIGTERM leaves no server that ignored it behind.
    let deadline = Instant::now() + CLOSE_WAIT;
    while !exited(pid) && Instant::now() < deadline {
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    kill_group(pid, libc::SIGKILL);
    let _ = child.wait().await;
  }
}

/// A stdio server's process, leading its group: dropped before `close`
/// took it (a start that failed or timed out), the whole group is killed.
struct Group(Option<tokio::process::Child>);

impl Group {
  fn into_child(mut self) -> Option<tokio::process::Child> {
    self.0.take()
  }
}

impl Drop for Group {
  fn drop(&mut self) {
    // Unreaped (nothing waited on it), so its pid is still its group's.
    if let Some(pid) = self.0.as_ref().and_then(tokio::process::Child::id) {
      kill_group(pid, libc::SIGKILL);
    }
  }
}

/// Whether the child `pid` has exited, left unreaped (or is not ours to
/// wait for).
#[allow(unsafe_code)]
fn exited(pid: u32) -> bool {
  let id: libc::id_t = pid;
  // SAFETY: waitid writes only into `info`, a zeroed siginfo_t of ours;
  // WNOWAIT leaves the child to be reaped by its `Child`.
  unsafe {
    let mut info: libc::siginfo_t = std::mem::zeroed();
    let r = libc::waitid(
      libc::P_PID,
      id,
      &mut info,
      libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
    );
    r != 0 || info.si_signo != 0
  }
}

/// Signals process group `pgid`.
#[allow(unsafe_code)]
pub(crate) fn kill_group(pgid: u32, signal: libc::c_int) {
  let Ok(pgid) = libc::pid_t::try_from(pgid) else {
    return;
  };
  // SAFETY: killpg takes plain integers and touches no memory of ours.
  unsafe {
    libc::killpg(pgid, signal);
  }
}

impl Drop for Conn {
  fn drop(&mut self) {
    if let Some(s) = self.service.get_mut().expect("mcp lock").as_ref() {
      s.cancellation_token().cancel();
    }
  }
}

/// What the node tells servers, and hears from them.
#[derive(Clone)]
struct Handler {
  changed: Arc<AtomicBool>,
  hub: Arc<tokio::sync::Notify>,
}

impl ClientHandler for Handler {
  fn get_info(&self) -> ClientConfig {
    ClientConfig::new(
      ClientCapabilities::default(),
      Implementation::new("stitch", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
  }

  fn on_tool_list_changed(
    &self,
    _: NotificationContext<RoleClient>,
  ) -> impl Future<Output = ()> + Send + '_ {
    self.changed.store(true, Ordering::SeqCst);
    self.hub.notify_one();
    std::future::ready(())
  }
}

/// How a call failed before the server answered it.
fn service_error(server: &str, e: &ServiceError) -> String {
  match e {
    ServiceError::TransportClosed => format!(
      "MCP server {server} stopped during the call (it starts again on next use); \
       the call may or may not have taken effect"
    ),
    ServiceError::Timeout { timeout } => format!(
      "MCP server {server} did not answer within {}s; the call was cancelled",
      timeout.as_secs()
    ),
    ServiceError::McpError(e) => format!("MCP server {server}: {} ({})", e.message, e.code.0),
    ServiceError::TransportSend(e) => format!("MCP server {server}: {}", causes(&*e.error)),
    other => format!("MCP server {server}: {other}"),
  }
}

/// Why a server did not start, without rmcp's transport type names: what
/// was being done, then each cause (`connection refused`).
fn start_error(e: &rmcp::service::ClientInitializeError) -> String {
  match e {
    rmcp::service::ClientInitializeError::TransportError { error, context } => {
      format!("{context}: {}", causes(&*error.error))
    }
    other => other.to_string(),
  }
}

/// An error and its sources, one after the other. rmcp's HTTP transport
/// keeps reqwest's error without its sources (`connection refused`), so
/// that one is looked into.
fn causes(e: &(dyn std::error::Error + 'static)) -> String {
  use rmcp::transport::streamable_http_client::StreamableHttpError;
  use rmcp::transport::worker::WorkerQuitReason;
  type Http = StreamableHttpError<reqwest::Error>;
  let http =
    e.downcast_ref::<Http>()
      .or_else(|| match e.downcast_ref::<WorkerQuitReason<Http>>() {
        Some(WorkerQuitReason::Fatal { error, .. }) => Some(error),
        _ => None,
      });
  if let Some(StreamableHttpError::Client(inner)) = http {
    return causes(inner);
  }
  let mut out = e.to_string();
  let mut source = e.source();
  while let Some(s) = source {
    let s_text = s.to_string();
    if !out.contains(&s_text) {
      out.push_str(": ");
      out.push_str(&s_text);
    }
    source = s.source();
  }
  out
}

/// Cancels a request when the call waiting for it is dropped (the user
/// interrupted it, its thread was cancelled).
struct CancelOnDrop {
  peer: Peer<RoleClient>,
  id: Option<RequestId>,
}

impl Drop for CancelOnDrop {
  fn drop(&mut self) {
    let Some(id) = self.id.take() else { return };
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
      return;
    };
    let peer = self.peer.clone();
    rt.spawn(async move {
      let n = CancelledNotification::new(CancelledNotificationParam::new(
        Some(id),
        Some("interrupted".into()),
      ));
      let _ = peer
        .send_notification(ClientNotification::CancelledNotification(n))
        .await;
    });
  }
}

impl Server {
  fn timeout(&self) -> Duration {
    Duration::from_secs(
      self
        .settings
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT)
        .clamp(1, MAX_TIMEOUT),
    )
  }

  fn startup_timeout(&self) -> Duration {
    Duration::from_secs(
      self
        .settings
        .startup_timeout_secs
        .unwrap_or(DEFAULT_STARTUP)
        .max(1),
    )
  }

  /// Whether `tool` (unprefixed) is offered at all.
  fn offers(&self, tool: &str) -> bool {
    self.settings.tools.is_empty() || self.settings.tools.iter().any(|p| glob(p, tool))
  }

  /// Whether a call of `tool` runs without asking (yolo aside).
  pub fn allows(&self, tool: &str) -> bool {
    self.settings.approval.as_deref() == Some("allow")
      || self.settings.allow.iter().any(|p| glob(p, tool))
  }

  async fn close(&self) {
    let conn = {
      let mut st = self.state.lock().await;
      st.stopped = true;
      st.conn.take()
    };
    if let Some(conn) = conn {
      conn.close().await;
    }
  }

  /// The connection, started (again) when there is none.
  async fn connection(&self) -> Result<Arc<Conn>, String> {
    let mut st = self.state.lock().await;
    if st.stopped || self.closed.load(Ordering::SeqCst) {
      return Err(format!("MCP server {} was stopped", self.name));
    }
    if let Some(c) = &st.conn {
      if c.alive() {
        return Ok(c.clone());
      }
      eprintln!("mcp {}: the server stopped; starting it again", self.name);
      if let Some(old) = st.conn.take() {
        old.close().await;
      }
    }
    if let Some(at) = st.retry_at
      && Instant::now() < at
    {
      return Err(format!(
        "MCP server {} is not running ({}); next try in {}s",
        self.name,
        st.error.as_deref().unwrap_or("it failed"),
        (at - Instant::now()).as_secs() + 1
      ));
    }
    let started = match tokio::time::timeout(self.startup_timeout(), self.start()).await {
      Ok(r) => r,
      Err(_) => Err(format!(
        "it did not start within {}s",
        self.startup_timeout().as_secs()
      )),
    };
    match started {
      Ok(conn) => {
        let conn = Arc::new(conn);
        if st.failures > 0 {
          eprintln!(
            "mcp {}: running again after {} failed starts",
            self.name, st.failures
          );
        }
        st.failures = 0;
        st.retry_at = None;
        st.error = None;
        st.conn = Some(conn.clone());
        st.listing = None;
        st.list_failures = 0;
        st.list_retry_at = None;
        // Listed again on the new connection.
        conn.changed.store(true, Ordering::SeqCst);
        Ok(conn)
      }
      Err(e) => {
        st.failures += 1;
        st.retry_at = Some(Instant::now() + backoff(st.failures));
        // Once while it stays down the same way, not at every try.
        if st.error.as_ref() != Some(&e) {
          eprintln!("mcp {}: {e}", self.name);
        }
        st.error = Some(e.clone());
        Err(format!("MCP server {} did not start: {e}", self.name))
      }
    }
  }

  /// Starts the server and initializes the session.
  async fn start(&self) -> Result<Conn, String> {
    let changed = Arc::new(AtomicBool::new(true));
    let handler = Handler {
      changed: changed.clone(),
      hub: self.tools_changed.clone(),
    };
    // `key = "github"` or `key = "secret:github"`: the global secret of
    // that name (a key stored before secrets were, from wherever it is).
    let key = match &self.settings.key {
      Some(k) => {
        let k = k.strip_prefix("secret:").unwrap_or(k);
        Some(
          self
            .vault
            .secret("global", k)
            .or_else(|| crate::keys::get(&self.data, k, "mcp"))
            .ok_or_else(|| format!("no secret `{k}`: store it with `stitch secret set {k}`"))?,
        )
      }
      None => None,
    };
    let mut child = None;
    let service = match (&self.settings.command, &self.settings.url) {
      (Some(command), None) => {
        let mut process = self.spawn(command, key)?;
        let (Some(stdout), Some(stdin)) = (process.stdout.take(), process.stdin.take()) else {
          return Err("its standard input and output were not set up".into());
        };
        // Guarded from here: a start that fails or times out kills it.
        let group = Group(Some(process));
        let service = handler
          .serve((stdout, stdin))
          .await
          .map_err(|e| start_error(&e))?;
        child = Some(group);
        service
      }
      (None, Some(url)) => {
        use rmcp::transport::StreamableHttpClientTransport;
        use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
        let mut headers = std::collections::HashMap::new();
        for (k, v) in &self.settings.headers {
          let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .map_err(|e| format!("header {k}: {e}"))?;
          let value =
            reqwest::header::HeaderValue::from_str(v).map_err(|e| format!("header {k}: {e}"))?;
          headers.insert(name, value);
        }
        let mut config =
          StreamableHttpClientTransportConfig::with_uri(url.as_str()).custom_headers(headers);
        if let Some(key) = key {
          config = config.auth_header(key);
        }
        let transport = StreamableHttpClientTransport::with_client(http_client(), config);
        handler
          .serve(transport)
          .await
          .map_err(|e| start_error(&e))?
      }
      _ => return Err("give it either `command` (stdio) or `url` (streamable HTTP)".into()),
    };
    Ok(Conn::new(service, child, changed))
  }

  /// A stdio server's process: spawned (the command resolved to a path,
  /// so it is `posix_spawn`ed, never forked), leading its own process
  /// group, its stderr into the node's log.
  fn spawn(&self, command: &str, key: Option<String>) -> Result<tokio::process::Child, String> {
    use std::process::Stdio;
    // `secret:<name>` and `var:<name>` given as their values (global ones:
    // a server serves every place).
    let mut env = BTreeMap::new();
    for (k, v) in &self.settings.env {
      let value = match v.strip_prefix("secret:") {
        Some(name) => self
          .vault
          .secret("global", name)
          .or_else(|| crate::keys::get(&self.data, name, "mcp"))
          .ok_or_else(|| format!("env {k}: no secret `{name}`"))?,
        None => self
          .vault
          .resolve("global", v)
          .map_err(|e| format!("env {k}: {e}"))?,
      };
      env.insert(k.clone(), value);
    }
    if let Some(key) = key {
      let var = self
        .settings
        .key_env
        .clone()
        .ok_or("`key` for a stdio server needs `key_env`, the variable to put it in")?;
      env.insert(var, key);
    }
    let cwd = match &self.settings.cwd {
      Some(d) => expand_home(d),
      None => std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/")),
    };
    let program = resolve_program(command, env.get("PATH").map(String::as_str), &cwd)?;
    let mut cmd = tokio::process::Command::new(&program);
    cmd
      .args(&self.settings.args)
      .envs(&env)
      .current_dir(&cwd)
      .process_group(0)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped())
      .kill_on_drop(true);
    let mut child = cmd
      .spawn()
      .map_err(|e| format!("{}: {e}", program.display()))?;
    if let Some(stderr) = child.stderr.take() {
      let name = self.name.clone();
      tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          eprintln!("mcp {name}: {line}");
        }
      });
    }
    Ok(child)
  }

  /// The server's tools offered, listed again after it said they changed
  /// or it started again; the last known ones while it is down, or while
  /// listing them failed and has not been tried again yet (the start's
  /// backoff). The listing is shared and waited for without locking the
  /// server, so a server slow to list stalls no call.
  pub async fn tools(&self) -> Result<Vec<Tool>, String> {
    let conn = match self.connection().await {
      Ok(c) => c,
      Err(e) => return self.state.lock().await.tools.clone().ok_or(e),
    };
    let listing = {
      let mut st = self.state.lock().await;
      if let Some(tools) = &st.tools
        && !conn.changed.load(Ordering::SeqCst)
      {
        return Ok(tools.clone());
      }
      if let Some(at) = st.list_retry_at
        && Instant::now() < at
      {
        let e = format!(
          "MCP server {} did not list its tools ({}); next try in {}s",
          self.name,
          st.list_error.as_deref().unwrap_or("it failed"),
          (at - Instant::now()).as_secs() + 1
        );
        return st.tools.clone().ok_or(e);
      }
      match &st.listing {
        Some(l) => l.clone(),
        None => {
          conn.changed.store(false, Ordering::SeqCst);
          let (peer, wait, name) = (conn.peer.clone(), self.startup_timeout(), self.name.clone());
          let l: Listing = futures_util::FutureExt::shared(Box::pin(async move {
            match tokio::time::timeout(wait, peer.list_all_tools()).await {
              Ok(Ok(tools)) => Ok(tools),
              Ok(Err(e)) => Err(service_error(&name, &e)),
              Err(_) => Err(format!("MCP server {name} did not list its tools in time")),
            }
          }) as std::pin::Pin<Box<_>>);
          st.listing = Some(l.clone());
          l
        }
      }
    };
    let listed = listing.clone().await;
    let mut st = self.state.lock().await;
    // The first caller back records what came of it.
    let first = st.listing.as_ref().is_some_and(|l| l.ptr_eq(&listing));
    match listed {
      Ok(listed) => {
        let tools: Vec<Tool> = listed
          .into_iter()
          .filter(|t| self.offers(&t.name))
          .collect();
        if first {
          st.listing = None;
          st.tools = Some(tools.clone());
          st.list_failures = 0;
          st.list_retry_at = None;
          st.list_error = None;
        }
        Ok(tools)
      }
      Err(e) => {
        if first {
          st.listing = None;
          conn.changed.store(true, Ordering::SeqCst);
          st.list_failures += 1;
          st.list_retry_at = Some(Instant::now() + backoff(st.list_failures));
          if st.list_error.as_ref() != Some(&e) {
            eprintln!("mcp {}: {e}", self.name);
          }
          st.list_error = Some(e.clone());
        }
        st.tools.clone().ok_or(e)
      }
    }
  }

  /// Calls `tool` (unprefixed); the result as JSON (`CallToolResult`).
  pub async fn call(&self, tool: &str, arguments: Option<Value>) -> Result<Value, String> {
    let conn = self.connection().await?;
    let mut params = CallToolRequestParams::new(tool.to_owned());
    match arguments {
      None | Some(Value::Null) => {}
      Some(Value::Object(map)) => params.arguments = Some(map),
      Some(_) => return Err("the arguments must be an object".into()),
    }
    let peer = conn.peer.clone();
    let handle = peer
      .send_cancellable_request(
        ClientRequest::CallToolRequest(CallToolRequest::new(params)),
        PeerRequestOptions::with_timeout(self.timeout()),
      )
      .await
      .map_err(|e| service_error(&self.name, &e))?;
    let mut guard = CancelOnDrop {
      peer,
      id: Some(handle.id.clone()),
    };
    let answer = handle.await_response().await;
    // Answered, or cancelled already on a timeout.
    guard.id = None;
    match answer {
      Ok(ServerResult::CallToolResult(r)) => serde_json::to_value(r).map_err(|e| e.to_string()),
      Ok(_) => Err(format!(
        "MCP server {}: an answer Stitch does not support (a task, or a request for input)",
        self.name
      )),
      Err(e) => Err(service_error(&self.name, &e)),
    }
  }

  /// Whether the node stopped it (removed, or the node stopping): it
  /// never starts again.
  async fn is_stopped(&self) -> bool {
    self.closed.load(Ordering::SeqCst) || self.state.lock().await.stopped
  }

  /// When the server, down after failing to start or to list its tools,
  /// is tried again (on its next use); `None` when it is not failing.
  pub async fn retry_at(&self) -> Option<Instant> {
    let st = self.state.lock().await;
    let running = st.conn.as_ref().is_some_and(|c| c.alive());
    if running {
      st.list_retry_at
    } else {
      st.retry_at.filter(|_| st.error.is_some())
    }
  }

  /// What the server is doing, for `stitch mcp`.
  pub async fn status(&self) -> String {
    let st = self.state.lock().await;
    match (&st.conn, &st.error) {
      (Some(c), _) if c.alive() => "running".into(),
      (_, Some(e)) => format!("failed: {e}"),
      _ => "not started".into(),
    }
  }
}

/// The HTTP client for streamable HTTP servers.
fn http_client() -> reqwest::Client {
  // Errors when a provider is installed already, which is fine.
  let _ = rustls::crypto::ring::default_provider().install_default();
  reqwest::Client::builder()
    .user_agent(concat!("stitch/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(Duration::from_secs(20))
    .build()
    .expect("an HTTP client with default settings builds")
}

/// `command` as a path: as given when it has a `/` (`~` expanded, relative
/// to `cwd`), else the first executable of that name on `PATH` (the
/// server's own, else the node's).
fn resolve_program(command: &str, path: Option<&str>, cwd: &Path) -> Result<PathBuf, String> {
  if command.contains('/') {
    let p = expand_home(command);
    return Ok(if p.is_absolute() { p } else { cwd.join(p) });
  }
  let path = path
    .map(str::to_owned)
    .or_else(|| std::env::var("PATH").ok())
    .unwrap_or_default();
  for dir in std::env::split_paths(&path) {
    let candidate = dir.join(command);
    if is_executable(&candidate) {
      return Ok(candidate);
    }
  }
  Err(format!(
    "`{command}` is not on the node's PATH: give its full path, or PATH in `env`"
  ))
}

fn is_executable(p: &Path) -> bool {
  use std::os::unix::fs::PermissionsExt;
  std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A server another of the user's nodes shares (`share = true` there,
/// design §2.14), as that node's `capabilities` record lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteServer {
  pub node: String,
  pub server: String,
  pub tools: Vec<RemoteTool>,
}

/// One of its tools: its own name, description and input schema (JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTool {
  pub name: String,
  pub description: String,
  pub schema: String,
}

/// Runs a tool of a server another node shares, there, or queues the call
/// for when that node is back (the node implements it, `crate::remote`).
pub trait Remote: Send + Sync {
  fn call<'a>(&'a self, thread: &'a RecordId, call: RemoteCall<'a>) -> BoxFuture<'a, ToolOutcome>;
  fn queue<'a>(&'a self, thread: &'a RecordId, call: RemoteCall<'a>) -> BoxFuture<'a, ToolOutcome>;
}

/// A call of a tool on another node.
#[derive(Clone, Copy, Debug)]
pub struct RemoteCall<'a> {
  pub node: &'a str,
  pub server: &'a str,
  /// The tool's own name, unprefixed.
  pub tool: &'a str,
  /// Its name as the model sees it.
  pub name: &'a str,
  pub input: &'a Value,
}

/// Other nodes' servers, as a loop sees them.
#[derive(Clone, Default)]
pub struct Others {
  /// Shared with this node: the first node by name, for a server name
  /// not configured here.
  pub shared: Vec<RemoteServer>,
  /// Configured on another node without sharing: (node, server).
  pub unshared: Vec<(String, String)>,
  pub remote: Option<Arc<dyn Remote>>,
}

/// The MCP tools of one loop: what its thread enables, listed before each
/// model call.
#[derive(Default)]
pub struct McpTools {
  /// The servers some entry may turn a tool of on.
  enabled: Vec<Arc<Server>>,
  /// The thread's tool entries, every layer in order (`crate::toolset`).
  entries: Vec<Entry>,
  /// Every server configured here, for saying why a tool is not offered.
  configured: Vec<String>,
  /// Every call runs without asking (yolo).
  pub yolo: bool,
  /// The thread is read-only: calls that would ask are refused.
  pub read_only: bool,
  /// Yolo turned on, tools granted in the thread's place since
  /// (`Rules::live`).
  pub live: crate::rules::Live,
  pub approver: Option<Approver>,
  /// Where images and long text go.
  scratch: PathBuf,
  listed: Mutex<Vec<Listed>>,
  others: Others,
  /// The hazard guard (design §2.28): the user's own threads, judging in
  /// auto mode.
  pub(crate) hazard: Option<Arc<crate::hazard::Guard>>,
}

#[derive(Clone)]
enum Target {
  Here(Arc<Server>),
  /// A server another node shares: (node, server).
  There(String, String),
}

#[derive(Clone)]
struct Listed {
  def: ToolDef,
  target: Target,
  tool: String,
  /// A stand-in for a server that could not be reached (its tools never
  /// listed), with why: `<server>__unavailable`, which tries again.
  down: Option<String>,
}

/// The stand-in tool of a server that is down.
const UNAVAILABLE: &str = "unavailable";

/// What `queue_call` takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueInput {
  tool: String,
  #[serde(default)]
  arguments: Value,
}

/// Whether "always" answers allowed `server`'s `tool` in the thread's place.
fn allowed_in(granted: &crate::rules::Granted, server: &Server, tool: &str) -> bool {
  granted
    .mcp
    .get(&server.name)
    .is_some_and(|allow| allow.iter().any(|p| glob(p, tool)))
}

impl McpTools {
  /// The tools `entries` turn on among `hub`'s servers (the last entry
  /// matching a tool deciding, `crate::toolset`).
  pub fn new(hub: &Mcp, entries: &[Entry], scratch: PathBuf) -> McpTools {
    let servers = hub.servers();
    let enabled = servers
      .iter()
      .filter(|s| {
        entries
          .iter()
          .any(|e| e.on && may_match_server(&e.pattern, &s.name))
      })
      .cloned()
      .collect();
    McpTools {
      enabled,
      entries: entries.to_vec(),
      configured: servers.iter().map(|s| s.name.clone()).collect(),
      yolo: false,
      read_only: false,
      live: crate::rules::Live::default(),
      approver: None,
      scratch,
      listed: Mutex::new(Vec::new()),
      others: Others::default(),
      hazard: None,
    }
  }

  /// With other nodes' servers: those shared that are not configured here
  /// are offered as theirs, their calls run there.
  pub fn with_others(mut self, mut others: Others) -> McpTools {
    others
      .shared
      .retain(|r| !self.configured.contains(&r.server));
    others
      .shared
      .sort_by(|a, b| (&a.node, &a.server).cmp(&(&b.node, &b.server)));
    let mut seen: Vec<String> = Vec::new();
    others.shared.retain(|r| {
      let first = !seen.contains(&r.server);
      seen.push(r.server.clone());
      first
    });
    self.others = others;
    self
  }

  pub fn is_empty(&self) -> bool {
    self.enabled.is_empty()
  }

  /// Whether a tool of `server` named `name` is on.
  fn on(&self, server: &str, name: &str) -> bool {
    decide(&self.entries, false, |p| matches_tool(p, server, name))
  }

  /// Starts what is not running and lists the tools (cached between
  /// calls), then the tools other nodes share.
  pub async fn prepare(&self) {
    let mut listed: Vec<Listed> = Vec::new();
    // Servers start side by side: one slow to start delays the step once.
    let all = futures_util::future::join_all(self.enabled.iter().map(|s| s.tools())).await;
    for (server, tools) in self.enabled.iter().zip(all) {
      let tools = match tools {
        Ok(tools) => tools,
        // Not left out silently: the thread is told, by a stand-in that
        // says why and tries again (issue a2vf). Not for a server the
        // node stopped.
        Err(e) => {
          if !server.is_stopped().await
            && let Some(name) = exposed_name(&server.name, UNAVAILABLE)
          {
            listed.push(Listed {
              def: unavailable_definition(&server.name, &name, &e),
              target: Target::Here(server.clone()),
              tool: String::new(),
              down: Some(e),
            });
          }
          continue;
        }
      };
      for t in tools {
        let Some(name) = exposed_name(&server.name, &t.name) else {
          eprintln!(
            "mcp {}: tool `{}` left out: its name is too long",
            server.name, t.name
          );
          continue;
        };
        if !self.on(&server.name, &name) {
          continue;
        }
        if listed.iter().any(|l| l.def.name == name) {
          eprintln!(
            "mcp {}: tool `{}` left out: {name} is taken",
            server.name, t.name
          );
          continue;
        }
        listed.push(Listed {
          def: definition(&server.name, &name, &t),
          target: Target::Here(server.clone()),
          tool: t.name.to_string(),
          down: None,
        });
      }
    }
    if self.others.remote.is_some() {
      for r in &self.others.shared {
        for t in &r.tools {
          let Some(name) = exposed_name(&r.server, &t.name) else {
            continue;
          };
          if !self.on(&r.server, &name) || listed.iter().any(|l| l.def.name == name) {
            continue;
          }
          listed.push(Listed {
            def: remote_definition(r, &name, t),
            target: Target::There(r.node.clone(), r.server.clone()),
            tool: t.name.clone(),
            down: None,
          });
        }
      }
    }
    listed.sort_by(|a, b| a.def.name.cmp(&b.def.name));
    *self.listed.lock().expect("mcp lock") = listed;
  }

  /// Whether some tool listed runs on another node.
  fn any_remote(&self) -> bool {
    self
      .listed
      .lock()
      .expect("mcp lock")
      .iter()
      .any(|l| matches!(l.target, Target::There(..)))
  }

  pub fn definitions(&self) -> Vec<ToolDef> {
    let mut out: Vec<ToolDef> = self
      .listed
      .lock()
      .expect("mcp lock")
      .iter()
      .map(|l| l.def.clone())
      .collect();
    if self.any_remote() {
      out.push(ToolDef {
        name: "queue_call".into(),
        description: "Queue a call of a tool that runs on another of the user's machines \
          (its description says which) for when that machine is back, when a call failed \
          because it could not be reached. Its result comes to this thread as a report, \
          which wakes it."
          .into(),
        input_schema: json!({
          "type": "object", "additionalProperties": false,
          "properties": {
            "tool": {"type": "string", "description": "The tool's name, as you call it (`mail__search`)."},
            "arguments": {"type": "object", "description": "Its arguments, as you would call it with."}
          },
          "required": ["tool"]
        }),
      });
    }
    out
  }

  /// Why `name`, an MCP tool's name, is not one of these tools, when its
  /// server is configured here or on another node.
  pub fn why_not(&self, thread: &RecordId, name: &str) -> Option<String> {
    let (server, _) = name.split_once(SEPARATOR)?;
    let enable = format!("{server}__*");
    let turn_on = crate::toolset::how_to_change(
      thread,
      if crate::toolset::group_named(server).is_some() {
        &enable
      } else {
        server
      },
      true,
    );
    if let Some(e) = self.down_error(server) {
      return Some(format!(
        "{name} is unavailable: MCP server {server} is enabled for this thread but could not \
         be reached, so its tools are missing: {e}. Tell the user it is down (`stitch mcp` on \
         this node shows it); {server}{SEPARATOR}{UNAVAILABLE} tries it again."
      ));
    }
    if self.configured.iter().any(|s| s == server) {
      return Some(format!(
        "{name} is not enabled for this thread: MCP server {server} is on this node, but this \
         thread's tool set leaves it out (or {name} out, or the server does not offer it). The \
         user can turn it on with {turn_on}."
      ));
    }
    if let Some(r) = self.others.shared.iter().find(|r| r.server == server) {
      return Some(format!(
        "{name} is not enabled for this thread: MCP server {server} runs on {}, which shares \
         it, but this thread's tool set leaves it out (or {name} out, or the server does not \
         offer it). The user can turn it on with {turn_on}.",
        r.node
      ));
    }
    let on: Vec<&str> = self
      .others
      .unshared
      .iter()
      .filter(|(_, s)| s == server)
      .map(|(n, _)| n.as_str())
      .collect();
    (!on.is_empty()).then(|| {
      format!(
        "{name} is unavailable here: MCP server {server} is on {}, which does not share it \
         with the user's other nodes (`share = true` in its [mcp.servers.{server}] there would).",
        on.join(", ")
      )
    })
  }

  /// Whether `name` is one of these tools.
  pub fn has(&self, name: &str) -> bool {
    (name == "queue_call" && self.any_remote())
      || self
        .listed
        .lock()
        .expect("mcp lock")
        .iter()
        .any(|l| l.def.name == name)
  }

  /// The name the model sees for `tool` (unprefixed, exactly as the
  /// server names it) of this node's `server`, when it is listed.
  pub fn name_of(&self, server: &str, tool: &str) -> Option<String> {
    self
      .listed
      .lock()
      .expect("mcp lock")
      .iter()
      .find(|l| l.tool == tool && matches!(&l.target, Target::Here(s) if s.name == server))
      .map(|l| l.def.name.clone())
  }

  /// Whether tool `name` is one of this node's own servers' (not another
  /// node's shared one, nor `queue_call`).
  pub fn is_here(&self, name: &str) -> bool {
    self
      .find(name)
      .is_some_and(|l| matches!(l.target, Target::Here(_)))
  }

  /// Why `server`'s tools are missing, when it could not be reached.
  fn down_error(&self, server: &str) -> Option<String> {
    self
      .listed
      .lock()
      .expect("mcp lock")
      .iter()
      .find(|l| matches!(&l.target, Target::Here(s) if s.name == server))
      .and_then(|l| l.down.clone())
  }

  fn find(&self, name: &str) -> Option<Listed> {
    self
      .listed
      .lock()
      .expect("mcp lock")
      .iter()
      .find(|l| l.def.name == name)
      .cloned()
  }

  pub fn call<'a>(
    &'a self,
    thread: &'a RecordId,
    name: &'a str,
    input: &'a Value,
  ) -> BoxFuture<'a, ToolOutcome> {
    Box::pin(async move {
      if name == "queue_call" {
        return self.queue(thread, input).await;
      }
      let Some(l) = self.find(name) else {
        return ToolOutcome::err(format!("no tool {name}"));
      };
      let server = match &l.target {
        Target::Here(server) if l.down.is_some() => {
          // The stand-in: tries again, asking nothing.
          return match server.tools().await {
            Ok(tools) => ToolOutcome::ok(format!(
              "MCP server {} is back: its tools ({}) are offered from the next step on.",
              server.name,
              tools.len()
            )),
            Err(e) => ToolOutcome::err(format!(
              "{e}. Its tools stay missing until it is back: tell the user MCP server {} is \
               down (`stitch mcp` on this node shows it).",
              server.name
            )),
          };
        }
        Target::Here(server) => server,
        Target::There(node, server) => {
          let Some(remote) = &self.others.remote else {
            return ToolOutcome::err(format!(
              "{name} runs on {node}, which this loop cannot reach"
            ));
          };
          let call = RemoteCall {
            node,
            server,
            tool: &l.tool,
            name,
            input,
          };
          return remote.call(thread, call).await;
        }
      };
      let granted = self.live.get();
      let yolo = (self.yolo || granted.yolo) && !self.read_only && !granted.read_only;
      let allowed = yolo || server.allows(&l.tool) || allowed_in(&granted, server, &l.tool);
      // The hazard guard (design §2.28, auto mode): a call that would run
      // may ask; without a verdict, it does not run (a name that only reads
      // needs none).
      let verdict = match &self.hazard {
        Some(g) if !self.read_only && !granted.read_only => {
          g.check(crate::hazard::Action::Tool {
            name: name.to_owned(),
            input: input.clone(),
          })
          .await
        }
        _ => None,
      };
      if let Some(v) = &verdict {
        crate::hazard::record(v);
      }
      if allowed && let Some(v) = verdict.as_ref().filter(|v| v.fails()) {
        return ToolOutcome::err(crate::hazard::unavailable(v));
      }
      let guarded = allowed && verdict.as_ref().is_some_and(crate::hazard::Verdict::asks);
      let stopped = verdict
        .as_ref()
        .filter(|_| guarded)
        .map(crate::hazard::declined)
        .unwrap_or_default();
      let approved = if allowed && !guarded {
        false
      } else {
        let Some(approver) = &self.approver else {
          return ToolOutcome::err(format!(
            "this call needs the user's approval, and nobody is here to give it{stopped}"
          ));
        };
        if self.read_only || granted.read_only {
          return ToolOutcome::err(format!(
            "this thread is read-only: {name} is not among the calls its server's `allow` lets \
             run, so it is refused, never asked"
          ));
        }
        let live = self.live.clone();
        let (here, tool) = (server.clone(), l.tool.clone());
        let decision = approver(ApprovalRequest {
          grant: if guarded {
            Vec::new()
          } else {
            vec![l.tool.clone()]
          },
          grant_server: Some(server.name.clone()),
          allowed_now: (!guarded).then(|| -> Arc<dyn Fn() -> bool + Send + Sync> {
            Arc::new(move || {
              let granted = live.get();
              granted.yolo || allowed_in(&granted, &here, &tool)
            })
          }),
          verdict: verdict.clone(),
          user_only: guarded,
          ..ApprovalRequest::new(
            thread.clone(),
            format!("mcp:{}", server.name),
            crate::hazard::question(
              &format!("{name} {}", one_line_json(input)),
              verdict.as_ref(),
            ),
          )
        })
        .await;
        if !decision.allow {
          return ToolOutcome::err(match decision.reason {
            Some(r) => format!("the user declined this call: {r}{stopped}"),
            None => format!("the user declined this call{stopped}"),
          });
        }
        true
      };
      let mut out = match server.call(&l.tool, Some(input.clone())).await {
        Ok(result) => outcome(&result, &self.scratch),
        Err(e) => ToolOutcome::err(e),
      };
      if approved {
        out.output = format!("(approved by the user)\n{}", out.output);
      }
      out
    })
  }

  /// `queue_call`: a call of a tool another node runs, for when it is back.
  async fn queue(&self, thread: &RecordId, input: &Value) -> ToolOutcome {
    let q: QueueInput = match serde_json::from_value(input.clone()) {
      Ok(q) => q,
      Err(e) => return ToolOutcome::err(e.to_string()),
    };
    let found = self.find(&q.tool);
    let (Some(l), Some(remote)) = (found, &self.others.remote) else {
      return ToolOutcome::err(format!(
        "{} is not a tool of this thread that runs on another node",
        q.tool
      ));
    };
    let Target::There(node, server) = &l.target else {
      return ToolOutcome::err(format!("{} runs on this node: call it directly", q.tool));
    };
    let arguments = if q.arguments.is_null() {
      json!({})
    } else {
      q.arguments
    };
    let call = RemoteCall {
      node,
      server,
      tool: &l.tool,
      name: &q.tool,
      input: &arguments,
    };
    remote.queue(thread, call).await
  }
}

/// A tool another node shares, as the model sees it: said to run there.
fn remote_definition(r: &RemoteServer, name: &str, t: &RemoteTool) -> ToolDef {
  let mut description = format!("MCP server {} on {}: ", r.server, r.node);
  description.push_str(if t.description.trim().is_empty() {
    &t.name
  } else {
    t.description.trim()
  });
  if description.chars().count() > MAX_DESCRIPTION {
    description = format!(
      "{}…",
      description
        .chars()
        .take(MAX_DESCRIPTION)
        .collect::<String>()
    );
  }
  let mut schema: Value = serde_json::from_str(&t.schema).unwrap_or_else(|_| json!({}));
  if let Value::Object(m) = &mut schema
    && !m.contains_key("type")
  {
    m.insert("type".into(), json!("object"));
  }
  ToolDef {
    name: name.to_owned(),
    description,
    input_schema: schema,
  }
}

/// Arguments as shown for approval: compact, cut.
pub(crate) fn one_line_json(v: &Value) -> String {
  let s = v.to_string();
  if s.chars().count() > 2_000 {
    format!("{}…", s.chars().take(2_000).collect::<String>())
  } else {
    s
  }
}

/// The stand-in of a server that could not be reached.
fn unavailable_definition(server: &str, name: &str, error: &str) -> ToolDef {
  let mut description = format!(
    "MCP server {server} is enabled for this thread but could not be reached, so its tools \
     are missing: {error}. Tell the user it is down rather than that its tools do not exist. \
     Call this to try it again: it says why it is still down, or that it is back (its tools \
     then offered from the next step)."
  );
  if description.chars().count() > MAX_DESCRIPTION {
    description = format!(
      "{}…",
      description
        .chars()
        .take(MAX_DESCRIPTION)
        .collect::<String>()
    );
  }
  ToolDef {
    name: name.to_owned(),
    description,
    input_schema: json!({"type": "object", "additionalProperties": false, "properties": {}}),
  }
}

/// A tool as the model sees it.
fn definition(server: &str, name: &str, t: &Tool) -> ToolDef {
  let mut description = format!("MCP server {server}: ");
  match t.description.as_deref() {
    Some(d) if !d.trim().is_empty() => description.push_str(d.trim()),
    _ => description.push_str(t.title.as_deref().unwrap_or(&t.name)),
  }
  if description.chars().count() > MAX_DESCRIPTION {
    description = format!(
      "{}…",
      description
        .chars()
        .take(MAX_DESCRIPTION)
        .collect::<String>()
    );
  }
  let mut schema = Value::Object((*t.input_schema).clone());
  if let Value::Object(m) = &mut schema
    && !m.contains_key("type")
  {
    m.insert("type".into(), json!("object"));
  }
  ToolDef {
    name: name.to_owned(),
    description,
    input_schema: schema,
  }
}

/// A `CallToolResult` (as JSON) as a tool outcome: text, images to the
/// model, the rest named.
pub(crate) fn outcome(result: &Value, scratch: &Path) -> ToolOutcome {
  let mut parts: Vec<String> = Vec::new();
  let mut images = Vec::new();
  let blocks = result
    .get("content")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  for b in &blocks {
    let str_of = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("");
    match str_of("type") {
      "text" => parts.push(str_of("text").to_owned()),
      "image" => {
        let mime = str_of("mimeType");
        match image(str_of("data"), mime, scratch) {
          Ok(path) => {
            parts.push(format!("[image {mime}, shown after the results]"));
            images.push(ToolImage {
              path,
              media_type: mime.to_owned(),
            });
          }
          Err(e) => parts.push(format!("[an image that could not be read: {e}]")),
        }
      }
      "audio" => parts.push(format!("[audio {}: not passed on]", str_of("mimeType"))),
      "resource_link" => parts.push(format!(
        "[resource link: {} {}{}]",
        str_of("uri"),
        str_of("name"),
        b.get("mimeType")
          .and_then(Value::as_str)
          .map(|m| format!(" ({m})"))
          .unwrap_or_default()
      )),
      "resource" => {
        let r = b.get("resource").cloned().unwrap_or(Value::Null);
        let uri = r.get("uri").and_then(Value::as_str).unwrap_or("");
        match r.get("text").and_then(Value::as_str) {
          Some(text) => parts.push(format!("[resource {uri}]\n{text}")),
          None => parts.push(format!(
            "[resource {uri}: {} content, not passed on]",
            r.get("mimeType")
              .and_then(Value::as_str)
              .unwrap_or("binary")
          )),
        }
      }
      other => parts.push(format!("[{other} content: not passed on]")),
    }
  }
  if blocks.is_empty()
    && let Some(s) = result.get("structuredContent")
  {
    parts.push(serde_json::to_string_pretty(s).unwrap_or_default());
  }
  let mut text = cut(parts.join("\n\n"), scratch);
  let failed = result.get("isError").and_then(Value::as_bool) == Some(true);
  if text.is_empty() {
    text = if failed {
      "(failed, saying nothing)"
    } else {
      "(done, nothing returned)"
    }
    .into();
  }
  ToolOutcome {
    ok: !failed,
    output: text,
    images,
    spent: None,
    attachments: Vec::new(),
    blobs: Vec::new(),
    generated: Vec::new(),
  }
}

/// Text past `MAX_CHARS` cut, the whole saved in `scratch` with its path
/// said.
pub(crate) fn cut(text: String, scratch: &Path) -> String {
  if text.chars().count() <= MAX_CHARS {
    return text;
  }
  let saved = save(scratch, text.as_bytes(), "txt").ok();
  let cut: String = text.chars().take(MAX_CHARS).collect();
  match saved {
    Some(p) => format!(
      "{cut}\n\n[… cut at {MAX_CHARS} characters; all of it is in {}]",
      p.display()
    ),
    None => format!("{cut}\n\n[… cut at {MAX_CHARS} characters]"),
  }
}

/// A base64 image written into `scratch`.
fn image(data: &str, mime: &str, scratch: &Path) -> Result<PathBuf, String> {
  use base64::Engine;
  let bytes = base64::engine::general_purpose::STANDARD
    .decode(data.trim())
    .map_err(|e| e.to_string())?;
  let ext = match mime {
    "image/png" => "png",
    "image/jpeg" => "jpg",
    "image/gif" => "gif",
    "image/webp" => "webp",
    other => return Err(format!("{other} is not an image type the models take")),
  };
  save(scratch, &bytes, ext)
}

/// Writes `bytes` as `<hash>.<ext>` in `dir`.
pub(crate) fn save(dir: &Path, bytes: &[u8], ext: &str) -> Result<PathBuf, String> {
  std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
  let path = dir.join(format!("mcp-{}.{ext}", blake3::hash(bytes).to_hex()));
  if !path.exists() {
    let tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    std::fs::write(tmp.path(), bytes).map_err(|e| e.to_string())?;
    tmp.persist(&path).map_err(|e| e.to_string())?;
  }
  Ok(path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn patterns_match_runs_of_characters() {
    assert!(glob("get_*", "get_issue"));
    assert!(glob("*", ""));
    assert!(glob("a*b*c", "aXbYc"));
    assert!(glob("github__*_issue", "github__get_issue"));
    assert!(!glob("get_*", "list_issues"));
    assert!(!glob("a*bc", "abc_"));
    assert!(!glob("exact", "exactly"));
  }

  #[test]
  fn names_are_namespaced_and_bounded() {
    assert_eq!(
      exposed_name("gh", "get.issue/v2").unwrap(),
      "gh__get_issue_v2"
    );
    assert!(exposed_name("gh", "").is_none());
    assert_eq!(exposed_name("gh", &"x".repeat(60)).unwrap().len(), 64);
    assert!(exposed_name("gh", &"x".repeat(61)).is_none());
    assert!(valid_server_name("my-server2"));
    for bad in ["", "My", "a_b", "a.b"] {
      assert!(!valid_server_name(bad), "{bad}");
    }
  }
}
