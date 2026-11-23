//! The hazard guard's questions and auto-land, the user's alone (design
//! §2.28), on a real node process against mock models.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{Config, ProviderConfig, ThreadSettings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A mock model fed as the test goes: each request waits for the next
/// response the test pushes (it learns IDs on the way); bodies recorded.
#[derive(Clone, Default)]
struct Model {
  responses: Arc<Mutex<VecDeque<String>>>,
  seen: Arc<Mutex<Vec<Value>>>,
}

impl Model {
  async fn serve(&self) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let me = self.clone();
    tokio::spawn(async move {
      loop {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
          let n = sock.read(&mut chunk).await.unwrap();
          if n == 0 {
            break;
          }
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let len: usize = head
              .lines()
              .find_map(|l| {
                l.strip_prefix("content-length:")
                  .map(|v| v.trim().parse().unwrap())
              })
              .unwrap_or(0);
            while buf.len() < end + 4 + len {
              let n = sock.read(&mut chunk).await.unwrap();
              buf.extend_from_slice(&chunk[..n]);
            }
            me.seen
              .lock()
              .unwrap()
              .push(serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap_or(Value::Null));
            break;
          }
        }
        let response = loop {
          if let Some(r) = me.responses.lock().unwrap().pop_front() {
            break r;
          }
          tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let _ = sock.write_all(response.as_bytes()).await;
        let _ = sock.shutdown().await;
      }
    });
    base
  }

  fn push(&self, response: String) {
    self.responses.lock().unwrap().push_back(response);
  }
}

/// Gemini's Decisions API for the landing gate: every question answered
/// `p` as the test sets it (in hundredths), the inputs kept.
async fn serve_decisions(
  p: Arc<std::sync::atomic::AtomicU32>,
) -> (String, Arc<Mutex<Vec<String>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let (log, p) = (log.clone(), p.clone());
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let body: Value = loop {
          let n = sock.read(&mut chunk).await.unwrap();
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let len: usize = head
              .lines()
              .find_map(|l| {
                l.strip_prefix("content-length:")
                  .map(|v| v.trim().parse().unwrap())
              })
              .unwrap_or(0);
            while buf.len() < end + 4 + len {
              let n = sock.read(&mut chunk).await.unwrap();
              buf.extend_from_slice(&chunk[..n]);
            }
            break serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap_or(Value::Null);
          }
        };
        log
          .lock()
          .unwrap()
          .push(body["input"].as_str().unwrap_or_default().to_owned());
        let p = f64::from(p.load(std::sync::atomic::Ordering::SeqCst)) / 100.0;
        let answers: serde_json::Map<String, Value> = body["questions"]
          .as_object()
          .map(|q| {
            q.keys()
              .map(|k| {
                (
                  k.clone(),
                  json!({"type": "noul", "noul": p, "confidence": 0.9}),
                )
              })
              .collect()
          })
          .unwrap_or_default();
        let reply = json!({
          "answers": answers,
          "usage": {"total_input_tokens": 300, "total_output_tokens": 0}
        })
        .to_string();
        let response = format!(
          "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
           connection: close\r\n\r\n{reply}",
          reply.len()
        );
        let _ = sock.write_all(response.as_bytes()).await;
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

fn sse(events: Vec<(&str, Value)>) -> String {
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

fn shell(id: &str, command: &str) -> String {
  shell_with(id, json!({"command": command}))
}

/// A command run as a job (`background`): the turn goes on meanwhile.
fn job(id: &str, command: &str) -> String {
  shell_with(id, json!({"command": command, "background": true}))
}

fn shell_with(id: &str, input: Value) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": "shell_run", "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
}

fn answer(text: &str) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 80}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
}

struct World {
  dir: PathBuf,
  data: PathBuf,
  path: String,
  /// The node's home: credentials a fixer must not read.
  home: PathBuf,
  _tmp: tempfile::TempDir,
}

impl World {
  fn stitch(&self, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_stitch"))
      .arg("--data")
      .arg(&self.data)
      .args(args)
      .env("PATH", &self.path)
      .env("HOME", &self.home)
      .env("SSH_AUTH_SOCK", "/agent-SENTINEL")
      .env("GH_TOKEN", "gh-SENTINEL")
      .env_remove("XDG_CONFIG_HOME")
      .env_remove("GH_CONFIG_DIR")
      .env_remove("STITCH_SUPERVISED")
      .env_remove("INVOCATION_ID")
      .env_remove("STITCH_THREAD")
      .output()
      .unwrap();
    format!(
      "{}{}",
      String::from_utf8_lossy(&out.stdout),
      String::from_utf8_lossy(&out.stderr)
    )
  }
}

impl Drop for World {
  fn drop(&mut self) {
    let _ = Command::new(env!("CARGO_BIN_EXE_stitch"))
      .arg("--data")
      .arg(&self.data)
      .args(["node", "stop"])
      .output();
    let _ = std::fs::remove_dir_all(&self.dir);
  }
}

