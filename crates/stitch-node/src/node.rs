//! The running node: the lock, the iroh endpoint, the sync loop and the
//! local socket.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, bind, load_or_create_key};
use stitch_providers::StopReason;
use stitch_records::{Attachment, BlobHash, RecordId, RepoId};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::net::UnixListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::api::{self, Reply, Request};
use crate::chat::{self, ChatContext, ChatEvent, ChatInput, call_summary};
use crate::config::Config;
use crate::place::{self, Place};
use crate::tools::{ApprovalRequest, Approver, Decision, NodeTools, Rules, ShellTools, Shells};
use crate::toolset::{Entry, Selection};
use crate::{Error, Result, VERSION};

/// Unix socket paths are limited to about 104 bytes on macOS.
const MAX_SOCKET_PATH: usize = 100;

/// Threads' working folders in the data directory, one per thread with
/// no place (design §2.17).
pub const THREAD_FOLDERS: &str = "workdirs";

/// How a turn is told of the home directory: on a phone, the app's
/// container, which cannot be written.
/// What a turn is told of the thread it is: its ID, whose node is where
/// it started (a thread runs where it was continued last), and that
/// `threads` marks it. Said in every thread's context, so the same each
/// turn.
pub(crate) fn who_said(thread: &RecordId) -> String {
  format!(
    "\nThis thread is {thread}: that ID is you (the node in an ID is where a thread started, \
     not where it runs). `threads` marks it \"(you)\"; a report or a list naming it names \
     you, not another thread.\n"
  )
}

pub(crate) fn home_label(home: &Path) -> String {
  if stitch_shell::CAN_SPAWN {
    format!("home directory {}", home.display())
  } else {
    format!(
      "app container {} (not writable: not a place to write files)",
      home.display()
    )
  }
}

/// The node's socket: `node.sock` in the data directory, or where that
/// path is too long for a Unix socket, `/tmp/stitch-<uid>/<hash>.sock`, the
/// hash of the data directory's path. That directory must belong to this
/// user and be private to it, so no other user can stand in for the node.
pub fn socket_path(data: &Path) -> Result<PathBuf> {
  let inside = data.join("node.sock");
  if inside.as_os_str().len() <= MAX_SOCKET_PATH {
    return Ok(inside);
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
    let uid = std::fs::metadata(data)
      .map_err(Error::io(data.display()))?
      .uid();
    let dir = PathBuf::from(format!("/tmp/stitch-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
      Ok(()) => {}
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
      Err(e) => return Err(Error::io(dir.display())(e)),
    }
    let meta = std::fs::symlink_metadata(&dir).map_err(Error::io(dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
      return Err(Error::Config(format!(
        "{} is not a private directory of this user; remove it, or use a shorter data directory path",
        dir.display()
      )));
    }
    let canonical = std::fs::canonicalize(data).map_err(Error::io(data.display()))?;
    let hash = blake3::hash(canonical.as_os_str().as_encoded_bytes()).to_hex();
    Ok(dir.join(format!("{}.sock", &hash[..16])))
  }
  #[cfg(not(unix))]
  Err(Error::Config(format!(
    "{}: path too long for a socket",
    inside.display()
  )))
}

/// The longest a stop or restart someone asked for waits for the node to
/// be idle (docs/self-improve.md §2.1, the drain): then it stops what
/// runs. 30 minutes; `STITCH_DRAIN_SECS` for tests. A restart nobody asked
/// for (a new build installed, a newer client) is never cut: it waits for
/// idle, until turns can be held at a step boundary.
pub(crate) fn drain() -> Duration {
  let secs = std::env::var("STITCH_DRAIN_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(30 * 60);
  Duration::from_secs(secs)
}

/// How long a new build's restart waits for idle before the user is told
/// (once: which build, what keeps the node busy, how to restart now). It
/// is never cut for it. 60 minutes; `STITCH_RESTART_NOTICE_SECS` for tests.
pub fn restart_notice() -> Duration {
  let secs = std::env::var("STITCH_RESTART_NOTICE_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(60 * 60);
  Duration::from_secs(secs)
}

/// What a stopped node's process does next: start its successor.
#[derive(Clone, Debug)]
pub struct Restart {
  /// Why, for the log.
  pub why: String,
}

/// `<data>/restarting`: a node stopped to be started again (by its process
/// or its service) writes it before it lets go of the lock, so a client
/// finding no node waits for the successor rather than starting a node of
/// its own (with its own environment and sandbox). It counts for
/// [`RESTARTING_FOR`]; the successor removes it once it serves.
pub fn restarting_marker(data: &Path) -> PathBuf {
  data.join("restarting")
}

/// How long a `restarting` marker holds clients back.
pub const RESTARTING_FOR: Duration = Duration::from_secs(60);

/// Whether a restart's successor is due: the marker, fresh.
pub fn restart_due(data: &Path) -> bool {
  std::fs::metadata(restarting_marker(data))
    .and_then(|m| m.modified())
    .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < RESTARTING_FOR))
}

/// `<data>/cut-shells.json`: what a stopping node cut in threads' shells
/// (design §2.2, issue nqxw), by thread, the note each is told at the next
/// start (then removed: told once).
fn cut_shells_file(data: &Path) -> PathBuf {
  data.join("cut-shells.json")
}

/// Before the shells close as the node stops: each thread whose shells
/// run something gets a note kept for the next start.
async fn keep_cut_shells(s: &Arc<Shared>) {
  let running = s.shells.running_by_thread().await;
  if running.is_empty() {
    return;
  }
  let restart = s.restart.lock().expect("restart lock").clone();
  let pending = s.pending();
  let (verb, why) = match (restart, pending) {
    (Some(r), _) => ("restarted", Some(r.why)),
    (None, why) => ("stopped", why),
  };
  let at = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string();
  let why = why.map(|w| format!(" ({w})")).unwrap_or_default();
  let notes: Vec<serde_json::Value> = running
    .into_iter()
    .map(|(thread, what)| {
      let text = format!(
        "[Stitch: the node {verb}{why} at {at} and cut what ran in your shells: {}; the \
         sessions are gone: start again what you still need.]",
        what.join("; ")
      );
      serde_json::json!({"thread": thread.to_string(), "text": s.vault.redact(&text)})
    })
    .collect();
  let file = cut_shells_file(&s.data);
  // Notes an earlier stop left untold (none started since) are kept too.
  let mut all: Vec<serde_json::Value> = std::fs::read(&file)
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default();
  all.extend(notes);
  match serde_json::to_vec(&all) {
    Ok(b) => {
      if let Err(e) = std::fs::write(&file, b) {
        eprintln!("{}: {e}", file.display());
      }
    }
    Err(e) => eprintln!("{}: {e}", file.display()),
  }
}

/// At start, once threads resume: the notes `keep_cut_shells` left, told
/// as a job's end is (woven, or waking the thread), once.
fn tell_cut_shells(s: &Arc<Shared>) {
  let file = cut_shells_file(&s.data);
  let Ok(bytes) = std::fs::read(&file) else {
    return;
  };
  if let Err(e) = std::fs::remove_file(&file) {
    eprintln!("{}: {e}", file.display());
    return;
  }
  let notes: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap_or_default();
  for n in notes {
    let (Some(thread), Some(text)) = (
      n.get("thread")
        .and_then(serde_json::Value::as_str)
        .and_then(|t| t.parse::<RecordId>().ok()),
      n.get("text").and_then(serde_json::Value::as_str),
    ) else {
      continue;
    };
    eprintln!("thread {thread}: told what the last stop cut in its shells");
    crate::detached::notify(s, &thread, text.to_owned(), Vec::new());
  }
}

/// What the process running the node says of itself
/// (docs/self-improve.md §2.1), set once before the node starts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Supervision {
  /// A service (its systemd unit, its LaunchAgent) runs it and starts it
  /// again when it exits with status 75.
  pub supervised: bool,
}

static SUPERVISION: std::sync::OnceLock<Supervision> = std::sync::OnceLock::new();
type InstalledCheck = Box<dyn Fn() -> bool + Send + Sync>;
static INSTALLED_DIFFERS: std::sync::OnceLock<InstalledCheck> = std::sync::OnceLock::new();

/// Says how this process runs, once (the binary that runs the node).
pub fn set_supervision(s: Supervision) {
  let _ = SUPERVISION.set(s);
}

fn supervision() -> Supervision {
  SUPERVISION.get().copied().unwrap_or_default()
}

/// Says how to tell that another build is installed where this process's
/// binary is (a newer client is then handed over to by a restart into it).
pub fn set_installed_check(f: impl Fn() -> bool + Send + Sync + 'static) {
  let _ = INSTALLED_DIFFERS.set(Box::new(f));
}

fn installed_differs() -> bool {
  INSTALLED_DIFFERS.get().is_some_and(|f| f())
}

pub struct NodeHandle {
  shutdown: watch::Sender<bool>,
  task: JoinHandle<Result<()>>,
  shared: Arc<Shared>,
  /// None on iOS and visionOS: no other process can reach it there.
  pub socket: Option<PathBuf>,
  pub endpoint: String,
}

impl NodeHandle {
  /// For tests in the crate (`apps::ws`'s, `detached`'s).
  #[cfg(test)]
  pub(crate) fn shared(&self) -> &Arc<Shared> {
    &self.shared
  }

  /// A client connected in process (the app), served as one on the
  /// socket is. Run on the node's runtime: the connection's task goes
  /// there.
  pub fn client(
    &self,
  ) -> impl std::future::Future<Output = Result<crate::client::Client>> + Send + 'static {
    let s = self.shared.clone();
    async move {
      let (ours, theirs) = tokio::io::duplex(1 << 16);
      tokio::spawn(async move {
        let (r, w) = tokio::io::split(theirs);
        if let Err(e) = serve(&s, r, w, crate::peer::Peer::App).await {
          eprintln!("client: {e}");
        }
      });
      let (r, w) = tokio::io::split(ours);
      crate::client::Client::greet(Box::new(r), Box::new(w)).await
    }
  }

  /// What an app's origin answers the phone's web view (its URL scheme
  /// handler, `apps::serve`). Run on the node's runtime.
  pub fn serve_app(
    &self,
    req: crate::apps::serve::Request,
  ) -> impl std::future::Future<Output = crate::apps::serve::Response> + Send + 'static {
    let s = self.shared.clone();
    async move { crate::apps::serve::serve(&s, req, crate::apps::serve::Client::App).await }
  }

  /// The notices this node's threads sent since last asked (`notify`):
  /// the phone's app shows them, since a phone cannot push to itself.
  pub fn take_notices(&self) -> Vec<crate::notices::Notice> {
    self.shared.notices.take()
  }

  /// Stops the node and waits for it.
  pub async fn stop(self) -> Result<()> {
    let _ = self.shutdown.send(true);
    self.wait().await
  }

  /// Waits until the node stops (a `Stop` request, or `stop`).
  pub async fn wait(self) -> Result<()> {
    self
      .task
      .await
      .map_err(|e| Error::Protocol(format!("node task: {e}")))?
  }

  /// `wait`, then whether its process should start its successor (a
  /// restart was asked, or a new build installed), and why.
  pub async fn wait_restart(self) -> Result<Option<Restart>> {
    let s = self.shared.clone();
    self.wait().await?;
    Ok(s.restart.lock().expect("restart lock").clone())
  }

  /// What restarts the node once nothing runs, for the reason it is
  /// given: its process then starts its successor (`wait_restart`), as
  /// when its binary was replaced by another build.
  /// Threads at work, chats and calls for other nodes hold it as long as
  /// they run; commands left in shells (a server, a watch) at most
  /// [`drain`], then they are cut (issue ubbm: a dev server kept a node
  /// on its old build for hours).
  /// Still waiting past [`restart_notice`], the user is told once.
  pub fn restart_when_idle_fn(&self) -> impl Fn(String) + Send + Sync + 'static {
    self.restart_when_idle_noticing(drain(), restart_notice())
  }

  /// `restart_when_idle_fn`, commands in shells holding it at most
  /// `shells`.
  pub fn restart_when_idle_within(
    &self,
    shells: Duration,
  ) -> impl Fn(String) + Send + Sync + 'static {
    self.restart_when_idle_noticing(shells, restart_notice())
  }

  /// `restart_when_idle_within`, the user told once when it still waits
  /// after `notice`.
  pub fn restart_when_idle_noticing(
    &self,
    shells: Duration,
    notice: Duration,
  ) -> impl Fn(String) + Send + Sync + 'static {
    let s = self.shared.clone();
    move |why: String| {
      let r = Restart { why: why.clone() };
      s.shells_hold
        .lock()
        .expect("shells hold lock")
        .get_or_insert_with(|| std::time::Instant::now() + shells);
      s.restart_notice
        .lock()
        .expect("restart notice lock")
        .get_or_insert(notice);
      s.stop_when_idle(why, Some(r), false);
    }
  }

  /// A sender that stops the node, for signal handlers.
  pub fn stopper(&self) -> watch::Sender<bool> {
    self.shutdown.clone()
  }

  /// The app went to the background (`false`) or came back: memory's
  /// encoder is unloaded meanwhile (iOS lets no app in the background use
  /// the GPU), and a sleep due waits for the foreground (memory's hourly
  /// tick, which fires again as the app resumes).
  pub fn set_foreground(&self, on: bool) {
    self.shared.memory.suspend(!on);
  }

  /// Tells iroh the network may have changed: the app back in front after
  /// being suspended (its relay connection and paths perhaps gone dead
  /// meanwhile, which iroh does not notice on its own: no link changed),
  /// or a network change iOS told of. Harmless when nothing changed. Run
  /// on the node's runtime.
  pub fn network_change(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
    let shared = self.shared.clone();
    async move {
      // Connections kept from before may be dead: dialed afresh.
      shared.net.forget_connections();
      shared.net.endpoint().network_change().await
    }
  }

  /// What runs that the process going to sleep would cut short (chats,
  /// loops at work, commands, calls for other nodes; not a detached thread
  /// waiting on an approval or a report): an app keeps itself awake for
  /// it. Run on the node's runtime.
  pub fn busy(&self) -> impl std::future::Future<Output = Vec<String>> + Send + 'static {
    let s = self.shared.clone();
    async move { s.busy().await }
  }
}

/// By MCP server, its tools allowed in a place.
type McpGrants = std::collections::BTreeMap<String, Vec<String>>;

pub(crate) struct Shared {
  pub(crate) data: PathBuf,
  pub(crate) config: Config,
  pub(crate) net: Net,
  pub(crate) shutdown: watch::Sender<bool>,
  pub(crate) shells: Arc<Shells>,
  pub(crate) detached: crate::detached::Detachments,
  /// Where projects are checked out here.
  checkouts: Mutex<place::Checkouts>,
  /// The web tools and their browser.
  web: Arc<crate::web::Web>,
  /// The user's browsers, linked through Stitch's extension (§2.18).
  pub(crate) browsers: Arc<crate::browser::Browsers>,
  pub(crate) memory: Arc<crate::memory::Memory>,
  /// The MCP servers, started on first use.
  pub(crate) mcp: Arc<crate::mcp::Mcp>,
  /// Tools written in Otter, compiled once (`crate::otter`).
  pub(crate) scripts: crate::otter::Scripts,
  /// Held while this node's `capabilities` are worked out (`remote`).
  pub(crate) publishing: tokio::sync::Mutex<()>,
  /// The shell's approval mode (`ask`, `allow`: yolo, `off`), switched at
  /// run time and saved.
  approval: Mutex<String>,
  /// Shell prefixes allowed per place (`[shell.places]`), grown by
  /// "always" answers at run time and saved.
  grants: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
  /// MCP tools allowed per place, by server (`[mcp.places]`), grown and
  /// saved likewise.
  mcp_grants: Mutex<std::collections::BTreeMap<String, McpGrants>>,
  /// Otter tools allowed per place (`[otter.places]`), grown and saved
  /// likewise.
  otter_grants: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
  /// Each provider's model list, and when it was asked (`models`).
  pub(crate) models:
    tokio::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<String>)>>,
  /// The models' context windows, as their providers said.
  pub(crate) windows: crate::window::Windows,
  /// Chats running for clients.
  chats: AtomicUsize,
  /// Calls run here for other nodes: asked (`remote::answer`) or queued
  /// (`remote::take`).
  pub(crate) remote_calls: AtomicUsize,
  /// A newer client came while busy, a new build was installed, or a
  /// restart was asked: the node stops once idle.
  upgrading: AtomicBool,
  /// Why it waits to stop, for `stitch node status`.
  pending: Mutex<Option<String>>,
  /// Since when it waits to stop (the stop's first `stop_when_idle`).
  pending_since: Mutex<Option<jiff::Timestamp>>,
  /// After how long a new build's restart still waiting for idle is told
  /// to the user, once ([`restart_notice`]); `None`: never (a restart
  /// for a newer client, a stop asked).
  restart_notice: Mutex<Option<Duration>>,
  /// Since when a stop someone asked for waits: past [`drain`], it cuts
  /// what runs.
  cut: Mutex<Option<std::time::Instant>>,
  /// Until when commands in shells hold a stop that waits for idle (a new
  /// build's restart): past it, only the rest of `busy` does.
  shells_hold: Mutex<Option<std::time::Instant>>,
  /// Whether its process, once stopped, starts its successor (a restart)
  /// or only ends (a newer client starts its own; a service takes over).
  restart: Mutex<Option<Restart>>,
  /// Folder sync's locks and the turns it waits for.
  pub(crate) folder_sync: crate::folders::FolderSync,
  /// A phone's clones of projects, one fetch at a time each.
  pub(crate) clones: crate::clones::Clones,
  /// Push notifications to the user's phones, when this node has a key.
  pub(crate) pusher: crate::apns::Pusher,
  /// What it pushes, shown on this machine's desktop too.
  #[cfg(not(any(target_os = "ios", target_os = "visionos")))]
  pub(crate) toasts: crate::toasts::Toasts,
  /// Apps' state gathered and changed, and the desktop's server of them.
  pub(crate) apps: crate::apps::Apps,
  /// The timer of the schedules pinned here (design §2.3).
  pub(crate) schedules: crate::schedules::Schedules,
  /// Events from outside programs, their asks and the wakes gathering
  /// (docs/events.md).
  pub(crate) events: crate::events::Events,
  /// Loose ends: whether their pass runs (design §2.10).
  pub(crate) loose_ends: crate::loose::LooseEnds,
  /// Decisions: the service's cache (design §2.29).
  pub(crate) decide: crate::decide::Service,
  /// Digests: what runs, what waits, what was tried (design §2.25).
  pub(crate) digests: crate::digests::Digests,
  /// Rooms' rounds running here (design §2.27).
  pub(crate) rooms: crate::rooms::Rooms,
  /// The permits of the list reads (`Shared::list_read`).
  list_reads: tokio::sync::Semaphore,
  /// Notices for the user (`notify`), and those waiting for this node's
  /// own app.
  pub(crate) notices: crate::notices::Notices,
  /// Calls this node rang that wait for the phone's word, and their limits
  /// (`call_user`).
  pub(crate) rings: std::sync::Arc<crate::rings::Rings>,
  /// The secrets' values this node holds and the variables (design
  /// §2.19): for redaction and for shells' environments.
  pub(crate) vault: Arc<crate::secrets::Vault>,
  /// SSH: connections, remote sessions' far ends, this device's key
  /// (design §2.20).
  pub(crate) ssh: crate::ssh::Ssh,
  /// What the loops running here stream, for other nodes to follow
  /// (design §2.6, §2.7 "Continue on another node").
  pub(crate) live: crate::live::Live,
}

/// How many list reads (`Threads`, `Live`, `Transcript`: each walks the
/// index under its lock) run on the blocking pool at once. The rest wait as
/// plain futures, not as threads: a flood of them (a client refreshing on
/// every change while the machine swaps) once filled the pool's 512 threads
/// with waiters on the index lock and starved every tool.
pub(crate) const LIST_READS: usize = 4;

