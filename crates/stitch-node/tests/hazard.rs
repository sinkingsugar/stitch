//! The hazard guard on a running node (design §2.28), in auto mode, against a
//! mock model and a mock Decisions API: a command the decision model
//! clears runs, its verdict kept with its result; one it flags becomes a
//! question that starts with the verdict and carries it, and runs once
//! the user allows it.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, IndexedRecord, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Reads one HTTP request: its first line and JSON body.
async fn read_request(sock: &mut tokio::net::TcpStream) -> (String, Value) {
  let mut buf = Vec::new();
  let mut chunk = [0u8; 65536];
  loop {
    let n = sock.read(&mut chunk).await.unwrap();
    buf.extend_from_slice(&chunk[..n]);
    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
      let head = String::from_utf8_lossy(&buf[..end]).to_string();
      let len: usize = head
        .lines()
        .find_map(|l| {
          l.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(|v| v.trim().parse().unwrap())
        })
        .unwrap_or(0);
      while buf.len() < end + 4 + len {
        let n = sock.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
      }
      let body = serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap_or(Value::Null);
      return (head.lines().next().unwrap_or("").to_owned(), body);
    }
  }
}

/// Anthropic's Messages API: `responses` in order, one per connection.
async fn serve_model(responses: Vec<String>) -> String {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  tokio::spawn(async move {
    for response in responses {
      let (mut sock, _) = listener.accept().await.unwrap();
      read_request(&mut sock).await;
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  base
}

/// Gemini's Decisions API: 0.95 for an input naming `drop_everything`,
/// else 0.05; the inputs kept.
async fn serve_decisions() -> (String, Arc<Mutex<Vec<String>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let log = log.clone();
      tokio::spawn(async move {
        let (line, body) = read_request(&mut sock).await;
        assert!(line.starts_with("POST /v1beta/decisions"), "{line}");
        let input = body["input"].as_str().unwrap_or_default().to_owned();
        let p = if input.contains("drop_everything") {
          0.95
        } else {
          0.05
        };
        log.lock().unwrap().push(input);
        let reply = json!({
          "answers": {"catastrophic": {"type": "noul", "noul": p, "confidence": 0.95}},
          "usage": {"total_input_tokens": 120, "total_output_tokens": 0}
        })
        .to_string();
        let response = format!(
          "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
           connection: close\r\n\r\n{reply}",
          reply.len()
        );
        sock.write_all(response.as_bytes()).await.unwrap();
        sock.shutdown().await.unwrap();
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

fn tool_use(id: &str, command: &str) -> String {
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
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json!({"command": command}).to_string()}}),
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

fn setup(data: &Path, home: &Path, anthropic: &str, gemini: &str) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    // Auto: yolo, and what runs is the guard's to stop.
    shell: Some(stitch_node::config::ShellSettings {
      approval: Some("auto".into()),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(anthropic.into()),
      ..ProviderConfig::default()
    },
  );
  c.providers.insert(
    "gemini".into(),
    ProviderConfig {
      kind: "gemini".into(),
      base_url: Some(gemini.into()),
      ..ProviderConfig::default()
    },
  );
  c.save(data).unwrap();
  keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  keys::set(data, "gemini", "g-test", Store::File).unwrap();
  let root = RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn records(home: &Path, kind: &str) -> Vec<IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index.records(Some(kind)).unwrap()
}

async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..300 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

/// Every file's text under `dir`.
fn texts(dir: &Path) -> Vec<String> {
  let mut out = Vec::new();
  for e in std::fs::read_dir(dir).unwrap().flatten() {
    let p = e.path();
    if p.is_dir() {
      out.extend(texts(&p));
    } else if let Ok(t) = std::fs::read_to_string(&p) {
      out.push(t);
    }
  }
  out
}

/// Answering a question is the user's alone, from no sandbox: inside one
/// (an agent's `just quick`) the node rightly refuses the test's own
/// answer, so the tests that answer as the user are not run there. On
/// Linux the node's sense of "confined" is `no_new_privs` (`NoNewPrivs:` in
/// `/proc/<pid>/status`, as `peer::confined` reads it), which the fixers'
/// sandbox sets. With `CI` set (GitHub Actions does) it never skips: the
/// tests must run there, so a sandboxed CI fails them loudly.
fn outside_any_sandbox() -> bool {
  let ok = if std::env::var_os("CI").is_some() {
    true
  } else if cfg!(target_os = "macos") {
    stitch_shell::Sandbox::available(None).is_ok()
  } else if cfg!(target_os = "linux") {
    std::fs::read_to_string("/proc/self/status").is_ok_and(|s| {
      s.lines()
        .find_map(|l| l.strip_prefix("NoNewPrivs:"))
        .is_some_and(|v| v.trim() == "0")
    })
  } else {
    true
  };
  if !ok {
    eprintln!("inside a sandbox: only the user answers questions; not run");
  }
  ok
}

#[tokio::test(flavor = "multi_thread")]
async fn in_auto_the_guard_runs_what_it_clears_and_asks_what_it_flags() {
  if !outside_any_sandbox() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (safe, risky) = (d.join("safe.txt"), d.join("risky.txt"));
  let base = serve_model(vec![
    tool_use("t1", &format!("echo ok > '{}'", safe.display())),
    tool_use(
      "t2",
      &format!("echo drop_everything > '{}'", risky.display()),
    ),
    answer("Done."),
  ])
  .await;
  let (gemini, inputs) = serve_decisions().await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, &gemini);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&Request::Detach {
      goal: "Tidy the project".into(),
      model: None,
      repo: None,
      cwd: Some(d.display().to_string()),
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
  else {
    panic!()
  };

  // The flagged command is a question, its verdict first and kept.
  let q = until("the guard's question", || {
    records(&home, "question").into_iter().next()
  })
  .await;
  assert!(safe.exists(), "the cleared command ran without asking");
  assert!(!risky.exists());
  let command = q.fields["command"].as_str().unwrap();
  assert!(command.starts_with("⚠ Hazard guard 0.95"), "{command}");
  assert!(command.contains("echo drop_everything"), "{command}");
  assert_eq!(q.fields["guard_score"], 0.95);
  assert_eq!(q.fields["guard_asked"], true);
  assert_eq!(q.fields["guard_mode"], "on");
  assert!(q.fields.get("grant").is_none(), "no \"always\" for it");
  // The decision model read the user's goal.
  assert!(inputs.lock().unwrap()[0].contains("\"user_goal\": \"Tidy the project\""));

  for _ in 0..200 {
    match c
      .request(&Request::Answer {
        question: q.id.to_string(),
        allow: true,
        reason: None,
        always: false,
      })
      .await
    {
      Ok(_) => break,
      Err(e) if e.to_string().contains("no thread") => {
        tokio::time::sleep(Duration::from_millis(50)).await
      }
      Err(e) => panic!("{e}"),
    }
  }
  until("the allowed command", || risky.exists().then_some(())).await;
  until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| r.fields.get("outcome").is_some())
  })
  .await;
  // Each call's result keeps its verdict, and the decisions are metered.
  let all = texts(&home).join("\n");
  assert!(
    all.contains("\"guard\":{"),
    "verdicts kept with the results"
  );
  assert!(all.contains("\"score\":0.05"));
  assert!(all.contains("gemini/gemini-decision-1"));
  let _ = thread;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
