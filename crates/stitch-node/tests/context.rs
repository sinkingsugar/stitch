//! A thread is told the instruction files that apply where it works and
//! the skills it can use: in its system prompt when it starts, and with
//! the next message when they change.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<Value>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    for response in responses {
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 65536];
      loop {
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
          log
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap());
          break;
        }
      }
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
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

/// The model asks for one shell command.
#[allow(dead_code)]
fn tool_use(id: &str, command: &str) -> String {
  let input = json!({"command": command}).to_string();
  let (a, b) = input.split_at(input.len() / 2);
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Let me check."}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": id, "name": "shell_run", "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": a}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": b}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
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

fn setup(data: &Path, home: &Path, anthropic: &str) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    // Apps not served: a test node never takes the fixed port 7177.
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
  c.save(data).unwrap();
  keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  let root = RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn chat(thread: Option<&str>, text: &str, cwd: &Path) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: Some(cwd.display().to_string()),
    global: false,
    env: Vec::new(),
    tools: Vec::new(),
    skills: Vec::new(),
    place: None,
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

fn git(dir: &Path, args: &[&str]) {
  let ok = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .unwrap()
    .status
    .success();
  assert!(ok, "git {args:?}");
}

/// The text of the last message sent.
fn last_user(body: &Value) -> Vec<String> {
  body["messages"].as_array().unwrap().last().unwrap()["content"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|c| c["text"].as_str().map(str::to_owned))
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_are_told_their_instructions_and_skills() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  std::fs::create_dir_all(app.join("src")).unwrap();
  git(&app, &["init", "--quiet"]);
  git(
    &app,
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "first",
    ],
  );
  std::fs::write(app.join("CLAUDE.md"), "@AGENTS-shared.md\n").unwrap();
  std::fs::write(
    app.join("AGENTS-shared.md"),
    "Run cargo test before committing.",
  )
  .unwrap();
  let home = d.join("home");
  std::fs::create_dir_all(home.join("skills/release")).unwrap();
  std::fs::write(home.join("AGENTS.md"), "Never push unless asked.").unwrap();
  std::fs::write(
    home.join("skills/release/SKILL.md"),
    "---\nname: release\ndescription: Cut a release.\n---\nTag it.\n",
  )
  .unwrap();

  let (base, sent) = serve(vec![answer("one"), answer("two"), answer("three")]).await;
  let data = d.join("data");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let done = c
    .chat(&chat(None, "hello", &app.join("src")), |_| None)
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  let system = sent.lock().unwrap()[0]["system"][0]["text"]
    .as_str()
    .unwrap()
    .to_owned();
  assert!(
    system.contains("## Yours (AGENTS.md in the home workspace)\n\nNever push unless asked."),
    "{system}"
  );
  assert!(
    system.contains("## CLAUDE.md\n\nRun cargo test before committing."),
    "{system}"
  );
  assert!(system.contains("- release: Cut a release. ("), "{system}");

  // The project's instructions change: the next message carries them,
  // once; the system prompt stays as it was.
  std::fs::write(app.join("AGENTS.md"), "Use rustfmt with two spaces.").unwrap();
  c.chat(&chat(Some(&thread), "again", &app), |_| None)
    .await
    .unwrap();
  c.chat(&chat(Some(&thread), "and again", &app), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(bodies[1]["system"][0]["text"].as_str().unwrap(), system);
    let second = last_user(&bodies[1]);
    assert_eq!(second.len(), 2, "{second:?}");
    assert!(second[0].starts_with("[Stitch: the instructions and skills for this thread changed"));
    assert!(second[0].contains("## AGENTS.md\n\nUse rustfmt with two spaces."));
    assert!(
      !second[0].contains("CLAUDE.md"),
      "AGENTS.md wins over CLAUDE.md"
    );
    assert_eq!(second[1], "again");
    assert_eq!(last_user(&bodies[2]), ["and again"]);
  }
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();

  // The transcript marks the update, without the files.
  let index = std::sync::Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let node_name = "mbp".parse().unwrap();
  // The thread lives in the project's store.
  let repo = index
    .lock()
    .unwrap()
    .record(&thread.parse().unwrap())
    .unwrap()
    .unwrap()
    .repo;
  let root = RecordRoot::open(&data.join("projects").join(repo.as_str())).unwrap();
  let ctx = stitch_node::chat::ChatContext {
    node: &node_name,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  let entries = stitch_node::chat::transcript(&ctx, &thread.parse().unwrap()).unwrap();
  let again = entries
    .iter()
    .find(|(_, t)| t.ends_with("again") && t.starts_with('['))
    .unwrap();
  assert_eq!(again.1, "[instructions updated]\nagain");
  // What a client shows says the update; what it matches a message it
  // sent by is the text alone (issue 2026-11-02T102522Z-mbp-2fkd: the
  // app's bubble for a message sent to another node's turn stayed, its
  // entry never equal to the text typed).
  let shown = stitch_node::chat::transcript_shown(&ctx, &thread.parse().unwrap(), None).unwrap();
  let again = shown
    .iter()
    .find(|e| e.who == "you" && e.said.ends_with("again") && e.said.starts_with('['))
    .unwrap();
  assert_eq!(again.said, "[instructions updated]\nagain");
  assert_eq!(again.typed, "again");
  assert!(again.id.is_some());
}

