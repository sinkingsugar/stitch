//! Tools written in Otter on a running node: a home workspace's skill
//! brings its scripts, the model (a mock) sees and calls them, a call
//! with effects waits for the client's approval, and an "always" answer is
//! kept for the place in config.toml (`[otter.places]`).

use std::path::Path;
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

fn tool_call(id: &str, name: &str, input: Value) -> String {
  let input = input.to_string();
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
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
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

fn chat(thread: Option<&str>, text: &str) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: None,
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
  }
}

const HI: &str = r#"
@fn(Hi input: {input: {} env: {} working-dir: String} output: String params: {} {
  input.input.name | If({IsString} {ExpectString} {"nobody"}) = name
  f"Hi, {name}!"
})
{definition: {name: "hi" description: "Greets."} use: "Hi"}
"#;

const STAMP: &str = r#"
@fn(Stamp input: {input: {} env: {} working-dir: String} output: String params: {} {
  "stamping" | Log
  "stamped"
})
{definition: {name: "stamp" description: "Logs, so it asks."} use: "Stamp"}
"#;

fn tool_names(body: &Value) -> Vec<String> {
  body["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|t| t["name"].as_str().unwrap().to_owned())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skills_scripts_are_tools_that_ask_and_are_granted() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_call("t1", "hi", json!({"name": "Ann"})),
    tool_call("t2", "stamp", json!({})),
    answer("Greeted and stamped."),
    tool_call("t3", "stamp", json!({})),
    answer("Stamped again, unasked."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let tools = home.join("skills/demo/tools");
  std::fs::create_dir_all(&tools).unwrap();
  std::fs::write(
    home.join("skills/demo/SKILL.md"),
    "---\nname: demo\ndescription: Demo tools.\n---\nUse hi and stamp.\n",
  )
  .unwrap();
  std::fs::write(tools.join("hi.otr"), HI).unwrap();
  std::fs::write(tools.join("stamp.shs"), STAMP).unwrap();
  // Not offered, and nothing else is the worse for it.
  std::fs::write(tools.join("broken.otter"), "@fn(Broken {").unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let mut asked: Vec<(String, String, Vec<String>)> = Vec::new();
  let mut results = Vec::new();
  let done = c
    .chat(&chat(None, "Greet Ann, then stamp"), |r| match r {
      Reply::Approve {
        session,
        command,
        grant,
        ..
      } => {
        asked.push((session.clone(), command.clone(), grant.clone()));
        let (answer, decided) = tokio::sync::oneshot::channel();
        answer.send(stitch_node::tools::Decision::always()).unwrap();
        Some(decided)
      }
      Reply::ToolResult { ok, output } => {
        results.push((*ok, output.clone()));
        None
      }
      _ => None,
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(
    asked,
    [(
      "otter:demo".to_owned(),
      "stamp {}".to_owned(),
      vec!["stamp".to_owned()]
    )]
  );
  assert_eq!(
    results,
    [
      (true, "Hi, Ann!".to_owned()),
      (true, "(approved by the user)\nstamped".to_owned())
    ]
  );
  // The grant is kept for the place, and the next call is not asked.
  let saved = Config::load(&data).unwrap().otter.unwrap();
  assert_eq!(saved.places["global"].allow, ["stamp"]);
  let mut results = Vec::new();
  c.chat(&chat(Some(&thread), "Stamp again"), |r| match r {
    Reply::Approve { .. } => panic!("granted: not asked again"),
    Reply::ToolResult { ok, output } => {
      results.push((*ok, output.clone()));
      None
    }
    _ => None,
  })
  .await
  .unwrap();
  assert_eq!(results, [(true, "stamped".to_owned())]);

  {
    let bodies = sent.lock().unwrap();
    let names = tool_names(&bodies[0]);
    // After the built-in tools, by name.
    for t in [
      "hi",
      "otter_check",
      "otter_describe",
      "otter_run",
      "otter_search",
      "read_skill",
      "stamp",
    ] {
      assert!(names.contains(&t.to_owned()), "{t} in {names:?}");
    }
    assert!(!names.iter().any(|n| n == "broken" || n == "Broken"));
    // Stitch's lazy skills' tools are not listed: the skill's line names
    // them, read_skill gives their parameters.
    assert!(
      !names
        .iter()
        .any(|n| n.starts_with("google_flights") || n == "youtube_video"),
      "{names:?}"
    );
    let system = bodies[0].to_string();
    assert!(
      system.contains("its tools google_flights_cheap_dates, google_flights_search, called with Tool.Call in otter_run"),
      "{system}"
    );
    let at = |n: &str| names.iter().position(|x| x == n).unwrap();
    assert!(at("shell_run") < at("hi"));
    assert!(at("hi") < at("stamp"));
    let hi = bodies[0]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .find(|t| t["name"] == "hi")
      .unwrap()
      .clone();
    assert_eq!(hi["description"], "Greets.");
    assert_eq!(hi["input_schema"]["type"], "object");
  }

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A script's `Tool.Call`s on a running node: each the thread's own call,
/// approved as its tool is (the question saying it comes from otter_run),
/// otter_run itself unasked; a weaving tool refused.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_calls_the_threads_tools_each_approved_as_its_own() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let code = "{name: \"Ann\"} | Tool.Call(\"hi\") = a\n{} | Tool.Call(\"stamp\") = b\nf\"{a} {b}\"";
  let (base, _) = serve(vec![
    tool_call("t1", "otter_run", json!({ "code": code })),
    tool_call(
      "t2",
      "otter_run",
      json!({"code": "{goal: \"x\"} | Tool.Call(\"spawn\")"}),
    ),
    answer("Done."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let tools = home.join("skills/demo/tools");
  std::fs::create_dir_all(&tools).unwrap();
  std::fs::write(
    home.join("skills/demo/SKILL.md"),
    "---\nname: demo\ndescription: Demo tools.\n---\nUse hi and stamp.\n",
  )
  .unwrap();
  std::fs::write(tools.join("hi.otr"), HI).unwrap();
  std::fs::write(tools.join("stamp.otr"), STAMP).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut asked: Vec<(String, String)> = Vec::new();
  let mut results = Vec::new();
  c.chat(&chat(None, "Greet and stamp from a script"), |r| match r {
    Reply::Approve {
      session, command, ..
    } => {
      asked.push((session.clone(), command.clone()));
      let (answer, decided) = tokio::sync::oneshot::channel();
      answer.send(stitch_node::tools::Decision::allow()).unwrap();
      Some(decided)
    }
    Reply::ToolResult { ok, output } => {
      results.push((*ok, output.clone()));
      None
    }
    _ => None,
  })
  .await
  .unwrap();
  assert_eq!(
    asked,
    [(
      "otter:demo".to_owned(),
      "[from otter_run, Tool.Call] stamp {}".to_owned()
    )]
  );
  assert_eq!(results[0], (true, "root: \"Hi, Ann! stamped\"".to_owned()));
  let (ok, out) = &results[1];
  assert!(
    !ok && out.contains("spawn cannot be called from a script"),
    "{out}"
  );
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// Edge talk's database, the little the search reads (its schema's
/// messages, conversations and contentless FTS5 index).
fn edge_talk_db(path: &Path) {
  let db = rusqlite::Connection::open(path).unwrap();
  db.pragma_update(None, "journal_mode", "WAL").unwrap();
  db.execute_batch(
    "CREATE TABLE conversations (id TEXT PRIMARY KEY NOT NULL, actor_id TEXT, title TEXT,
       created_at TEXT, updated_at TEXT);
     CREATE TABLE messages (id TEXT PRIMARY KEY NOT NULL, conversation_id TEXT, sender TEXT,
       message TEXT, created_at TEXT, tool_id TEXT, tool_name TEXT, is_system BOOLEAN DEFAULT 0,
       sequence_number INTEGER DEFAULT 0, attachment BLOB);
     CREATE VIRTUAL TABLE messages_fts USING fts5(id UNINDEXED, message, content='',
       contentless_delete=1, contentless_unindexed=1,
       tokenize='porter unicode61 remove_diacritics 1');
     INSERT INTO conversations VALUES ('c1', NULL, 'Otters', '2025-01-01 10:00:00',
       '2025-01-01 10:00:00');
     INSERT INTO messages (id, conversation_id, sender, message, created_at) VALUES
       ('m1', 'c1', 'user', 'Tell me everything about sea otters and their kelp forests.',
        '2025-01-01 10:00:00');
     INSERT INTO messages_fts (rowid, id, message) VALUES
       (1, 'm1', 'Tell me everything about sea otters and their kelp forests.');",
  )
  .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn edge_talks_history_is_offered_where_its_database_is() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    answer("Nothing from edge talk here."),
    tool_call("t1", "read_skill", json!({"name": "edge-talk"})),
    tool_call(
      "t2",
      "otter_run",
      json!({"code": "{query: \"kelp\"} | Tool.Call(\"edge_talk_search\")"}),
    ),
    answer("Found it."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  // The group container, as the iOS app gives it; no database yet.
  let container = d.join("group");
  std::fs::create_dir_all(container.join("Documents")).unwrap();
  stitch_node::otter::edge_talk::set_container(container.clone());
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, "Hello"), |_| None).await.unwrap();
  let names = tool_names(&sent.lock().unwrap()[0]);
  assert!(
    !names.iter().any(|n| n.starts_with("edge_talk")),
    "{names:?}"
  );

  // Once it is there: listed, and run unasked.
  edge_talk_db(&container.join("Documents/edge-talk.db"));
  let mut results = Vec::new();
  c.chat(&chat(None, "What did I ask about kelp?"), |r| match r {
    Reply::Approve { .. } => panic!("edge talk's tools run unasked"),
    Reply::ToolResult { ok, output } => {
      results.push((*ok, output.clone()));
      None
    }
    _ => None,
  })
  .await
  .unwrap();
  // Lazy: not listed, but named in the skill's line, read_skill giving
  // their parameters, and called through otter_run, unasked.
  let body = sent.lock().unwrap()[1].clone();
  let names = tool_names(&body);
  assert!(
    !names.iter().any(|n| n.starts_with("edge_talk")),
    "{names:?}"
  );
  assert!(
    body.to_string().contains(
      "its tools edge_talk_conversations, edge_talk_memories, edge_talk_read, edge_talk_search, called with Tool.Call"
    ),
    "{body}"
  );
  assert_eq!(results.len(), 2);
  let (ok, out) = &results[0];
  assert!(*ok && out.contains("## edge_talk_search\n"), "{out}");
  assert!(out.contains("Parameters (JSON Schema): {"), "{out}");
  let (ok, out) = &results[1];
  assert!(*ok, "{out}");
  assert!(
    out.contains("- 2025-01-01 10:00 [c1] Otters\\n  user: Tell me everything about sea otters"),
    "{out}"
  );

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_skill_turned_off_takes_its_tools() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![answer("No flights here.")]).await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut req = chat(None, "Hello");
  if let Request::Chat { skills, .. } = &mut req {
    *skills = vec!["-google-flights".into(), "-otter".into()];
  }
  c.chat(&req, |_| None).await.unwrap();
  let names = tool_names(&sent.lock().unwrap()[0]);
  assert!(names.contains(&"shell_run".to_owned()));
  assert!(
    !names
      .iter()
      .any(|n| n.starts_with("google_flights") || n.starts_with("otter_")),
    "{names:?}"
  );
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}