impl Shared {
  /// Runs a list read on the blocking pool, at most `LIST_READS` at a time.
  pub(crate) async fn list_read<T: Send + 'static>(
    self: &Arc<Self>,
    f: impl FnOnce(Arc<Shared>) -> T + Send + 'static,
  ) -> Result<T> {
    let _permit = self
      .list_reads
      .acquire()
      .await
      .map_err(|e| Error::Config(e.to_string()))?;
    let s = self.clone();
    tokio::task::spawn_blocking(move || f(s))
      .await
      .map_err(|e| Error::Config(e.to_string()))
  }

  pub(crate) fn state(&self) -> &Arc<NodeState> {
    self.net.state()
  }

  /// A receiver that changes when the node stops.
  pub(crate) fn stopping(&self) -> watch::Receiver<bool> {
    self.shutdown.subscribe()
  }

  /// What runs that stopping the node would cut short: chats, loops at
  /// work, commands in shells, tool calls run for other nodes. A detached thread waiting on an approval
  /// or on `wait` is not: it resumes when the node starts again.
  pub(crate) async fn busy(&self) -> Vec<String> {
    let (mut out, shells) = self.busy_parts().await;
    out.extend(shells);
    out
  }

  /// `busy`, as what runs in the node (threads, chats, calls) and the
  /// commands running in shells.
  async fn busy_parts(&self) -> (Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let loops: Vec<String> = {
      let steers = self.detached.steers.lock().expect("steers lock");
      let waiting = self.detached.waiting.lock().expect("waiting lock");
      let asking = self.detached.asking.lock().expect("asking lock");
      let index = self.state().index.lock().expect("index lock");
      steers
        .keys()
        .filter(|t| {
          let resumes = (waiting.contains(*t) || asking.contains_key(*t))
            && t.parse::<RecordId>().ok().is_some_and(|id| {
              index.record(&id).ok().flatten().is_some_and(|r| {
                r.fields
                  .get("detached")
                  .and_then(serde_json::Value::as_bool)
                  == Some(true)
              })
            });
          !resumes
        })
        .cloned()
        .collect()
    };
    match loops.len() {
      0 if self.chats.load(Ordering::SeqCst) > 0 => out.push("a chat starting".into()),
      0 => {}
      1 => out.push(format!("thread {} working", loops[0])),
      n => out.push(format!("{n} threads working")),
    }
    match self.remote_calls.load(Ordering::SeqCst) {
      0 => {}
      1 => out.push("a tool call for another node".into()),
      n => out.push(format!("{n} tool calls for other nodes")),
    }
    let shells = self
      .shells
      .listing()
      .await
      .into_iter()
      .filter(|r| r.kind == "shell")
      .map(|r| format!("`{}` running in a shell", r.what))
      .collect();
    (out, shells)
  }

  /// Whether commands in shells no longer hold the stop waiting (past
  /// `shells_hold`), and else for how long they still do.
  fn shells_held(&self) -> Option<Duration> {
    let until = (*self.shells_hold.lock().expect("shells hold lock"))?;
    Some(until.saturating_duration_since(std::time::Instant::now()))
  }

  /// Stops the node now. When its process is to start it again (a
  /// restart), clients are told to wait for the successor first
  /// (`<data>/restarting`), before the lock is let go.
  pub(crate) fn shut_down(&self) {
    if self.restart.lock().expect("restart lock").is_some() {
      let marker = restarting_marker(&self.data);
      if let Err(e) = std::fs::write(&marker, std::process::id().to_string()) {
        eprintln!("{}: {e}", marker.display());
      }
    }
    let _ = self.shutdown.send(true);
  }

  /// Stops the node once nothing runs, for `why`; with `restart`, its
  /// process then starts its successor (`NodeHandle::restart`). With
  /// `cut` (a stop or restart someone asked for), it waits at most
  /// [`drain`], then stops what runs; without, it waits for idle. A second
  /// call while one waits changes only what follows the stop (a restart
  /// asked wins over a plain stop) and whether it may cut.
  pub(crate) fn stop_when_idle(self: &Arc<Self>, why: String, restart: Option<Restart>, cut: bool) {
    if let Some(r) = restart {
      self.restart.lock().expect("restart lock").get_or_insert(r);
    }
    if cut {
      self
        .cut
        .lock()
        .expect("cut lock")
        .get_or_insert_with(std::time::Instant::now);
    }
    if self.upgrading.swap(true, Ordering::SeqCst) {
      return;
    }
    eprintln!("{why} is waiting: stopping once idle");
    *self.pending.lock().expect("pending lock") = Some(why.clone());
    let started = std::time::Instant::now();
    *self.pending_since.lock().expect("pending lock") = Some(jiff::Timestamp::now());
    let s = self.clone();
    let mut stopped = self.shutdown.subscribe();
    let drain = drain();
    tokio::spawn(async move {
      let mut told = false;
      loop {
        tokio::select! {
          _ = tokio::time::sleep(Duration::from_secs(2)) => {}
          _ = stopped.changed() => return,
        }
        let (others, shells) = s.busy_parts().await;
        if others.is_empty() && shells.is_empty() {
          eprintln!("idle: stopping for {why}");
          s.shut_down();
          return;
        }
        // Only commands in shells left, held long enough: a server never
        // ends, and the build waits no longer for it.
        if others.is_empty() && s.shells_held().is_some_and(|d| d.is_zero()) {
          eprintln!(
            "{why}: only commands in shells run, waited for long enough: stopping now, cutting {}",
            shells.join("; ")
          );
          s.shut_down();
          return;
        }
        let busy: Vec<String> = others.into_iter().chain(shells).collect();
        // Asked for, and never idle (threads at work all along, schedules
        // firing): it stops anyway, and detached threads resume where
        // they were.
        let since = *s.cut.lock().expect("cut lock");
        if since.is_some_and(|t| t.elapsed() >= drain) {
          eprintln!(
            "{why} waited {} s: stopping now ({})",
            drain.as_secs(),
            busy.join("; ")
          );
          s.shut_down();
          return;
        }
        // A new build's restart is never cut, but not waited for in
        // silence either: past its notice, the user is told once.
        let notice = *s.restart_notice.lock().expect("restart notice lock");
        if !told && since.is_none() && notice.is_some_and(|n| started.elapsed() >= n) {
          told = true;
          let restart = s.restart.lock().expect("restart lock").clone();
          let what = restart.map_or_else(|| why.clone(), |r| r.why);
          let body = format!(
            "{what}: the node waits to restart into it since {}, busy: {}. It restarts once \
             idle; `stitch node restart --now` restarts it now (running turns resume).",
            s.pending_since().unwrap_or_default(),
            busy.join("; ")
          );
          eprintln!("{what}: still waiting, told the user ({})", busy.join("; "));
          let s2 = s.clone();
          tokio::spawn(async move {
            let title = format!("Stitch on {}: restart waiting", s2.state().node);
            if let Err(e) = crate::notices::send(&s2, None, Some(&title), &body).await {
              eprintln!("restart notice not sent: {e}");
            }
          });
        }
      }
    });
  }

  /// Since when the node waits to stop, in local time.
  fn pending_since(&self) -> Option<String> {
    let t = (*self.pending_since.lock().expect("pending lock"))?;
    Some(
      t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string(),
    )
  }

  /// What `stitch node status` says of a stop it waits to make.
  fn pending(&self) -> Option<String> {
    self.pending.lock().expect("pending lock").clone()
  }

  /// The shell tools for a loop on `root`, starting in `cwd` when given.
  /// The tools for a loop; `client_env`, the environment of the client
  /// it runs for (its PATH, SSH agent, credential helpers), is the
  /// environment of the shell sessions it starts, instead of the node's.
  /// `place` decides what is granted there; a `read_only` thread is
  /// refused what the rules do not allow, never asked.
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn shell_tools(
    self: &Arc<Self>,
    root: &RecordRoot,
    place: &Place,
    read_only: bool,
    thread: Option<&RecordId>,
    cwd: Option<&Path>,
    approver: Approver,
    client_env: Option<&[(String, String)]>,
    selection: &Selection,
  ) -> NodeTools {
    self.shell_tools_with(
      root, place, read_only, thread, cwd, approver, client_env, selection, None,
    )
  }

  /// `shell_tools`, for a read-only room participant in its room's turn
  /// when `room_ask` is given (design §2.27): what the read-only mode
  /// refuses asks the user instead. Its tools are made writable, kept
  /// read-only by `live` until an answer lifts it.
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn shell_tools_with(
    self: &Arc<Self>,
    root: &RecordRoot,
    place: &Place,
    read_only: bool,
    thread: Option<&RecordId>,
    cwd: Option<&Path>,
    approver: Approver,
    client_env: Option<&[(String, String)]>,
    selection: &Selection,
    room_ask: Option<crate::rules::RoomAsk>,
  ) -> NodeTools {
    let room_ask = room_ask.filter(|_| read_only && thread.is_some());
    let read_only = read_only && room_ask.is_none();
    let settings = self.config.shell.clone().unwrap_or_default();
    // An app's thread (design §2.22): every call passes its guard, built
    // below; never yolo, none of the user's own allowances.
    let app = thread.and_then(|t| crate::apps::threads::owner_of(self, t));
    // Read at each check: switched to read-only during the turn, the next
    // call is (switched the other way, the next turn).
    let live = self.live_for(place, thread);
    let live = match &room_ask {
      Some(r) => r.live(live),
      None => live,
    };
    let live = match &app {
      Some(_) => {
        let user = live;
        crate::rules::Live::new(move || crate::rules::Granted {
          read_only: user.get().read_only,
          ..Default::default()
        })
      }
      None => live,
    };
    // A fixer of Stitch (docs/self-improve.md §4): sandboxed where the
    // system has a sandbox, else asked, whatever the node's mode; never
    // yolo; what could take the user's powers always asks (`Rules::fixer`).
    // The harness-issues schedule's triage thread too: a fixer's report
    // wakes it, so it is held as a fixer is.
    let fixer = app.is_none() && thread.is_some_and(|t| crate::selfimprove::in_fixer_mode(self, t));
    let live = crate::fixer::live(fixer, live);
    // A fixer's own worktree and branches, beside its checkout
    // (`fixer::own_worktree`): its children's too, who do its hands-on
    // work there.
    let own_issue = thread
      .filter(|_| fixer)
      .and_then(|t| crate::selfimprove::own_issue(self, t));
    // Its issue's other repositories' checkouts (an app's fix: the core's),
    // whose worktrees pair with its own (docs/self-improve.md §6).
    let paired: Vec<PathBuf> = own_issue
      .as_ref()
      .map(|i| crate::selfimprove::paired_checkouts(self, i, self.place_dir(place).as_deref()))
      .unwrap_or_default();
    let mut rules = Rules {
      // An app's thread has no shell: its mode only keeps it from yolo.
      mode: match &app {
        Some(_) => "ask".into(),
        None if fixer => crate::fixer::mode(),
        None => self.approval_in(place),
      },
      fixer,
      own_issue: own_issue.as_ref().map(|i| i.to_string()),
      room_ask: room_ask.clone(),
      read_only,
      read_only_allow: settings.read_only_allow.clone(),
      live: live.clone(),
      ..Rules::default()
    };
    if app.is_none() {
      rules.allow.extend(settings.allow);
    }
    crate::fixer::confine_rules(fixer, &mut rules);
    let base: Vec<(String, String)> = client_env.map(<[_]>::to_vec).unwrap_or_default();
    let path_of = |env: &[(String, String)]| {
      env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.clone())
    };
    // Commands in the shell reach this node, with this binary first on PATH.
    let mut env: Vec<(String, String)> = base
      .iter()
      .filter(|(k, _)| k != "PATH" && k != "STITCH_DATA")
      .cloned()
      .collect();
    env.push(("STITCH_DATA".to_owned(), self.data.display().to_string()));
    let path = path_of(&base).unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    match std::env::current_exe()
      .ok()
      .and_then(|e| e.parent().map(Path::to_owned))
    {
      Some(dir) => env.push(("PATH".into(), format!("{}:{path}", dir.display()))),
      None => env.push(("PATH".into(), path)),
    }
    // Variables, and what `[env]` maps (config.toml's, then the place's):
    // secrets given by reference, never shown (design §2.19). Read again
    // as each session starts (`place_env`); this reading is the sandbox's.
    let base = env.clone();
    for (k, v) in self.place_env(place, false) {
      env.retain(|(e, _)| *e != k);
      env.push((k, v));
    }
    let place_env = {
      let node = Arc::downgrade(self);
      let place = place.clone();
      crate::tools::PlaceEnv(Arc::new(move || {
        node
          .upgrade()
          .map(|s| s.place_env(&place, true))
          .unwrap_or_default()
      }))
    };
    let yolo = crate::rules::runs_unasked(&rules.mode) && !rules.read_only;
    // An app's thread: where it reads and writes (its place, or a global
    // thread's own folder, never a directory holding the node's data),
    // and its guard, which every call passes.
    let start_dir = cwd
      .map(Path::to_owned)
      .filter(|p| p.is_dir())
      .or_else(|| self.fallback_dir())
      .unwrap_or_else(|| root.dir().to_owned());
    let guard = match (&app, thread) {
      (Some(owner), Some(t)) => {
        let roots: Vec<PathBuf> = match self.place_dir(place) {
          Some(d) if !crate::apps::threads::holds_data(&self.data, &d) => vec![d],
          Some(_) => Vec::new(),
          None if *place == Place::Global && start_dir.starts_with(self.thread_folder(t)) => {
            vec![start_dir.clone()]
          }
          None => Vec::new(),
        };
        let aliases = match (place, self.place_dir(place)) {
          (Place::Folder(key), Some(dir)) if crate::folders::keeps_copies() => {
            place::Aliases::new(self.places().folders.paths(key), &dir)
          }
          _ => place::Aliases::default(),
        };
        Some(Arc::new(crate::apps::guard::Guard::new(
          self,
          owner.clone(),
          t.clone(),
          place.clone(),
          roots,
          selection
            .bound
            .iter()
            .flat_map(|b| {
              b.files
                .iter()
                .filter_map(|f| f.parent().map(Path::to_owned))
            })
            .collect(),
          start_dir.clone(),
          aliases,
        )))
      }
      _ => None,
    };
    let approver = match &guard {
      Some(g) => crate::apps::threads::approver(g.clone(), approver),
      None => approver,
    };
    // The hazard guard (design §2.28): the user's own threads, not an
    // app's or a family's (their guard is their own). A fixer's (and the
    // triage's) is the fixers' role, on its shell only: it judges what
    // would ask there (`ShellTools::approve`).
    // What it reads of the files a command runs: the thread's place (or
    // its own folder), never what is outside (design §2.28).
    let guard_roots: Vec<PathBuf> = self
      .place_dir(place)
      .map_or_else(|| vec![start_dir.clone()], |d| vec![d]);
    let hazard = match thread {
      Some(t) if app.is_none() && !fixer && selection.bound.is_none() => {
        let mut g = crate::hazard::Guard::for_thread(self, t, crate::hazard::Role::User);
        g.roots = guard_roots.clone();
        Some(Arc::new(g))
      }
      _ => None,
    };
    let fixers_guard = match thread {
      Some(t) if fixer => {
        let mut g = crate::hazard::Guard::for_thread(self, t, crate::hazard::Role::Fixers);
        let worktree_of = |main: &Path, i: &RecordId| {
          let base = main.file_name()?.to_string_lossy().into_owned();
          Some(main.with_file_name(format!(
            "{base}-{}",
            crate::mirror::branch_prefix(i.as_str())
          )))
        };
        let worktree = own_issue
          .as_ref()
          .and_then(|i| worktree_of(&self.place_dir(place)?, i));
        let pairs: Vec<PathBuf> = own_issue
          .as_ref()
          .map(|i| paired.iter().filter_map(|p| worktree_of(p, i)).collect())
          .unwrap_or_default();
        g.agent = Some(crate::hazard::fixer_agent(
          own_issue.as_ref().map(RecordId::as_str),
          worktree.as_deref(),
          own_issue.is_none() && crate::selfimprove::is_triage(self, t),
        ));
        g.roots = guard_roots
          .iter()
          .cloned()
          .chain(worktree)
          .chain(pairs)
          .collect();
        Some(Arc::new(g))
      }
      _ => None,
    };
    let mut mcp = self.mcp_tools(
      root,
      &selection.mcp(),
      approver.clone(),
      yolo,
      app.is_none(),
    );
    mcp.read_only = rules.read_only;
    mcp.live = live.clone();
    mcp.hazard = hazard.clone();
    // A thread with no place has its working folder (`start_dir`); with
    // nothing known, the home directory, except on a phone, where it is the
    // app's container, which cannot be written: a shared working folder.
    let start = start_dir;
    let place_dir = self.place_dir(place);
    let sandbox_for = |read_only: bool| {
      (rules.mode == "sandbox").then(|| {
        let mut sandbox = crate::sandbox::sandbox(
          place_dir.as_deref(),
          &start,
          &self.data,
          &settings.writable,
          &env,
          read_only,
        );
        crate::fixer::confine_sandbox(fixer, &mut sandbox, &self.data, &env);
        if !read_only {
          for p in &paired {
            crate::sandbox::add_checkout(&mut sandbox, p);
          }
        }
        if let Some(issue) = &own_issue {
          crate::fixer::own_worktree(&mut sandbox, issue.as_str());
        }
        sandbox
      })
    };
    let sandbox = sandbox_for(read_only);
    // For sessions started after a switch to read-only during the turn.
    let read_only_sandbox = if read_only {
      sandbox.clone()
    } else {
      sandbox_for(true)
    };
    if let Some(sandbox) = &sandbox {
      rules.protected = sandbox.writable.clone();
      rules.protected.push(self.data.clone());
      rules.signals_confined = stitch_shell::Sandbox::confines_signals();
      rules.place = place_dir.clone();
    }
    // On a phone, a folder's paths on the other nodes name its copy.
    let aliases = match (place, &place_dir) {
      (Place::Folder(key), Some(dir)) if crate::folders::keeps_copies() => {
        place::Aliases::new(self.places().folders.paths(key), dir)
      }
      _ => place::Aliases::default(),
    };
    // A thread with no place has its working folder for one: written
    // without asking, sandbox or not (a phone has none).
    let own = (place == &Place::Global && start.starts_with(self.data.join(THREAD_FOLDERS)))
      .then(|| start.clone());
    let edits = crate::edits::Editing {
      cwd: start.clone(),
      place: place_dir.or(own),
      clone_of: match place {
        Place::Project(repo) if crate::clones::keeps_clones() => Some(repo.clone()),
        _ => None,
      },
      read_only: rules.read_only,
      room_ask: rules.room_ask.clone(),
      yolo,
      live: rules.live.clone(),
      approver: Some(approver.clone()),
      // An app's thread writes its place (and own folder) only: the
      // sandbox's other directories (temporary ones, toolchains, the
      // node's records) are not an app's.
      sandbox: if app.is_some() { None } else { sandbox.clone() },
      aliases,
      hazard: hazard.clone(),
    };
    let mut otter = self.otter_tools(place, selection);
    if app.is_some() {
      // No code of its own (the language's tools).
      otter.language = false;
      otter.edge_talk = None;
    }
    otter.yolo = yolo;
    otter.read_only = rules.read_only;
    otter.live = live;
    otter.approver = Some(approver.clone());
    otter.cwd = start.clone();
    otter.scratch = self.data.join("downloads");
    // The turn's capability profiles, and its thread's taint read at each
    // call (docs/events.md, "Capability profiles").
    let ceiling = crate::profile::Ceiling::new(selection.profiles.clone(), self, thread);
    otter.ceiling = Some(ceiling.clone());
    let shell = ShellTools {
      shells: self.shells.clone(),
      rules,
      approver: Some(approver),
      cwd: start,
      env: base,
      clean_env: client_env.is_some(),
      place_env: Some(place_env),
      kind: if settings.user_shell == Some(true) {
        stitch_shell::ShellKind::User
      } else {
        stitch_shell::ShellKind::Bash
      },
      sandbox,
      read_only_sandbox,
      redact: {
        let vault = self.vault.clone();
        Some(stitch_shell::Redact(Arc::new(move |t: &str| {
          vault.redact(t)
        })))
      },
      // A fixer reaches no SSH host (the node's keys, other machines'
      // credentials).
      ssh: (!fixer).then(|| Arc::downgrade(self)),
      notes: Default::default(),
      hazard: hazard.or(fixers_guard),
    };
    NodeTools {
      shell,
      memory: self.memory.clone(),
      state: self.state().clone(),
      web: self.web.clone(),
      search_key: self.web.search_key(),
      blobs: self.state().blobs.clone(),
      scratch: self.data.join("downloads"),
      // Always replaced by the caller with the thread's model.
      model: (self.config.default_model.clone())
        .unwrap_or_else(|| crate::config::DEFAULT_MODEL.into()),
      shared: Arc::downgrade(self),
      peers: self
        .state()
        .known_peers()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.name)
        .filter(|n| n != self.state().node.as_str())
        .collect(),
      mcp,
      otter,
      groups: crate::toolset::GROUPS
        .iter()
        .map(|(g, _)| *g)
        .filter(|g| group_offered(selection, fixer, g))
        .collect(),
      remote_skills: selection.remote_skills.clone(),
      edits,
      started: std::time::Instant::now(),
      attended: client_env.is_some(),
      image_model: crate::imagegen::model(&self.config, &self.data),
      asks_more_steps: false,
      guard,
      ceiling,
    }
  }

  /// Model calls a turn of a chat the user is in takes before it asks for
  /// as many again (`[chat] max_steps`).
  pub(crate) fn chat_steps(&self) -> u64 {
    self
      .config
      .chat
      .as_ref()
      .and_then(|c| c.max_steps)
      .filter(|n| *n > 0)
      .unwrap_or(chat::CHAT_STEPS)
  }

  /// The Otter tools of the skills `selection` has on in `place`: their
  /// scripts compiled, or taken as compiled when unchanged, and the
  /// language's own with the `otter` skill. `[otter]` is
  /// read as config.toml says now, as `[mcp]` is.
  pub(crate) fn otter_tools(
    &self,
    place: &Place,
    selection: &Selection,
  ) -> crate::otter::OtterTools {
    let family = selection.bound.is_some();
    // An app's thread: none of the user's `[otter]` allowances.
    let settings = match Config::load(&self.data) {
      _ if family => Default::default(),
      Ok(c) => c.otter.unwrap_or_default(),
      Err(_) => self.config.otter.clone().unwrap_or_default(),
    };
    let mut scripts = Vec::new();
    let mut language = false;
    for k in self.skills_for(place, selection) {
      if !selection.skill_on(&k.name) {
        continue;
      }
      // The language's own tools come with the `otter` skill (Stitch's,
      // or one of the same name that replaces it).
      language |= k.name == "otter";
      match k.from.as_deref() {
        None => {
          if let Some(dir) = k.path.parent() {
            scripts.extend(
              crate::otter::scripts_in(&k.name, dir)
                .into_iter()
                .map(|s| crate::otter::Script { lazy: k.lazy, ..s }),
            );
          }
        }
        Some(crate::apps::BUILT_IN) => {
          if let Some(b) = crate::apps::SKILLS.iter().find(|b| b.name == k.name) {
            scripts.extend(crate::otter::built_in_scripts(b));
          }
        }
        // Another node's: its files are not here.
        Some(_) => {}
      }
    }
    let mut tools = crate::otter::OtterTools::new_for(&self.scripts, &scripts, settings, family);
    tools.language = language;
    tools.edge_talk = crate::otter::edge_talk::db();
    tools
  }

  /// `[mcp]` as config.toml says now: read again for each loop, so a
  /// change applies from the next one on.
  pub(crate) fn mcp_settings(&self) -> crate::mcp::McpSettings {
    match Config::load(&self.data) {
      Ok(c) => c.mcp.unwrap_or_default(),
      Err(e) => {
        eprintln!("mcp: {e}; using the configuration the node started with");
        self.config.mcp.clone().unwrap_or_default()
      }
    }
  }

  /// The MCP tools `entries` turn on, this node's and those other nodes
  /// share, for a loop on a thread of `root`. What this node shares is
  /// published again when it changed.
  fn mcp_tools(
    self: &Arc<Self>,
    root: &RecordRoot,
    entries: &[Entry],
    approver: Approver,
    yolo: bool,
    others: bool,
  ) -> crate::mcp::McpTools {
    self.mcp.configure(&self.mcp_settings().servers);
    let mut tools = crate::mcp::McpTools::new(&self.mcp, entries, self.data.join("downloads"));
    // An app's thread: this node's servers only (design §2.22).
    if others {
      tools = tools.with_others(crate::remote::others(self, root));
    }
    tools.yolo = yolo;
    tools.approver = Some(approver);
    let s = self.clone();
    tokio::spawn(async move { crate::remote::publish(&s).await });
    tools
  }

  /// Every item of a selection's tool and skill sets and its state, as
  /// `/tools` and `stitch thread tools` show them.
  pub(crate) fn set_items(
    &self,
    place: &Place,
    sel: &Selection,
  ) -> (Vec<api::SetItem>, Vec<api::SetItem>) {
    use crate::toolset::{GROUPS, decide, group_named};
    let mut tools = Vec::new();
    for (g, names) in GROUPS {
      let available = match *g {
        "shell" => self.approval() != "off",
        "web" => !self
          .web
          .definitions(self.web.search_key().is_some())
          .is_empty(),
        "memory" => self.memory.available(),
        // The device's, where the app lends them (iOS, visionOS).
        "calendar" | "maps" => crate::personal::host().is_some(),
        _ => true,
      };
      tools.push(api::SetItem {
        name: (*g).to_owned(),
        kind: "group".into(),
        on: sel.group_on(g),
        available,
        node: None,
        note: names.join(", "),
      });
    }
    self.mcp.configure(&self.mcp_settings().servers);
    let servers: Vec<String> = self.mcp.servers().iter().map(|s| s.name.clone()).collect();
    // A server is on when an entry for all of it says so; its patterns
    // are listed on their own.
    let server_on = |server: &str| {
      decide(&sel.tools, false, |p| {
        !p.contains("__") && group_named(p).is_none() && crate::mcp::glob(p, server)
      })
    };
    for name in &servers {
      tools.push(api::SetItem {
        name: name.clone(),
        kind: "mcp".into(),
        on: server_on(name),
        available: true,
        node: None,
        note: "MCP server".into(),
      });
    }
    // Other nodes': shared ones run there; others are only named.
    let others = crate::remote::listing(self);
    let mut servers = servers;
    for r in &others.shared {
      tools.push(api::SetItem {
        name: r.server.clone(),
        kind: "mcp".into(),
        on: server_on(&r.server),
        available: true,
        node: Some(r.node.clone()),
        note: "MCP server, shared".into(),
      });
      servers.push(r.server.clone());
    }
    for (node, server) in &others.unshared {
      if servers.contains(server) {
        continue;
      }
      tools.push(api::SetItem {
        name: server.clone(),
        kind: "mcp".into(),
        on: server_on(server),
        available: false,
        node: Some(node.clone()),
        note: format!("MCP server, not shared by {node}"),
      });
      servers.push(server.clone());
    }
    let mut seen: Vec<String> = Vec::new();
    for e in sel.tools.iter().rev() {
      let p = &e.pattern;
      if seen.contains(p) || group_named(p).is_some() || servers.contains(p) {
        continue;
      }
      seen.push(p.clone());
      if p.contains("__") || p.contains('*') {
        let server = p.split_once("__").map_or(p.as_str(), |(s, _)| s);
        tools.push(api::SetItem {
          name: p.clone(),
          kind: "pattern".into(),
          on: e.on,
          available: servers.iter().any(|s| crate::mcp::glob(server, s)),
          node: None,
          note: "MCP tools matching".into(),
        });
      } else {
        tools.push(api::SetItem {
          name: p.clone(),
          kind: "mcp".into(),
          on: e.on,
          available: false,
          node: None,
          note: "no MCP server of that name on this node".into(),
        });
      }
    }
    let mut skills: Vec<api::SetItem> = self
      .skills(place)
      .into_iter()
      .map(|k| api::SetItem {
        on: sel.skill_on(&k.name),
        name: k.name,
        kind: "skill".into(),
        available: true,
        node: k.from,
        note: k.description,
      })
      .collect();
    let mut seen: Vec<String> = Vec::new();
    for e in sel.skills.iter().rev() {
      let p = &e.pattern;
      if seen.contains(p) || p.contains('*') || skills.iter().any(|k| &k.name == p) {
        continue;
      }
      seen.push(p.clone());
      skills.push(api::SetItem {
        name: p.clone(),
        kind: "skill".into(),
        on: e.on,
        available: false,
        node: None,
        note: "no skill of that name here".into(),
      });
    }
    (tools, skills)
  }

  /// The skills a thread in `place` can have.
  /// The home workspace's and the place's (the place's winning by name).
  /// The skills a selection may have in `place`: an app's thread's are
  /// those its grant binds, as they were granted, never another of the
  /// same name (design §2.22); everyone else's, `skills`.
  pub(crate) fn skills_for(
    &self,
    place: &Place,
    selection: &Selection,
  ) -> Vec<crate::context::Skill> {
    let Some(bound) = &selection.bound else {
      return self.skills(place);
    };
    let mut out: Vec<crate::context::Skill> = Vec::new();
    for k in self.all_skills(place) {
      let ok = match k.from.as_deref() {
        None => bound.files.contains(&k.path),
        Some(crate::apps::BUILT_IN) => bound.built_in.contains(&k.name),
        Some(_) => false,
      };
      if ok && !out.iter().any(|o| o.name == k.name) {
        out.push(k);
      }
    }
    out
  }

  /// Every skill in `place`, the home's, the place's and Stitch's own, the
  /// same name more than once when they are (home's first).
  pub(crate) fn all_skills(&self, place: &Place) -> Vec<crate::context::Skill> {
    let mut out: Vec<crate::context::Skill> = self
      .state()
      .home()
      .as_ref()
      .map(|h| crate::context::skills(h.dir()))
      .unwrap_or_default();
    out.extend(self.place_files(place).listed());
    for b in crate::apps::SKILLS {
      out.push(crate::context::Skill {
        name: b.name.into(),
        description: b.description.into(),
        path: PathBuf::new(),
        from: Some(crate::apps::BUILT_IN.into()),
        lazy: b.lazy,
        lazy_tools: Vec::new(),
      });
    }
    out
  }

  pub(crate) fn skills(&self, place: &Place) -> Vec<crate::context::Skill> {
    let own = self.place_files(place).listed();
    let mut out: Vec<crate::context::Skill> = self
      .state()
      .home()
      .as_ref()
      .map(|h| crate::context::skills(h.dir()))
      .unwrap_or_default()
      .into_iter()
      .filter(|s| !own.iter().any(|o| o.name == s.name))
      .collect();
    out.extend(own);
    // Stitch's own, unless the user's or the place's takes the name;
    // `edge-talk` only where edge talk's database is, `browser` only while
    // a browser is linked and enabled (its tools are offered then only).
    let edge_talk = crate::otter::edge_talk::db().is_some();
    let browser = self.browsers.any_enabled();
    for b in crate::apps::SKILLS {
      if b.name == crate::otter::edge_talk::SKILL && !edge_talk {
        continue;
      }
      if b.name == "browser" && !browser {
        continue;
      }
      if !out.iter().any(|s| s.name == b.name) {
        out.push(crate::context::Skill {
          name: b.name.into(),
          description: b.description.into(),
          path: PathBuf::new(),
          from: Some(crate::apps::BUILT_IN.into()),
          lazy: b.lazy,
          lazy_tools: Vec::new(),
        });
      }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
  }

  /// What `place` says of itself (`<place>/.stitch/`): from its files when
  /// it is here, published when they changed; else from its record.
  pub(crate) fn place_files(&self, place: &Place) -> crate::placeset::PlaceFiles {
    if *place == Place::Global {
      return Default::default();
    }
    let state = self.state();
    let dir = self.place_dir(place);
    crate::placeset::of(
      state.home().as_ref(),
      &state.index,
      &state.node,
      &place.key(),
      dir.as_deref(),
    )
  }

  /// What a shell in `place` gets over the node's environment: the
  /// variables and what `[env]` maps (config.toml's, then the place's),
  /// as they are now, the place as it is now (a folder folded into a
  /// project since: the project's). What does not resolve is left out,
  /// said in the node's log when `log`.
  pub(crate) fn place_env(&self, place: &Place, log: bool) -> Vec<(String, String)> {
    let place = self.places().now(place);
    let key = place.key();
    let files = self.place_files(&place);
    let (extra, problems) = self
      .vault
      .env_for(&key, &[&self.config.env, &files.settings.env]);
    if log {
      for p in problems {
        eprintln!("env of {key}: {p}");
      }
    }
    extra
      .into_iter()
      .filter(|(k, _)| k != "PATH" && k != "STITCH_DATA")
      .collect()
  }

  /// Whether a place's settings say `memory = false` (design §2.9).
  pub(crate) fn place_memory_off(&self, place: &Place) -> bool {
    self.place_files(place).settings.memory == Some(false)
  }

  /// A place whose settings say `memory = false` keeps its threads out of
  /// memory (design §2.9): one whose memory was never switched is taken
  /// out before its turn, so `/memory on` still brings one back for good.
  /// A detached thread is taken out as it starts (`detached::start`).
  pub(crate) fn place_memory(
    &self,
    place: &Place,
    root: &RecordRoot,
    thread: &RecordId,
  ) -> Result<()> {
    if self.place_memory_off(place) && !self.memory.is_switched(thread) {
      self.memory.remember(self.state(), root, thread, false)?;
    }
    Ok(())
  }

  /// Whether `thread` is in memory, counting its place's `memory = false`
  /// before its next turn applies it; without one, whether a new thread in
  /// the place of `cwd` (or global) would be. `None` when asked about
  /// neither (or about no thread ID).
  pub(crate) fn memory_of(
    &self,
    thread: Option<&str>,
    cwd: Option<&str>,
    global: bool,
  ) -> Option<bool> {
    let place_off = |place: &Place| self.place_memory_off(place);
    match thread {
      Some(t) => {
        let t: RecordId = t.parse().ok()?;
        if self.memory.is_off(&t) {
          return Some(false);
        }
        if self.memory.is_switched(&t) {
          return Some(true);
        }
        Some(
          !self
            .thread_place(&t)
            .is_ok_and(|(_, place)| place_off(&place)),
        )
      }
      None if global => Some(true),
      // A checkout no thread was started in yet is not known as its
      // project's directory: seen here, so its settings are read.
      None => cwd.map(|_| {
        let here = self.place_of_dir(cwd);
        self.saw(&here);
        !place_off(&here.0)
      }),
    }
  }

  /// The tool and skill entries of a thread in `place` whose shells start
  /// in `start`, every layer in order (design §2.14): the node's `[mcp]`
  /// lists, the place's settings, then the thread's own changes
  /// (`thread`: none for a new one).
  pub(crate) fn selection(
    &self,
    place: &Place,
    start: Option<&Path>,
    thread: Option<&RecordId>,
  ) -> Selection {
    let files = self.place_files(place);
    let mut sel = Selection {
      tools: self.mcp_settings().enabled(start),
      skills: files.skill_entries(),
      remote_skills: Vec::new(),
      read_only_switch: thread.and_then(|t| self.read_only_switch(t)),
      bound: None,
      profiles: Vec::new(),
    };
    sel.tools.extend(files.tool_entries());
    if let Some(t) = thread {
      let (tools, skills) =
        crate::toolset::thread_changes(&self.state().index.lock().expect("index lock"), t);
      sel.tools.extend(tools);
      sel.skills.extend(skills);
      // An app's thread has what its app was granted, last, whatever came
      // before, and only the skills its grant binds (design §2.22).
      if let Some(scope) = crate::apps::threads::scope_of(self, t, Some(place)) {
        let (tools, skills) = crate::apps::threads::ThreadNeeds::entries(scope.needs.as_ref());
        sel.tools.extend(tools);
        sel.skills.extend(skills);
        sel.bound = Some(match &scope.needs {
          Some(_) => crate::apps::threads::bound_now(&scope.skills),
          None => Default::default(),
        });
      }
    }
    // A place skill whose files are another node's is read from its record
    // (not an app's thread's: nothing of it is bound).
    sel.remote_skills = files
      .skills
      .iter()
      .filter(|k| k.path.is_none() && sel.skill_on(&k.name) && sel.bound.is_none())
      .map(|k| crate::toolset::RemoteSkill {
        name: k.name.clone(),
        node: files.from.clone().unwrap_or_default(),
        text: k.text.clone(),
        files: Vec::new(),
      })
      .collect();
    // Stitch's own skills are read the same way, with their files.
    let listed = self.skills(place);
    for b in crate::apps::SKILLS {
      let built_in = listed
        .iter()
        .any(|k| k.name == b.name && k.from.as_deref() == Some(crate::apps::BUILT_IN))
        && sel
          .bound
          .as_ref()
          .is_none_or(|bd| bd.built_in.iter().any(|n| n == b.name));
      if built_in && sel.skill_on(b.name) {
        sel.remote_skills.push(crate::toolset::RemoteSkill {
          name: b.name.into(),
          node: crate::apps::BUILT_IN.into(),
          text: b.text.into(),
          files: b
            .files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect(),
        });
      }
    }
    sel
  }

  /// The shell's approval mode now.
  pub(crate) fn approval(&self) -> String {
    self.approval.lock().expect("approval lock").clone()
  }

  /// The approval mode in `place` now: its settings' `approval` when set
  /// (design §2.28, stricter or looser than the node's), else the node's.
  pub(crate) fn approval_in(&self, place: &Place) -> String {
    let node = self.approval();
    match self.place_files(place).settings.approval {
      Some(a) => a.effective(&node, &approval_mode(None)),
      None => node,
    }
  }

  /// The approval mode where `thread` works now (`approval_in`); the
  /// node's when its place is not known.
  pub(crate) fn approval_of_thread(&self, thread: &RecordId) -> String {
    match self.thread_place(thread) {
      Ok((_, place)) => self.approval_in(&place),
      Err(_) => self.approval(),
    }
  }

  /// Sets the shell's approval mode, for the loops that start from now,
  /// and saves it in config.toml; none: the default (`approval_mode`).
  fn set_approval(&self, mode: Option<&str>) -> Result<()> {
    let mut config = Config::load(&self.data)?;
    config.shell.get_or_insert_with(Default::default).approval = mode.map(str::to_owned);
    config.save(&self.data)?;
    *self.approval.lock().expect("approval lock") = approval_mode(mode.map(str::to_owned));
    Ok(())
  }

  /// What is granted in `place` now, read at each check: yolo, and what
  /// "always" answers allowed there.
  pub(crate) fn live(self: &Arc<Self>, place: &Place) -> crate::rules::Live {
    self.live_for(place, None)
  }

  /// What is granted in a place, and whether `thread` is read-only now
  /// (the user may switch it during a turn).
  pub(crate) fn live_for(
    self: &Arc<Self>,
    place: &Place,
    thread: Option<&RecordId>,
  ) -> crate::rules::Live {
    let weak = Arc::downgrade(self);
    let key = place.key();
    let place = place.clone();
    let thread = thread.cloned();
    crate::rules::Live::new(move || match weak.upgrade() {
      Some(s) => crate::rules::Granted {
        read_only: thread.as_ref().is_some_and(|t| s.read_only(t)),
        yolo: crate::rules::runs_unasked(&s.approval_in(&place)),
        allow: s.granted(&key),
        mcp: s
          .mcp_grants
          .lock()
          .expect("grants lock")
          .get(&key)
          .cloned()
          .unwrap_or_default(),
        otter: s
          .otter_grants
          .lock()
          .expect("grants lock")
          .get(&key)
          .cloned()
          .unwrap_or_default(),
      },
      None => crate::rules::Granted::default(),
    })
  }

  /// Shell prefixes allowed in a place (its key) beyond the node's.
  pub(crate) fn granted(&self, place: &str) -> Vec<String> {
    self
      .grants
      .lock()
      .expect("grants lock")
      .get(place)
      .cloned()
      .unwrap_or_default()
  }

  /// Allows `prefixes` from now on in a place (an "always" answer), and
  /// saves them in config.toml.
  pub(crate) fn grant(&self, place: &str, prefixes: &[String]) -> Result<()> {
    let mut grants = self.grants.lock().expect("grants lock");
    let mut config = Config::load(&self.data)?;
    let saved = &mut config
      .shell
      .get_or_insert_with(Default::default)
      .places
      .entry(place.to_owned())
      .or_default()
      .allow;
    let mut changed = false;
    for p in prefixes {
      if !saved.contains(p) {
        saved.push(p.clone());
        changed = true;
      }
    }
    let now = saved.clone();
    if changed {
      config.save(&self.data)?;
    }
    grants.insert(place.to_owned(), now);
    Ok(())
  }

  /// Allows `server`'s `tools` from now on in a place (an "always"
  /// answer), and saves them in config.toml.
  pub(crate) fn grant_mcp(&self, place: &str, server: &str, tools: &[String]) -> Result<()> {
    let mut grants = self.mcp_grants.lock().expect("grants lock");
    let mut config = Config::load(&self.data)?;
    let allow = &mut config
      .mcp
      .get_or_insert_with(Default::default)
      .places
      .entry(place.to_owned())
      .or_default()
      .allow;
    let saved = allow.entry(server.to_owned()).or_default();
    let mut changed = false;
    for t in tools {
      if !saved.contains(t) {
        saved.push(t.clone());
        changed = true;
      }
    }
    let now = allow.clone();
    if changed {
      config.save(&self.data)?;
    }
    grants.insert(place.to_owned(), now);
    Ok(())
  }

  /// Allows Otter `tools` from now on in a place (an "always" answer),
  /// and saves them in config.toml.
  pub(crate) fn grant_otter(&self, place: &str, tools: &[String]) -> Result<()> {
    let mut grants = self.otter_grants.lock().expect("grants lock");
    let mut config = Config::load(&self.data)?;
    let saved = &mut config
      .otter
      .get_or_insert_with(Default::default)
      .places
      .entry(place.to_owned())
      .or_default()
      .allow;
    let mut changed = false;
    for t in tools {
      if !saved.contains(t) {
        saved.push(t.clone());
        changed = true;
      }
    }
    let now = saved.clone();
    if changed {
      config.save(&self.data)?;
    }
    grants.insert(place.to_owned(), now);
    Ok(())
  }

  /// Allows browser `sites` from now on in a place (an "always" answer to
  /// a question asked in the browser), and saves them in config.toml.
  pub(crate) fn grant_browser(&self, place: &str, sites: &[String]) -> Result<()> {
    let mut config = Config::load(&self.data)?;
    let saved = &mut config
      .browser
      .get_or_insert_with(Default::default)
      .places
      .entry(place.to_owned())
      .or_default()
      .sites;
    let mut changed = false;
    for t in sites {
      if !saved.contains(t) {
        saved.push(t.clone());
        changed = true;
      }
    }
    if changed {
      config.save(&self.data)?;
    }
    self.browsers.engine.grant(place, sites);
    Ok(())
  }

  /// The system prompt for a new thread, with its context.
  pub(crate) fn system_prompt(&self, detached: bool, context: &str) -> String {
    let state = self.state();
    let mut out = crate::prompt::root(
      state.home().as_ref(),
      state.node.as_str(),
      detached,
      context,
    );
    out.push_str(&crate::prompt::ssh_hosts(
      &crate::ssh::hosts(self)
        .into_iter()
        .map(|h| h.name)
        .collect::<Vec<_>>(),
    ));
    out
  }

  /// The instructions and skills of a thread in `place` whose shells start
  /// in `start`, the skills its selection turns on (`crate::context`).
  pub(crate) fn context(
    &self,
    place: &Place,
    start: Option<&Path>,
    selection: &Selection,
    thread: Option<&RecordId>,
  ) -> String {
    let home = self.state().home();
    let top = self.place_dir(place);
    // A skill's lazy tools are named in its line (design §2.17).
    let otter = self.otter_tools(place, selection);
    // Stitch's own `browser` skill goes with the tools it is about: not
    // listed where the thread is never given them (group off, a fixer).
    let fixer = selection.bound.is_none()
      && thread.is_some_and(|t| crate::selfimprove::in_fixer_mode(self, t));
    let browser = group_offered(selection, fixer, "browser");
    let skills: Vec<crate::context::Skill> = self
      .skills_for(place, selection)
      .into_iter()
      .filter(|s| selection.skill_on(&s.name))
      .filter(|s| {
        browser || !(s.name == "browser" && s.from.as_deref() == Some(crate::apps::BUILT_IN))
      })
      .map(|mut s| {
        s.lazy_tools = otter
          .lazy_of(&s.name)
          .into_iter()
          .map(|d| d.name.clone())
          .collect();
        s
      })
      .collect();
    let yours = crate::instructions::for_place(
      self.state(),
      place,
      &place.label(self.state().node.as_str(), &self.places()),
    );
    // An app's thread (design §2.22): its place's instructions and its
    // skills; nothing of the user's home, nodes or machines.
    if selection.bound.is_some() {
      let mut out = crate::context::render(None, yours, top.as_deref(), start, &skills, &[]);
      out.push_str(&crate::apps::threads::turn_note(thread, start));
      return out;
    }
    let mut out = crate::context::render(
      home.as_ref().map(RecordRoot::dir),
      yours,
      top.as_deref(),
      start,
      &skills,
      &self.tool_notes(selection),
    );
    // Said each turn (a change reaches the model as the instructions'
    // do): a thread continued on another node has that node's shells, and
    // its prompt names the node it started on.
    if !out.is_empty() {
      out.push('\n');
    }
    out.push_str(&self.where_note(place, start, thread));
    out.push_str(&self.folded_note(place));
    // Said once the user switched the thread's mode: a change reaches the
    // model at its next turn, as the instructions' do.
    if let Some(read_only) = selection.read_only_switch {
      out.push_str(&mode_note(read_only));
    }
    // A checkout taken for a folder for want of git: the agent can install it.
    if matches!(place, Place::Folder(_))
      && let Some(top) = self
        .place_dir(place)
        .and_then(|d| place::unread_checkout(&d))
    {
      out.push_str(&format!(
        "\n# git is missing here\n\n{} Offer to install it.\n",
        place::unread_checkout_note(&top)
      ));
    }
    out
  }

  /// What a turn is told about the node it runs on, each turn: a thread
  /// continued on another node keeps the prompt it started with, whose
  /// "This node" and rules are that node's, and earlier turns' paths.
  fn where_note(&self, place: &Place, start: Option<&Path>, thread: Option<&RecordId>) -> String {
    let me = &self.state().node;
    let home = std::env::var_os("HOME")
      .map(|h| home_label(Path::new(&h)))
      .map(|h| format!(", {h}"))
      .unwrap_or_default();
    let mut out = format!(
      "# Where this turn runs\n\nOn node {me} ({} {}{home}): its shells, files, checkouts and \
       tools are {me}'s, whatever node this thread started on. A thread runs where it is \
       continued: a chat on the node of the client it is typed into; a message or report for \
       it on the node that continued it last. Earlier turns may have run on another node: the \
       paths, shells and tools they used were that node's.\n",
      std::env::consts::OS,
      std::env::consts::ARCH,
    );
    // Who it is, so it knows itself in `threads` and in reports (2026-10-19:
    // a spawned thread saw its own ID listed as working and stopped "to
    // avoid a duplicate", naming itself as the other thread).
    if let Some(t) = thread {
      out.push_str(&who_said(t));
    }
    if place != &Place::Global
      && let Some(dir) = self.place_dir(place)
    {
      let label = place.label(me.as_str(), &self.places());
      let what = match place {
        Place::Folder(_) if crate::folders::keeps_copies() => {
          ", this device's copy of the folder: its paths on other nodes (`~/…` or absolute, \
           as earlier turns wrote them) name it here too"
        }
        Place::Project(_) if crate::clones::keeps_clones() => {
          ", this device's clone of the project: a path inside its checkout on another node is \
           the same path inside this clone"
        }
        _ => "",
      };
      out.push_str(&format!(
        "\nThis thread's place, {label}, is at {} here{what}.\n",
        dir.display()
      ));
    } else if let Place::Project(_) = place {
      // Said, never left out: a turn that started in the home directory
      // took another repository there for the one the user picked.
      let label = place.label(me.as_str(), &self.places());
      out.push_str(&format!(
        "\nThis thread's place is the project {label}, which has no checkout on {me}: \
         its files are not here. Do not take another repository for it; say so, and offer \
         to clone it.\n"
      ));
    }
    // Its working folder syncs with the other nodes (design §2.3): named
    // even when the client's directory is the start, so files written in
    // it elsewhere are found.
    if place == &Place::Global {
      let own = start.filter(|d| d.starts_with(self.data.join(THREAD_FOLDERS)));
      if let Some(dir) = own {
        let synced = if thread.is_some_and(|t| self.thread_folder(t) == dir) {
          " It syncs with the user's other nodes: what earlier turns wrote in it on any node \
           is here."
        } else {
          ""
        };
        out.push_str(&format!(
          "\nThis thread's working folder: {} (relative paths start here; write files here, \
           not under ~).{synced}\n",
          dir.display()
        ));
      } else if let Some(t) = thread {
        let dir = self.thread_folder(t);
        let used = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some());
        if used {
          out.push_str(&format!(
            "\nThis thread's working folder is {} here: it syncs with the user's other \
             nodes, so files earlier turns wrote in it on any node (apps, documents) are \
             there, though this turn starts in the client's directory.\n",
            dir.display()
          ));
        }
      }
    }
    if !stitch_shell::CAN_SPAWN {
      out.push_str(
        "\nNo programs run here: no shells and no browser, whatever earlier turns had. Read \
         files and list directories with view_file, change them with edit_file and write_file; \
         work that needs a machine goes to one of the user's others with `spawn`.\n",
      );
    }
    out
  }

  /// A folder that became a git checkout on another node is a project
  /// since, and stopped syncing (design §2.16): where this node still has
  /// its copy, the turn is told, so the agent here can reconcile the two.
  fn folded_note(&self, place: &Place) -> String {
    let Place::Project(repo) = place else {
      return String::new();
    };
    let places = self.places();
    let me = self.state().node.as_str();
    let mut out = String::new();
    for (key, became) in places.folders.folded_into(repo) {
      let Some(copy) = places.folders.dir(key, me).or_else(|| {
        let d = crate::folders::copy_dir(&self.data, key);
        d.is_dir().then_some(d)
      }) else {
        continue;
      };
      if crate::folders::is_checkout(&copy) {
        continue;
      }
      let (node, path) = became.split_once(':').unwrap_or(("another node", became));
      out.push_str(&format!(
        "\n# A folder that became this project\n\nThe folder {key} became a git checkout on \
         {node} ({path}), so it is this project now and its files no longer sync. This node \
         still has its copy, {}, as it was. If the two differ, reconcile them: compare the copy \
         with a clone of the project, carry edits over, or ask the user.\n",
        copy.display()
      ));
    }
    out
  }

  /// What a thread is told about its tools (design §2.14): those that run
  /// on another node, and entries that are on but name nothing here.
  fn tool_notes(&self, selection: &Selection) -> Vec<String> {
    use crate::toolset::group_named;
    let others = crate::remote::listing(self);
    let mut out = Vec::new();
    for r in &others.shared {
      let on = r.tools.iter().any(|t| {
        crate::mcp::exposed_name(&r.server, &t.name)
          .is_some_and(|n| selection.tool_on(&r.server, &n))
      });
      if on {
        out.push(format!(
          "{}__*: MCP server {} on {}: its calls run there, under {}'s rules (an approval may \
           be asked there). When {} cannot be reached, queue_call runs a call once it is back.",
          r.server, r.server, r.node, r.node, r.node
        ));
      }
    }
    let here: Vec<String> = self.mcp.servers().iter().map(|x| x.name.clone()).collect();
    let mut said: Vec<&str> = Vec::new();
    for e in selection.tools.iter().rev() {
      let p = e.pattern.as_str();
      if said.contains(&p) || p.contains('*') || p.contains("__") || group_named(p).is_some() {
        continue;
      }
      said.push(p);
      if !e.on || here.iter().any(|h| h == p) || others.shared.iter().any(|r| r.server == p) {
        continue;
      }
      match others.unshared.iter().find(|(_, s)| s == p) {
        Some((node, _)) => out.push(format!(
          "{p}: turned on for this thread, but MCP server {p} is on {node}, which does not share it."
        )),
        None => out.push(format!(
          "{p}: turned on for this thread, but no node has an MCP server of that name."
        )),
      }
    }
    out
  }

  /// The projects the home workspace lists.
  fn projects(&self) -> Vec<place::Project> {
    place::projects(&self.state().index.lock().expect("index lock"))
  }

  /// What the home workspace says about places.
  pub(crate) fn places(&self) -> place::Places {
    place::Places::load(&self.state().index.lock().expect("index lock"))
  }

  /// The folder location a new thread in `place` records: where the
  /// client is, home-relative.
  pub(crate) fn folder_of(&self, place: &Place, cwd: Option<&str>) -> Option<chat::FolderAt> {
    let cwd = cwd?;
    matches!(place, Place::Folder(_)).then(|| {
      let dir = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.into());
      (
        self.state().node.to_string(),
        place::home_relative_str(&dir.display().to_string()),
      )
    })
  }

  /// A place named by its key (`Place::key`), for a client with no
  /// directory there (the phone): it and its root, and for a folder where
  /// it is (its `~` path is the same on every node; another path is one
  /// node's).
  pub(crate) fn place_named(
    &self,
    key: &str,
  ) -> Result<(RecordRoot, Place, Option<chat::FolderAt>)> {
    match Place::of_key(key) {
      Some(Place::Folder(f)) => {
        let (root, place) = self.placement(None, false, &(Place::Folder(f.clone()), None))?;
        let at = match f.split_once(':') {
          Some((node, path)) if !f.starts_with('~') => (node.to_owned(), path.to_owned()),
          _ => (self.state().node.to_string(), f),
        };
        Ok((root, place, Some(at)))
      }
      Some(Place::Project(repo)) => {
        let (root, place) = self.placement(Some(repo.as_str()), false, &(Place::Global, None))?;
        Ok((root, place, None))
      }
      Some(Place::Global) => {
        let (root, place) = self.placement(None, true, &(Place::Global, None))?;
        Ok((root, place, None))
      }
      None => Err(Error::Config(format!(
        "{key}: a place is `global`, `project:<repo>` or `folder:<folder>`"
      ))),
    }
  }

  /// The place of the directory a client runs in, with its checkout.
  pub(crate) fn place_of_dir(&self, cwd: Option<&str>) -> (Place, Option<place::Checkout>) {
    let state = self.state();
    let Some(cwd) = cwd else {
      return (Place::Global, None);
    };
    let home = state.home();
    let here = Place::of_dir(
      Path::new(cwd),
      state.node.as_str(),
      home.as_ref().map(RecordRoot::dir),
      &self.places(),
    );
    // A checkout taken for a folder for want of git: said once in the log.
    if matches!(here.0, Place::Folder(_))
      && let Some(top) = place::unread_checkout(Path::new(cwd))
    {
      static SAID: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
      let mut said = SAID.lock().expect("said lock");
      if !said.contains(&top) {
        eprintln!("{}", place::unread_checkout_note(&top));
        said.push(top);
      }
    }
    here
  }

  /// Where a new thread goes: the repository asked for, or the place it
  /// starts in (`global`: none). A project seen for the first time is
  /// listed in the home workspace and gets its record store.
  pub(crate) fn placement(
    &self,
    repo: Option<&str>,
    global: bool,
    here: &(Place, Option<place::Checkout>),
  ) -> Result<(RecordRoot, Place)> {
    let state = self.state();
    let held = |id: &RepoId| {
      state
        .root(id)
        .ok_or_else(|| Error::Config(format!("repository {id} is not held here")))
    };
    if let Some(r) = repo {
      let root = held(&r.parse()?)?;
      let place = match root.config().role {
        Role::Home => Place::Global,
        Role::Project => Place::Project(root.repo().clone()),
        Role::Folder => {
          return Err(Error::Config(format!(
            "{} holds a folder's files, not threads",
            root.repo()
          )));
        }
      };
      return Ok((root, place));
    }
    let home = || {
      state.home().ok_or_else(|| {
        Error::Config("no home workspace here: create one with `stitch setup --name <name>`".into())
      })
    };
    match here {
      _ if global => Ok((home()?, Place::Global)),
      (Place::Project(id), co) => {
        if let Some(co) = co {
          self.list_project(id, co, false)?;
          self.saw(here);
        }
        Ok((held(id)?, here.0.clone()))
      }
      (p, _) => Ok((home()?, p.clone())),
    }
  }

  /// Notes the checkout a client runs in, when it is a project's.
  /// Notes where a project is checked out here (a phone's clone).
  pub(crate) fn saw_checkout(&self, repo: &RepoId, top: &Path) {
    self
      .checkouts
      .lock()
      .expect("checkouts lock")
      .saw(repo, top);
  }

  /// Every checkout of any project this node has seen that is still there.
  pub(crate) fn known_checkouts(&self) -> Vec<PathBuf> {
    self.checkouts.lock().expect("checkouts lock").all()
  }

  pub(crate) fn saw(&self, here: &(Place, Option<place::Checkout>)) {
    if let (Place::Project(id), Some(co)) = here {
      self
        .checkouts
        .lock()
        .expect("checkouts lock")
        .saw(id, &co.top);
    }
  }

  /// Lists a project in the home workspace unless it is, and holds its
  /// record store.
  fn list_project(&self, repo: &RepoId, co: &place::Checkout, split: bool) -> Result<()> {
    let state = self.state();
    if state.root(repo).is_some() {
      return Ok(());
    }
    if !self.projects().iter().any(|p| &p.repo == repo) {
      let home = state
        .home()
        .ok_or_else(|| Error::Config("no home workspace here".into()))?;
      let mut draft = stitch_records::Draft::new(
        stitch_records::kinds::PROJECT,
        stitch_records::Author::node(state.node.clone()),
      )?
      .body(format!("# {}\n", co.name()))
      .field("repo", repo.as_str())?
      .field("root", co.root.as_str())?
      .field("name", co.name())?;
      if let Some(r) = &co.remote {
        draft = draft.field("remote", r.as_str())?;
      }
      if split {
        draft = draft.field("split", true)?;
      }
      home.publish(&state.node, draft)?;
    }
    state.hold_projects()?;
    Ok(())
  }

  /// Folds the folder a checkout was before `git init` into its project,
  /// and lists the project (its name, its store) when its folder is folded
  /// into it, now or before: a thread continued there after `git init`
  /// starts none, which would have listed it.
  pub(crate) fn fold(&self, here: &(Place, Option<place::Checkout>)) -> Result<()> {
    let state = self.state();
    let Some(home) = state.home() else {
      return Ok(());
    };
    let folded_here = {
      let mut index = state.index.lock().expect("index lock");
      place::fold(&mut index, &home, &state.node, &here.0, here.1.as_ref())?;
      match here {
        (Place::Project(id), Some(co)) => {
          let places = place::Places::load(&index);
          let key = places
            .folders
            .key(state.node.as_str(), &co.top.display().to_string());
          places.folders.project(&key) == Some(id)
        }
        _ => false,
      }
    };
    if let (true, Place::Project(id), Some(co)) = (folded_here, &here.0, &here.1) {
      self.list_project(id, co, false)?;
      self.saw(here);
    }
    Ok(())
  }

  /// Splits the checkout `cwd` is in off into a project of its own (a fork
  /// that is its own thing); its name.
  pub(crate) fn split(&self, cwd: &str) -> Result<String> {
    let co = place::Checkout::of(Path::new(cwd))
      .ok_or_else(|| Error::Config(format!("{cwd} is not in a git checkout with commits")))?;
    let remote = co.remote.as_deref().ok_or_else(|| {
      Error::Config("a fork is told apart by its remote: this checkout has no `origin`".into())
    })?;
    let repo = place::project_repo(&co.root, Some(remote));
    self.list_project(&repo, &co, true)?;
    self
      .checkouts
      .lock()
      .expect("checkouts lock")
      .saw(&repo, &co.top);
    Ok(format!("{} ({repo})", co.name()))
  }

  /// A thread's repository and place.
  /// Whether a thread is read-only: what the rules do not allow is
  /// refused, never asked (`spawn`'s `read_only`, inherited by what it
  /// spawns).
  /// The user's switch, when there is one, decides; else the thread's
  /// record as it was created (design §2.2, read-only threads).
  pub(crate) fn read_only(&self, thread: &RecordId) -> bool {
    crate::meta::read_only(&self.state().index.lock().expect("index lock"), thread)
  }

  /// The thread's mode as the user last switched it (`meta::read_only_switch`).
  pub(crate) fn read_only_switch(&self, thread: &RecordId) -> Option<bool> {
    crate::meta::read_only_switch(&self.state().index.lock().expect("index lock"), thread)
  }

  /// Switches a thread between read-only and writable, for the user (the
  /// CLI, the TUI, the app; never a tool). Switched to read-only, its
  /// shells here close (what runs in them ends): the next command starts
  /// one in the read-only sandbox.
  pub(crate) async fn set_read_only(self: &Arc<Self>, thread: &RecordId, on: bool) -> Result<()> {
    let (s, t) = (self.clone(), thread.clone());
    tokio::task::spawn_blocking(move || {
      crate::meta::set(
        &s,
        &t,
        "approval",
        if on { "read-only" } else { "default" }.into(),
        stitch_records::Author::user(s.state().node.clone()),
      )
    })
    .await
    .map_err(|e| Error::Config(e.to_string()))??;
    if on {
      self.shells.close_sessions(thread).await;
    }
    Ok(())
  }

  /// After a sync: threads switched to read-only on another node have
  /// their writable shells here closed.
  pub(crate) async fn close_switched(self: &Arc<Self>) {
    for thread in self.shells.writable_threads().await {
      if self.read_only(&thread) {
        self.shells.close_sessions(&thread).await;
      }
    }
  }

  pub(crate) fn thread_place(&self, id: &RecordId) -> Result<(RecordRoot, Place)> {
    self.thread_place_in(id, &self.places())
  }

  /// The record store a new thread (or schedule) following `root`'s
  /// record goes in: a project's own, even when that record is a
  /// folder's left in home, the folder since folded into the project.
  /// There it would carry no folder and be global.
  pub(crate) fn root_for(&self, root: RecordRoot, place: &Place) -> Result<RecordRoot> {
    match place {
      Place::Project(repo) if root.repo() != repo => self
        .state()
        .root(repo)
        .ok_or_else(|| Error::Config(format!("repository {repo} is not held here"))),
      _ => Ok(root),
    }
  }

  /// `thread_place` with the places read once by the caller (a list of a
  /// thousand rows read them for each).
  pub(crate) fn thread_place_in(
    &self,
    id: &RecordId,
    places: &place::Places,
  ) -> Result<(RecordRoot, Place)> {
    let state = self.state();
    let record = state
      .index
      .lock()
      .expect("index lock")
      .record(id)?
      .ok_or_else(|| Error::Config(format!("no thread {id} here")))?;
    let root = state
      .root(&record.repo)
      .ok_or_else(|| Error::Config(format!("repository {} is not held here", record.repo)))?;
    let home = state.home().map(|h| h.repo().clone());
    Ok((root, Place::of_thread(&record, home.as_ref(), places)))
  }

  /// The place's directory here: a project's checkout, a folder, or a
  /// phone's copy of a folder (design §2.16).
  pub(crate) fn place_dir(&self, place: &Place) -> Option<PathBuf> {
    match place {
      Place::Folder(key) => crate::folders::location(self, key).filter(|d| d.is_dir()),
      _ => place.dir(
        self.state().node.as_str(),
        &mut self.checkouts.lock().expect("checkouts lock"),
        &self.places(),
      ),
    }
  }

  /// `stitch project rebind` in the checkout `cwd` is in: the user's say
  /// that it is the repository of a project re-created from a new first
  /// commit at the same origin (design §2.3), so that project follows it
  /// (`reroot`). Never done by itself: identity is not merged by path or
  /// origin. `project` names it (its name or ID) when several could be.
  pub(crate) fn rebind(&self, cwd: &str, project: Option<&str>) -> Result<String> {
    let co = place::Checkout::of(Path::new(cwd))
      .ok_or_else(|| Error::Config(format!("{cwd} is not in a git checkout with commits")))?;
    let remote = co.remote.clone().ok_or_else(|| {
      Error::Config(
        "a re-created repository is known by its origin: this checkout has no `origin`".into(),
      )
    })?;
    let places = self.places();
    let projects = &places.projects;
    let here = place::project_of(&co, projects);
    let show = |p: &place::Project| format!("{} ({})", p.name, p.repo);
    let same: Vec<&place::Project> = projects
      .iter()
      .filter(|p| !p.split && p.remote.as_deref() == Some(remote.as_str()) && p.root != co.root)
      .collect();
    if let Some(p) = same.iter().find(|p| p.reroots.contains(&co.root)) {
      self.saw_checkout(&p.repo, &co.top);
      return Ok(format!("{} already follows this checkout", show(p)));
    }
    if same.is_empty() {
      return Err(Error::Config(format!(
        "no project listed with origin {remote} has another first commit: nothing to rebind          (this checkout is {here})"
      )));
    }
    let list = |ps: &[&place::Project]| ps.iter().map(|p| show(p)).collect::<Vec<_>>().join(", ");
    let chosen: Vec<&place::Project> = match project {
      Some(n) => same
        .iter()
        .copied()
        .filter(|p| p.name == n || p.repo.as_str() == n)
        .collect(),
      None => same.clone(),
    };
    let p = match chosen.as_slice() {
      [p] => *p,
      [] => {
        return Err(Error::Config(format!(
          "no project named {} has origin {remote}: it is one of {}",
          project.unwrap_or_default(),
          list(&same)
        )));
      }
      _ => {
        return Err(Error::Config(format!(
          "several projects have origin {remote}: {}; name one with --project",
          list(&chosen)
        )));
      }
    };
    if !place::recreates(&co, &p.repo, projects) {
      return Err(Error::Config(format!(
        "{} was listed after the project of this checkout's first commit ({here}): a project \
         follows its repository only forward, to a newer history",
        show(p)
      )));
    }
    self.reroot(&p.repo, &co)?;
    Ok(format!(
      "{} follows this checkout now: its threads find it, new threads here belong to it",
      show(p)
    ))
  }

  /// Has project `repo` follow its repository, re-created from a new first
  /// commit at the same origin, to checkout `co` (`place::recreates`, the
  /// user's `rebind`): a `project` record with `reroot` and no `root`
  /// (which builds before it skip), and the checkout noted.
  pub(crate) fn reroot(&self, repo: &RepoId, co: &place::Checkout) -> Result<()> {
    let state = self.state();
    let places = self.places();
    let label = Place::Project(repo.clone()).label(state.node.as_str(), &places);
    let short: String = co.root.chars().take(12).collect();
    let home = state
      .home()
      .ok_or_else(|| Error::Config("no home workspace here".into()))?;
    let mut draft = stitch_records::Draft::new(
      stitch_records::kinds::PROJECT,
      stitch_records::Author::node(state.node.clone()),
    )?
    .body(format!("# {label} re-created\n"))
    .field("repo", repo.as_str())?
    .field("reroot", co.root.as_str())?;
    if let Some(r) = &co.remote {
      draft = draft.field("remote", r.as_str())?;
    }
    home.publish(&state.node, draft)?;
    state.index.lock().expect("index lock").refresh(&home)?;
    eprintln!(
      "project {label}: its repository was re-created from a new root ({short}): following it to {}",
      co.top.display()
    );
    self.saw_checkout(repo, &co.top);
    Ok(())
  }

  /// Where a thread's shell sessions start, the client being in `cwd`: a
  /// global `thread` with no directory of the client's has its working
  /// folder (`thread_folder`), made here the first time.
  pub(crate) fn start_dir(
    &self,
    place: &Place,
    cwd: Option<&Path>,
    thread: Option<&RecordId>,
  ) -> Option<PathBuf> {
    let dir = self.place_dir(place);
    let start = place::start_dir(dir, cwd);
    let Some(thread) = thread.filter(|_| place == &Place::Global) else {
      return start;
    };
    // On a phone the home directory is the app's container, which cannot
    // be written: never a start, even named (a spawn's `~`).
    let container = |d: &Path| {
      !stitch_shell::CAN_SPAWN
        && std::env::var_os("HOME").is_some_and(|h| {
          let h = PathBuf::from(h);
          d == h || std::fs::canonicalize(&h).is_ok_and(|h| d == h)
        })
    };
    if start
      .as_deref()
      .is_some_and(|d| d.is_dir() && !container(d))
    {
      return start;
    }
    let folder = self.thread_folder(thread);
    match std::fs::create_dir_all(&folder) {
      Ok(()) => Some(folder),
      Err(e) => {
        eprintln!(
          "thread {thread}: no working folder {}: {e}",
          folder.display()
        );
        start
      }
    }
  }

  /// The working folder of a thread with no place (design §2.17), in the
  /// data directory: the thread's conversation's, so its forks and
  /// compactions, which read it, have it too.
  pub(crate) fn thread_folder(&self, thread: &RecordId) -> PathBuf {
    self
      .data
      .join(THREAD_FOLDERS)
      .join(crate::fork::origin(self, thread).as_str())
  }

  /// Where a loop with no directory known starts: the home directory, but
  /// on a phone, whose home is the app's container, which cannot be
  /// written, a working folder shared by such loops.
  fn fallback_dir(&self) -> Option<PathBuf> {
    if stitch_shell::CAN_SPAWN {
      return std::env::var_os("HOME").map(PathBuf::from);
    }
    let dir = self.data.join(THREAD_FOLDERS).join("_");
    std::fs::create_dir_all(&dir).ok().map(|()| dir)
  }

  /// The latest thread of a place, else the latest global one.
  pub(crate) fn last_thread(&self, here: &Place) -> Result<Option<RecordId>> {
    let state = self.state();
    let home = state.home().map(|h| h.repo().clone());
    let threads = state.index.lock().expect("index lock").listed_threads()?;
    Ok(
      place::ordered(threads, home.as_ref(), here, false, &self.places())
        .into_iter()
        .next()
        .map(|(r, ..)| r.id),
    )
  }
}

