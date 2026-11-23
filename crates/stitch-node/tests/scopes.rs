//! Thread and effort scopes (design §2.27, docs/rooms.md §3): a message
//! the user attaches threads or efforts to carries their briefing
//! (`Part::Scope`), sent again byte for byte at later turns however those
//! threads move on; clients get chips, not the briefing; an effort is its
//! root's compaction chain with every thread spawned under it, cycles cut;
//! `read_thread` reads a thread (redacted) and `Request::Scope` searches a
//! scope's own threads only.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request, ScopeTarget, SecretOp};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_records::{
  Author, Draft, Event, Msg, NodeName, Part, RecordId, Ref, Role as Who, ScopeKind, ToolCall,
  ToolResult, kinds,
};
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

fn tool_call(id: &str, name: &str, input: Value) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
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

fn setup(data: &Path, home: &Path, anthropic: &str) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
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
  c.save(data).unwrap();
  keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  let root = home_root(home);
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

/// The home workspace, made the first time.
fn home_root(home: &Path) -> RecordRoot {
  RecordRoot::open(home)
    .or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home))
    .unwrap()
}

fn chat(thread: Option<&str>, text: &str, scopes: Vec<ScopeTarget>) -> Request {
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
    scopes,
  }
}

fn target(kind: ScopeKind, id: &RecordId) -> ScopeTarget {
  ScopeTarget {
    kind,
    id: id.to_string(),
  }
}

fn at(s: &str) -> Timestamp {
  format!("2026-11-11T{s}Z").parse().unwrap()
}

fn msg(id: &str, role: Who, t: &str, text: &str) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: at(t),
    role,
    from: None,
    model: (role == Who::Assistant).then(|| "anthropic/claude-opus-5-5".into()),
    parts: vec![Part::Text { text: text.into() }],
    native: None,
  })
}

fn exchange(user: &str, reply: &str) -> Vec<Event> {
  vec![
    msg("m1", Who::User, "10:00:00", user),
    msg("m2", Who::Assistant, "10:00:05", reply),
  ]
}

/// A thread record and its one segment: compacted from `compacts`, or
/// spawned by `spawned_by`.
fn thread(
  root: &RecordRoot,
  node: &NodeName,
  title: &str,
  compacts: Option<&RecordId>,
  spawned_by: Option<&RecordId>,
  events: Vec<Event>,
) -> RecordId {
  thread_as(root, node, None, title, compacts, spawned_by, events)
}

/// `thread`, under an ID chosen beforehand when given.
fn thread_as(
  root: &RecordRoot,
  node: &NodeName,
  chosen: Option<RecordId>,
  title: &str,
  compacts: Option<&RecordId>,
  spawned_by: Option<&RecordId>,
  events: Vec<Event>,
) -> RecordId {
  let mut d = Draft::new(kinds::THREAD, Author::user(node.clone())).unwrap();
  if let Some(old) = compacts {
    d = d
      .field("compacted", true)
      .unwrap()
      .reference("continues", Ref::to(old.clone()))
      .unwrap();
  }
  if let Some(p) = spawned_by {
    d = d
      .field("detached", true)
      .unwrap()
      .field("goal", title)
      .unwrap()
      .reference("spawned-by", Ref::to(p.clone()))
      .unwrap();
  }
  let d = d.body(format!("# {title}\n"));
  let id = match chosen {
    Some(id) => {
      let r = d.into_record(id.clone(), Timestamp::now());
      root.publish_record(&r).unwrap();
      id
    }
    None => root.publish(node, d).unwrap().id,
  };
  if !events.is_empty() {
    root.publish_segment(node, &id, None, None, events).unwrap();
  }
  id
}

