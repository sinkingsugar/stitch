//! One `agy acp` process: JSON-RPC over its standard input and output
//! (the Agent Client Protocol), its updates routed to their sessions.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::mcp::{Router, SessionEvent};

/// How long a request other than a prompt may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

/// A JSON-RPC error, or the process gone (`code` 0).
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
  pub code: i64,
  pub message: String,
}

impl RpcError {
  pub fn gone(why: &str) -> RpcError {
    RpcError {
      code: 0,
      message: why.to_owned(),
    }
  }
}

impl std::fmt::Display for RpcError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    if self.code == 0 {
      write!(f, "{}", self.message)
    } else {
      write!(f, "{} ({})", self.message, self.code)
    }
  }
}

type Waiting = Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>;

struct Wire {
  writer: mpsc::UnboundedSender<String>,
  waiting: Waiting,
  alive: AtomicBool,
}

impl Wire {
  fn send(&self, msg: &Value) {
    let _ = self.writer.send(msg.to_string());
  }
}

/// What starts a process.
pub struct Spawn<'a> {
  pub program: &'a Path,
  pub args: &'a [String],
  pub home: &'a Path,
  pub cwd: &'a Path,
  pub env: &'a [(String, String)],
  pub log: &'a Path,
}

pub struct AgyProc {
  wire: Arc<Wire>,
  next: AtomicU64,
  /// Turns under way in it (`Busy`): it is never stopped meanwhile.
  busy: AtomicUsize,
  child: Mutex<Option<tokio::process::Child>>,
}

/// A turn under way in a process, counted there while it lives.
pub struct Busy(Arc<AgyProc>);

impl Busy {
  pub fn new(proc: Arc<AgyProc>) -> Busy {
    proc.busy.fetch_add(1, Ordering::SeqCst);
    Busy(proc)
  }
}

impl Drop for Busy {
  fn drop(&mut self) {
    self.0.busy.fetch_sub(1, Ordering::SeqCst);
  }
}

impl std::fmt::Debug for AgyProc {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("AgyProc")
      .field("alive", &self.alive())
      .finish()
  }
}

impl Drop for AgyProc {
  fn drop(&mut self) {
    if let Some(mut c) = self.child.lock().expect("child lock").take() {
      let _ = c.start_kill();
    }
  }
}

impl AgyProc {
  /// Starts the process and initializes it.
  pub async fn spawn(s: Spawn<'_>, router: Arc<Router>) -> Result<Arc<AgyProc>, String> {
    let log = std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(s.log)
      .map_err(|e| format!("{}: {e}", s.log.display()))?;
    let mut cmd = tokio::process::Command::new(s.program);
    cmd
      .args(s.args)
      .env("HOME", s.home)
      .current_dir(s.cwd)
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::from(log))
      .kill_on_drop(true);
    for (k, v) in s.env {
      cmd.env(k, v);
    }
    let mut child = cmd
      .spawn()
      .map_err(|e| format!("could not start {}: {e}", s.program.display()))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (writer, mut lines) = mpsc::unbounded_channel::<String>();
    let wire = Arc::new(Wire {
      writer,
      waiting: Mutex::new(HashMap::new()),
      alive: AtomicBool::new(true),
    });
    tokio::spawn(async move {
      while let Some(line) = lines.recv().await {
        if stdin.write_all(line.as_bytes()).await.is_err()
          || stdin.write_all(b"\n").await.is_err()
          || stdin.flush().await.is_err()
        {
          break;
        }
      }
    });
    tokio::spawn(read(wire.clone(), router, BufReader::new(stdout)));
    let proc = Arc::new(AgyProc {
      wire,
      next: AtomicU64::new(1),
      busy: AtomicUsize::new(0),
      child: Mutex::new(Some(child)),
    });
    proc
      .request(
        "initialize",
        json!({
          "protocolVersion": 1,
          "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
          "clientInfo": {"name": "stitch", "version": env!("CARGO_PKG_VERSION")},
        }),
      )
      .await
      .map_err(|e| format!("agy did not start: {e}"))?;
    Ok(proc)
  }

  /// Whether a turn is under way in it.
  pub fn busy(&self) -> bool {
    self.busy.load(Ordering::SeqCst) > 0
  }

  pub fn alive(&self) -> bool {
    self.wire.alive.load(Ordering::SeqCst)
  }

  /// Sends a request; its answer comes on the receiver.
  pub fn start(&self, method: &str, params: Value) -> oneshot::Receiver<Result<Value, RpcError>> {
    let id = self.next.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    if !self.alive() {
      let _ = tx.send(Err(RpcError::gone("agy is not running")));
      return rx;
    }
    self
      .wire
      .waiting
      .lock()
      .expect("waiting lock")
      .insert(id, tx);
    self
      .wire
      .send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    rx
  }

  /// A request and its answer, within `REQUEST_TIMEOUT`.
  pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
    match tokio::time::timeout(REQUEST_TIMEOUT, self.start(method, params)).await {
      Ok(Ok(r)) => r,
      Ok(Err(_)) => Err(RpcError::gone("agy exited")),
      Err(_) => Err(RpcError::gone(&format!("agy did not answer {method}"))),
    }
  }

  /// Closes a session, not waiting for the answer.
  pub fn close(&self, session: &str) {
    drop(self.start("session/close", json!({"sessionId": session})));
  }

  pub fn notify(&self, method: &str, params: Value) {
    self
      .wire
      .send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
  }
}