/// Takes the data directory's lock and writes this process's ID into it,
/// or says which process holds it.
/// `stitch mode` (and `stitch yolo`): the approval mode, changed to `to`
/// (`ask`, `auto`, `yolo`; design §2.28) when given. A thread's shell asks
/// the user first; nothing else of the node's may change it.
/// In a place whose settings set their own mode (`cwd`'s), the lines say
/// that mode first, then the node's.
async fn set_mode(
  s: &Arc<Shared>,
  peer: crate::peer::Peer,
  to: Option<&str>,
  cwd: Option<&str>,
) -> Result<Reply> {
  if let Some(to) = to {
    let stored = match to.trim() {
      "ask" | "default" => None,
      "auto" => Some(crate::rules::AUTO),
      "yolo" | "allow" => Some("allow"),
      other => {
        return Err(Error::Config(format!("mode `{other}`: ask, auto or yolo")));
      }
    };
    match crate::peer::identify(s, peer).await {
      crate::peer::Caller::User => {}
      crate::peer::Caller::Thread(t) => {
        crate::hazard::ask_user(
          s,
          &t,
          &format!("stitch mode {to}: the approval mode of every thread here"),
        )
        .await?;
      }
      crate::peer::Caller::Node => {
        return Err(Error::Config(
          "the approval mode is the user's to set: from their terminal or app".into(),
        ));
      }
    }
    s.set_approval(stored)?;
  }
  let node = s.approval();
  let place = s.place_of_dir(cwd).0;
  let own = match &place {
    Place::Global => None,
    p => s.place_files(p).settings.approval,
  };
  Ok(Reply::Lines(match own {
    None => vec![mode_line(&node)],
    Some(a) => {
      let here = a.effective(&node, &approval_mode(None));
      let line = mode_line(&here);
      // Without its hint: `stitch mode` changes the node's, not this.
      let line = match line.rfind(" (stitch mode") {
        Some(i) if line.ends_with(')') => line[..i].to_owned(),
        _ => line,
      };
      let file = s.place_dir(&place).map_or_else(
        || ".stitch/settings.toml".to_owned(),
        |d| d.join(".stitch/settings.toml").display().to_string(),
      );
      vec![
        line,
        format!(
          "this place sets it (approval = \"{}\" in {file}), over the node's mode",
          a.as_str()
        ),
        format!("the node's mode, elsewhere: {}", mode_line(&node)),
      ]
    }
  }))
}