fn digest(root: &RecordRoot, node: &NodeName, thread: &RecordId, headline: &str, step: &str) {
  let mut item = toml::Table::new();
  item.insert("kind".into(), "finding".into());
  item.insert(
    "text".into(),
    "the group chat ran blind parallel rounds".into(),
  );
  item.insert("at".into(), format!("{thread}#m2").into());
  let d = Draft::new(stitch_store::DIGEST_KIND, Author::node(node.clone()))
    .unwrap()
    .field("thread", thread.as_str())
    .unwrap()
    .field("upto", format!("{thread}#m2").as_str())
    .unwrap()
    .field("step", step)
    .unwrap()
    .field("model", "anthropic/claude-sonnet-5-5")
    .unwrap()
    .field("usd", 0.01)
    .unwrap()
    .field("headline", headline)
    .unwrap()
    .field("items", toml::Value::Array(vec![toml::Value::Table(item)]))
    .unwrap()
    .reference("thread", Ref::to(thread.clone()))
    .unwrap()
    .body(format!("# Digest of {thread}\n"));
  root.publish(node, d).unwrap();
}

async fn lines(c: &mut Client, r: &Request) -> Vec<String> {
  match c.request(r).await.unwrap() {
    Reply::Lines(l) => l,
    other => panic!("{other:?}"),
  }
}

fn texts(message: &Value) -> Vec<String> {
  message["content"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|b| b["text"].as_str().map(str::to_owned))
    .collect()
}

/// The text of every tool result a request body carries.
fn tool_results(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
    .filter(|b| b["type"] == "tool_result")
    .map(|b| match &b["content"] {
      Value::String(s) => s.clone(),
      v => v.to_string(),
    })
    .collect()
}