async fn read(
  wire: Arc<Wire>,
  router: Arc<Router>,
  stdout: BufReader<tokio::process::ChildStdout>,
) {
  let mut lines = stdout.lines();
  while let Ok(Some(line)) = lines.next_line().await {
    let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
      continue;
    };
    let method = msg.get("method").and_then(Value::as_str);
    match (method, msg.get("id")) {
      (Some(m), Some(id)) => answer_agent(&wire, m, id, &msg),
      (Some("session/update"), None) => {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        if let (Some(sid), Some(update)) = (
          params.get("sessionId").and_then(Value::as_str),
          params.get("update"),
        ) {
          router.send(sid, SessionEvent::Update(update.clone()));
        }
      }
      (Some(_), None) => {}
      (None, Some(id)) => {
        let Some(id) = id.as_u64() else { continue };
        let tx = wire.waiting.lock().expect("waiting lock").remove(&id);
        if let Some(tx) = tx {
          let r = match msg.get("error") {
            Some(e) => Err(RpcError {
              code: e.get("code").and_then(Value::as_i64).unwrap_or(-1),
              message: e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_owned(),
            }),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
          };
          let _ = tx.send(r);
        }
      }
      (None, None) => {}
    }
  }
  wire.alive.store(false, Ordering::SeqCst);
  for (_, tx) in wire.waiting.lock().expect("waiting lock").drain() {
    let _ = tx.send(Err(RpcError::gone("agy exited")));
  }
}

/// What agy asks of the client: permission for a tool (Stitch's own
/// tools only; the hook decides first), and nothing else.
fn answer_agent(wire: &Wire, method: &str, id: &Value, msg: &Value) {
  let reply = match method {
    "session/request_permission" => {
      let params = msg.get("params").cloned().unwrap_or(Value::Null);
      let name = params
        .pointer("/toolCall/name")
        .and_then(Value::as_str)
        .unwrap_or("");
      let want = if name.starts_with("mcp_") {
        "allow"
      } else {
        "reject"
      };
      let options = params
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
      let pick = options.iter().find(|o| {
        o.get("kind")
          .and_then(Value::as_str)
          .is_some_and(|k| k.starts_with(want))
      });
      match pick.and_then(|o| o.get("optionId")) {
        Some(option) => json!({"jsonrpc": "2.0", "id": id, "result": {
          "outcome": {"outcome": "selected", "optionId": option}}}),
        None => {
          json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": {"outcome": "cancelled"}}})
        }
      }
    }
    _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported"}}),
  };
  wire.send(&reply);
}