/// What the approval mode is, in a line.
pub(crate) fn mode_line(mode: &str) -> String {
  match mode {
    "allow" => {
      "yolo: every command runs without asking, unjudged (stitch mode ask|auto)".to_owned()
    }
    crate::rules::AUTO => "auto: commands run without asking unless the hazard guard judges they \
      could do severe damage that cannot be undone (then they ask); without its verdict they \
      do not run (stitch mode ask|yolo)"
      .to_owned(),
    "off" => "the shell is off ([shell] approval = \"off\")".to_owned(),
    "sandbox" => {
      "ask: commands run in a sandbox; what reaches beyond it asks first (stitch mode auto|yolo)"
        .to_owned()
    }
    _ => "ask: commands outside the allowed set ask first (stitch mode auto|yolo)".to_owned(),
  }
}

/// The shell's approval mode, `configured` or not: `sandbox` by default
/// where shells can be sandboxed, `ask` elsewhere, and `ask` for a
/// `sandbox` this machine cannot give (said once in the log).
fn approval_mode(configured: Option<String>) -> String {
  let sandbox = crate::tools::sandbox_available();
  match configured {
    Some(mode) if mode == "sandbox" && !sandbox => "ask".into(),
    Some(mode) => mode,
    None if sandbox => "sandbox".into(),
    None => "ask".into(),
  }
}

fn lock(data: &Path) -> Result<std::fs::File> {
  let mut f = probe(data)?;
  let path = data.join("node.lock");
  f.set_len(0).map_err(Error::io(path.display()))?;
  f.rewind().map_err(Error::io(path.display()))?;
  write!(f, "{}", std::process::id()).map_err(Error::io(path.display()))?;
  f.flush().map_err(Error::io(path.display()))?;
  Ok(f)
}

/// Takes the lock without writing anything; it is released on drop.
fn probe(data: &Path) -> Result<std::fs::File> {
  let path = data.join("node.lock");
  let mut f = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(&path)
    .map_err(Error::io(path.display()))?;
  match f.try_lock() {
    Ok(()) => Ok(f),
    Err(std::fs::TryLockError::WouldBlock) => {
      let mut pid = String::new();
      let _ = f.read_to_string(&mut pid);
      Err(Error::AlreadyRunning(pid.trim().to_owned()))
    }
    Err(std::fs::TryLockError::Error(e)) => Err(Error::io(path.display())(e)),
  }
}

/// What the node holding the data directory's lock wrote in it (its
/// pid), if one holds it. Only reads: the lock file is opened read-only
/// and never created, so a caller that may not write the data directory
/// (a sandboxed shell: `stitch node builds` in a fixer's) still sees the
/// node; a lock file that is not there means no node.
fn holder(data: &Path) -> Option<String> {
  let mut f = std::fs::File::open(data.join("node.lock")).ok()?;
  match f.try_lock() {
    // Free: no node. Dropping the file releases the lock at once, but
    // for a process forked meanwhile, until it executes: a start right
    // then waits it out (`LOCK_SETTLE`).
    Ok(()) => None,
    Err(std::fs::TryLockError::WouldBlock) => {
      let mut pid = String::new();
      let _ = f.read_to_string(&mut pid);
      Some(pid.trim().to_owned())
    }
    Err(std::fs::TryLockError::Error(_)) => None,
  }
}

/// Whether a node holds the data directory's lock.
pub fn running(data: &Path) -> bool {
  holder(data).is_some()
}

/// The process ID of the node holding the lock, if one does.
pub fn node_pid(data: &Path) -> Option<u32> {
  holder(data)?.parse().ok()
}

/// Raises the process's soft limit on open files to its hard limit (at
/// most 10,240, macOS's `OPEN_MAX`), once. macOS starts programs with 256:
/// a node's shells, peers, MCP servers, SQLite and the browser go past it,
/// as do the tests' nodes running side by side ("Too many open files").
#[allow(unsafe_code)]
fn raise_open_files() {
  #[cfg(unix)]
  {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
      // SAFETY: getrlimit and setrlimit read and write the struct given.
      unsafe {
        let mut lim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
          return;
        }
        let want = lim.rlim_max.min(10_240);
        if lim.rlim_cur < want {
          lim.rlim_cur = want;
          libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
        }
      }
    });
  }
}

/// Starts the node for `data` and returns once it serves its socket.
pub async fn start(data: &Path) -> Result<NodeHandle> {
  std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
  let lock = lock_settled(data, LOCK_SETTLE).await?;
  start_inner(data, lock).await
}

/// How long `start` lets a busy lock come free before it says another
/// node runs. The lock is an `flock` on an open file, and a process forked
/// anywhere in this one (a shell, a server, a build) holds a copy of it
/// until it executes its program: a node started in the process where
/// another just stopped (the tests' restarts) found it held for that
/// moment now and then (CI on 2026-11-04: `AlreadyRunning` with the
/// process's own pid, no holder left to see a moment later).
const LOCK_SETTLE: Duration = Duration::from_secs(1);

/// `lock`, trying again while the lock is held, for at most `settle`.
async fn lock_settled(data: &Path, settle: Duration) -> Result<std::fs::File> {
  let start = std::time::Instant::now();
  loop {
    match lock(data) {
      Err(Error::AlreadyRunning(_)) if start.elapsed() < settle => {
        tokio::time::sleep(Duration::from_millis(20)).await;
      }
      r => return r,
    }
  }
}

/// The data directory's lock, taken for a node about to start: held from
/// before the node's own start (its process checks and keeps its build
/// first), so no client starts another meanwhile.
pub struct NodeLock(std::fs::File);

/// Takes the lock, waiting while another node holds it (at most
/// `timeout`; `None`: as long as it takes).
pub fn lock_wait(data: &Path, timeout: Option<Duration>) -> Result<NodeLock> {
  std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
  let start = std::time::Instant::now();
  loop {
    match lock(data) {
      Ok(f) => return Ok(NodeLock(f)),
      Err(Error::AlreadyRunning(pid)) if timeout.is_none_or(|t| start.elapsed() < t) => {
        let _ = pid;
        std::thread::sleep(Duration::from_millis(200));
      }
      Err(e) => return Err(e),
    }
  }
}

/// `start`, with the lock already taken (`lock_wait`).
pub async fn start_locked(data: &Path, lock: NodeLock) -> Result<NodeHandle> {
  start_inner(data, lock.0).await
}

async fn start_inner(data: &Path, lock: std::fs::File) -> Result<NodeHandle> {
  let _ = crate::selfimprove::NODE_STARTED.set(std::time::Instant::now());
  raise_open_files();
  let socket = if cfg!(mobile) {
    None
  } else {
    Some(socket_path(data)?)
  };
  let config = Config::load(data)?;
  let node = config.node_name()?;

  let mut index = Index::open(&data.join("index.sqlite"))?;
  let mut roots = BTreeMap::new();
  let mut dirs: Vec<PathBuf> = index.roots()?.into_values().map(PathBuf::from).collect();
  // No home workspace where the index has it, but one where `setup` puts
  // it: the data directory moved (an iOS app's container does when the app
  // is updated). Indexing it again records where it is now.
  let home_opens = dirs
    .iter()
    .any(|d| RecordRoot::open(d).is_ok_and(|r| r.config().role == Role::Home));
  if !home_opens {
    dirs.push(crate::setup::default_home(data));
  }
  for dir in dirs {
    if let Ok(root) = RecordRoot::open(&dir) {
      // Turns cut short when the node last stopped: their steps, sealed.
      let sealed = root.seal_all()?;
      if sealed > 0 {
        eprintln!("{}: {sealed} turn(s) left open sealed", root.repo());
      }
      index.refresh(&root)?;
      roots.insert(root.repo().clone(), root);
    }
  }
  // Segments one per turn: a root from before, migrated once (design §2.5).
  let referenced: std::collections::HashSet<String> = index
    .records(None)?
    .iter()
    .filter_map(|r| {
      r.fields
        .get("continues_at")
        .and_then(serde_json::Value::as_str)
    })
    .map(str::to_owned)
    .collect();
  for root in roots.values() {
    if let Some(m) = crate::migrate::migrate(
      &mut index,
      root,
      &referenced,
      &data.join("migrated-segments"),
    )? {
      eprintln!(
        "{}: segments one per turn: {} threads, {} segments now {} (the files before are in {}){}",
        root.repo(),
        m.threads,
        m.before,
        m.after,
        data.join("migrated-segments").display(),
        if m.kept.is_empty() {
          String::new()
        } else {
          format!("; left as they were: {}", m.kept.join(", "))
        }
      );
    }
  }
  let peers_path = data.join("peers.toml");
  let policy = Policy {
    blobs: match config.blobs.as_deref() {
      Some("lazy") => BlobPolicy::Lazy,
      _ => BlobPolicy::All,
    },
  };
  let mut state = NodeState::new(
    node,
    roots,
    index,
    BlobStore::open(&data.join("blobs"))?,
    Peers::load(&peers_path)?,
    Some(peers_path),
    policy,
  )
  .with_projects(data.join("projects"));
  // A peer's sync that brought records: answers, cancellations,
  // assignments and memory for this node.
  let (synced, mut arrived) = mpsc::unbounded_channel::<()>();
  state.observer = Some(Box::new(move |e| {
    if let stitch_net::Event::Session { result: Ok(r), .. } = e
      && !r.received.is_empty()
    {
      let _ = synced.send(());
    }
  }));
  // What comes into the repositories, pushed to the other nodes
  // (`push_changes`).
  let (changed, changes) = mpsc::unbounded_channel::<stitch_store::Change>();
  state
    .index
    .lock()
    .expect("index lock")
    .changes()
    .subscribe(move |c| {
      let _ = changed.send(c.clone());
    });
  state.hold_projects()?;
  let discovery = match config.discovery.as_deref() {
    Some("local") => Discovery::Local,
    _ => Discovery::N0,
  };
  let key = load_or_create_key(&data.join("node.key"))?;
  let endpoint = bind(key, discovery, config.port).await?;
  let endpoint_id = endpoint.id().to_string();
  let net = Net::spawn(endpoint, Arc::new(state));

  let listener = match &socket {
    Some(socket) => {
      let _ = std::fs::remove_file(socket);
      let listener = UnixListener::bind(socket).map_err(Error::io(socket.display()))?;
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
          .map_err(Error::io(socket.display()))?;
      }
      Some(listener)
    }
    None => None,
  };
  let (shutdown, mut stopped) = watch::channel(false);
  // SIGTERM (`stitch node stop` when the socket cannot be used, launchd,
  // systemd) stops the node as cleanly as a Stop request.
  #[cfg(unix)]
  {
    let stop = shutdown.clone();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
      .map_err(Error::io("SIGTERM handler"))?;
    tokio::spawn(async move {
      if term.recv().await.is_some() {
        let _ = stop.send(true);
      }
    });
  }
  let web = Arc::new(crate::web::Web::new(
    data,
    config.browser.clone().unwrap_or_default(),
    config.search.clone().unwrap_or_default(),
  ));
  let browsers = crate::browser::Browsers::new(data, config.browser.clone().unwrap_or_default());
  let memory = Arc::new(crate::memory::Memory::new(
    data,
    &config.memory.clone().unwrap_or_default(),
  ));
  let approval = approval_mode(config.shell.as_ref().and_then(|s| s.approval.clone()));
  let grants = config
    .shell
    .as_ref()
    .map(|s| {
      s.places
        .iter()
        .map(|(k, v)| (k.clone(), v.allow.clone()))
        .collect()
    })
    .unwrap_or_default();
  let mcp_grants = config
    .mcp
    .as_ref()
    .map(|m| {
      m.places
        .iter()
        .map(|(k, v)| (k.clone(), v.allow.clone()))
        .collect()
    })
    .unwrap_or_default();
  let otter_grants = config
    .otter
    .as_ref()
    .map(|s| {
      s.places
        .iter()
        .map(|(k, v)| (k.clone(), v.allow.clone()))
        .collect()
    })
    .unwrap_or_default();
  let vault = Arc::new(crate::secrets::Vault::default());
  {
    let index = net.state().index.lock().expect("index lock");
    vault.reload(data, &config, Some(&index));
  }
  let shared = Arc::new(Shared {
    data: data.to_owned(),
    config,
    net,
    shutdown: shutdown.clone(),
    shells: Arc::new(Shells::default()),
    detached: Default::default(),
    checkouts: Mutex::new(place::Checkouts::load(data)),
    web,
    browsers,
    memory,
    mcp: crate::mcp::Mcp::with_vault(data, vault.clone()),
    scripts: Default::default(),
    publishing: tokio::sync::Mutex::new(()),
    models: Default::default(),
    windows: Default::default(),
    approval: Mutex::new(approval),
    grants: Mutex::new(grants),
    mcp_grants: Mutex::new(mcp_grants),
    otter_grants: Mutex::new(otter_grants),
    chats: AtomicUsize::new(0),
    remote_calls: AtomicUsize::new(0),
    upgrading: AtomicBool::new(false),
    pending: Mutex::new(None),
    pending_since: Mutex::new(None),
    restart_notice: Mutex::new(None),
    cut: Mutex::new(None),
    shells_hold: Mutex::new(None),
    restart: Mutex::new(None),
    folder_sync: Default::default(),
    clones: Default::default(),
    pusher: Default::default(),
    #[cfg(not(any(target_os = "ios", target_os = "visionos")))]
    toasts: Default::default(),
    apps: Default::default(),
    schedules: Default::default(),
    events: Default::default(),
    loose_ends: Default::default(),
    decide: Default::default(),
    digests: Default::default(),
    rooms: Default::default(),
    list_reads: tokio::sync::Semaphore::new(LIST_READS),
    notices: Default::default(),
    rings: Default::default(),
    vault,
    ssh: Default::default(),
    live: Default::default(),
  });
  let _ = shared.detached.since.set(jiff::Timestamp::now());
  // Apps in a browser on this machine; a node that cannot have its port
  // runs on without.
  #[cfg(not(mobile))]
  {
    let port = shared
      .config
      .apps
      .as_ref()
      .and_then(|a| a.port)
      .unwrap_or(crate::apps::http::PORT);
    match crate::apps::http::start(&shared, port, shutdown.subscribe()).await {
      Ok(Some(d)) => {
        let _ = shared.apps.desktop.set(d);
      }
      Ok(None) => {}
      Err(e) => eprintln!("{e}"),
    }
  }
  // A dashboard pinned from an older built-in moves to this one's.
  match crate::apps::live::upgrade_dashboard(&shared) {
    Ok(true) => crate::apps::sync_home(&shared),
    Ok(false) => {}
    Err(e) => eprintln!("apps: the dashboard not moved to this version: {e}"),
  }
  // The web client (design §2.24) is pinned, or moved to this build's.
  match crate::apps::live::ensure_client(&shared) {
    Ok(true) => crate::apps::sync_home(&shared),
    Ok(false) => {}
    Err(e) => eprintln!("apps: the web client not pinned: {e}"),
  }
  // A background job tells its thread when it ends; a person's typing is
  // woven into a running loop only.
  let weak = Arc::downgrade(&shared);
  shared
    .shells
    .set_notifier(Arc::new(move |thread: RecordId, text: String| {
      if let Some(s) = weak.upgrade() {
        // A job's last lines: no secret in them (design §2.19).
        let text = s.vault.redact(&text);
        crate::detached::notify(&s, &thread, text, Vec::new());
      }
    }));
  let weak = Arc::downgrade(&shared);
  shared.shells.set_weaver(Arc::new(
    move |thread: RecordId, text: String, untold: chat::Untold| {
      weak.upgrade().is_some_and(|s| {
        let text = s.vault.redact(&text);
        crate::detached::weave(&s, &thread, text, untold)
      })
    },
  ));
  let weak = Arc::downgrade(&shared);
  shared.state().set_answerer(Box::new(move |peer, question| {
    let s = weak.clone();
    Box::pin(async move {
      match s.upgrade() {
        Some(s) => answer(&s, &peer, &question).await,
        None => {
          postcard::to_stdvec(&Reply::Error("the node is stopping".into())).unwrap_or_default()
        }
      }
    })
  }));
  // Memory folds what the records hold, then sleeps once a day fleet-wide.
  if shared.memory.available() {
    let s = shared.clone();
    let mut stop = shutdown.subscribe();
    tokio::spawn(async move {
      let mut tick = tokio::time::interval(Duration::from_secs(3600));
      loop {
        tokio::select! {
          _ = tick.tick() => {
            let (m, st) = (s.memory.clone(), s.state().clone());
            let _ = tokio::task::spawn_blocking(move || m.refresh(&st)).await;
            sleep_memory(&s).await;
          }
          _ = stop.changed() => break,
        }
      }
    });
  }
  // App containers an earlier run of this node left (conmon keeps them
  // past a crash): removed, this node's only (design §2.23).
  #[cfg(target_os = "linux")]
  {
    let s = shared.clone();
    tokio::spawn(async move {
      let n = crate::apps::containers::storage::clean_at_start(&s).await;
      if n > 0 {
        eprintln!("app containers: {n} left by an earlier run removed");
      }
    });
  }
  // Detached threads this node started and that had not ended resume;
  // assignments for it start.
  {
    let s = shared.clone();
    tokio::spawn(async move {
      crate::detached::resume_all(&s).await;
      // Threads whose shells the last stop cut are told.
      tell_cut_shells(&s);
      // Questions an earlier run left open that no resumed thread takes up.
      let n = crate::detached::sweep(&s);
      if n > 0 {
        eprintln!("approvals: {n} left open by an earlier run withdrawn");
      }
      crate::weave::claim_all(&s).await;
      crate::remote::claim_calls(&s);
      crate::weave::deliver(&s);
    });
  }
  // Questions whose asking ended without withdrawing them, now and then.
  {
    let s = shared.clone();
    let mut stop = shutdown.subscribe();
    tokio::spawn(async move {
      let mut tick = tokio::time::interval(Duration::from_secs(60));
      tick.tick().await;
      loop {
        tokio::select! {
          _ = tick.tick() => {
            let n = crate::detached::sweep(&s);
            if n > 0 {
              eprintln!("approvals: {n} nothing asked any more withdrawn");
            }
          }
          _ = stop.changed() => break,
        }
      }
    });
  }
  // Deletions that arrived while it was stopped, carried out.
  {
    let s = shared.clone();
    tokio::spawn(async move { crate::delete::honour(&s).await });
  }
  // The schedules pinned here fire as they come due (design §2.3).
  tokio::spawn(crate::schedules::run(shared.clone(), shutdown.subscribe()));
  // Loose ends: the free gate hourly, a model only on what it lets
  // through (design §2.10).
  tokio::spawn(crate::loose::run(shared.clone(), shutdown.subscribe()));
  // Digests' backfill: what has none or a stale one, a few minutes after
  // the start and hourly (design §2.25).
  tokio::spawn(crate::digests::run(shared.clone(), shutdown.subscribe()));
  // Secrets: keys stored before listed, what arrived while stopped opened,
  // what nodes paired since lack sealed for them (design §2.19).
  {
    let s = shared.clone();
    tokio::spawn(async move { crate::secrets::after_sync(&s).await });
  }
  // What this node shares, published: its shared servers start for it;
  // published again as their tools change or a failed one is retried.
  tokio::spawn(crate::remote::follow(shared.clone(), shutdown.subscribe()));
  {
    let s = shared.clone();
    let mut stop = shutdown.subscribe();
    tokio::spawn(async move {
      loop {
        tokio::select! {
          got = arrived.recv() => {
            if got.is_none() {
              break;
            }
            // One pass for a burst of sessions.
            while arrived.try_recv().is_ok() {}
            crate::weave::after_sync(&s).await;
          }
          _ = stop.changed() => break,
        }
      }
    });
  }

  // Every change pushed to the peers within seconds (design §2.6); what
  // another process indexed found by polling.
  tokio::spawn(push_changes(shared.clone(), changes, shutdown.subscribe()));
  {
    let s = shared.clone();
    let mut stop = shutdown.subscribe();
    tokio::spawn(async move {
      let mut tick = tokio::time::interval(POLL_CHANGES);
      loop {
        tokio::select! {
          _ = tick.tick() => {
            let st = s.state().clone();
            let _ = tokio::task::spawn_blocking(move || {
              st.index.lock().expect("index lock").poll_changes()
            })
            .await;
          }
          _ = stop.changed() => break,
        }
      }
    });
  }

  let interval = shared.config.sync_interval_secs.unwrap_or(300);
  if interval > 0 {
    let s = shared.clone();
    let mut stop = shutdown.subscribe();
    tokio::spawn(async move {
      let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(10),
        Duration::from_secs(interval),
      );
      loop {
        tokio::select! {
          _ = tick.tick() => { sync_all(&s, None).await; }
          _ = stop.changed() => break,
        }
      }
    });
  }

  let sock = socket.clone();
  let served = shared.clone();
  let task = tokio::spawn(async move {
    let _lock = lock;
    let shared = served;
    let accept = async |l: &Option<UnixListener>| match l {
      Some(l) => l.accept().await,
      None => std::future::pending().await,
    };
    loop {
      tokio::select! {
        accepted = accept(&listener) => {
          if let Ok((stream, _)) = accepted {
            let s = shared.clone();
            // Who connects, from the socket (never from what it says).
            let peer = crate::peer::Peer::Process(
              stream.peer_cred().ok().and_then(|c| c.pid()).map(|p| p as u32),
            );
            tokio::spawn(async move {
              let (r, w) = stream.into_split();
              if let Err(e) = serve(&s, r, w, peer).await {
                eprintln!("client: {e}");
              }
            });
          }
        }
        _ = stopped.changed() => break,
      }
    }
    if let Some(sock) = &sock {
      let _ = std::fs::remove_file(sock);
    }
    shared.detached.abort_all();
    // What the loops stopped had written, sealed.
    for root in shared.state().roots() {
      let mut index = shared.state().index.lock().expect("index lock");
      match root.seal_all() {
        Ok(0) => {}
        Ok(_) => {
          let _ = index.refresh(&root);
        }
        Err(e) => eprintln!("{}: open turns not sealed: {e}", root.repo()),
      }
    }
    // No more questions from other nodes, and those being answered lose
    // their asker (what they run is dropped), before the servers stop,
    // which then start for nothing (`Mcp::stop_all`).
    shared.net.forget_connections();
    shared.net.endpoint().close().await;
    shared.web.shutdown().await;
    shared.mcp.stop_all().await;
    // What runs in threads' shells is cut: each is told at the next start.
    keep_cut_shells(&shared).await;
    shared.shells.close_all().await;
    shared.ssh.close_all().await;
    Ok(())
  });
  Ok(NodeHandle {
    shutdown,
    task,
    shared,
    socket,
    endpoint: endpoint_id,
  })
}

/// Memory's sleep, when the fleet is due one, and the gist it may bring
/// (edge talk's REM: one model call a cycle at most).
pub(crate) async fn sleep_memory(s: &Arc<Shared>) {
  if !s.memory.available() || s.memory.suspended() {
    return;
  }
  let (m, st) = (s.memory.clone(), s.state().clone());
  let slept = tokio::task::spawn_blocking(move || m.maybe_sleep(&st))
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or(false);
  if slept {
    let configured = s
      .config
      .memory
      .as_ref()
      .and_then(|m| m.gist_model.as_deref());
    let model = crate::modelset::named(s, None, Some(configured.unwrap_or("quick"))).await;
    if let Err(e) = s.memory.gist(s.state(), &s.config, &s.data, &model).await {
      eprintln!("memory: no gist: {e}");
    }
  }
}

/// Syncs every held repository with every known peer (or the one named),
/// all peers at once: a summary each (one round trip), then a session for
/// each repository that differs. A line for each repository synced and
/// for each peer, and one line in the log for the whole sync.
pub(crate) async fn sync_all(s: &Arc<Shared>, only: Option<&str>) -> Vec<String> {
  let started = std::time::Instant::now();
  let peers: Vec<_> = known_peers(s)
    .await
    .into_iter()
    .filter(|p| only.is_none_or(|o| p.name == o || p.id == o))
    .collect();
  if let Some(o) = only
    && peers.is_empty()
  {
    return vec![format!("no peer `{o}`")];
  }
  let state = s.state().clone();
  let _ = tokio::task::spawn_blocking(move || state.hold_projects()).await;
  crate::folders::hold_all(s);
  let repos = held_repos(s);
  let mut lines = Vec::new();
  let mut tally = Tally::default();
  let reports = sync_peers(s, &peers, &repos, true).await;
  // The home workspace may list projects new here: they are held, and
  // synced with the peers that brought them.
  let state = s.state().clone();
  let new = tokio::task::spawn_blocking(move || state.hold_projects())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or(0);
  crate::folders::hold_all(s);
  let more: Vec<RepoId> = held_repos(s)
    .into_iter()
    .filter(|r| !repos.contains(r))
    .collect();
  let mut reports = reports;
  if new > 0 || !more.is_empty() {
    let reached: Vec<_> = peers
      .iter()
      .zip(&reports)
      .filter(|(_, r)| r.is_ok())
      .map(|(p, _)| p.clone())
      .collect();
    let again = sync_peers(s, &reached, &more, true).await;
    for (p, r) in reached.iter().zip(again) {
      if let (Some(i), Ok(r)) = (peers.iter().position(|q| q.name == p.name), r)
        && let Ok(first) = &mut reports[i]
      {
        first.synced.extend(r.synced);
        first.same.extend(r.same);
        first.not_held.extend(r.not_held);
        first.elapsed += r.elapsed;
      }
    }
  }
  for (peer, report) in peers.iter().zip(&reports) {
    tally.add(peer, report, &mut lines);
  }
  tally.log(s, "sync", started.elapsed());
  // What arrived may hold memory, answers, cancellations, assignments.
  crate::weave::after_sync(s).await;
  lines
}