const SECRET: &str = "hunter2-SCOPE-SECRET-77";

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_briefs_a_message_once_and_its_threads_are_read_and_searched() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  std::fs::create_dir_all(&home).unwrap();
  let root = home_root(&home);
  let node: NodeName = "mbp".parse().unwrap();

  // The effort: a conversation compacted once, a thread spawned from its
  // first part, and one spawned from that, which printed a secret.
  let p = thread(
    &root,
    &node,
    "Rooms design",
    None,
    None,
    exchange(
      "Keep the prompt cache prefix stable when rooms relay",
      "Decided: rooms relay deltas.",
    ),
  );
  let mut events = exchange("Write it down", "Written.");
  events.push(Event::ToolCall(ToolCall {
    id: "c1".into(),
    at: at("10:00:06"),
    name: "write_file".into(),
    input: json!({"path": "docs/rooms.md", "content": "x"}),
  }));
  events.push(Event::ToolResult(ToolResult {
    id: "c1".into(),
    at: at("10:00:07"),
    ok: true,
    output: json!({"text": "wrote docs/rooms.md"}),
  }));
  let p2 = thread(&root, &node, "✂ Rooms design", Some(&p), None, events);
  let child = thread(
    &root,
    &node,
    "Survey edge talk's group chat",
    None,
    Some(&p),
    exchange(
      "Survey edge talk's group chat",
      "The group chat ran blind parallel rounds.",
    ),
  );
  digest(
    &root,
    &node,
    &child,
    "Edge talk's group chat surveyed",
    "2099-01-01T00:00:00Z",
  );
  let grandchild = thread(
    &root,
    &node,
    "Check the token",
    None,
    Some(&child),
    exchange("Check the token", &format!("The token is {SECRET}.")),
  );
  // Elsewhere: an unrelated thread saying the same words, and two threads
  // spawned by each other.
  let unrelated = thread(
    &root,
    &node,
    "Taxes",
    None,
    None,
    exchange("prompt cache prefix for taxes", "Unrelated."),
  );
  let y_id = RecordId::random(Timestamp::now(), &node);
  let x = thread(
    &root,
    &node,
    "Loop X",
    None,
    Some(&y_id),
    exchange("x", "x"),
  );
  let y = thread_as(
    &root,
    &node,
    Some(y_id),
    "Loop Y",
    None,
    Some(&x),
    exchange("y", "y"),
  );
  let x2 = thread(
    &root,
    &node,
    "Loop X again",
    None,
    Some(&y),
    exchange("x", "x"),
  );
  let (base, sent) = serve(vec![
    // A message with an effort attached.
    answer("Seen."),
    // The next turn reads a thread of it by an anchor.
    tool_call(
      "r1",
      "read_thread",
      json!({"thread": format!("{child}#m2")}),
    ),
    answer("Read it."),
    // A message sent with a thread attached.
    answer("Noted."),
  ])
  .await;
  setup(&data, &home, &base);

  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  assert!(matches!(
    c.request(&Request::Secret {
      op: SecretOp::Set {
        name: "token".into(),
        scope: "global".into(),
        value: Some(SECRET.into()),
        device_only: None,
        file: false,
      },
      cwd: None,
    })
    .await
    .unwrap(),
    Reply::Ok | Reply::Lines(_) | Reply::Secret(_)
  ));

  // What an effort resolves to, from any thread of it: the root chain and
  // the spawn tree, in order.
  let preview = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Effort, &child)],
      query: None,
    },
  )
  .await
  .remove(0);
  assert!(
    preview.starts_with("<scope>\n[Stitch: the user attached"),
    "{preview}"
  );
  assert!(preview.ends_with("</scope>"));
  assert!(
    preview.contains(&format!("(effort:{p2}, 4 threads)")),
    "{preview}"
  );
  let order: Vec<usize> = [&p, &p2, &child, &grandchild]
    .iter()
    .map(|t| {
      preview
        .find(&format!("### {t}"))
        .unwrap_or_else(|| panic!("{t}: {preview}"))
    })
    .collect();
  assert!(order.windows(2).all(|w| w[0] < w[1]), "{preview}");
  assert!(preview.contains(&format!("compaction of {p}")));
  assert!(preview.contains(&format!("spawned by {child}")));
  assert!(preview.contains("digest (fresh"), "{preview}");
  assert!(preview.contains(&format!(
    "[finding] the group chat ran blind parallel rounds ({child}#m2)"
  )));
  assert!(preview.contains("no digest yet; goal: Check the token"));
  assert!(preview.contains("wrote path docs/rooms.md"), "{preview}");
  assert!(!preview.contains(&unrelated.to_string()));
  assert!(!preview.contains(SECRET), "the briefing is redacted");
  // A thread alone: its chain, not what it spawned.
  let alone = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Thread, &p)],
      query: None,
    },
  )
  .await
  .remove(0);
  assert!(alone.contains(&format!("### {p2}")) && !alone.contains(&format!("### {child}")));
  // A spawn cycle: each thread once, and it ends.
  let cycle = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Effort, &x2)],
      query: None,
    },
  )
  .await
  .remove(0);
  for t in [&x, &y, &x2] {
    assert_eq!(cycle.matches(&format!("### {t}")).count(), 1, "{cycle}");
  }
  // A scope that names no thread is refused.
  let refused = c
    .request(&Request::Scope {
      scopes: vec![ScopeTarget {
        kind: ScopeKind::Thread,
        id: "2026-11-11T100000Z-mbp-zzzz".into(),
      }],
      query: None,
    })
    .await;
  assert!(
    matches!(refused, Ok(Reply::Error(_)) | Err(_)),
    "{refused:?}"
  );
  // The scope's own threads searched by keyword: the unrelated thread
  // saying the same words is not.
  let found = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Effort, &p)],
      query: Some("prompt cache prefix".into()),
    },
  )
  .await;
  assert!(found[1].contains(&format!("[{p}#m1] user")), "{found:?}");
  assert!(!found[1].contains(&unrelated.to_string()), "{found:?}");

  // A message with the effort attached: the briefing after its text.
  let done = c
    .chat(
      &chat(
        None,
        "look at this",
        vec![target(ScopeKind::Effort, &child)],
      ),
      |_| None,
    )
    .await
    .unwrap();
  let Reply::ChatDone { thread: t, .. } = done else {
    panic!("{done:?}")
  };
  let first = {
    let bodies = sent.lock().unwrap();
    let m = bodies[0]["messages"][0].clone();
    let said = texts(&m);
    assert_eq!(said[0], "look at this");
    assert!(said[1].starts_with("<scope>"), "{said:?}");
    assert!(said[1].contains("Edge talk's group chat surveyed"));
    m
  };

  // The child's digest moves on; the next turn sends the message as it
  // was, byte for byte (the prompt cache holds), and a fresh preview
  // says the new digest.
  digest(
    &root,
    &node,
    &child,
    "Edge talk's group chat revisited",
    "2099-02-01T00:00:00Z",
  );
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
  c.chat(&chat(Some(&t), "and now?", vec![]), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(
      bodies[1]["messages"][0], first,
      "the scoped message is sent again as it was"
    );
    assert_eq!(
      serde_json::to_string(&bodies[1]["messages"][0]).unwrap(),
      serde_json::to_string(&first).unwrap()
    );
    let results = tool_results(&bodies[2]);
    assert_eq!(results.len(), 1, "{results:?}");
    assert!(
      results[0].contains(&format!("[{child}#m2] assistant")),
      "{results:?}"
    );
    assert!(
      results[0].contains("Edge talk's group chat revisited"),
      "{results:?}"
    );
  }
  let fresh = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Effort, &child)],
      query: None,
    },
  )
  .await
  .remove(0);
  assert!(
    fresh.contains("Edge talk's group chat revisited"),
    "{fresh}"
  );

  // A message sent with a thread attached: a turn whose message carries it.
  lines(
    &mut c,
    &Request::Send {
      thread: t.clone(),
      text: "compare with this".into(),
      from: None,
      keep: false,
      interject: false,
      scopes: vec![target(ScopeKind::Thread, &unrelated)],
    },
  )
  .await;
  for _ in 0..200 {
    if sent.lock().unwrap().len() >= 4 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  {
    let bodies = sent.lock().unwrap();
    let last = bodies[3]["messages"]
      .as_array()
      .unwrap()
      .last()
      .unwrap()
      .clone();
    let said = texts(&last);
    assert!(said.iter().any(|s| s == "compare with this"), "{said:?}");
    assert!(
      said
        .iter()
        .any(|s| s.starts_with("<scope>") && s.contains(&format!("thread:{unrelated}"))),
      "{said:?}"
    );
  }
  // Wait for that turn to be written.
  for _ in 0..200 {
    let l = lines(
      &mut c,
      &Request::Transcript {
        thread: t.clone(),
        after: None,
        before: None,
        tail: None,
      },
    )
    .await;
    if l[0].contains("Noted.") {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  // Clients get chips, not the briefing.
  let l = lines(
    &mut c,
    &Request::Transcript {
      thread: t.clone(),
      after: None,
      before: None,
      tail: None,
    },
  )
  .await;
  let tr: Value = serde_json::from_str(&l[0]).unwrap();
  let users: Vec<&Value> = tr["entries"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|e| e["kind"] == "user")
    .collect();
  assert_eq!(users[0]["text"], "look at this");
  assert_eq!(
    users[0]["scopes"],
    json!([{"kind": "effort", "id": p2.to_string(), "title": "Rooms design", "threads": 4}])
  );
  assert!(users[1].get("scopes").is_none());
  let last_user = users.last().unwrap();
  assert_eq!(last_user["text"], "compare with this");
  assert_eq!(last_user["scopes"][0]["kind"], "thread");
  assert_eq!(last_user["scopes"][0]["id"], unrelated.to_string());
  assert!(!l[0].contains("<scope>"), "no briefing in the transcript");

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node_handle.wait().await.unwrap();
}

/// `read_thread`, called by a read-only thread's model on an anchor: the
/// header, the digest, the artefacts, the window around the message, a
/// secret redacted.
#[tokio::test(flavor = "multi_thread")]
async fn read_thread_reads_around_an_anchor_in_a_read_only_thread() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  // The thread to read is written first, so its ID is known to the mock.
  std::fs::create_dir_all(&home).unwrap();
  let root = home_root(&home);
  let node: NodeName = "mbp".parse().unwrap();
  let mut events = Vec::new();
  for i in 1..=30 {
    events.push(msg(
      &format!("m{i}"),
      if i % 2 == 1 {
        Who::User
      } else {
        Who::Assistant
      },
      &format!("10:{i:02}:00"),
      &if i == 12 {
        format!("The key is {SECRET}, keep it.")
      } else {
        format!("message number {i}")
      },
    ));
  }
  events.push(Event::ToolCall(ToolCall {
    id: "c1".into(),
    at: at("10:31:00"),
    name: "shell_run".into(),
    input: json!({"command": "git commit -m 'Scopes'"}),
  }));
  events.push(Event::ToolResult(ToolResult {
    id: "c1".into(),
    at: at("10:31:01"),
    ok: true,
    output: json!({"text": "[docs-rooms abc1234] Scopes\n 1 file changed"}),
  }));
  let target_thread = thread(&root, &node, "A long thread", None, None, events);
  digest(
    &root,
    &node,
    &target_thread,
    "A long thread, digested",
    "2000-01-01T00:00:00Z",
  );
  let (base, sent) = serve(vec![
    tool_call(
      "r1",
      "read_thread",
      json!({"thread": format!("{target_thread}#m12"), "limit": 6}),
    ),
    answer("Read."),
  ])
  .await;
  setup(&data, &home, &base);
  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Secret {
    op: SecretOp::Set {
      name: "key".into(),
      scope: "global".into(),
      value: Some(SECRET.into()),
      device_only: None,
      file: false,
    },
    cwd: None,
  })
  .await
  .unwrap();
  // A thread spawned read-only reads it.
  let reply = c
    .request(&Request::Detach {
      goal: "Read the long thread".into(),
      model: None,
      repo: None,
      cwd: None,
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
      read_only: true,
      // Attached as `stitch chat --detach --scope` does (the meeting bot).
      scopes: vec![stitch_node::scopes::parse_target(&target_thread.to_string()).unwrap()],
    })
    .await
    .unwrap();
  let Reply::Detached(reader) = reply else {
    panic!("{reply:?}")
  };
  for _ in 0..400 {
    if sent.lock().unwrap().len() >= 2 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let result = {
    let bodies = sent.lock().unwrap();
    assert!(
      bodies[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "read_thread"),
      "offered to a read-only thread"
    );
    let first = bodies[0].to_string();
    assert!(
      first.contains(&target_thread.to_string()) && first.contains("A long thread"),
      "a detached thread is briefed on its scopes: {first}"
    );
    tool_results(&bodies[1]).remove(0)
  };
  assert!(
    result.contains(&format!("Thread {target_thread} \"A long thread\"")),
    "{result}"
  );
  assert!(result.contains("Digest (stale"), "{result}");
  assert!(result.contains("commit abc1234 \"Scopes\""), "{result}");
  assert!(
    result.contains("Messages 9–14 of 30, around m12"),
    "{result}"
  );
  assert!(
    result.contains(&format!("[{target_thread}#m12] assistant")),
    "{result}"
  );
  assert!(
    !result.contains(&format!("[{target_thread}#m8]")),
    "{result}"
  );
  assert!(!result.contains(SECRET), "redacted: {result}");
  assert!(
    result.contains("earlier: from 3") && result.contains("later: from 15"),
    "{result}"
  );

  // The spawned thread is out of memory (a spawn's default): its scope
  // says so, and a search of it still finds its exact words.
  for _ in 0..200 {
    let l = lines(
      &mut c,
      &Request::Transcript {
        thread: reader.clone(),
        after: None,
        before: None,
        tail: None,
      },
    )
    .await;
    if l[0].contains("Read.") {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let reader_target = ScopeTarget {
    kind: ScopeKind::Thread,
    id: reader.clone(),
  };
  let found = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![reader_target],
      query: Some("long thread".into()),
    },
  )
  .await;
  assert!(found[0].contains("[memory: off]"), "{}", found[0]);
  assert!(found[1].contains("1 of them [memory: off]"), "{}", found[1]);
  assert!(
    found[1].contains(&format!("[{reader}#m1] user")),
    "{}",
    found[1]
  );
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node_handle.wait().await.unwrap();
}

/// A big effort (60 threads, each with a full digest) stays within the
/// briefing's budget: its root in full, the rest folded to one line each
/// as it must, said so.
#[tokio::test(flavor = "multi_thread")]
async fn a_big_effort_is_briefed_within_its_budget() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  std::fs::create_dir_all(&home).unwrap();
  let root = home_root(&home);
  let node: NodeName = "mbp".parse().unwrap();
  let p = thread(
    &root,
    &node,
    "The big one",
    None,
    None,
    exchange("go", "going"),
  );
  let mut leaves = Vec::new();
  for i in 0..60 {
    let t = thread(
      &root,
      &node,
      &format!("Leaf {i}"),
      None,
      Some(&p),
      exchange(&format!("leaf {i}"), "done"),
    );
    let items: Vec<toml::Value> = (0..12)
      .map(|k| {
        let mut item = toml::Table::new();
        item.insert("kind".into(), "decision".into());
        item.insert(
          "text".into(),
          format!("{k}: {}", "a long decision ".repeat(20)).into(),
        );
        item.insert("at".into(), format!("{t}#m2").into());
        toml::Value::Table(item)
      })
      .collect();
    let dr = Draft::new(stitch_store::DIGEST_KIND, Author::node(node.clone()))
      .unwrap()
      .field("thread", t.as_str())
      .unwrap()
      .field("upto", format!("{t}#m2").as_str())
      .unwrap()
      .field("model", "m")
      .unwrap()
      .field("usd", 0.0)
      .unwrap()
      .field("headline", format!("Leaf {i} headline").as_str())
      .unwrap()
      .field("items", toml::Value::Array(items))
      .unwrap()
      .reference("thread", Ref::to(t.clone()))
      .unwrap()
      .body("# d\n".to_owned());
    root.publish(&node, dr).unwrap();
    leaves.push(t);
  }
  setup(&data, &home, "http://127.0.0.1:9");
  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let b = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![target(ScopeKind::Effort, &leaves[30])],
      query: None,
    },
  )
  .await
  .remove(0);
  assert!(b.len() <= 20_000, "{} bytes", b.len());
  assert!(b.contains("(effort:"), "{b}");
  assert!(b.contains("61 threads"), "{b}");
  assert!(b.contains(&format!("### {p}")), "the root in full");
  assert!(b.contains("said in one line"), "{b}");
  for (i, t) in leaves.iter().enumerate() {
    assert!(b.contains(&t.to_string()), "leaf {i} listed");
  }
  assert!(b.ends_with("</scope>"));
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node_handle.wait().await.unwrap();
}