/// A message said in a voice call (design §2.26) tells the model, after
/// its text, that the reply is read aloud; the note is written with it, so
/// the next turn sends the same history (the provider's cache holds), and
/// the system prompt is untouched. The transcript shows the text alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_spoken_message_asks_for_a_reply_to_read_aloud() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let home = d.join("home");
  std::fs::create_dir_all(&home).unwrap();
  let (base, sent) = serve(vec![answer("Sure."), answer("Done.")]).await;
  let data = d.join("data");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let mut spoken = chat(None, "what time is it", &d);
  if let Request::Chat { spoken: s, .. } = &mut spoken {
    *s = true;
  }
  let done = c.chat(&spoken, |_| None).await.unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  c.chat(&chat(Some(&thread), "thanks", &d), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    let first = last_user(&bodies[0]);
    assert_eq!(first[0], "what time is it", "{first:?}");
    assert_eq!(first[1], stitch_node::chat::SPOKEN_NOTE);
    assert!(first[1].contains("transcription"));
    assert!(
      !bodies[0]["system"][0]["text"]
        .as_str()
        .unwrap()
        .contains("voice call")
    );
    // The next turn's history holds the same message, note and all; the
    // typed message gets none.
    let messages = bodies[1]["messages"].as_array().unwrap();
    assert_eq!(
      messages[0]["content"], bodies[0]["messages"][0]["content"],
      "the spoken message is sent again as it was"
    );
    assert_eq!(last_user(&bodies[1]), ["thanks"]);
  }
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();

  let index = std::sync::Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let node_name = "mbp".parse().unwrap();
  let repo = index
    .lock()
    .unwrap()
    .record(&thread.parse().unwrap())
    .unwrap()
    .unwrap()
    .repo;
  let root = RecordRoot::open(&data.join("projects").join(repo.as_str()))
    .or_else(|_| RecordRoot::open(&home))
    .unwrap();
  let ctx = stitch_node::chat::ChatContext {
    node: &node_name,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  let shown = stitch_node::chat::transcript_shown(&ctx, &thread.parse().unwrap(), None).unwrap();
  let said = shown.iter().find(|e| e.who == "you").unwrap();
  assert_eq!(said.said, "what time is it");
  assert_eq!(said.typed, "what time is it");
  assert!(said.spoken, "the app marks it as said");
  let typed = shown
    .iter()
    .find(|e| e.who == "you" && e.typed == "thanks")
    .unwrap();
  assert!(!typed.spoken);
}