/// The trusted peers, read off the runtime.
async fn known_peers(s: &Shared) -> Vec<stitch_net::Peer> {
  let state = s.state().clone();
  tokio::task::spawn_blocking(move || state.known_peers())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default()
}

/// Every repository held here, the home workspace first.
fn held_repos(s: &Shared) -> Vec<RepoId> {
  let mut roots = s.state().roots();
  roots.sort_by_key(|r| r.config().role != Role::Home);
  roots.into_iter().map(|r| r.repo().clone()).collect()
}

/// Syncs `repos` with each peer, all at once; each peer's report, in the
/// peers' order.
async fn sync_peers(
  s: &Shared,
  peers: &[stitch_net::Peer],
  repos: &[RepoId],
  seal: bool,
) -> Vec<Result<stitch_net::PeerReport, String>> {
  let how = stitch_net::PeerSync {
    seal,
    connect: SYNC_CONNECT,
    parallel: SYNC_PARALLEL,
  };
  futures_util::future::join_all(peers.iter().map(|peer| async move {
    let addr = peer.addr().map_err(|e| e.to_string())?;
    match tokio::time::timeout(SYNC_LIMIT, s.net.sync_peer(&addr, repos, how)).await {
      Err(_) => Err(format!("still syncing after {}s", SYNC_LIMIT.as_secs())),
      Ok(Err(stitch_net::AskError::Unreachable(e))) => Err(e),
      Ok(Err(stitch_net::AskError::Failed(e))) => Err(e.to_string()),
      Ok(Ok(r)) => Ok(r),
    }
  }))
  .await
}

/// What a sync did with each peer, for its line in the log.
#[derive(Default)]
struct Tally {
  peers: Vec<(String, PeerTally)>,
}

#[derive(Default)]
struct PeerTally {
  repos: usize,
  same: usize,
  received: usize,
  sent: usize,
  blobs: usize,
  took: Duration,
  version: u32,
  failed: Option<String>,
  errors: usize,
}

impl Tally {
  /// A peer's report: its lines (each repository synced, then the peer),
  /// and its part of the log line.
  fn add(
    &mut self,
    peer: &stitch_net::Peer,
    report: &Result<stitch_net::PeerReport, String>,
    lines: &mut Vec<String>,
  ) {
    let mut t = PeerTally::default();
    match report {
      Err(why) => {
        lines.push(format!("{}: {why}", peer.name));
        // An offline peer is news on a phone, every interval's noise
        // elsewhere: the log line says it once.
        t.failed = Some(why.clone());
      }
      Ok(r) => {
        t.version = r.version;
        t.same = r.same.len();
        t.took = r.elapsed;
        for (repo, result) in &r.synced {
          t.repos += 1;
          match result {
            Ok(x) => {
              t.received += x.received.len();
              t.sent += x.sent;
              t.blobs += x.blobs_received + x.blobs_sent;
              lines.push(format!(
                "{} {repo}: {} in, {} out, blobs {} in, {} out",
                peer.name,
                x.received.len(),
                x.sent,
                x.blobs_received,
                x.blobs_sent
              ));
            }
            Err(e) => {
              t.errors += 1;
              eprintln!("sync: {} {repo}: {e}", peer.name);
              lines.push(format!("{} {repo}: {e}", peer.name));
            }
          }
        }
        let mut line = format!(
          "{}: {} the same, {} synced in {:.2}s",
          peer.name,
          r.same.len(),
          r.synced.len(),
          r.elapsed.as_secs_f64()
        );
        if !r.not_held.is_empty() {
          line.push_str(&format!(", {} not held there", r.not_held.len()));
        }
        if r.version < 3 {
          line.push_str(" (an older node: every repository synced)");
        }
        lines.push(line);
      }
    }
    self.peers.push((peer.name.clone(), t));
  }

  /// One line: `sync 0.12s: geek 18 same, 2 synced 0.10s (3 in, 1 out),
  /// mbp not reached (not reached within 10s)`; each peer's time and
  /// failure kept for `stitch peer ls`. `quiet`: nothing said when no file
  /// moved and every peer was reached (a push that found all the same).
  fn log(&self, s: &Shared, what: &str, took: Duration) {
    self.log_if(s, what, took, false)
  }

  fn log_if(&self, s: &Shared, what: &str, took: Duration, quiet: bool) {
    let mut parts = Vec::new();
    let mut moved = false;
    for (name, t) in &self.peers {
      match &t.failed {
        Some(why) => {
          // A push says a peer not reached once, not at every change
          // while it stays away (a phone asleep).
          let known = s
            .state()
            .seen()
            .peer
            .get(name)
            .is_some_and(|p| p.failed.is_some());
          s.state().note_failed(name, why);
          if quiet && known {
            continue;
          }
          moved = true;
          parts.push(format!("{name} not reached ({why})"));
        }
        None => {
          s.state().note_synced(name, t.took);
          moved |= t.received + t.sent + t.blobs + t.errors > 0;
          let mut p = format!("{name} {} same", t.same);
          if t.repos > 0 {
            p.push_str(&format!(
              ", {} synced ({} in, {} out",
              t.repos, t.received, t.sent
            ));
            if t.blobs > 0 {
              p.push_str(&format!(", {} blobs", t.blobs));
            }
            if t.errors > 0 {
              p.push_str(&format!(", {} failed", t.errors));
            }
            p.push(')');
          }
          p.push_str(&format!(" {:.2}s", t.took.as_secs_f64()));
          if t.version < 3 {
            p.push_str(" v2");
          }
          parts.push(p);
        }
      }
    }
    if parts.is_empty() || (quiet && !moved) {
      return;
    }
    eprintln!("{what} {:.2}s: {}", took.as_secs_f64(), parts.join(", "));
  }

  fn received(&self) -> usize {
    self.peers.iter().map(|(_, t)| t.received).sum()
  }
}

/// Pushes what comes into the repositories (the index's change feed) to
/// every peer within seconds: changes gathered until a second passes
/// without one (three at most), then each peer gets a summary of the
/// repositories changed and a session for those that differ (design §2.6).
/// What came from a peer goes on to the others, not back. A peer not
/// reached is left to the interval's sync for a minute, unless it comes
/// back meanwhile; one push at a time per peer.
///
/// It holds the node only while it pushes: a node stopped lets go of its
/// endpoint (and its port) at once, not after a push being gathered.
async fn push_changes(
  s: Arc<Shared>,
  mut changes: mpsc::UnboundedReceiver<stitch_store::Change>,
  mut stop: watch::Receiver<bool>,
) {
  let weak = Arc::downgrade(&s);
  drop(s);
  use std::collections::{BTreeMap, BTreeSet};
  let busy: Arc<std::sync::Mutex<BTreeSet<String>>> = Arc::default();
  // Changes waiting for a peer that was busy: its next push takes them.
  let waiting: Arc<std::sync::Mutex<BTreeMap<String, BTreeSet<RepoId>>>> = Arc::default();
  loop {
    let first = tokio::select! {
      c = changes.recv() => match c { Some(c) => c, None => return },
      _ = stop.changed() => return,
    };
    // Repository → the peers its files came from (`None`: this node).
    let mut batch: BTreeMap<RepoId, BTreeSet<Option<String>>> = BTreeMap::new();
    let Some(s) = weak.upgrade() else { return };
    let home = s.state().home().map(|h| h.repo().clone());
    let state = s.state().clone();
    drop(s);
    let mut add = |c: stitch_store::Change| {
      if home.as_ref() == Some(&c.repo) {
        // A `revoke` may have come: a node no longer trusted is cut off.
        let st = state.clone();
        tokio::task::spawn_blocking(move || st.recheck_trust());
      }
      batch.entry(c.repo).or_default().insert(c.origin);
    };
    add(first);
    let until = tokio::time::Instant::now() + PUSH_GATHER_MAX;
    loop {
      let quiet = tokio::time::Instant::now() + PUSH_GATHER;
      tokio::select! {
        got = tokio::time::timeout_at(quiet.min(until), changes.recv()) => match got {
          Ok(Some(c)) => add(c),
          Ok(None) => return,
          Err(_) => break,
        },
        _ = stop.changed() => return,
      }
    }
    drop(state);
    let Some(s) = weak.upgrade() else { return };
    if *stop.borrow() {
      return;
    }
    let peers = known_peers(&s).await;
    for peer in peers {
      let mut repos: BTreeSet<RepoId> = batch
        .iter()
        .filter(|(_, from)| {
          from
            .iter()
            .any(|o| o.as_deref() != Some(peer.name.as_str()))
        })
        .map(|(r, _)| r.clone())
        .collect();
      {
        let mut busy = busy.lock().expect("busy lock");
        if busy.contains(&peer.name) {
          waiting
            .lock()
            .expect("waiting lock")
            .entry(peer.name.clone())
            .or_default()
            .extend(repos);
          continue;
        }
        if let Some(more) = waiting.lock().expect("waiting lock").remove(&peer.name) {
          repos.extend(more);
        }
        if repos.is_empty() {
          continue;
        }
        busy.insert(peer.name.clone());
      }
      let (weak, stop, busy, waiting) = (
        Arc::downgrade(&s),
        stop.clone(),
        busy.clone(),
        waiting.clone(),
      );
      tokio::spawn(push_to(weak, stop, peer, repos, busy, waiting));
    }
  }
}

/// Pushes `repos` to one peer, then what came for it meanwhile
/// (`waiting`), until nothing is left; `busy` holds its name meanwhile.
/// A peer not reached is tried again: first when its last failure is a
/// minute old (or it reaches this node), then after 5 seconds, doubling
/// to a minute, for half an hour; then the interval's sync, the peer
/// reaching this node or the next change takes it from there. So an
/// answer the user gave reaches a node back from a restart or a sleep
/// within seconds, not at the next interval's sync. It holds the node
/// only while it pushes, never while it waits.
async fn push_to(
  weak: std::sync::Weak<Shared>,
  mut stop: watch::Receiver<bool>,
  peer: stitch_net::Peer,
  mut repos: std::collections::BTreeSet<RepoId>,
  busy: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
  waiting: Arc<
    std::sync::Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<RepoId>>>,
  >,
) {
  if let Some(ago) = weak.upgrade().and_then(|s| failed_since(&s, &peer.name))
    && ago < PUSH_BACKOFF
    && !until_due(&weak, &mut stop, &peer.name, PUSH_BACKOFF - ago).await
  {
    return;
  }
  let mut retry = PUSH_RETRY_FIRST;
  let mut failing: Option<std::time::Instant> = None;
  loop {
    let Some(s) = weak.upgrade() else { return };
    if *stop.borrow() {
      return;
    }
    let started = std::time::Instant::now();
    let list: Vec<RepoId> = repos.iter().cloned().collect();
    let reports = sync_peers(&s, std::slice::from_ref(&peer), &list, false).await;
    let mut tally = Tally::default();
    tally.add(&peer, &reports[0], &mut Vec::new());
    tally.log_if(&s, "push", started.elapsed(), true);
    if tally.received() > 0 {
      crate::weave::after_sync(&s).await;
    }
    drop(s);
    let ok = reports[0].is_ok();
    let again = if ok {
      retry = PUSH_RETRY_FIRST;
      failing = None;
      false
    } else {
      failing.get_or_insert(started).elapsed() < PUSH_RETRY_FOR
    };
    if again {
      if !until_due(&weak, &mut stop, &peer.name, retry).await {
        return;
      }
      retry = (retry * 2).min(PUSH_BACKOFF);
    }
    // What came for it meanwhile goes now, with what failed when it is
    // tried again (both locks held: nothing is left waiting with nobody
    // to take it).
    let mut busy = busy.lock().expect("busy lock");
    let more = waiting.lock().expect("waiting lock").remove(&peer.name);
    match (ok, again, more) {
      (true, _, Some(more)) => repos = more,
      (false, true, more) => repos.extend(more.unwrap_or_default()),
      (false, false, Some(more)) => {
        // Given up for now: what came meanwhile waits for the next push.
        waiting
          .lock()
          .expect("waiting lock")
          .insert(peer.name.clone(), more);
        busy.remove(&peer.name);
        return;
      }
      (_, _, None) => {
        busy.remove(&peer.name);
        return;
      }
    }
  }
}

/// How long ago `peer` last failed to be reached (none: reached since).
fn failed_since(s: &Shared, peer: &str) -> Option<Duration> {
  let failed = s.state().seen().peer.get(peer)?.failed?;
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  Some(Duration::from_secs(now.saturating_sub(failed)))
}

/// Waits `wait`, or less when `peer` reaches this node meanwhile (its
/// failure cleared); false when the node stops or is gone.
async fn until_due(
  weak: &std::sync::Weak<Shared>,
  stop: &mut watch::Receiver<bool>,
  peer: &str,
  wait: Duration,
) -> bool {
  let until = tokio::time::Instant::now() + wait;
  loop {
    let now = tokio::time::Instant::now();
    if now >= until {
      return true;
    }
    tokio::select! {
      _ = tokio::time::sleep((until - now).min(Duration::from_secs(1))) => {}
      _ = stop.changed() => return false,
    }
    let Some(s) = weak.upgrade() else {
      return false;
    };
    if *stop.borrow() {
      return false;
    }
    if failed_since(&s, peer).is_none() {
      return true;
    }
  }
}

/// How long a sync waits for more changes before it pushes them.
const PUSH_GATHER: Duration = Duration::from_secs(1);
/// The longest changes are held, however many keep coming.
const PUSH_GATHER_MAX: Duration = Duration::from_secs(3);
/// How long a peer that could not be reached waits for its next push (the
/// longest wait between its retries).
const PUSH_BACKOFF: Duration = Duration::from_secs(60);
/// The first retry of a push that did not reach its peer.
const PUSH_RETRY_FIRST: Duration = Duration::from_secs(5);
/// How long a push that does not reach its peer is tried again.
const PUSH_RETRY_FOR: Duration = Duration::from_secs(1800);
/// How often what another process indexed is looked for.
const POLL_CHANGES: Duration = Duration::from_secs(5);
/// Repositories synced with one peer at once.
const SYNC_PARALLEL: usize = 4;
/// A peer's whole sync is given up after this (sessions are given up on
/// sooner when they stop moving: `stitch_net` gives each 30 seconds idle).
const SYNC_LIMIT: Duration = Duration::from_secs(1800);

/// Pushes to the user's phones (not this node): with this node's APNs key
/// when it has one, else through the relay (`[apns]`, read now so `stitch
/// apns set` applies at once); a line for each. On a desktop it is shown
/// there too (`toasts`), a line saying so first.
pub(crate) async fn push(s: &Arc<Shared>, push: &crate::apns::Push<'_>) -> Vec<String> {
  #[cfg(not(any(target_os = "ios", target_os = "visionos")))]
  let desktop = s.toasts.show(&s.data, s.state().node.as_str(), push).await;
  #[cfg(any(target_os = "ios", target_os = "visionos"))]
  let desktop: Option<String> = None;
  let mut lines: Vec<String> = desktop.into_iter().collect();
  lines.extend(push_to_phones(s, push).await);
  lines
}

async fn push_to_phones(s: &Arc<Shared>, push: &crate::apns::Push<'_>) -> Vec<String> {
  // No `[apns]`: through the app team's relay.
  let settings = Config::load(&s.data)
    .ok()
    .and_then(|c| c.apns)
    .unwrap_or_default();
  let devices: Vec<_> = crate::apns::devices(&s.state().index.lock().expect("index lock"))
    .into_iter()
    .filter(|d| d.node != s.state().node.as_str())
    .collect();
  if devices.is_empty() {
    return vec!["no phone has registered for pushes yet".into()];
  }
  s.pusher.send(&s.data, &settings, &devices, push).await
}

/// Whether a thread with `selection` is offered the tools of `group`: the
/// selection has it on, and a fixer is never given the user's own browser,
/// signed in as them. The tools and the prompt's skills list agree here.
fn group_offered(selection: &Selection, fixer: bool, group: &str) -> bool {
  selection.group_on(group) && !(fixer && group == "browser")
}

/// What the model is told of its thread's mode once the user switched it.
pub(crate) fn mode_note(read_only: bool) -> String {
  if read_only {
    "\n# This thread's mode\n\nThis thread is now read-only (switched by the user): only the \
     built-in read-only commands run (shell_run's description lists them), its shells write \
     only temporary directories, and write_file and edit_file are refused. Only the user can \
     make it writable again: suggest it in your reply when the work needs it.\n"
      .into()
  } else {
    "\n# This thread's mode\n\nThis thread is now writable (switched by the user): the rules in \
     shell_run's description apply, and write_file and edit_file work.\n"
      .into()
  }
}

/// A thread's mode as the user reads it.
pub(crate) fn mode_word(read_only: bool) -> &'static str {
  if read_only { "read-only" } else { "writable" }
}

/// Syncs one repository with every known peer now (a folder's change, so
/// the others have it at once); a line for each.
pub(crate) async fn sync_one(s: &Arc<Shared>, repo: &RepoId) -> Vec<String> {
  let started = std::time::Instant::now();
  let peers = known_peers(s).await;
  let repos = [repo.clone()];
  let reports = sync_peers(s, &peers, &repos, true).await;
  let mut tally = Tally::default();
  let mut lines = Vec::new();
  for (peer, report) in peers.iter().zip(&reports) {
    tally.add(peer, report, &mut lines);
  }
  tally.log_if(s, &format!("sync {repo}"), started.elapsed(), true);
  lines
}

/// How long a peer has to be reached for a sync: an offline one costs no
/// more than this, once per sync.
const SYNC_CONNECT: Duration = Duration::from_secs(10);

/// Fetches blobs from whichever known peer has them, several at once;
/// how many arrived.
pub(crate) async fn fetch_blobs(s: &Shared, hashes: &[BlobHash]) -> usize {
  use futures_util::stream::{FuturesUnordered, StreamExt};
  let peers: Vec<_> = s
    .state()
    .known_peers()
    .unwrap_or_default()
    .iter()
    .filter_map(|p| p.addr().ok())
    .collect();
  if peers.is_empty() {
    return 0;
  }
  let (mut got, mut next) = (0, hashes.iter());
  let mut running = FuturesUnordered::new();
  loop {
    while running.len() < BLOB_FETCHES
      && let Some(h) = next.next()
    {
      running.push(s.net.fetch_blob_any(&peers, h, BLOB_FIND));
    }
    match running.next().await {
      Some(Ok(true)) => got += 1,
      Some(_) => {}
      None => return got,
    }
  }
}

/// Blobs fetched at once.
const BLOB_FETCHES: usize = 8;
/// How long a peer has to say it holds a blob.
const BLOB_FIND: Duration = Duration::from_secs(20);

/// A file's media type: an image's from its bytes (a `Photo.jpg` may be
/// PNG, and providers refuse a label its bytes contradict), else from its
/// extension.
pub fn media_type_of(path: &Path) -> &'static str {
  use std::io::Read;
  let mut head = [0u8; 64];
  let n = std::fs::File::open(path)
    .and_then(|f| f.take(head.len() as u64).read(&mut head))
    .unwrap_or(0);
  stitch_providers::media::sniff_image(&head[..n]).unwrap_or_else(|| media_type(path))
}

/// A file's media type from its extension (`media_type_of` reads an
/// image's from its bytes).
pub fn media_type(path: &Path) -> &'static str {
  let ext = path
    .extension()
    .and_then(|e| e.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  match ext.as_str() {
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "heic" => "image/heic",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "pdf" => "application/pdf",
    "mp4" | "m4v" => "video/mp4",
    "mov" => "video/quicktime",
    "webm" => "video/webm",
    "mpeg" | "mpg" => "video/mpeg",
    "avi" => "video/x-msvideo",
    "3gp" => "video/3gpp",
    "mp3" => "audio/mpeg",
    "m4a" => "audio/mp4",
    "aac" => "audio/aac",
    "wav" => "audio/wav",
    "aif" | "aiff" => "audio/aiff",
    "ogg" | "oga" => "audio/ogg",
    "flac" => "audio/flac",
    "md" => "text/markdown",
    "txt" | "log" => "text/plain",
    "rs" | "py" | "js" | "ts" | "swift" | "c" | "h" | "cpp" | "go" | "sh" | "shs" | "toml"
    | "yaml" | "yml" => "text/plain",
    "csv" => "text/csv",
    "html" => "text/html",
    "json" => "application/json",
    "usdz" => "model/vnd.usdz+zip",
    _ => "application/octet-stream",
  }
}

/// Every unasked note's memories, each with the note's cue measures, its
/// raw and centered distance, and what came of it: `helped` (credited),
/// `dismissed` (the dismiss marker, or cited unused), `ignored` (neither),
/// `unknown` (no feedback recorded: an exchange cut short, or a note from
/// before this was kept).
fn memory_notes(s: &Arc<Shared>) -> Result<Vec<String>> {
  use serde_json::Value;
  use stitch_records::{Event, Part};
  // The outcomes, by the note they judge.
  let mut outcome: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
  let threads = {
    let index = s.state().index.lock().expect("index lock");
    for r in index.records(Some("memory"))? {
      for f in r.fields["facts"].as_array().into_iter().flatten() {
        let Some(note) = f.get("note").and_then(Value::as_str) else {
          continue;
        };
        let hexes = |k: &str| -> Vec<String> {
          f.get(k)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|h| h.as_str().map(str::to_owned))
            .collect()
        };
        let e = outcome.entry(note.to_owned()).or_default();
        e.0.extend(hexes("credit"));
        e.1.extend(hexes("penalize"));
      }
    }
    index.records(Some(stitch_records::kinds::THREAD))?
  };
  let mut lines = vec![
    "note\thandle\traw\tcentered\tcue_distinctive\tcue_chars\tnovelty\tceiling\twandering\tfresh\toutcome"
      .to_owned(),
  ];
  for t in threads {
    let Ok((root, _)) = s.thread_place(&t.id) else {
      continue;
    };
    let state = s.state();
    let ctx = chat::ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    let Ok(events) = chat::thread_events(&ctx, &t.id) else {
      continue;
    };
    for e in events {
      let Event::Msg(m) = e else { continue };
      for p in &m.parts {
        let Part::Recalled {
          measures: Some(measures),
          ..
        } = p
        else {
          continue;
        };
        let place = format!("{}:{}#{}", root.repo(), t.id, m.id);
        let cue = &measures["cue"];
        let (helped, dismissed) = outcome.get(&place).cloned().unwrap_or_default();
        let judged = outcome.contains_key(&place);
        for mem in measures["memories"].as_array().into_iter().flatten() {
          let handle = mem["handle"].as_str().unwrap_or("");
          let result = if helped.iter().any(|h| h == handle) {
            "helped"
          } else if dismissed.iter().any(|h| h == handle) {
            "dismissed"
          } else if judged {
            "ignored"
          } else {
            "unknown"
          };
          lines.push(format!(
            "{place}\t{handle}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{result}",
            mem["raw"],
            mem["centered"],
            cue["distinctive"],
            cue["chars"],
            cue["novelty"],
            cue["ceiling"],
            cue["wandering"],
            cue["fresh"],
          ));
        }
      }
    }
  }
  Ok(lines)
}

/// What runs in the background: shell commands here, then detached
/// threads anywhere, oldest first in each.
async fn jobs(s: &Arc<Shared>) -> Result<Vec<crate::api::Job>> {
  let me = s.state().node.to_string();
  let listed = s.shells.listing().await;
  let index = s.state().index.lock().expect("index lock");
  let title = |t: &RecordId| index.title(t).ok().flatten().unwrap_or_default();
  let mut out: Vec<crate::api::Job> = listed
    .into_iter()
    .map(|r| crate::api::Job {
      kind: r.kind.into(),
      id: r.id,
      title: title(&r.thread),
      thread: r.thread.to_string(),
      node: me.clone(),
      what: r.what,
      state: r.state,
      since: r.started.to_string(),
    })
    .collect();
  for t in index.records(Some(stitch_records::kinds::THREAD))? {
    let state = chat::detached_state(&index, &t)?;
    if let Some(state) = state.filter(|st| st == "running" || st == "waiting") {
      out.push(crate::api::Job {
        kind: "thread".into(),
        id: t.id.to_string(),
        title: title(&t.id),
        thread: t.id.to_string(),
        node: t.node.clone(),
        what: t
          .fields
          .get("goal")
          .and_then(serde_json::Value::as_str)
          .unwrap_or("")
          .to_owned(),
        state,
        since: t.created.to_string(),
      });
    }
  }
  Ok(out)
}

/// Stops a job (see `Request::StopJob`).
async fn stop_job(s: &Arc<Shared>, id: &str) -> Result<String> {
  match id.contains('/') || id.starts_with("job:") {
    true => s.shells.stop(id).await.map_err(Error::Config),
    false => {
      let thread: RecordId = id.parse()?;
      crate::detached::cancel(s, &thread)?;
      Ok(format!("thread {thread} cancelled"))
    }
  }
}

/// A file on this machine, put in the blob store.
pub(crate) fn attachment(s: &Shared, path: &Path) -> Result<Attachment> {
  let (hash, bytes) = s.state().blobs.put_file(path)?;
  Ok(Attachment {
    hash,
    media_type: media_type_of(path).to_owned(),
    name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
    bytes,
  })
}

/// An attachment as a client names it: a file on this machine, or an
/// image already in the blob store (`blake3:…`, what the web client's
/// `POST /_stitch/blob` answered), which a paste or a drop is.
pub(crate) fn attachment_arg(s: &Shared, arg: &str) -> Result<Attachment> {
  // `blake3:<hex>#<percent-encoded file name>`: a file the web client
  // uploaded (`POST /_stitch/blob`), with the name it had.
  let (arg, name) = arg.split_once('#').map_or((arg, None), |(a, n)| {
    let n = url::form_urlencoded::parse(format!("n={n}").as_bytes())
      .next()
      .map(|(_, v)| crate::apps::clean_name(&v));
    (a, n)
  });
  if let Ok(hash) = arg.parse::<stitch_records::BlobHash>() {
    let bytes = std::fs::read(s.state().blobs.path(&hash))
      .map_err(|_| Error::Config(format!("no file {arg} here")))?;
    let media_type = match stitch_providers::media::sniff_image(&bytes) {
      Some(t) => t.to_owned(),
      None => file_type(name.as_deref(), &bytes),
    };
    return Ok(Attachment {
      hash,
      media_type,
      name,
      bytes: bytes.len() as u64,
    });
  }
  attachment(s, Path::new(arg))
}

/// The media type of an uploaded file that is no image: its name's
/// extension (as for files attached in the TUI), else a PDF's signature,
/// else text if it reads as UTF-8 without NULs, else opaque.
pub(crate) fn file_type(name: Option<&str>, bytes: &[u8]) -> String {
  if let Some(n) = name {
    let t = media_type(Path::new(n));
    if t != "application/octet-stream" {
      return t.to_owned();
    }
  }
  if bytes.starts_with(b"%PDF-") {
    return "application/pdf".into();
  }
  let head = &bytes[..bytes.len().min(8192)];
  // A cut multi-byte character at the end of the head is still text.
  let text = match std::str::from_utf8(head) {
    Ok(_) => true,
    Err(e) => e.error_len().is_none(),
  };
  if text && !head.contains(&0) {
    return "text/plain".into();
  }
  "application/octet-stream".into()
}

fn stop_text(s: &StopReason) -> String {
  match s {
    StopReason::EndTurn => "end_turn".into(),
    StopReason::MaxTokens => "max_tokens".into(),
    StopReason::ToolUse => "tool_use".into(),
    StopReason::Refusal { category } => {
      format!("refusal: {}", category.as_deref().unwrap_or("unspecified"))
    }
    StopReason::Other(o) => o.clone(),
  }
}

async fn chat(
  s: &Arc<Shared>,
  req: Request,
  tx: &mpsc::UnboundedSender<Reply>,
  approver: Approver,
  steer: chat::Steer,
  owner_default: bool,
) -> Result<Reply> {
  // A turn for someone a client serves (the Discord bridge): its model
  // calls say who, for a key-holding proxy to account by (design §2.21);
  // `chat_steered` adds the thread once it is known.
  let end_user = match &req {
    Request::Chat {
      end_user: Some(u), ..
    } => Some(u.clone()),
    _ => None,
  };
  let reply = match end_user {
    Some(u) => {
      let headers = vec![("x-stitch-end-user".to_owned(), u)];
      stitch_providers::with_attribution(
        headers,
        chat_steered(s, req, tx, approver, &steer, owner_default),
      )
      .await
    }
    None => chat_steered(s, req, tx, approver, &steer, owner_default).await,
  };
  // What it did not take is the client's to send.
  steer.close();
  reply
}