/// Opens a detached thread with `goal`, its shells in `cwd`.
async fn detach(c: &mut Client, goal: &str, cwd: &Path) -> String {
  match c
    .request(&Request::Detach {
      goal: goal.into(),
      model: None,
      repo: None,
      cwd: Some(cwd.display().to_string()),
      parent: None,
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: None,
      global: true,
      node: None,
      context: None,
      no_tools: false,
      tools: Vec::new(),
      skills: Vec::new(),
      keep: false,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  {
    Reply::Detached(t) => t,
    other => panic!("{other:?}"),
  }
}

/// The question whose command has `has`, once it is there.
async fn question_with(w: &World, has: &str) -> (String, Value) {
  let start = Instant::now();
  loop {
    let index = stitch_store::Index::open(&w.data.join("index.sqlite")).unwrap();
    if let Some(q) = index
      .records(Some("question"))
      .unwrap()
      .into_iter()
      .find(|q| {
        q.fields
          .get("command")
          .and_then(Value::as_str)
          .is_some_and(|c| c.contains(has))
      })
    {
      return (q.id.to_string(), q.fields.clone());
    }
    drop(index);
    let _ = w.stitch(&["attention"]);
    assert!(
      start.elapsed() < Duration::from_secs(90),
      "no question with {has}: {}\n{}",
      w.stitch(&["attention"]),
      std::fs::read_to_string(w.data.join("node.log")).unwrap_or_default()
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
  }
}

/// In auto mode, on a real node process: what the hazard guard stopped is
/// the user's alone to answer (a thread's own process answering it is
/// refused), and an agent's `stitch issue settings --auto-land on` asks
/// the user, whatever the mode, so no thread turns auto-land on for
/// itself (design §2.28).
#[tokio::test(flavor = "multi_thread")]
async fn guard_questions_and_auto_land_are_the_users_alone() {
  let model = Model::default();
  let base = model.serve().await;
  let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
    .join(format!("stitch-guard-asks-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&dir);
  std::fs::create_dir_all(dir.join("home")).unwrap();
  let work = dir.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let bin = Path::new(env!("CARGO_BIN_EXE_stitch"))
    .parent()
    .unwrap()
    .to_owned();
  let tmp = tempfile::tempdir().unwrap();
  let w = World {
    home: dir.join("home"),
    data: dir.join("data"),
    path: format!(
      "{}:{}",
      bin.display(),
      std::env::var("PATH").unwrap_or_default()
    ),
    dir,
    _tmp: tmp,
  };
  let setup = w.stitch(&["setup", "--name", "mbp", "--local"]);
  assert!(setup.contains("node mbp"), "{setup}");
  let mut c = Config::load(&w.data).unwrap();
  c.threads = Some(ThreadSettings {
    titles: Some(false),
    ..Default::default()
  });
  c.shell = Some(stitch_node::config::ShellSettings {
    approval: Some("auto".into()),
    ..Default::default()
  });
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(base),
      ..ProviderConfig::default()
    },
  );
  // The decision model flags the first command, then nothing.
  let flag = Arc::new(std::sync::atomic::AtomicU32::new(95));
  let (gemini, _) = serve_decisions(flag.clone()).await;
  c.providers.insert(
    "gemini".into(),
    ProviderConfig {
      kind: "gemini".into(),
      base_url: Some(gemini),
      ..ProviderConfig::default()
    },
  );
  c.save(&w.data).unwrap();
  for (p, k) in [("anthropic", "sk-test"), ("gemini", "g-test")] {
    stitch_node::keys::set(&w.data, p, k, stitch_node::keys::Store::File).unwrap();
  }
  let _ = w.stitch(&["jobs"]);
  let mut client = Client::connect(&w.data).await.unwrap();

  // Thread X: a command the guard stops.
  model.push(shell("x1", "echo drop_everything > x.txt"));
  detach(&mut client, "Tidy up", &work).await;
  let (qx, fields) = question_with(&w, "drop_everything").await;
  assert_eq!(fields["user_only"], true, "{fields}");
  assert!(
    fields["command"]
      .as_str()
      .unwrap()
      .starts_with("⚠ Hazard guard")
  );
  flag.store(5, std::sync::atomic::Ordering::SeqCst);

  // Thread Y: turns auto-land on (in the background: the node asks the
  // user), then answers X's question from its own shell.
  model.push(job("y1", "stitch issue settings --auto-land on"));
  model.push(shell("y2", &format!("stitch answer {qx} y; echo rc=$?")));
  model.push(answer("Done."));
  model.push(answer("Stopped."));
  detach(&mut client, "Make fixes land by themselves", &work).await;
  let (qy, fields) = question_with(&w, "stitch issue settings").await;
  assert_eq!(fields["user_only"], true, "{fields}");
  let answered = {
    let start = Instant::now();
    loop {
      if let Some(t) = model
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|b| b["messages"].to_string())
        .find(|m| m.contains("rc="))
      {
        break t;
      }
      assert!(
        start.elapsed() < Duration::from_secs(90),
        "{}",
        w.stitch(&["attention"])
      );
      tokio::time::sleep(Duration::from_millis(300)).await;
    }
  };
  assert!(answered.contains("only the user answers"), "{answered}");
  assert!(!answered.contains("rc=0"), "{answered}");
  assert!(
    !work.join("x.txt").exists(),
    "the stopped command did not run"
  );
  // Auto-land stays off until the user says so; the user declines.
  let said = w.stitch(&["issue", "settings"]);
  assert!(said.contains("auto-land: off"), "{said}");
  let said = w.stitch(&["answer", &qy, "n"]);
  // The node's own check of a caller (`peer::confined`).
  if stitch_node::sandbox::confined_here() {
    // This test's own process is the "user" here, and the node refuses
    // it as a thread's command would be: it cannot answer. CI (not
    // sandboxed) runs the rest.
    assert!(said.contains("runs in a sandbox"), "{said}");
    assert!(
      std::env::var_os("CI").is_none(),
      "guard_asks: sandboxed under CI: the user's answers must be checked unsandboxed there"
    );
    eprintln!(
      "guard_asks: the user's answers cannot be checked in a sandbox: not fully run (that \
       needs to run unsandboxed)"
    );
    return;
  }
  assert!(!said.contains("only the user"), "{said}");
  let said = w.stitch(&["answer", &qx, "n"]);
  assert!(!said.contains("only the user"), "{said}");
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert!(w.stitch(&["issue", "settings"]).contains("auto-land: off"));
  eprintln!("guard_asks: full run, the user's answers checked");
}