/// `spawn` with `scopes`: the child's goal carries the briefing.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_threads_goal_carries_its_scopes() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  std::fs::create_dir_all(&home).unwrap();
  let root = home_root(&home);
  let node: NodeName = "mbp".parse().unwrap();
  let earlier = thread(
    &root,
    &node,
    "Earlier work",
    None,
    None,
    exchange("do the thing", "Did the thing."),
  );
  let (base, sent) = serve(vec![
    tool_call(
      "s1",
      "spawn",
      json!({"goal": "Review the earlier work", "wake": false, "scopes": [format!("thread:{earlier}")]}),
    ),
    answer("ok"),
    answer("ok"),
  ])
  .await;
  setup(&data, &home, &base);
  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, "spawn a reviewer", vec![]), |_| None)
    .await
    .unwrap();
  let mut child_first = None;
  for _ in 0..400 {
    child_first = sent.lock().unwrap().iter().find_map(|b| {
      let m = &b["messages"][0];
      texts(m)
        .iter()
        .any(|t| t == "Review the earlier work")
        .then(|| texts(m))
    });
    if child_first.is_some() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let said = child_first.expect("the child ran");
  let at = said
    .iter()
    .position(|t| t == "Review the earlier work")
    .unwrap();
  assert!(said[at + 1].starts_with("<scope>"), "{said:?}");
  assert!(
    said[at + 1].contains(&format!("### {earlier} \"Earlier work\"")),
    "{said:?}"
  );
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node_handle.wait().await.unwrap();
}