async fn chat_steered(
  s: &Arc<Shared>,
  req: Request,
  tx: &mpsc::UnboundedSender<Reply>,
  approver: Approver,
  steer: &chat::Steer,
  owner_default: bool,
) -> Result<Reply> {
  // Resuming is a chat with no message, on a thread, whose loop goes on
  // from what is written (`chat::resume`).
  let (resuming, req) = match req {
    Request::Resume { thread, model } => (
      true,
      Request::Chat {
        thread: Some(thread),
        text: String::new(),
        attachments: Vec::new(),
        model,
        repo: None,
        cwd: None,
        global: true,
        env: Vec::new(),
        tools: Vec::new(),
        skills: Vec::new(),
        place: None,
        memory: None,
        end_user: None,
        node: None,
        spoken: false,
        scopes: Vec::new(),
      },
    ),
    req => (false, req),
  };
  let Request::Chat {
    thread,
    text,
    attachments,
    model,
    repo,
    cwd,
    global,
    env,
    tools: tool_changes,
    skills: skill_changes,
    place: named,
    memory: chosen_memory,
    end_user,
    node: target,
    spoken,
    scopes,
  } = req
  else {
    unreachable!("only chats come here")
  };
  let state = s.state();
  let here = s.place_of_dir(cwd.as_deref());
  s.fold(&here)?;
  let thread: Option<RecordId> = match thread.as_deref() {
    None => None,
    Some("last") => Some(
      s.last_thread(&here.0)?
        .ok_or_else(|| Error::Config("no thread yet".into()))?,
    ),
    Some(id) => Some(id.parse()?),
  };
  // A thread compacted into another goes on there.
  let mut thread = thread.map(|t| crate::fork::current(s, &t));
  // A room's message: its participants answer, each in its own thread
  // (design §2.27); the room itself runs no model.
  if let Some(t) = &thread
    && crate::rooms::is_room_here(s, t)
  {
    if resuming {
      return Err(Error::Config(format!(
        "{t} is a room: it has no turn to resume; send it a message"
      )));
    }
    let atts = attachments
      .iter()
      .map(|p| attachment_arg(s, p))
      .collect::<Result<Vec<_>>>()?;
    let said = crate::rooms::Said {
      text,
      attachments: atts,
      scope: crate::scopes::scoped(s, &scopes).await?,
      spoken,
      from: end_user.map(|u| format!("end user {u}")),
    };
    return crate::rooms::chat(s, t, said, Some(tx), steer.clone()).await;
  }
  // Typing into a conversation says the user read what was there (design
  // §2.25); an app's end user is not the user.
  if let Some(t) = &thread
    && !resuming
    && end_user.is_none()
  {
    crate::meta::read_on_send(s, t);
  }
  let (root, place, folder) = match (&thread, named.as_deref()) {
    (Some(t), _) => {
      let (root, place) = s.thread_place(t)?;
      (root, place, None)
    }
    (None, Some(key)) => s.place_named(key)?,
    (None, None) => {
      let (root, place) = s.placement(repo.as_deref(), global, &here)?;
      let folder = s.folder_of(&place, cwd.as_deref());
      (root, place, folder)
    }
  };
  if place == here.0 {
    s.saw(&here);
  }
  // A new chat the client put on another node (design §2.7, "A new chat
  // on another node"): the thread is written here, set to run there, and
  // its first message goes there as below.
  let target = target
    .filter(|n| !n.is_empty() && *n != "here" && *n != state.node.as_str())
    .filter(|_| thread.is_none());
  if let Some(n) = &target {
    let new = crate::elsewhere::NewThread {
      text: &text,
      folder: folder.as_ref(),
      tools: &crate::toolset::parse_all(&tool_changes)?,
      skills: &crate::toolset::parse_all(&skill_changes)?,
      memory: chosen_memory,
    };
    thread = Some(crate::elsewhere::new_thread_on(s, n, &root, &place, new).await?);
  }
  // The user had it run on another node (design §2.7, "Continue on
  // another node"): the message goes there, and its turn streams here.
  // The app's and the web client's chats also go to the thread's owner when
  // the user chose nothing (`weave::owner_elsewhere`); the TUI's run here.
  let elsewhere = match &thread {
    Some(_) if target.is_some() => target,
    Some(t) => match crate::weave::pinned_elsewhere(s, t) {
      Some(n) => Some(n),
      None if owner_default && !resuming => match crate::weave::owner_elsewhere(s, t) {
        Some(n) if crate::elsewhere::runs_programs(s, &n).await.is_ok() => Some(n),
        _ => None,
      },
      None => None,
    },
    None => None,
  };
  if let Some(t) = &thread
    && let Some(node) = elsewhere
  {
    if resuming {
      return Err(Error::Config(format!(
        "thread {t} runs on {node}: its turns are {node}'s to resume (send it a message), or \
         bring it back here first (`/run-on here`, Continue here)"
      )));
    }
    // Its files go with it (their blobs sync there), and the model this
    // client picked, which that node uses if it runs it.
    let atts = attachments
      .iter()
      .map(|p| attachment_arg(s, p))
      .collect::<Result<Vec<_>>>()?;
    // Refused here when one names no thread; resolved again there.
    crate::scopes::scoped(s, &scopes).await?;
    let typed = crate::elsewhere::Typed {
      text: &text,
      attachments: &atts,
      model: model.as_deref(),
      spoken,
      scopes: &scopes,
      room_turn: None,
    };
    return crate::elsewhere::chat(s, t, &root, &node, typed, tx, steer).await;
  }
  // Another node's turn: refused before anything is written (compacting
  // it below would start a thread from it); asked again once caught up.
  if !resuming && let Some(t) = &thread {
    refuse_if_held_elsewhere(
      &ChatContext {
        node: &state.node,
        root: &root,
        index: &state.index,
        blobs: &state.blobs,
      },
      t,
    )?;
  }
  let atts = attachments
    .iter()
    .map(|p| attachment_arg(s, p))
    .collect::<Result<Vec<_>>>()?;
  // The threads and efforts attached, resolved once: the briefing written
  // with the message (`Part::Scope`).
  let scope = crate::scopes::scoped(s, &scopes).await?;
  // The model the client picked, else the thread's own (design §2.7, "A
  // thread's model"), else the node's default.
  let picked = model.clone();
  let model = crate::modelset::chat_model(s, thread.as_ref(), &place, model.as_deref()).await;
  let (provider, provider_name, model) = s
    .config
    .resolve_authenticated(&s.data, Some(&model))
    .await?;
  let full_model = format!("{provider_name}/{model}");
  // Past its share of the model's window: it continues in a new thread,
  // from a summary and its last messages.
  // (Not when resuming: its step is sent again as written.) Not while a
  // loop here runs on it either: the new thread's turn would run beside
  // that one (issue 2026-11-04T102014Z-gamer-mfnh); this chat waits for
  // that loop or goes into it, on this thread, as below. Held while it
  // compacts and for the turn: a message for the old one waits for it
  // (`detached::turn_with` takes the old thread's lock, then the new's).
  let mut compaction_locks = Vec::new();
  if let Some(t) = &thread
    && !resuming
    && crate::fork::needs_compacting(s, t, &full_model).await
    && let Ok(held) = s.detached.locks.of(t).try_lock_owned()
  {
    let _ = tx.send(Reply::Note(compacting_note(s, &full_model).await));
    let (new, said) = crate::fork::compact(s, t, &full_model).await?;
    let _ = tx.send(Reply::Compacted(said));
    thread = Some(new);
    compaction_locks.push(held);
  }
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  // A new thread's ID is chosen here: its set's changes, and memory,
  // name it before it is written.
  let new_id = thread
    .is_none()
    .then(|| RecordId::random(jiff::Timestamp::now(), &state.node));
  let this = thread.as_ref().or(new_id.as_ref()).expect("a thread");
  if !stitch_providers::attribution().is_empty() {
    stitch_providers::attribute("x-stitch-thread", &this.to_string());
  }
  // Its feed, followed from other nodes too (one that has it run here).
  let live_key = this.to_string();
  // A folder's files as other nodes left them, before the turn reads them;
  // what the turn changes, published when it ends (design §2.16). A
  // phone's project cloned first, so its directory is the place's. A
  // project with no checkout here: found, or cloned once the user says yes
  // (a thread already written asks), else this chat fails saying so.
  crate::checkout::ensure(s, &place, Some(this), thread.as_ref().map(|_| &root)).await?;
  let _folder_turn = crate::folders::turn(s, &place, Some(this)).await;
  let start = s.start_dir(&place, cwd.as_deref().map(Path::new), Some(this));
  let mut tool_entries = crate::toolset::parse_all(&tool_changes)?;
  // A turn for someone a client serves (the Discord bridge's users):
  // never the groups that are off by default, whatever was asked.
  if end_user.is_some() {
    tool_entries.retain(|e| crate::toolset::enables_guarded(std::slice::from_ref(e)).is_none());
    let (guarded, _) =
      crate::toolset::thread_changes(&state.index.lock().expect("index lock"), this);
    for g in crate::toolset::OFF_BY_DEFAULT {
      if guarded
        .iter()
        .rev()
        .find(|e| e.pattern == *g)
        .is_none_or(|e| e.on)
      {
        tool_entries.push(crate::toolset::Entry::off(g));
      }
    }
  }
  crate::toolset::write_changes(
    &root,
    &state.index,
    &state.node,
    this,
    &tool_entries,
    &crate::toolset::parse_all(&skill_changes)?,
  )?;
  // A new thread's memory, as `/memory` chose before it started, wins
  // over its place's.
  match chosen_memory.filter(|_| new_id.is_some() && s.memory.available()) {
    Some(on) => {
      s.memory.remember(state, &root, this, on)?;
    }
    None => s.place_memory(&place, &root, this)?,
  }
  let selection = s.selection(&place, start.as_deref(), Some(this));
  // The client's environment, for the shells this chat starts.
  let mut tools = s.shell_tools(
    &root,
    &place,
    s.read_only(this),
    Some(this),
    start.as_deref(),
    approver,
    (!env.is_empty()).then_some(env.as_slice()),
    &selection,
  );
  // The user is in this chat: at its step limit it asks them for more.
  tools.asks_more_steps = true;
  tools.model = full_model.clone();
  let context = s.context(&place, start.as_deref(), &selection, Some(this));
  let system = thread.is_none().then(|| s.system_prompt(false, &context));
  // Continued on another node too: caught up first, so this turn does not
  // fork the thread from an older step.
  if let Some(t) = &thread {
    crate::weave::catch_up(s, t, false).await;
    // The user writing in a spawned thread takes it over: its spawn budget
    // is lifted (design §2.7).
    if !resuming {
      match crate::weave::lift_budget(s, t) {
        Ok(true) => eprintln!("thread {t}: the user wrote in it; its spawn budget is lifted"),
        Ok(false) => {}
        Err(e) => eprintln!("thread {t}: its budget was not lifted: {e}"),
      }
    }
  }
  // Resumed only by the node whose turn it was, and not a detached loop
  // at work: another node may be running it now, and two loops on a
  // thread fork it (a client forks it first, `chat::Unfinished`). Asked
  // before waiting for a loop here: a detached one is not waited for (one
  // of this node's chats that answers it meanwhile, `chat::resume` finds
  // answered).
  if resuming && let Some(t) = &thread {
    match chat::unfinished(&ctx, t)? {
      None => {
        return Err(Error::Config(format!(
          "nothing to resume: thread {t}'s last turn ended"
        )));
      }
      Some(chat::Unfinished {
        elsewhere: Some(node),
        ..
      }) => {
        return Err(Error::Config(format!(
          "thread {t}'s turn is {node}'s loop's, which may be running it: fork the thread to resume it here"
        )));
      }
      Some(_) => {}
    }
  }
  // A message typed into a thread whose turn another node's loop has is
  // refused too, for every client: a chat here would be a second loop on
  // it (2026-10-17, `…isev`: the phone chatted beside gamer's loop). It
  // goes to that turn (`send` with `interject`), or the thread is forked.
  // One of this node's own loops is waited for, as before.
  if !resuming && let Some(t) = &thread {
    refuse_if_held_elsewhere(&ctx, t)?;
  }
  // Loops never run twice at once on a thread (a report may be waking it).
  // A new thread's too, under the ID chosen for it: a thread it spawns
  // may send it a message during this first turn (issue …-umbm).
  let lock = Some(s.detached.locks.of(this));
  // One already running: the client is told this waits for it, and an
  // interrupt meanwhile ends this chat with its message unsent; so does
  // the client leaving (a TUI quit or killed while it waited: its message
  // was sent later to no one, 2026-10-21).
  //
  // While it waits, its message and what its client queues go into the
  // loop that runs (when one runs here), woven in between its steps as
  // that loop's own client's typing is (decided 2026-10-21, after four
  // messages queued behind a turn a job's note woke were never seen by
  // it): each taken is said (`Woven`, the message itself as ID 0), and
  // one taken makes this chat end with that turn (stop `woven`), having
  // no turn of its own; what that loop ends without taking stays this
  // chat's, its message then sent as its own turn after it.
  let mut relayed: Option<chat::RelayOutcome> = None;
  let _held = match (&lock, &thread) {
    (Some(l), Some(t)) => Some(match l.try_lock() {
      Ok(held) => held,
      Err(_) => {
        let _ = tx.send(Reply::Waiting(true));
        let first = chat::Interjection {
          id: None,
          from: None,
          text: text.clone(),
          attachments: atts.clone(),
          reports: Vec::new(),
          untold: None,
          asker: None,
          relayed: None,
          scope: scope.clone(),
        };
        let on_taken = {
          let tx = tx.clone();
          move |id: Option<u64>| {
            let _ = tx.send(Reply::Woven(vec![id.unwrap_or(0)]));
          }
        };
        let mut relaying = false;
        let mut look = tokio::time::interval(Duration::from_millis(250));
        let held = loop {
          // A loop may start on it after this began waiting (one being
          // set up holds the lock before it steers).
          if !relaying && let Some(to) = crate::detached::running_steer(s, t) {
            relaying = steer.relay(to, first.clone(), on_taken.clone());
          }
          tokio::select! {
            held = l.lock() => break held,
            () = steer.stopped() => {
              if steer.relayed_first() {
                // Its message is in the turn that runs: the interrupt is
                // for that turn, and this chat ends with it.
                if let Some(to) = steer.relaying_to() {
                  to.interrupt();
                }
                let held = l.lock().await;
                relayed = steer.unrelay();
                break held;
              }
              steer.unrelay();
              return Ok(Reply::Unsent);
            }
            () = steer.left() => {
              // Gone: what that loop took stays in it, the rest is not
              // sent (no one is there to see it go).
              steer.unrelay();
              return Ok(Reply::Unsent);
            }
            _ = look.tick(), if !relaying => {}
          }
        };
        if relaying && relayed.is_none() {
          relayed = steer.unrelay();
        }
        let _ = tx.send(Reply::Waiting(false));
        held
      }
    }),
    // A new thread: nothing else knows its ID yet, so nothing holds it;
    // held from here, what is sent to it waits for this turn.
    (Some(l), None) => Some(l.lock().await),
    (None, _) => None,
  };
  if let Some(outcome) = relayed {
    if outcome.taken.contains(&None) {
      // Its message went into the turn that ran: nothing more to send.
      // What that turn did not take is still the client's (never said
      // woven), which it sends after.
      let t = thread.as_ref().expect("relayed on a thread");
      let segment = chat::history(&ctx, t)?
        .last_segment
        .map(|s| s.to_string())
        .unwrap_or_default();
      let stop = if steer.interrupted() {
        "interrupted"
      } else {
        "woven"
      };
      let thread_usd = crate::detached::spent_by_chain(s, t, None).and_then(|(_, u)| u);
      return Ok(Reply::ChatDone {
        thread: t.to_string(),
        segment,
        model: full_model.clone(),
        stop: stop.into(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        usd: None,
        forked: false,
        thread_usd,
      });
    }
    // Not taken: its own turn now, with what was queued behind it.
    for i in outcome.back.into_iter().filter(|i| i.id.is_some()) {
      steer.push(i);
    }
  }
  // When the turn began, for what memory observes after it; and the
  // memories recalled for this message unasked.
  let since = jiff::Timestamp::now();
  let note = match thread.as_ref().or(new_id.as_ref()) {
    Some(t) if s.memory.available() && !resuming => {
      let n = match &thread {
        Some(t) => chat::thread_messages(&ctx, t).map_or(0, |m| m.len()),
        None => 0,
      };
      let place = format!("{}:{t}#m{}", root.repo(), n + 1);
      s.memory.before_turn(state, &root, t, &place, &text).await
    }
    _ => None,
  };
  let recalled = note
    .as_ref()
    .map(|n| (n.block.clone(), n.handles.clone(), Some(n.measures.clone())));
  let mut fetched = false;
  let mut compacted = false;
  // Notes for the thread (a background job ended, a report came) are woven
  // into this loop while it runs.
  let mut steering = thread
    .as_ref()
    .or(new_id.as_ref())
    .map(|t| crate::detached::Steering::new(s, t, steer.clone()));
  loop {
    let input = ChatInput {
      thread: thread.clone(),
      id: new_id.clone(),
      text: text.clone(),
      attachments: atts.clone(),
      provider: provider.clone(),
      model: model.clone(),
      system: system.clone(),
      from: None,
      detached: None,
      limits: chat::Limits {
        steps: Some(s.chat_steps()),
        ..Default::default()
      },
      recalled: recalled.clone(),
      instruction: None,
      context: Some(context.clone()),
      folder: folder.clone(),
      steer: Some(steer.clone()),
      spoken,
      scope: scope.clone(),
      room_turn: None,
    };
    let mut on_event = |e: ChatEvent| {
      // Followed from other nodes too (a phone that has it run here).
      s.live.event(&live_key, &e);
      let _ = match e {
        ChatEvent::Thread(t) => tx.send(Reply::Thread(t.to_string())),
        ChatEvent::Woven(ids) => tx.send(Reply::Woven(ids)),
        ChatEvent::Note(text) => tx.send(Reply::Note(text)),
        ChatEvent::Text(t) => tx.send(Reply::Text(t)),
        ChatEvent::Thinking(t) => tx.send(Reply::Thinking(t)),
        ChatEvent::ToolCall { name, input } => {
          tx.send(Reply::ToolCall(call_summary(&name, &input)))
        }
        ChatEvent::ToolResult { ok, output, .. } => tx.send(Reply::ToolResult { ok, output }),
        ChatEvent::App { manifest, name } => tx.send(crate::apps::shown(s, &manifest, name)),
        ChatEvent::Retry(why) => tx.send(Reply::Retry(why)),
      };
    };
    let sent = match (resuming, &thread) {
      (true, Some(t)) => {
        let _ = tx.send(Reply::Thread(t.to_string()));
        chat::resume(
          &ctx,
          t,
          &provider,
          &model,
          input.limits,
          &tools,
          Some(steer.clone()),
          &mut on_event,
        )
        .await
        .and_then(|o| {
          o.ok_or_else(|| Error::Config(format!("nothing to resume: thread {t}'s last turn ended")))
        })
      }
      _ => chat::send(&ctx, input, &tools, &mut on_event).await,
    };
    // Messages sent to the thread and woven in: their senders hear its
    // reply (a chat writes no report of its own).
    let askers = steer.askers();
    if !askers.is_empty()
      && let Some(t) = thread.as_ref().or(new_id.as_ref())
    {
      crate::detached::answer_askers(s, &root, t, &sent, askers, None);
    }
    match sent {
      Ok(o) => {
        drop(steering.take());
        // The model picked is the thread's from now: the turns nobody
        // types and other nodes continuing it run on it.
        if let Some(picked) = &picked {
          let kept = if crate::modelset::ALIASES.contains(&picked.as_str()) {
            picked
          } else {
            &full_model
          };
          if let Err(e) = crate::meta::keep_model(s, &o.thread, kept) {
            eprintln!("thread {}: its model not kept: {e}", o.thread);
          }
        }
        let mut done = o.clone();
        if s.memory.available() {
          let shown = note.as_ref().map(|n| n.shown.clone()).unwrap_or_default();
          let answered =
            o.end == chat::End::Answered && !matches!(o.stop, StopReason::Refusal { .. });
          if answered && !steer.interrupted() && s.memory.may_follow_up(&o.thread) {
            // What the exchange surfaced may be worth an afterthought.
            let fresh = s
              .memory
              .after_turn(state, &root, &o.thread, since, shown, &o.text)
              .await
              .unwrap_or(0);
            if fresh > 0
              && let Some(f) = follow_up(
                s,
                &ctx,
                &root,
                &o.thread,
                &input_of(&provider, &model, &context),
                &tools,
                tx,
              )
              .await
            {
              done.usage.input_tokens += f.usage.input_tokens;
              done.usage.output_tokens += f.usage.output_tokens;
              done.usage.cache_read_tokens += f.usage.cache_read_tokens;
              done.usage.cache_write_tokens += f.usage.cache_write_tokens;
              done.usage.cache_write_1h_tokens += f.usage.cache_write_1h_tokens;
              done.usd = match (done.usd, f.usd) {
                (Some(a), Some(b)) => Some(a + b),
                (a, b) => a.or(b),
              };
              done.segment = f.segment;
            }
          } else {
            // Remembering happens after the answer is out.
            let (m, st, root, t) = (
              s.memory.clone(),
              state.clone(),
              root.clone(),
              o.thread.clone(),
            );
            let answer = o.text.clone();
            tokio::spawn(async move {
              let _ = m.after_turn(&st, &root, &t, since, shown, &answer).await;
            });
          }
          let s = s.clone();
          tokio::spawn(async move { sleep_memory(&s).await });
        }
        // The thread runs here now: reports held for it come to it here.
        crate::weave::deliver(s);
        // Its digest, when it said enough since the last (design §2.25).
        crate::digests::after_turn(s, &o.thread);
        // A new chat gets a title from its first exchange.
        if thread.is_none() && o.end == chat::End::Answered {
          crate::meta::title_later(
            s,
            o.thread.clone(),
            text.clone(),
            o.text.clone(),
            full_model.clone(),
          );
        }
        let thread_usd =
          crate::detached::spent_by_chain(s, &done.thread, None).and_then(|(_, u)| u);
        // Its client watched the turn end: the user saw it (loose ends).
        if let Err(e) = crate::loose::opened(s, &done.thread) {
          eprintln!("thread {}: not marked opened: {e}", done.thread);
        }
        return Ok(Reply::ChatDone {
          thread: done.thread.to_string(),
          segment: done.segment.to_string(),
          model: done.model,
          stop: stop_text(&done.stop),
          input_tokens: done.usage.input_tokens,
          output_tokens: done.usage.output_tokens,
          cache_read_tokens: done.usage.cache_read_tokens,
          cache_write_tokens: done.usage.cache_write_tokens,
          usd: done.usd,
          forked: done.forked,
          thread_usd,
        });
      }
      Err(Error::MissingBlobs(hashes)) if !fetched => {
        fetched = true;
        let n = fetch_blobs(s, &hashes).await;
        let _ = tx.send(Reply::Fetched(n));
      }
      // Too long for the model after all: compacted, and tried again once.
      Err(e)
        if !compacted
          && !matches!(e, Error::Kept { .. })
          && crate::fork::too_long(&e)
          && thread.is_some() =>
      {
        compacted = true;
        let _ = tx.send(Reply::Note(compacting_note(s, &full_model).await));
        let t = thread.clone().expect("checked");
        let (new, said) = crate::fork::compact(s, &t, &full_model).await?;
        let _ = tx.send(Reply::Compacted(said));
        // Its lock too (newer after older, as everywhere): nothing runs
        // the new thread beside this turn.
        compaction_locks.push(s.detached.locks.of(&new).lock_owned().await);
        thread = Some(new);
      }
      Err(e) => return Err(e),
    }
  }
}

/// Refuses a message typed into `thread` when another node's loop has its
/// turn (`chat::unfinished` names that node, as for a Resume): a chat here
/// would be a second loop on it, which forks it unsaid. Says which node,
/// and the two ways on: send it to that turn (`interject`), or fork.
/// A turn of this node's own (a detached loop here) is not refused: the
/// chat waits for it.
fn refuse_if_held_elsewhere(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<()> {
  if let Some(chat::Unfinished {
    elsewhere: Some(node),
    ..
  }) = chat::unfinished(ctx, thread)?
    && node != ctx.node.as_str()
  {
    return Err(Error::Config(format!(
      "thread {thread}'s turn is {node}'s: its loop may be running it there, and a chat here \
       would be a second loop on it. Send the message to that turn instead (`stitch send \
       {thread} --interject`; `send` with `interject`), or fork the thread to go on here"
    )));
  }
  Ok(())
}

/// What the client is told when a thread is compacted.
async fn compacting_note(s: &Shared, model: &str) -> String {
  format!(
    "[This thread is near the {}k-token window of {model}: it continues in a new thread, from \
     a summary of it and its last messages. The old one stays as it was.]",
    crate::window::of(s, model).await / 1000
  )
}

/// What a follow-up runs on: the exchange's provider, model and context.
struct Turn {
  provider: stitch_providers::Provider,
  model: String,
  context: String,
}

fn input_of(provider: &stitch_providers::Provider, model: &str, context: &str) -> Turn {
  Turn {
    provider: provider.clone(),
    model: model.to_owned(),
    context: context.to_owned(),
  }
}

/// Memory's afterthought: when the exchange surfaced memories that pass
/// the thread's cue, the model gets them with an instruction to add a
/// short afterthought or decline with the invisible marker (a decline is
/// written, never shown). The note is written as memory's message; the
/// instruction is not.
async fn follow_up(
  s: &Arc<Shared>,
  ctx: &ChatContext<'_>,
  root: &RecordRoot,
  thread: &RecordId,
  turn: &Turn,
  tools: &NodeTools,
  tx: &mpsc::UnboundedSender<Reply>,
) -> Option<chat::ChatOutcome> {
  let state = s.state();
  let note = s.memory.follow_up(state, root, thread)?;
  let since = jiff::Timestamp::now();
  let input = ChatInput {
    thread: Some(thread.clone()),
    id: None,
    text: String::new(),
    attachments: Vec::new(),
    provider: turn.provider.clone(),
    model: turn.model.clone(),
    system: None,
    from: Some("memory".into()),
    detached: None,
    limits: Default::default(),
    recalled: Some((
      note.block.clone(),
      note.handles.clone(),
      Some(note.measures.clone()),
    )),
    instruction: Some(crate::memory::follow_up_instruction()),
    context: Some(turn.context.clone()),
    folder: None,
    steer: None,
    spoken: false,
    scope: None,
    room_turn: None,
  };
  let mut on_event = |e: ChatEvent| {
    let _ = match e {
      ChatEvent::ToolCall { name, input } => tx.send(Reply::ToolCall(call_summary(&name, &input))),
      ChatEvent::ToolResult { ok, output, .. } => tx.send(Reply::ToolResult { ok, output }),
      ChatEvent::App { manifest, name } => tx.send(crate::apps::shown(s, &manifest, name)),
      _ => Ok(()),
    };
  };
  let o = chat::send(ctx, input, tools, &mut on_event).await.ok()?;
  if !stitch_memory::marker_only(&o.text) && !o.text.trim().is_empty() {
    let _ = tx.send(Reply::Afterthought(stitch_memory::without_markers(&o.text)));
  }
  let (m, st, root, t, answer) = (
    s.memory.clone(),
    state.clone(),
    root.clone(),
    thread.clone(),
    o.text.clone(),
  );
  tokio::spawn(async move {
    let _ = m
      .after_turn(&st, &root, &t, since, note.shown, &answer)
      .await;
  });
  Some(o)
}

/// The models of `node` (this one when `None` or named): them, why some
/// providers listed none, and its default.
pub(crate) async fn models_of(
  s: &Arc<Shared>,
  node: Option<&str>,
) -> Result<(Vec<String>, Vec<String>, String)> {
  let reply = match node.filter(|n| *n != s.state().node.as_str()) {
    Some(n) => peer_models(s, n).await?,
    None => models(s).await,
  };
  match reply {
    Reply::Models {
      models,
      problems,
      default,
    } => Ok((models, problems, default)),
    Reply::Error(e) => Err(Error::Config(e)),
    other => Err(Error::Protocol(format!("unexpected answer: {other:?}"))),
  }
}

/// Most models a refusal lists.
const LISTED: usize = 40;

/// Whether `node` (this one, or a peer asked over the network) can run
/// `model`; refused with the models it can run and its default when not.
/// A node that cannot be asked (offline, an older build) is not refused:
/// the assignment waits for it, and a model it cannot run fails there.
pub(crate) async fn check_model(s: &Arc<Shared>, node: &str, model: &str) -> Result<()> {
  let reply = if node == s.state().node.as_str() {
    models(s).await
  } else {
    match peer_models(s, node).await {
      Ok(r) => r,
      Err(_) => return Ok(()),
    }
  };
  let Reply::Models {
    models,
    problems,
    default,
  } = reply
  else {
    return Ok(());
  };
  if runs_listed(s, node, &models, model) != Some(false) {
    return Ok(());
  }
  let mut can: Vec<&str> = models.iter().take(LISTED).map(String::as_str).collect();
  if models.len() > LISTED {
    can.push("…");
  }
  Err(Error::Config(format!(
    "{node} cannot run {model}. It runs: {} (its default: {default}).{} Name one of those, or \
     leave the model out for its default.",
    if can.is_empty() {
      "nothing it can list".to_owned()
    } else {
      can.join(", ")
    },
    if problems.is_empty() {
      String::new()
    } else {
      format!(" Not listed: {}.", problems.join("; "))
    }
  )))
}

/// Whether `node` runs `model` by its list: `Some(true)` listed,
/// `Some(false)` not (its provider listed others, or this node has no
/// such provider), `None` when the list cannot say (a provider that
/// could not list its models, a local server without a models endpoint,
/// may still run it).
fn runs_listed(s: &Shared, node: &str, models: &[String], model: &str) -> Option<bool> {
  if models.iter().any(|m| m == model) {
    return Some(true);
  }
  let provider = model.split_once('/').map_or(model, |(p, _)| p);
  // A variant of a listed model (OpenRouter's `…:nitro`, `…:floor`) is
  // run where the model is.
  if provider == "openrouter"
    && let Some((base, _)) = model.rsplit_once(':')
    && models.iter().any(|m| m == base)
  {
    return Some(true);
  }
  let listed = models
    .iter()
    .any(|m| m.split_once('/').is_some_and(|(p, _)| p == provider));
  let unknown = node == s.state().node.as_str() && s.config.provider_config(provider).is_none();
  if listed || unknown { Some(false) } else { None }
}

/// Whether `node` (this one, or a peer asked) runs `model`, by its list
/// (`runs_listed`); `None` also when it cannot be asked.
pub(crate) async fn lists_model(s: &Arc<Shared>, node: &str, model: &str) -> Option<bool> {
  let reply = if node == s.state().node.as_str() {
    models(s).await
  } else {
    peer_models(s, node).await.ok()?
  };
  match reply {
    Reply::Models { models, .. } => runs_listed(s, node, &models, model),
    _ => None,
  }
}

/// How long a model list is reused.
const MODELS_FOR: Duration = Duration::from_secs(600);

/// The models of every configured provider (the built-in ones too), asked
/// at once; a provider without credentials, or that fails, says why.
///
/// Each provider's answer is reused for `MODELS_FOR`, except one without
/// credentials: it is asked again each time, so a key or sign-in added
/// while the node runs (the app's settings, `stitch auth`) shows at once.
async fn models(s: &Arc<Shared>) -> Reply {
  let mut cached = s.models.lock().await;
  let mut names: Vec<String> = s.config.providers.keys().cloned().collect();
  for n in [
    "anthropic",
    "openai",
    "grok",
    "gemini",
    "kimi",
    "deepseek",
    "openrouter",
  ] {
    if !names.iter().any(|x| x == n) {
      names.push(n.into());
    }
  }
  let fresh = |name: &String| {
    cached
      .get(name)
      .is_some_and(|(at, _)| at.elapsed() < MODELS_FOR)
  };
  // A task each: finding a provider's credentials may block on the
  // keychain (slow the first time a new build asks), which must not stall
  // the others' requests past their timeouts.
  let asks = names.iter().filter(|n| !fresh(n)).cloned().map(|name| {
    let s = s.clone();
    tokio::spawn(async move {
      let probe = format!("{name}/-");
      let listed = match s.config.resolve_authenticated(&s.data, Some(&probe)).await {
        Ok((provider, _, _)) => {
          match tokio::time::timeout(Duration::from_secs(15), provider.models()).await {
            Ok(r) => r.map_err(|e| e.to_string()),
            Err(_) => Err("timed out".into()),
          }
        }
        Err(e) => Err(e.to_string()),
      };
      (name, listed)
    })
  });
  let mut answers: std::collections::HashMap<String, std::result::Result<Vec<String>, String>> =
    Default::default();
  let mut problems = Vec::new();
  for asked in futures_util::future::join_all(asks).await {
    match asked {
      Ok((name, listed)) => {
        // Only a list is kept: a provider without credentials, or whose
        // listing failed, is asked again next time.
        if let Ok(ids) = &listed
          && !ids.is_empty()
        {
          cached.insert(name.clone(), (std::time::Instant::now(), ids.clone()));
        } else {
          cached.remove(&name);
        }
        answers.insert(name, listed);
      }
      Err(e) => problems.push(format!("a provider's listing failed: {e}")),
    }
  }
  let mut models = Vec::new();
  for name in &names {
    let listed = match answers.remove(name) {
      Some(l) => l,
      None => match cached.get(name) {
        Some((_, ids)) => Ok(ids.clone()),
        None => continue,
      },
    };
    match listed {
      Ok(ids) if ids.is_empty() => problems.push(format!("{name}: no models listed")),
      Ok(ids) => models.extend(ids.into_iter().map(|m| format!("{name}/{m}"))),
      Err(e) => problems.push(format!("{name}: {e}")),
    }
  }
  let default = crate::modelset::default_model(s, None).await;
  Reply::Models {
    models,
    problems,
    default,
  }
}

/// What one node asks another, on the network (new questions go last).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Question {
  /// Its models, as `Request::Models` answers them.
  Models,
  /// Run a tool of a server it shares (design §2.14): `Reply::ToolDone`.
  CallTool(crate::remote::CallTool),
  /// A directory there as the place of a thread spawned to it
  /// (`weave::resolve_place`): `Reply::Lines` with the `weave::Found` as
  /// JSON, or `Reply::Error`.
  Place(String),
  /// What the loops on a thread here streamed from `after` on, waited for
  /// a while (`live::Live::follow`); `after` `None`: where its feed stands.
  /// `Reply::Lines` with the `live::Followed` as JSON (design §2.7,
  /// "Continue on another node").
  Follow {
    thread: String,
    after: Option<u64>,
    assignment: Option<String>,
  },
  /// Stops the loop running on a thread here: the user stopped the turn
  /// on the node that follows it. `Reply::Ok`.
  Interrupt(String),
  /// Whether this node runs programs (shells): a thread can be set to run
  /// on it (design §2.7, "Continue on another node"). `Reply::Lines`:
  /// `yes`, or `no` and why; then, since 2026-11-09, `platform: <os>
  /// <arch>` and `default: <model>` (`elsewhere::answer_runs`).
  Runs,
  /// The step a thread's loop here runs the calls of now
  /// (`Request::InFlight`): `Reply::Lines`, it as JSON, or none.
  InFlight(String),
  /// The user pressed an action button this node offered (design §2.30,
  /// `Request::Press`): run here, `Reply::Lines` with the
  /// `actions::Pressed` as JSON, or `Reply::Error`.
  Press {
    thread: String,
    set: String,
    item: u32,
  },
  /// A phone says how a call this node rang went (`call_user`, design
  /// §2.3, "Calls"): `outcome` `answered`, `declined`, `missed` or
  /// `failed`. `Reply::Lines`, or `Reply::Error`.
  Ring { call: String, outcome: String },
}

/// Answers `peer`'s question with a `Reply`, postcard-encoded.
async fn answer(s: &Arc<Shared>, peer: &str, question: &[u8]) -> Vec<u8> {
  let reply = match postcard::from_bytes::<Question>(question) {
    Ok(Question::Follow {
      thread,
      after,
      assignment,
    }) => crate::elsewhere::answer_follow(s, &thread, after, assignment.as_deref()).await,
    Ok(Question::Interrupt(thread)) => crate::elsewhere::answer_interrupt(s, &thread),
    Ok(Question::Runs) => crate::elsewhere::answer_runs(s).await,
    Ok(Question::InFlight(thread)) => crate::elsewhere::answer_in_flight(s, &thread),
    Ok(Question::Press { thread, set, item }) => {
      crate::elsewhere::answer_press(s, &thread, &set, item).await
    }
    Ok(Question::Ring { call, outcome }) => crate::rings::answer(s, &call, &outcome),
    Ok(Question::Models) => models(s).await,
    Ok(Question::CallTool(call)) => crate::remote::answer(s, peer, call).await,
    Ok(Question::Place(path)) => match crate::weave::resolve_place(s, &path) {
      Ok(r) => Reply::Lines(vec![serde_json::to_string(&r.found()).unwrap_or_default()]),
      Err(e) => Reply::Error(e.to_string()),
    },
    Err(e) => Reply::Error(format!("a question this node does not know: {e}")),
  };
  postcard::to_stdvec(&reply).unwrap_or_default()
}

/// A directory on the peer `node` as a place (`Question::Place`): `None`
/// when it cannot say (unreachable, or a node from before the question),
/// an error when it is not a directory there.
pub(crate) async fn peer_place(
  s: &Arc<Shared>,
  node: &str,
  path: &str,
) -> Result<Option<crate::weave::Found>> {
  let state = s.state().clone();
  let name = node.to_owned();
  let Ok(Ok(Some(peer))) = tokio::task::spawn_blocking(move || state.find_peer(&name)).await else {
    return Ok(None);
  };
  let Ok(addr) = peer.addr() else {
    return Ok(None);
  };
  let question = postcard::to_stdvec(&Question::Place(path.to_owned()))
    .map_err(|e| Error::Protocol(e.to_string()))?;
  let Ok(Ok(asked)) =
    tokio::time::timeout(Duration::from_secs(15), s.net.ask(addr, question)).await
  else {
    return Ok(None);
  };
  match postcard::from_bytes::<Reply>(&asked) {
    Ok(Reply::Lines(lines)) => Ok(lines.first().and_then(|l| serde_json::from_str(l).ok())),
    Ok(Reply::Error(e)) if e.starts_with("a question this node does not know") => Ok(None),
    Ok(Reply::Error(e)) => Err(Error::Config(e)),
    _ => Ok(None),
  }
}

/// The models of the peer `node`, asked of it.
async fn peer_models(s: &Arc<Shared>, node: &str) -> Result<Reply> {
  let state = s.state().clone();
  let name = node.to_owned();
  let peer = tokio::task::spawn_blocking(move || state.find_peer(&name))
    .await
    .map_err(|e| Error::Config(e.to_string()))??
    .ok_or_else(|| Error::Config(format!("no peer `{node}`")))?;
  let question =
    postcard::to_stdvec(&Question::Models).map_err(|e| Error::Protocol(e.to_string()))?;
  // The peer asks each of its providers, for 15 seconds at most.
  let asked = tokio::time::timeout(Duration::from_secs(30), s.net.ask(peer.addr()?, question))
    .await
    .map_err(|_| Error::Config(format!("{node} did not answer in time")))??;
  postcard::from_bytes(&asked).map_err(|e| Error::Protocol(format!("{node}'s answer: {e}")))
}

/// Takes a thread out of memory or brings it back; a thread brought back
/// is observed in the background.
async fn remember(s: &Arc<Shared>, thread: &str, on: bool, cwd: Option<&str>) -> Result<Reply> {
  if !s.memory.available() {
    return Err(Error::Config(
      "memory is not installed on this node (`stitch memory install`)".into(),
    ));
  }
  let here = s.place_of_dir(cwd);
  let t: RecordId = match thread {
    "last" => s
      .last_thread(&here.0)?
      .ok_or_else(|| Error::Config("no thread yet".into()))?,
    id => id.parse()?,
  };
  let (root, _) = s.thread_place(&t)?;
  let changed = s.memory.remember(s.state(), &root, &t, on)?;
  if on {
    let (m, st, t2) = (s.memory.clone(), s.state().clone(), t.clone());
    tokio::spawn(async move {
      let _ = m.heal(&st, &root, &t2, jiff::Timestamp::now()).await;
    });
  }
  Ok(Reply::Lines(vec![match (on, changed) {
    (true, true) => format!("thread {t} is in memory again; its messages are being remembered"),
    (true, false) => format!("thread {t} is in memory"),
    (false, true) => format!("thread {t} is out of memory: never remembered, never recalled"),
    (false, false) => format!("thread {t} is already out of memory"),
  }]))
}

/// A thread's tool and skill sets (`Request::Toolset`): the changes given
/// written first; without a thread, a new one's in the place of `cwd`,
/// the changes applied but not written.
fn toolset(
  s: &Arc<Shared>,
  thread: Option<&str>,
  cwd: Option<&str>,
  global: bool,
  tools: &[String],
  skills: &[String],
) -> Result<Reply> {
  let (tools, skills) = (
    crate::toolset::parse_all(tools)?,
    crate::toolset::parse_all(skills)?,
  );
  let here = s.place_of_dir(cwd);
  let thread: Option<RecordId> = match thread {
    None => None,
    Some("last") => Some(
      s.last_thread(&here.0)?
        .ok_or_else(|| Error::Config("no thread yet".into()))?,
    ),
    Some(id) => Some(id.parse()?),
  };
  let state = s.state();
  let (place, selection) = match &thread {
    Some(t) => {
      let (root, place) = s.thread_place(t)?;
      crate::toolset::write_changes(&root, &state.index, &state.node, t, &tools, &skills)?;
      let start = s.start_dir(&place, cwd.map(Path::new), Some(t));
      (
        place.clone(),
        s.selection(&place, start.as_deref(), Some(t)),
      )
    }
    None => {
      let place = if global {
        Place::Global
      } else {
        here.0.clone()
      };
      let start = s.start_dir(&place, cwd.map(Path::new), None);
      let mut sel = s.selection(&place, start.as_deref(), None);
      sel.tools.extend(tools);
      sel.skills.extend(skills);
      (place, sel)
    }
  };
  let (tools, skills) = s.set_items(&place, &selection);
  Ok(Reply::Toolset {
    thread: thread.map(|t| t.to_string()),
    tools,
    skills,
  })
}

/// A session's output streaming to an attached client; it stops with the
/// connection, however that ends.
struct Streaming(JoinHandle<()>);

impl Drop for Streaming {
  fn drop(&mut self) {
    self.0.abort();
  }
}

/// Approvals waiting on this connection's client.
pub(crate) type Waiting = Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Decision>>>>;

/// A chat's approvals: `question` records, as a detached thread's are,
/// asked on the client's connection as well. A client that leaves answers
/// nothing: the question waits on the attention list, so the turn goes on
/// (design §2.3).
fn approver(s: Arc<Shared>, tx: mpsc::UnboundedSender<Reply>, waiting: Waiting) -> Approver {
  let next = Arc::new(AtomicU64::new(1));
  Arc::new(move |req: ApprovalRequest| {
    let inline = crate::detached::Inline {
      tx: tx.clone(),
      waiting: waiting.clone(),
      id: next.fetch_add(1, Ordering::Relaxed),
    };
    let s = s.clone();
    Box::pin(async move {
      match s.thread_place(&req.thread) {
        Ok((root, _)) => crate::detached::ask(&s, &root, req, None, Some(inline)).await,
        Err(e) => Decision::decline(Some(format!("the approval could not be recorded: {e}"))),
      }
    })
  })
}

pub(crate) async fn serve<R, W>(
  s: &Arc<Shared>,
  mut r: R,
  w: W,
  peer: crate::peer::Peer,
) -> Result<()>
where
  R: tokio::io::AsyncRead + Unpin + Send + 'static,
  W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
  let first = api::read::<Request, _>(&mut r).await?;
  // A program speaking the events protocol (docs/events.md): stable JSON
  // frames, whatever this node's version; never a fixer's.
  if let Some(Request::Hello { version }) = &first
    && version == crate::events::PROTOCOL
  {
    let mut w = w;
    if crate::fixer::connection(s, peer).await != crate::fixer::Connection::Free {
      let _ = crate::events::refuse(
        &mut w,
        "the node serves no events to a fixer's commands (docs/self-improve.md §5)",
      )
      .await;
      return Ok(());
    }
    return crate::events::serve(s, r, w).await;
  }
  let (tx, mut rx) = mpsc::unbounded_channel::<Reply>();
  let writer = tokio::spawn(async move {
    let mut w = w;
    while let Some(reply) = rx.recv().await {
      if api::write(&mut w, &reply).await.is_err() {
        break;
      }
    }
  });
  match first {
    Some(Request::Hello { version }) => {
      let differs = version != VERSION;
      let newer = crate::api::newer(&version, VERSION);
      let mut busy = if differs { s.busy().await } else { Vec::new() };
      // A newer client: when another build is installed where this
      // node's binary is, the node restarts into it (under a service,
      // exiting for it; else starting it itself). Otherwise, a node under
      // a service stays (the service starts only its own binary: the
      // client's is elsewhere), and one outside stops for the client to
      // start its own.
      let handover = if !newer {
        None
      } else if tokio::task::spawn_blocking(installed_differs)
        .await
        .unwrap_or(false)
      {
        Some(Some(Restart {
          why: format!("version {version}: the build installed"),
        }))
      } else if supervision().supervised {
        busy.push(format!(
          "nothing: a service runs this node from its installed binary; install the \
           stitch of version {version} there, and it restarts into it"
        ));
        None
      } else {
        Some(None)
      };
      let restarting = handover.is_some() && busy.is_empty();
      let _ = tx.send(Reply::Hello {
        version: VERSION.into(),
        node: s.state().node.to_string(),
        busy,
        restarting,
      });
      if differs {
        // Said before stopping; a client of another version goes no further.
        drop(tx);
        let _ = writer.await;
        if let Some(restart) = handover {
          let why = format!("version {version}");
          if restarting {
            eprintln!("stopping for {why}");
            if let Some(r) = restart {
              s.restart.lock().expect("restart lock").get_or_insert(r);
            }
            s.shut_down();
          } else {
            s.stop_when_idle(why, restart, false);
          }
        }
        return Ok(());
      }
    }
    _ => return Err(Error::Protocol("expected Hello".into())),
  }
  // A fixer's commands (told from their process, not from what they
  // say) are served only what is a fixer's (docs/self-improve.md §5).
  let who = crate::fixer::connection(s, peer).await;
  let waiting: Waiting = Arc::default();
  let mut running: Option<JoinHandle<()>> = None;
  // What the client says to the chat running on this connection.
  let mut steer = chat::Steer::default();
  // The shell session this connection is attached to, and its stream.
  let mut attached: Option<(RecordId, String, stitch_shell::Session, Streaming)> = None;
  // The browser profile this connection links, if it is one (`stitch
  // browser-host`).
  let mut browser_link: Option<Arc<crate::browser::Link>> = None;
  // A stopped node serves no one: its connections close.
  let mut stopping = s.shutdown.subscribe();
  loop {
    let req = tokio::select! {
      req = api::read::<Request, _>(&mut r) => match req {
        Ok(Some(req)) => req,
        Ok(None) => break,
        Err(e) => {
          steer.leave();
          return Err(e);
        }
      },
      _ = stopping.wait_for(|stopped| *stopped) => break,
    };
    let refused = crate::fixer::refuses(s, &who, &req);
    // What ends the connection (and the node) now.
    if refused.is_none() {
      match &req {
        Request::Stop => {
          let _ = tx.send(Reply::Ok);
          drop(tx);
          let _ = writer.await;
          s.shut_down();
          return Ok(());
        }
        &Request::Restart {
          now: true,
          successor,
        } => {
          let what = if successor {
            "a restart asked"
          } else {
            "a stop asked"
          };
          if successor {
            s.restart
              .lock()
              .expect("restart lock")
              .get_or_insert(Restart { why: what.into() });
          }
          eprintln!("stopping now: {what}");
          let _ = tx.send(Reply::Lines(vec![if successor {
            "restarting now: detached threads resume".into()
          } else {
            "stopping now".into()
          }]));
          drop(tx);
          let _ = writer.await;
          s.shut_down();
          return Ok(());
        }
        _ => {}
      }
    }
    // A request's handler that fails (`?` in an arm) is answered with its
    // error, `Reply::Error`; it never ends the connection, which only its
    // own I/O does. `Ok(None)`: answered already, or nothing to answer.
    let reply: Result<Option<Reply>> = async {
      let reply = match req {
      _ if refused.is_some() => Err(Error::Config(refused.unwrap_or_default())),
      Request::Hello { .. } => Err(Error::Protocol("Hello twice".into())),
      // A chat runs on its own, so this connection can still answer
      // approvals; it goes on to the end if the client leaves.
      req @ (Request::Chat { .. } | Request::Resume { .. }) => {
        if running.as_ref().is_some_and(|t| !t.is_finished()) {
          Err(Error::Protocol(
            "a chat already runs on this connection".into(),
          ))
        } else {
          steer = chat::Steer::open();
          let owner_default = peer.owner_default();
          let (s, tx, waiting, steer) = (s.clone(), tx.clone(), waiting.clone(), steer.clone());
          s.chats.fetch_add(1, Ordering::SeqCst);
          running = Some(tokio::spawn(async move {
            let approve = approver(s.clone(), tx.clone(), waiting);
            let reply = chat(&s, req, &tx, approve, steer, owner_default)
              .await
              .unwrap_or_else(|e| {
                if matches!(e, Error::Kept { .. }) {
                  let _ = tx.send(Reply::Kept);
                }
                Reply::Error(e.to_string())
              });
            s.chats.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(reply);
          }));
          return Ok(None);
        }
      }
      Request::Detach {
        goal,
        model,
        repo,
        cwd,
        parent,
        budget_usd,
        budget_tokens,
        budget_minutes,
        max_steps,
        global,
        node,
        context,
        no_tools,
        tools,
        skills,
        keep,
        memory,
        read_only,
        scopes,
      } => {
        async {
          let parent: Option<RecordId> = parent.map(|p| p.parse()).transpose()?;
          let spec = crate::weave::Spawn {
            goal,
            node,
            model,
            tools: !no_tools,
            from: crate::weave::parse_from(context.as_deref())?,
            budget: chat::Limits {
              usd: budget_usd,
              tokens: budget_tokens,
              until: budget_minutes
                .map(|m| jiff::Timestamp::now() + jiff::SignedDuration::from_secs(m as i64 * 60)),
              steps: max_steps.filter(|n| *n > 0),
            },
            repo,
            cwd,
            global,
            place: None,
            toolset: crate::toolset::parse_all(&tools)?,
            skills: crate::toolset::parse_all(&skills)?,
            wake: !keep,
            // `--memory` remembers it; recalling is every spawned
            // thread's default.
            remember: memory,
            recall: true,
            read_only,
            fix: None,
            own_budget: false,
          };
          crate::weave::spawn_scoped(s, parent.as_ref(), None, spec, &scopes)
            .await
            .map(|spawned| Reply::Detached(spawned.id.to_string()))
        }
        .await
      }
      Request::Send {
        thread,
        text,
        from,
        keep,
        interject,
        scopes,
      } => {
        async {
          let thread: RecordId = thread.parse()?;
          let from: Option<RecordId> = from.map(|f| f.parse()).transpose()?;
          let (node, sent) = crate::weave::send_as(
            s,
            from.as_ref(),
            &thread,
            &text,
            !keep,
            interject,
            None,
            &scopes,
          )
          .await?;
          Ok(Reply::Lines(vec![format!(
            "sent to thread {thread}, on {node}{}{}",
            if interject {
              format!(": {}", sent.said())
            } else {
              String::new()
            },
            from
              .map(|f| format!("; its reply is reported to thread {f}"))
              .unwrap_or_default()
          )]))
        }
        .await
      }
      Request::Scope { scopes, query } => async {
        let scoped = crate::scopes::scoped(s, &scopes)
          .await?
          .ok_or_else(|| Error::Config("no scope given".into()))?;
        let mut lines = vec![scoped.briefing];
        if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
          let texts: Vec<String> = scopes.iter().map(crate::scopes::target_text).collect();
          let (s2, q2) = (s.clone(), q.clone());
          let found = tokio::task::spawn_blocking(move || {
            crate::scopes::scope_search(&s2, &texts, &q2)
          })
          .await
          .map_err(|e| Error::Config(e.to_string()))??;
          lines.push(format!(
            "{}{}",
            found.header,
            if found.exact.is_empty() {
              format!("\nNo exact matches for \"{q}\".\n")
            } else {
              found.exact
            }
          ));
        }
        Ok(Reply::Lines(lines))
      }
      .await,
      Request::Answer {
        question,
        allow,
        reason,
        always,
      } => async {
        let q: RecordId = question.parse()?;
        // A question for the user alone (a landing, the fixers'
        // settings): never from a thread's commands, however detached.
        if crate::fixer::user_only(s, &q)
          && let Err(why) = crate::peer::the_user_alone(s, peer).await
        {
          return Err(Error::Config(why));
        }
        Ok(q)
      }
      .await
      .and_then(|q| {
        crate::detached::answer(
          s,
          &q,
          crate::tools::Decision {
            allow: allow || always,
            always,
            reason,
          },
        )
      })
      .map(|()| {
        // The thread may wait on another node.
        let s = s.clone();
        tokio::spawn(async move {
          sync_all(&s, None).await;
        });
        Reply::Ok
      }),
      Request::Split { cwd } => s.split(&cwd).map(Reply::Split),
      Request::Rebind { cwd, project } => s.rebind(&cwd, project.as_deref()).map(Reply::Rebind),
      Request::Rename { thread, title } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| crate::meta::rename(s, &t, &title))
        .map(|()| Reply::Ok),
      Request::Instructions {
        place,
        cwd,
        global,
        set,
      } => {
        let place = match place {
          Some(key) => Place::of_key(&key).ok_or_else(|| Error::Config(format!("no place {key}"))),
          None => {
            let here = if global {
              (Place::Global, None)
            } else {
              s.place_of_dir(cwd.as_deref())
            };
            // Written for a checkout seen for the first time: listed as a
            // project.
            match &set {
              Some(_) => s.placement(None, global, &here).map(|(_, p)| p),
              None => Ok(here.0.clone()),
            }
          }
        };
        place.and_then(|place| {
          if let Some(text) = &set {
            crate::instructions::set(s.state(), &place, text)?;
          }
          Ok(Reply::Instructions {
            place: place.label(s.state().node.as_str(), &s.places()),
            text: crate::instructions::current(s.state(), &place),
          })
        })
      }
      Request::RunOn { thread, node } => match thread.parse::<RecordId>() {
        Ok(t) => crate::weave::run_on(s, &t, node.as_deref())
          .await
          .map(|said| Reply::Lines(vec![said])),
        Err(e) => Err(e.into()),
      },
      Request::Runners => Ok(Reply::Lines(
        crate::elsewhere::runners(s)
          .await
          .into_iter()
          .map(|(n, runs)| format!("{n}\t{}", runs.word()))
          .collect(),
      )),
      Request::InFlight { thread } => crate::elsewhere::in_flight(s, &thread).await,
      Request::Threads {
        status,
        since,
        limit,
        thread,
        place,
        forest,
      } => {
        let mut q = vec!["all=1".to_owned(), "every=1".to_owned()];
        if forest == Some(true) {
          q.push("conversations=1".to_owned());
        }
        q.extend(place.map(|v| {
          let enc: String = v
            .bytes()
            .map(|b| {
              if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' || b == b'/' {
                (b as char).to_string()
              } else {
                format!("%{b:02X}")
              }
            })
            .collect();
          format!("place={enc}")
        }));
        q.extend(thread.map(|v| format!("id={v}")));
        q.extend(status.map(|v| format!("status={v}")));
        q.extend(since.map(|v| format!("since={}", v.replace('+', "%2B"))));
        q.extend(limit.map(|v| format!("limit={v}")));
        s.list_read(move |s| crate::apps::live::threads(&s, None, &q.join("&")))
          .await?
          .map(|v| Reply::Lines(vec![v.to_string()]))
          .map_err(Error::Config)
      }
      Request::Live { what, query } => {
        let q = query.unwrap_or_default();
        let v = match what.as_str() {
          "attention" => s.list_read(|s| crate::apps::live::attention(&s)).await?,
          "jobs" => crate::apps::live::jobs(s).await,
          "schedules" => {
            s.list_read(move |s| crate::apps::live::schedules(&s, &q))
              .await?
          }
          "places" => s.list_read(|s| crate::apps::live::places(&s)).await?,
          "loose-ends" => s.list_read(|s| crate::loose::live(&s)).await?,
          "events" => crate::events::live(s, &q),
          other => {
            return Err(Error::Config(format!(
              "live `{other}`: attention, jobs, schedules, loose-ends, places or events"
            )));
          }
        };
        Ok(Reply::Lines(vec![v.to_string()]))
      }
      Request::Transcript {
        thread,
        after,
        before,
        tail,
      } => {
        let t: RecordId = thread.parse()?;
        let window = crate::apps::threads::Window { after, before, tail };
        s.list_read(move |s| crate::apps::threads::transcript(&s, &t, window))
          .await
          .map(|v| Reply::Lines(vec![v.to_string()]))
      }
      Request::RunsOn { thread } => thread.parse::<RecordId>().map_err(Error::from).map(|t| {
        let pinned = crate::meta::runs_on(&s.state().index.lock().expect("index lock"), &t);
        Reply::Lines(pinned.into_iter().collect())
      }),
      Request::Pin { thread, on } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| crate::meta::pin(s, &t, on))
        .map(|()| Reply::Ok),
      Request::Press { thread, set, item } => {
        let t: RecordId = thread.parse()?;
        let pressed = crate::actions::press(s, &t, &set, item as usize).await?;
        Ok(Reply::Lines(vec![serde_json::json!(pressed).to_string()]))
      }
      Request::Read { thread, upto } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| {
          let upto = upto
            .map(|u| {
              u.parse::<jiff::Timestamp>()
                .map_err(|e| Error::Config(format!("upto `{u}`: {e} (an instant, as RFC 3339)")))
            })
            .transpose()?;
          crate::meta::read(s, &t, upto)
        })
        .map(|_| Reply::Ok),
      Request::Artefacts { thread } => s
        .list_read(move |s| crate::artefacts::of_conversation(&s, &thread))
        .await?
        .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::DigestStatus { sweep } => crate::digests::status_request(s, sweep.unwrap_or(false))
        .await
        .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::Digest { thread, update } => crate::digests::request(s, &thread, update.unwrap_or(false))
        .await
        .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::Touched { kind, value } => s
        .list_read(move |s| crate::artefacts::touched(&s, &kind, &value))
        .await?
        .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::ReadAll { place } => s
        .list_read(move |s| crate::meta::read_all(&s, place.as_deref()))
        .await?
        .map(|n| Reply::Lines(vec![n.to_string()])),
      Request::Opened { thread } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| crate::loose::opened(s, &t))
        .map(|_| Reply::Ok),
      Request::LooseEnds { check, dismiss } => {
        let mut lines = Vec::new();
        if let Some(t) = dismiss {
          let t: RecordId = t.parse()?;
          crate::loose::dismiss(s, &t)?;
          lines.push(format!("Dismissed {t} until it moves again."));
        }
        lines.extend(crate::loose::on_demand(s, check).await?);
        Ok(Reply::Lines(lines))
      }
      Request::LooseEndsAuto { mode, rules } => {
        async {
          if (mode.is_some() || rules.is_some())
            && let Err(why) = crate::peer::the_user_alone(s, peer).await
          {
            return Err(Error::Config(why.replace(
              "answers this question",
              "sets loose ends' auto-handling",
            )));
          }
          let a = if mode.is_some() || rules.is_some() {
            crate::loose::set_auto(s, mode.as_deref(), rules.as_deref())?
          } else {
            crate::loose::auto(s)
          };
          s.apps.live.bump(&["threads"]);
          Ok(Reply::Lines(crate::loose::auto_lines(&a)))
        }
        .await
      }
      Request::ReadOnly { thread, on } => match thread.parse::<RecordId>() {
        Ok(t) => {
          let set = match on {
            Some(on) => s.set_read_only(&t, on).await,
            None => Ok(()),
          };
          set.map(|()| {
            if let Some(repo) = on.and_then(|_| s.thread_place(&t).ok()) {
              let (s, repo) = (s.clone(), repo.0.repo().clone());
              tokio::spawn(async move { sync_one(&s, &repo).await });
            }
            Reply::Lines(vec![mode_word(s.read_only(&t)).to_owned()])
          })
        }
        Err(e) => Err(e.into()),
      },
      Request::Share {
        thread,
        effort,
        level,
        preview,
        confirm,
      } => async {
        let t: RecordId = thread.parse()?;
        let t = s
          .state()
          .index
          .lock()
          .expect("index lock")
          .compaction_newest(&t);
        let level = crate::share::Level::parse(&level)?;
        let s2 = s.clone();
        let p = tokio::task::spawn_blocking(move || {
          if preview {
            crate::share::preview(&s2, &t, effort, level)
          } else {
            crate::share::export(&s2, &t, effort, level, confirm.as_deref())
          }
        })
        .await
        .map_err(|e| Error::Config(e.to_string()))??;
        Ok(Reply::Lines(vec![
          serde_json::to_string(&p).expect("a preview serializes"),
        ]))
      }
      .await,
      Request::Import { bundle, preview } => async {
        let s2 = s.clone();
        let i = tokio::task::spawn_blocking(move || crate::share::import(&s2, &bundle, !preview))
          .await
          .map_err(|e| Error::Config(e.to_string()))??;
        Ok(Reply::Lines(vec![
          serde_json::to_string(&i).expect("an import serializes"),
        ]))
      }
      .await,
      Request::Quote { thread } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| crate::meta::quote(s, &t))
        .map(|p| Reply::Lines(vec![p.display().to_string()])),
      Request::Search {
        query,
        here,
        cwd,
        limit,
      } => {
        let place = here.then(|| s.place_of_dir(cwd.as_deref()).0);
        let places = std::sync::Mutex::new(HashMap::<RecordId, bool>::new());
        let keep = |t: &RecordId| match &place {
          None => true,
          Some(p) => *places
            .lock()
            .expect("places lock")
            .entry(t.clone())
            .or_insert_with(|| s.thread_place(t).is_ok_and(|(_, tp)| tp == *p)),
        };
        Ok(Reply::Hits(
          s.memory
            .search(s.state(), &query, &keep, limit.clamp(1, 200))
            .await,
        ))
      }
      Request::Fork {
        thread,
        upto,
        last,
        no_attachments,
        no_tools,
        summarize,
      } => {
        let r = async {
          let spec = crate::fork::ForkSpec {
            thread: thread.parse()?,
            upto,
            last,
            no_attachments,
            no_tools,
            summarize,
          };
          // What it reads that is on other nodes only (a summary reads the
          // attachments) is fetched once, and the fork tried again.
          // "Summarize and truncate" at its latest message is a
          // compaction (design §2.15): every client's.
          let forked = match crate::fork::requested(s, spec.clone()).await {
            Err(Error::MissingBlobs(hashes)) => {
              fetch_blobs(s, &hashes).await;
              crate::fork::requested(s, spec).await.map_err(|e| match e {
                Error::MissingBlobs(h) => Error::Config(format!(
                  "the fork needs {} file(s) of the thread that are not on this node, and no \
                   other node reachable now had them",
                  h.len()
                )),
                e => e,
              })
            }
            r => r,
          };
          forked.map(|t| Reply::Forked(t.to_string()))
        }
        .await;
        // Logged: a client may not show it (design §5, milestone 4).
        if let Err(e) = &r {
          eprintln!("fork of thread {thread}: failed: {e}");
        }
        r
      }
      Request::Jobs => jobs(s).await.map(Reply::Jobs),
      Request::StopJob { id } => stop_job(s, &id).await.map(|l| Reply::Lines(vec![l])),
      Request::Event {
        topic,
        payload,
        ask,
        wait,
      } => crate::events::publish_waiting(s, &topic, &payload, ask, wait)
        .await
        .map(Reply::Event),
      Request::EventStatus { id, wait_ms } => crate::events::status(
        s,
        &id,
        std::time::Duration::from_millis(wait_ms.min(3_600_000)),
      )
      .await
      .map(Reply::Event),
      Request::Events { input, cwd: _ } => {
        let input: crate::events::SubscribeInput =
          serde_json::from_str(&input).map_err(|e| Error::Config(e.to_string()))?;
        let text = crate::events::act(s, input).await?;
        Ok(Reply::Lines(text.lines().map(str::to_owned).collect()))
      }
      Request::Schedule { input, cwd } => {
        let input: crate::schedules::Input =
          serde_json::from_str(&input).map_err(|e| Error::Config(e.to_string()))?;
        let text = crate::schedules::act(s, crate::schedules::Caller::User { cwd }, input).await?;
        Ok(Reply::Lines(text.lines().map(str::to_owned).collect()))
      }
      Request::Secret { op, cwd } => crate::secrets::request(s, op, cwd).await,
      Request::Decide { input } => {
        let caller = crate::peer::identify(s, peer).await;
        let input: serde_json::Value =
          serde_json::from_str(&input).map_err(|e| Error::Config(e.to_string()))?;
        crate::decide::request(s, &caller, input)
          .await
          .map(Reply::Lines)
      }
      Request::ModelSettings { input } => {
        let input: crate::modelset::Input =
          serde_json::from_str(&input).map_err(|e| Error::Config(e.to_string()))?;
        crate::modelset::request(s, input).await.map(Reply::Lines)
      }
      Request::Ssh { op } => crate::ssh::request(s, op).await,
      Request::DeleteThreads {
        threads,
        spawned_by,
        dry_run,
        from_thread,
      } => crate::delete::request(s, threads, spawned_by, dry_run, from_thread)
        .await
        .map(Reply::Lines),
      Request::Rewind { thread, upto } => thread
        .parse()
        .map_err(Error::from)
        .and_then(|t| crate::rewind::rewind(s, &t, upto.as_deref()))
        .map(|()| Reply::Ok),
      Request::Cancel { thread } => thread
        .parse::<RecordId>()
        .map_err(Error::from)
        .and_then(|t| {
          // A room's round, or a participant speaking in one, stops here.
          if s.rooms.cancel(&t) {
            return Ok(());
          }
          if crate::rooms::is_room_here(s, &t) {
            return Err(Error::Config(format!("no round runs in room {t} here")));
          }
          crate::detached::cancel(s, &t)
        })
        .map(|()| Reply::Ok),
      Request::RoomCreate {
        title,
        place,
        from_thread,
        participants,
        scopes,
        mode,
        hop_cap,
        budget_usd,
      } => crate::rooms::create(
        s,
        crate::rooms::Create {
          title,
          place,
          from_thread,
          participants,
          scopes,
          mode,
          hop_cap,
          budget_usd,
        },
      )
      .await
      .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::RoomInfo { thread } => thread
        .parse::<RecordId>()
        .map_err(Error::from)
        .and_then(|t| crate::rooms::info(s, &t))
        .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::RoomUpdate {
        thread,
        add,
        remove,
        mute,
        unmute,
        mode,
        hop_cap,
      } => async {
        let t: RecordId = thread.parse()?;
        crate::rooms::update(
          s,
          &t,
          crate::rooms::Update {
            add,
            remove,
            mute,
            unmute,
            mode,
            hop_cap,
          },
        )
        .await
      }
      .await
      .map(|v| Reply::Lines(vec![v.to_string()])),
      Request::Interject {
        id,
        text,
        attachments,
      } => {
        // An attachment that cannot be read leaves the message unwoven:
        // the client sends it after, and hears why then.
        let atts: Result<Vec<_>> = attachments
          .iter()
          .map(|p| attachment_arg(s, p))
          .collect();
        if let Ok(attachments) = atts {
          steer.push(chat::Interjection {
            id: Some(id),
            from: None,
            text,
            attachments,
            reports: Vec::new(),
            untold: None,
            asker: None,
            relayed: None,
            scope: None,
          });
        }
        return Ok(None);
      }
      Request::Retract { id } => {
        steer.retract(id);
        return Ok(None);
      }
      Request::Interrupt => {
        steer.interrupt();
        return Ok(None);
      }
      Request::Suspend => {
        steer.suspend();
        return Ok(None);
      }
      Request::ShellAttach { id } => match s.shells.session_of(&id).await {
        None => Err(Error::Config(format!("no session {id} here"))),
        Some((thread, name, session)) => {
          // Attached elsewhere before: that stream stops.
          drop(attached.take());
          let mut output = session.subscribe();
          // Shown with the node's secrets hidden, as tools' output is
          // (design §2.19): a value split between two chunks too, what may
          // start one held back until the next chunk, or a moment.
          let mut redactor = s.vault.stream();
          let mut first = redactor.push(&session.screen_drawn());
          first.extend(redactor.flush());
          let _ = tx.send(Reply::ShellOutput(first));
          let tx = tx.clone();
          // Its end is the session's (the shell gone), not the output
          // channel's, which lives as long as anyone holds the session.
          let watched = session.clone();
          let stream = tokio::spawn(async move {
            let hold = Duration::from_millis(100);
            loop {
              tokio::select! {
                got = output.recv() => match got {
                  Ok(bytes) => {
                    let shown = redactor.push(&bytes);
                    if !shown.is_empty() && tx.send(Reply::ShellOutput(shown)).is_err() {
                      return;
                    }
                  }
                  Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                  Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    let _ = tx.send(Reply::ShellOutput(redactor.flush()));
                    let _ = tx.send(Reply::ShellEnded);
                    return;
                  }
                },
                () = tokio::time::sleep(hold), if redactor.holding() => {
                  let _ = tx.send(Reply::ShellOutput(redactor.flush()));
                }
                () = watched.ended() => {
                  // What it printed last, then the end.
                  while let Ok(bytes) = output.try_recv() {
                    let _ = tx.send(Reply::ShellOutput(redactor.push(&bytes)));
                  }
                  let _ = tx.send(Reply::ShellOutput(redactor.flush()));
                  let _ = tx.send(Reply::ShellEnded);
                  return;
                }
              }
            }
          });
          attached = Some((thread, name, session, Streaming(stream)));
          return Ok(None);
        }
      },
      Request::ShellInput(bytes) => {
        if let Some((thread, name, session, _)) = &attached
          && let Err(e) = s.shells.typed(thread, name, session, &bytes).await
        {
          let _ = tx.send(Reply::Error(e.to_string()));
        }
        return Ok(None);
      }
      Request::Approval {
        id,
        allow,
        reason,
        always,
      } => {
        if let Some(answer) = waiting.lock().expect("approvals lock").remove(&id) {
          let _ = answer.send(Decision {
            allow: allow || always,
            always,
            reason,
          });
        }
        return Ok(None);
      }
      Request::Sync { peer } => Ok(Reply::Synced(sync_all(s, peer.as_deref()).await)),
      Request::ProjectFetch { repo } => {
        let repo: RepoId = repo.parse()?;
        Ok(Reply::Lines(vec![
          crate::clones::refresh(s, &repo).await?.said(),
        ]))
      }
      Request::ProjectCloneQuestion { repo, again } => {
        let repo: RepoId = repo.parse()?;
        Ok(Reply::Lines(
          crate::clones::question(s, &repo, again)
            .await
            .into_iter()
            .collect(),
        ))
      }
      Request::ProjectClone { repo, allow } => {
        let repo: RepoId = repo.parse()?;
        Ok(Reply::Lines(vec![
          crate::clones::answer(s, &repo, allow).await?.said(),
        ]))
      }
      Request::ProjectSwitch { repo, branch } => {
        let repo: RepoId = repo.parse()?;
        let n = crate::clones::switch_branch(s, &repo, &branch).await?;
        Ok(Reply::Lines(vec![format!(
          "on {branch}: {n} file(s) changed"
        )]))
      }
      Request::ProjectDiscard { repo, path } => {
        let repo: RepoId = repo.parse()?;
        let n = crate::clones::discard_changes(s, &repo, path.as_deref()).await?;
        Ok(Reply::Lines(vec![format!("{n} file(s) put back")]))
      }
      Request::ProjectCommit {
        repo,
        message,
        new_branch,
        pull_request,
      } => {
        let repo: RepoId = repo.parse()?;
        let target = match new_branch {
          Some(name) => crate::clones::Target::NewBranch { name, pull_request },
          None => crate::clones::Target::Current,
        };
        let pushed = crate::clones::commit_push(s, &repo, &message, target).await?;
        Ok(Reply::Lines(vec![pushed.said()]))
      }
      Request::RegisterDevice { token, env, key } => {
        let home = s
          .state()
          .home()
          .ok_or_else(|| Error::Config("no home workspace here".into()))?;
        let new =
          crate::apns::register(&home, &s.state().index, &s.state().node, &token, &env, &key)?;
        if new {
          // The nodes that push learn of it now.
          sync_one(s, home.repo()).await;
        }
        Ok(Reply::Ok)
      }
      Request::RegisterVoip { token, env, key } => {
        let home = s
          .state()
          .home()
          .ok_or_else(|| Error::Config("no home workspace here".into()))?;
        let new = crate::apns::register_voip(
          &home,
          &s.state().index,
          &s.state().node,
          &token,
          &env,
          &key,
        )?;
        if new {
          sync_one(s, home.repo()).await;
        }
        Ok(Reply::Ok)
      }
      Request::RingOutcome {
        node,
        call,
        outcome,
      } => {
        crate::rings::tell(s, &node, &call, &outcome).await?;
        Ok(Reply::Ok)
      }
      r @ (Request::Apps
      | Request::AppOpen { .. }
      | Request::AppPin { .. }
      | Request::AppUnpin { .. }
      | Request::AppLive { .. }
      | Request::AppThreads { .. }
      | Request::AppContainer { .. }
      | Request::AppStorage { .. }
      | Request::AppExport { .. }
      | Request::Dashboard
      | Request::Client) => crate::apps::request(s, r).await,
      Request::ApnsTest => Ok(Reply::Lines(
        push(
          s,
          &crate::apns::Push {
            title: "Stitch",
            subtitle: None,
            body: &format!("A test from {}.", s.state().node),
            badge: None,
            question: None,
            node: s.state().node.as_str(),
            answered: false,
            thread: None,
          },
        )
        .await,
      )),
      Request::Notify {
        title,
        body,
        thread,
      } => {
        let thread = thread
          .filter(|t| !t.is_empty())
          .map(|t| {
            t.parse::<RecordId>()
              .map_err(|e| Error::Config(format!("thread {t}: {e}")))
          })
          .transpose()?;
        Ok(Reply::Lines(
          crate::notices::send(s, thread.as_ref(), title.as_deref(), &body).await?,
        ))
      }
      Request::Issue { input, thread: _ } => {
        // Who asks is the connection's process, not what it says
        // (`STITCH_THREAD` is the caller's to set).
        let caller = crate::peer::identify(s, peer).await;
        let input: serde_json::Value =
          serde_json::from_str(&input).map_err(|e| Error::Config(e.to_string()))?;
        crate::selfimprove::request(s, &caller, input)
          .await
          .map(Reply::Lines)
      }
      Request::FolderSync { key } => {
        crate::folders::hold_all(s);
        let p = crate::folders::pass_and_push(s, &key, None).await?;
        Ok(Reply::FolderPass {
          published: p.published,
          applied: p.applied,
          conflicts: p.conflicts,
          skipped: p.skipped,
        })
      }
      Request::FetchBlobs { hashes } => {
        let mut wanted = Vec::new();
        for h in hashes {
          let h: BlobHash = h.parse().map_err(|e| Error::Config(format!("{e}")))?;
          if !s.state().blobs.contains(&h) && !wanted.contains(&h) {
            wanted.push(h);
          }
        }
        Ok(Reply::Fetched(fetch_blobs(s, &wanted).await))
      }
      Request::Invite { ttl_secs } => {
        if s.state().home().is_none() {
          Err(Error::Config(
            "no home workspace here: a node that joins would be trusted by this node only, and get nothing to sync".into(),
          ))
        } else {
          let ticket = s.net.invite(Duration::from_secs(ttl_secs), None);
          Ok(Reply::Ticket(ticket.to_string()))
        }
      }
      Request::MemoryAudit => Ok(Reply::Lines(s.memory.audit())),
      // Every transcript parsed: off the runtime's workers.
      Request::MemoryNotes => {
        let s = s.clone();
        tokio::task::spawn_blocking(move || memory_notes(&s))
          .await
          .map_err(|e| Error::Protocol(format!("memory notes: {e}")))
          .and_then(|r| r)
          .map(Reply::Lines)
      }
      Request::Yolo { on } => {
        let to = on.map(|on| if on { "yolo" } else { "ask" });
        set_mode(s, peer, to, None).await
      }
      Request::Mode { mode, cwd } => set_mode(s, peer, mode.as_deref(), cwd.as_deref()).await,
      Request::Remember { thread, on, cwd } => remember(s, &thread, on, cwd.as_deref()).await,
      Request::Toolset {
        thread,
        cwd,
        global,
        tools,
        skills,
      } => toolset(
        s,
        thread.as_deref(),
        cwd.as_deref(),
        global,
        &tools,
        &skills,
      ),
      Request::Memory {
        thread,
        cwd,
        global,
      } => {
        let (facts, episodes) = s.memory.size();
        let thread_on = s
          .memory
          .available()
          .then(|| s.memory_of(thread.as_deref(), cwd.as_deref(), global))
          .flatten();
        Ok(Reply::Memory {
          available: s.memory.available(),
          facts: facts as u64,
          episodes: episodes as u64,
          thread_on,
        })
      }
      Request::EffectiveModel {
        thread,
        place,
        cwd,
        model,
      } => {
        let thread = thread.map(|t| t.parse::<RecordId>()).transpose()?;
        let place = if let Some(t) = &thread {
          s.thread_place(t)?.1
        } else if let Some(p) = place {
          s.place_named(&p)?.1
        } else {
          let here = s.place_of_dir(cwd.as_deref());
          s.saw(&here);
          here.0
        };
        Ok(Reply::Lines(vec![
          crate::modelset::chat_model(s, thread.as_ref(), &place, model.as_deref()).await,
        ]))
      }
      Request::Models { node } => match node.filter(|n| n != s.state().node.as_str()) {
        Some(node) => peer_models(s, &node).await,
        None => Ok(models(s).await),
      },
      Request::Status => {
        let state = s.state();
        let peers = state.known_peers().unwrap_or_default();
        let pending = match s.pending() {
          Some(why) => {
            let (others, shells) = s.busy_parts().await;
            let hold = match s.shells_held() {
              Some(d) if !shells.is_empty() => format!(
                "; commands in shells hold it {} min more at most",
                d.as_secs().div_ceil(60)
              ),
              _ => String::new(),
            };
            let busy: Vec<String> = others.into_iter().chain(shells).collect();
            let since = s
              .pending_since()
              .map(|t| format!(", waiting since {t}"))
              .unwrap_or_default();
            Some(if busy.is_empty() {
              format!("stopping for {why}")
            } else {
              format!(
                "stopping for {why} once idle{since} (now: {}){hold}",
                busy.join("; ")
              )
            })
          }
          None => None,
        };
        Ok(Reply::Status {
          node: state.node.to_string(),
          endpoint: s.net.endpoint().id().to_string(),
          repos: state.roots().iter().map(|r| r.repo().to_string()).collect(),
          peers: peers.into_iter().map(|p| p.name).collect(),
          build: crate::build().to_owned(),
          pending,
        })
      }
      Request::Restart { now, successor } => {
        // `now`: served above.
        debug_assert!(!now);
        let restart = successor.then(|| Restart {
          why: "a restart asked".into(),
        });
        let busy = s.busy().await;
        let what = if successor {
          "a restart asked"
        } else {
          "a stop asked"
        };
        s.stop_when_idle(what.into(), restart, true);
        let verb = if successor { "restarts" } else { "stops" };
        Ok(Reply::Lines(vec![if busy.is_empty() {
          format!("the node {verb} now (idle)")
        } else {
          format!("the node {verb} once idle (now: {})", busy.join("; "))
        }]))
      }
      Request::BrowserLink { hello } => match s.browsers.connect(&hello, tx.clone()) {
        Ok(link) => {
          if let Some(old) = browser_link.replace(link) {
            s.browsers.disconnect(&old);
          }
          return Ok(None);
        }
        Err(e) => Err(e),
      },
      Request::BrowserMsg(msg) => {
        if let Some(link) = &browser_link {
          s.browsers.incoming(link, &msg);
        }
        return Ok(None);
      }
      Request::BrowserCall {
        thread,
        tool,
        input,
        cwd,
      } => {
        let input: serde_json::Value = match serde_json::from_str(&input) {
          Ok(v) => v,
          Err(e) => {
            return Err(Error::Config(format!("the input is not JSON: {e}")));
          }
        };
        let thread_id: Option<RecordId> = thread.as_deref().and_then(|t| t.parse().ok());
        let key = thread.unwrap_or_else(|| "cli".into());
        // Outside a turn nothing can be asked: what asks is refused.
        let asker = crate::tools::Asker {
          approver: None,
          read_only: false,
          yolo: false,
        };
        let scratch = s.data.join("downloads");
        let ctx = crate::browser::engine::CallCtx {
          thread: &key,
          title: key.clone(),
          asker: &asker,
          thread_id: thread_id.as_ref(),
          started: std::time::Instant::now(),
          attended: true,
          scratch: &scratch,
          place: None,
          cwd: cwd.map(PathBuf::from),
        };
        let out = s
          .browsers
          .engine
          .call(&s.browsers, &ctx, &tool, &input)
          .await;
        let images = out
          .images
          .iter()
          .filter_map(|i| Some((i.media_type.clone(), std::fs::read(&i.path).ok()?)))
          .collect();
        Ok(Reply::ToolDone {
          ok: out.ok,
          output: out.output,
          images,
        })
      }
      Request::BrowserProfiles { disable } => s
        .browsers
        .profiles(disable.as_deref())
        .await
        .map(Reply::Lines)
        .map_err(Error::Config),
      Request::Stop => unreachable!("served above"),
      };
      reply.map(Some)
    }
    .await;
    match reply {
      Ok(Some(reply)) => {
        let _ = tx.send(reply);
      }
      Ok(None) => {}
      Err(e) => {
        let _ = tx.send(Reply::Error(e.to_string()));
      }
    }
  }
  // The client left: a chat of its still waiting for another turn is not
  // sent (one that began goes on to its end).
  steer.leave();
  if let Some(link) = browser_link {
    s.browsers.disconnect(&link);
  }
  drop(attached);
  drop(tx);
  let _ = writer.await;
  Ok(())
}

#[cfg(test)]
mod lock_tests {
  use super::*;

  /// A caller that may not write the data directory (a sandboxed shell)
  /// still sees the node holding its lock (issue ajt3): the lock file is
  /// only read, never created.
  #[cfg(unix)]
  #[test]
  fn the_holder_is_seen_without_writing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    assert!(!running(data));
    assert!(!data.join("node.lock").exists(), "a probe creates nothing");
    let held = lock(data).unwrap();
    // Not writable by this caller, as in the sandbox.
    std::fs::set_permissions(
      data.join("node.lock"),
      std::fs::Permissions::from_mode(0o444),
    )
    .unwrap();
    assert!(running(data));
    assert_eq!(node_pid(data), Some(std::process::id()));
    drop(held);
    // A process spawned by a parallel test holds a copy of each open file
    // it was forked with until it executes its program: the lock's, and
    // that of each probe, which takes the lock while it looks when the
    // lock is free. So the lock may be seen held for such a moment after
    // any sight of it free (issue 2026-11-05T124143Z-beast-zet7): wait
    // for it to be seen free, and check that one sight.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut seen = holder(data);
    while seen.is_some() && std::time::Instant::now() < deadline {
      std::thread::sleep(std::time::Duration::from_millis(20));
      seen = holder(data);
    }
    assert_eq!(seen, None);
  }

  /// A node stopped, a copy of its lock's open file still held for a
  /// moment (as by a process forked before it stopped, until it executes
  /// its program): a start waits it out instead of finding a node running
  /// (CI on 2026-11-04). A lock held for good still stops it.
  #[tokio::test]
  async fn a_start_waits_out_a_lock_held_for_a_moment() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let held = lock(&data).unwrap();
    // What a fork shares: the same open file, so the same lock.
    let copy = held.try_clone().unwrap();
    drop(held);
    assert!(matches!(lock(&data), Err(Error::AlreadyRunning(_))));
    let let_go = std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(200));
      drop(copy);
    });
    let taken = lock_settled(&data, Duration::from_secs(5)).await.unwrap();
    let_go.join().unwrap();

    let started = std::time::Instant::now();
    assert!(matches!(
      lock_settled(&data, Duration::from_millis(300)).await,
      Err(Error::AlreadyRunning(_))
    ));
    assert!(started.elapsed() >= Duration::from_millis(300));
    drop(taken);
  }
}

/// The built-in `browser` skill is listed only while a browser is linked
/// and enabled, as its tools are (issue 2026-11-10T194410Z-mbp-gcvm).
#[cfg(all(test, not(mobile)))]
mod browser_skill_tests {
  #[tokio::test]
  async fn the_browser_skill_is_listed_only_with_a_browser() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    crate::config::Config {
      node: Some("mbp".into()),
      discovery: Some("local".into()),
      sync_interval_secs: Some(0),
      ..Default::default()
    }
    .save(&data)
    .unwrap();
    let node = crate::start(&data).await.unwrap();
    let s = node.shared().clone();
    let place = crate::place::Place::Global;
    let has = |s: &super::Shared| s.skills(&place).iter().any(|k| k.name == "browser");
    assert!(!has(&s), "no browser linked: the skill is not listed");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let link = s
      .browsers
      .connect(
        r#"{"t":"hello","proto":1,"build":"dev","version":"0.1.0","profile":{"id":"abc","name":"","enabled":true},"browser":{"brand":"Google Chrome 153"}}"#,
        tx,
      )
      .unwrap();
    assert!(has(&s), "an enabled profile is connected: it is listed");

    // The prompt's list follows the thread's tools too: not for a selection
    // with the group off, nor for a fixer (which is never given the tools).
    let listed = |sel: &crate::toolset::Selection, t: Option<&stitch_records::RecordId>| {
      s.context(&place, None, sel, t)
        .contains("- browser: Work in the user's own Chrome")
    };
    let normal = crate::toolset::Selection::default();
    assert!(listed(&normal, None), "a normal selection lists it");
    let off = crate::toolset::Selection {
      tools: vec![crate::toolset::Entry::parse("-browser").unwrap()],
      ..Default::default()
    };
    assert!(!listed(&off, None), "group off: not listed");
    let fixer = stitch_records::RecordId::random(jiff::Timestamp::now(), &"mbp".parse().unwrap());
    s.detached
      .fixers
      .lock()
      .unwrap()
      .insert(fixer.as_str().to_owned());
    assert!(!listed(&normal, Some(&fixer)), "a fixer: not listed");
    let other = stitch_records::RecordId::random(jiff::Timestamp::now(), &"mbp".parse().unwrap());
    assert!(listed(&normal, Some(&other)), "another thread: listed");

    s.browsers.disconnect(&link);
    assert!(!has(&s), "disconnected again: not listed");
    node.stop().await.unwrap();
  }
}
