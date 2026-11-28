//! Detached threads on a running node, against a mock model: approvals
//! with nobody attached, reports to the thread that started them, budgets,
//! cancelling, and resuming after a restart.

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

/// Serves `responses` in order, one per connection; records request bodies.
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

fn tool_use(id: &str, command: &str) -> String {
  tool_use_in(id, command, 50)
}

/// `tool_use`, its prompt `input_tokens` long (what it costs).
fn tool_use_in(id: &str, command: &str, input_tokens: u64) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": input_tokens}}}),
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
  answer_in(text, 80)
}

/// `answer`, its prompt `input_tokens` long (what it costs).
fn answer_in(text: &str, input_tokens: u64) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": input_tokens}}}),
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
    // The approvals tested here are of commands the sandbox would let run.
    shell: Some(stitch_node::config::ShellSettings {
      approval: Some("ask".into()),
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

/// The records of `kind` in the home workspace, read from its files.
fn records(home: &Path, kind: &str) -> Vec<IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index.records(Some(kind)).unwrap()
}

/// Waits until `f` gives something.
async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..200 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

/// Allows a question. The test reads questions from disk, where the node
/// writes one a moment before its own index has it; an answer in that gap
/// is refused as "no thread … here" (nothing a client sees: they list what
/// the node's index holds), so ask again until the node knows it.
async fn answer_when_known(c: &mut Client, question: &str) {
  for _ in 0..200 {
    let reply = c
      .request(&Request::Answer {
        question: question.into(),
        allow: true,
        reason: None,
        always: false,
      })
      .await;
    match reply {
      Ok(_) => return,
      Err(e) if e.to_string().contains("no thread") => {
        tokio::time::sleep(Duration::from_millis(50)).await
      }
      Err(e) => panic!("{e}"),
    }
  }
  panic!("the node never knew {question}");
}

fn refs(home: &Path, r: &IndexedRecord, rel: &str) -> Option<String> {
  let root = RecordRoot::open(home).unwrap();
  root
    .read_record(&r.id)
    .unwrap()
    .refs
    .first(rel)
    .map(|x| x.id.to_string())
}

fn detach(goal: &str, parent: Option<&str>, tokens: Option<u64>) -> Request {
  Request::Detach {
    goal: goal.into(),
    model: None,
    repo: None,
    cwd: None,
    parent: parent.map(str::to_owned),
    budget_usd: None,
    budget_tokens: tokens,
    budget_minutes: None,
    max_steps: None,
    global: false,
    node: None,
    context: None,
    no_tools: false,
    tools: Vec::new(),
    skills: Vec::new(),
    keep: false,
    memory: false,
    read_only: false,
    scopes: Vec::new(),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_detached_thread_asks_works_reports_and_wakes_its_parent() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, sent) = serve(vec![
    answer("Parent here."),
    tool_use("t1", &format!("touch '{}'", made.display())),
    answer("Made the file."),
    answer("Noted: the file is there."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let parent_chat = Request::Chat {
    thread: None,
    text: "I am the parent".into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: None,
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
  };
  let Reply::ChatDone { thread: parent, .. } = c.chat(&parent_chat, |_| None).await.unwrap() else {
    panic!()
  };
  let Reply::Detached(child) = c
    .request(&detach("Make the file", Some(&parent), None))
    .await
    .unwrap()
  else {
    panic!()
  };

  // The write needs approval; with nobody attached it becomes a question.
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  assert_eq!(q.fields["approval"], true);
  assert!(q.title.as_deref().unwrap().starts_with("Run `touch"));
  assert!(!made.exists());
  answer_when_known(&mut c, &q.id.to_string()).await;

  // It runs, finishes and reports to its parent.
  let report = until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  assert!(made.exists());
  assert_eq!(report.fields["outcome"], "done");
  assert_eq!(
    refs(&home, &report, "reports-to").as_deref(),
    Some(parent.as_str())
  );
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&report.id)
    .unwrap()
    .body;
  assert!(body.contains("Made the file."), "{body}");
  let thread = records(&home, "thread")
    .into_iter()
    .find(|t| t.id.as_str() == child)
    .unwrap();
  assert_eq!(thread.fields["detached"], true);
  assert_eq!(thread.fields["goal"], "Make the file");
  assert_eq!(
    refs(&home, &thread, "spawned-by").as_deref(),
    Some(parent.as_str())
  );

  // The parent's agent was woken with the report.
  until("the parent's wake", || {
    (sent.lock().unwrap().len() >= 4).then_some(())
  })
  .await;
  let wake = sent.lock().unwrap()[3].clone();
  let last = wake["messages"].as_array().unwrap().last().unwrap().clone();
  let text = last["content"][0]["text"].as_str().unwrap();
  assert!(
    text.starts_with("[Report ") && text.contains("# Make the file: done"),
    "{text}"
  );
  assert!(text.contains("Made the file."));
  // The detached thread's own prompt says it runs on its own.
  assert!(
    sent.lock().unwrap()[1]["system"][0]["text"]
      .as_str()
      .unwrap()
      .contains("running detached")
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn budgets_stop_threads_and_cancelling_closes_their_questions() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // Over budget after one step (70 tokens of 10).
    tool_use("t1", "echo allowed"),
    // Waits for an approval, then is cancelled.
    tool_use("t2", "touch /nonexistent/never"),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let Reply::Detached(cheap) = c
    .request(&detach("Spend little", None, Some(10)))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = until("the budget report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(cheap.as_str()))
  })
  .await;
  assert_eq!(report.fields["outcome"], "over-budget");
  assert_eq!(
    sent.lock().unwrap().len(),
    1,
    "no model call after the budget ran out"
  );

  let Reply::Detached(slow) = c.request(&detach("Wait for me", None, None)).await.unwrap() else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(slow.as_str()))
  })
  .await;
  c.request(&Request::Cancel {
    thread: slow.clone(),
  })
  .await
  .unwrap();
  let report = until("the cancel report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(slow.as_str()))
  })
  .await;
  assert_eq!(report.fields["outcome"], "cancelled");
  // Its question is closed, so it leaves the attention list.
  let closed = records(&home, "answer")
    .into_iter()
    .any(|a| refs(&home, &a, "answers").as_deref() == Some(q.id.as_str()));
  assert!(closed);
  assert!(
    c.request(&Request::Cancel { thread: slow }).await.is_err(),
    "not running any more"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The default mode where shells can be sandboxed: a write runs at once,
/// a push asks.
#[tokio::test(flavor = "multi_thread")]
async fn in_the_sandbox_a_write_runs_and_a_push_asks() {
  // Linux needs the shell helper, which this test binary has not beside it.
  if stitch_shell::Sandbox::available(None).is_err() {
    eprintln!("no sandbox for this test here");
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, _) = serve(vec![
    tool_use("t1", &format!("touch '{}'", made.display())),
    tool_use("t2", &format!("cd '{}' && git push", d.display())),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config.shell = None;
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let Reply::Detached(t) = c
    .request(&detach("Make and push", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let q = until("the push's question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(t.as_str()))
  })
  .await;
  assert!(made.exists(), "the write ran without asking");
  assert!(
    q.title.as_deref().unwrap().contains("git push"),
    "{:?}",
    q.title
  );
  assert_eq!(
    records(&home, "question")
      .iter()
      .filter(|q| refs(&home, q, "thread").as_deref() == Some(t.as_str()))
      .count(),
    1,
    "only the push asked"
  );
  c.request(&Request::Cancel { thread: t }).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_waiting_for_another_turn_can_be_interrupted_unsent() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // The detached turn waits for an approval, holding its thread.
  let (base, sent) = serve(vec![tool_use("t1", "touch /nonexistent/never")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(busy) = c.request(&detach("Hold on", None, None)).await.unwrap() else {
    panic!()
  };
  until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(busy.as_str()))
  })
  .await;

  let mut chat = Client::connect(&data).await.unwrap();
  let req = Request::Chat {
    thread: Some(busy.clone()),
    text: "how is it going?".into(),
    attachments: Vec::new(),
    model: None,
    repo: None,
    cwd: None,
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
  };
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let mut waited = false;
  let done = tokio::time::timeout(
    Duration::from_secs(10),
    chat.chat_with(&req, Some(outgoing), |r| {
      if let Reply::Waiting(true) = r {
        waited = true;
        let _ = to_chat.send(Request::Interrupt);
      }
      None
    }),
  )
  .await
  .expect("the interrupt ends the wait")
  .unwrap();
  assert!(waited, "told it waits");
  assert!(matches!(done, Reply::Unsent), "{done:?}");
  assert_eq!(sent.lock().unwrap().len(), 1, "no model call for it");

  c.request(&Request::Cancel { thread: busy }).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A chat sent to a thread whose turn runs here (one a job's note woke)
/// goes into that turn, woven in between its steps with what its client
/// queued meanwhile, and ends with it: no turn of its own (2026-10-21,
/// four messages queued behind such a turn were never seen by it).
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_waiting_for_another_turn_is_woven_into_it_with_what_was_queued() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("t1", "touch /nonexistent/never"),
    answer("Got both."),
    answer("Should never be asked."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(busy) = c.request(&detach("Hold on", None, None)).await.unwrap() else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(busy.as_str()))
  })
  .await;

  let waited = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let chatting = tokio::spawn({
    let (data, busy, waited) = (data.clone(), busy.clone(), waited.clone());
    async move {
      let mut chat = Client::connect(&data).await.unwrap();
      let req = Request::Chat {
        thread: Some(busy),
        text: "first".into(),
        attachments: Vec::new(),
        model: None,
        repo: None,
        cwd: None,
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
      };
      let mut woven = Vec::new();
      let done = chat
        .chat_with(&req, Some(outgoing), |r| {
          match r {
            Reply::Waiting(true) => {
              waited.store(true, std::sync::atomic::Ordering::SeqCst);
              let _ = to_chat.send(Request::Interject {
                id: 1,
                text: "queued one".into(),
                attachments: Vec::new(),
                scopes: Vec::new(),
              });
            }
            Reply::Woven(ids) => woven.extend(ids.iter().copied()),
            _ => {}
          }
          None
        })
        .await;
      (done, woven)
    }
  });
  until("told it waits", || {
    waited
      .load(std::sync::atomic::Ordering::SeqCst)
      .then_some(())
  })
  .await;
  tokio::time::sleep(Duration::from_millis(500)).await;
  c.request(&Request::Answer {
    question: q.id.to_string(),
    allow: false,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  let (done, mut woven) = tokio::time::timeout(Duration::from_secs(20), chatting)
    .await
    .expect("the chat ended with the turn it went into")
    .unwrap();
  let done = done.unwrap();
  let Reply::ChatDone { stop, thread, .. } = &done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "woven");
  assert_eq!(thread, &busy);
  woven.sort();
  assert_eq!(woven, vec![0, 1], "both said woven");
  tokio::time::sleep(Duration::from_millis(500)).await;
  let bodies = sent.lock().unwrap().clone();
  assert_eq!(bodies.len(), 2, "no turn of its own");
  let last = bodies.last().unwrap().to_string();
  assert!(
    last.contains("first") && last.contains("queued one"),
    "{last}"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A client that leaves while its message waits for another turn on the
/// thread (a TUI quit or killed) takes its message with it: it was sent
/// later, when that turn ended, to no one (2026-10-21).
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_waiting_for_another_turn_is_unsent_when_its_client_leaves() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // The detached turn asks for a command, is told no, and ends.
  let (base, sent) = serve(vec![
    tool_use("t1", "touch /nonexistent/never"),
    answer("Not run."),
    answer("Should never be asked."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(busy) = c.request(&detach("Hold on", None, None)).await.unwrap() else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(busy.as_str()))
  })
  .await;

  let waited = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let chatting = tokio::spawn({
    let (data, busy, waited) = (data.clone(), busy.clone(), waited.clone());
    async move {
      let mut chat = Client::connect(&data).await.unwrap();
      let req = Request::Chat {
        thread: Some(busy),
        text: "sent to no one".into(),
        attachments: Vec::new(),
        model: None,
        repo: None,
        cwd: None,
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
      };
      let _ = chat
        .chat_with(&req, None, |r| {
          if let Reply::Waiting(true) = r {
            waited.store(true, std::sync::atomic::Ordering::SeqCst);
          }
          None
        })
        .await;
    }
  });
  until("told it waits", || {
    waited
      .load(std::sync::atomic::Ordering::SeqCst)
      .then_some(())
  })
  .await;
  // The client goes (its connection closes), then the other turn ends.
  chatting.abort();
  let _ = chatting.await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  c.request(&Request::Answer {
    question: q.id.to_string(),
    allow: false,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  until("the other turn's end", || {
    (sent.lock().unwrap().len() >= 2).then_some(())
  })
  .await;
  tokio::time::sleep(Duration::from_secs(2)).await;
  assert_eq!(sent.lock().unwrap().len(), 2, "no model call for it");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_node_resumes_and_finds_its_question() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("after-restart.txt");
  let command = format!("touch '{}'", made.display());
  let (base, sent) = serve(vec![
    tool_use("t1", &command),
    // After the restart its step is replayed, not asked again.
    answer("Done after a restart."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&detach("Survive a restart", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  // Answered while the node was down (as sync would bring it): the loop
  // that resumes finds its question answered before it asks again.
  let node_name: stitch_records::NodeName = "mbp".parse().unwrap();
  let answer =
    stitch_records::Draft::new("answer", stitch_records::Author::user(node_name.clone()))
      .unwrap()
      .field("choice", "allow")
      .unwrap()
      .reference("answers", stitch_records::Ref::to(q.id.clone()))
      .unwrap();
  RecordRoot::open(&home)
    .unwrap()
    .publish(&node_name, answer)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let report = until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  assert_eq!(report.fields["outcome"], "done");
  assert!(made.exists());
  // One question in all: the resumed loop found the old one.
  let questions = records(&home, "question")
    .into_iter()
    .filter(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
    .count();
  assert_eq!(questions, 1);
  assert_eq!(sent.lock().unwrap().len(), 2, "the step was replayed");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A thread waiting on an approval when its node stops resumes with the
/// same call and the same question, not a new model call (which may ask
/// for something else, as a real model did, losing the user's answer):
/// answered after the restart, the very command it asked for runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_resumed_after_a_restart_replays_its_waiting_call() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("asked-before.txt");
  let command = format!("touch '{}'", made.display());
  let (base, sent) = serve(vec![
    tool_use("t1", &command),
    // What a new model call after the restart would get: no call at all
    // (so the command would never run).
    answer("Done."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&detach("Replay after a restart", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  // Back, it waits on the same question; the user answers it then.
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  tokio::time::sleep(Duration::from_secs(2)).await;
  assert_eq!(
    sent.lock().unwrap().len(),
    1,
    "no model call before the answer"
  );
  c.request(&Request::Answer {
    question: q.id.to_string(),
    allow: true,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  until("the command it asked for", || made.exists().then_some(())).await;
  let questions: Vec<_> = records(&home, "question")
    .into_iter()
    .filter(|r| refs(&home, r, "thread").as_deref() == Some(thread.as_str()))
    .collect();
  assert_eq!(questions.len(), 1, "asked once: {questions:?}");
  until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  // The model's next call reads the call it made before the restart.
  let model_calls = sent.lock().unwrap().clone();
  assert_eq!(model_calls.len(), 2);
  let last = model_calls[1].to_string();
  assert!(last.contains("\"t1\""), "{last}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Issue n6rx (2026-11-02): a thread's command that stopped its node
/// (`pkill -f "stitch node"`: a SIGTERM, which stops the node as cleanly
/// as a Stop request) was replayed at every start, stopping it again, for
/// 40 minutes. A command the node stopped (cleanly) in while it ran is
/// replayed once; stopped in again, it is not run a third time: its
/// result says why, and the thread goes on.
///
/// One worker, as a small runner or container has (issue svf3,
/// 2026-11-22): the turn the note of the cut shells starts runs before
/// the resumed loop (the task spawned last runs first) unless the
/// resumed loop takes notes from the start; with more workers it was a
/// race, the call not replayed (or its third start reported "done").
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_command_the_node_stops_in_twice_is_not_run_a_third_time() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let runs = d.join("runs.txt");
  let command = format!("echo run >> '{}'; sleep 60", runs.display());
  let (base, sent) = serve(vec![tool_use("t1", &command), answer("Done.")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let ran = || {
    std::fs::read_to_string(&runs)
      .map(|t| t.lines().count())
      .unwrap_or(0)
  };
  let mut thread = None;
  for n in 1..=2 {
    let node = stitch_node::start(&data).await.unwrap();
    let mut c = Client::connect(&data).await.unwrap();
    if thread.is_none() {
      let Reply::Detached(t) = c
        .request(&detach("Stop the node", None, None))
        .await
        .unwrap()
      else {
        panic!()
      };
      thread = Some(t);
    }
    let t = thread.clone().unwrap();
    // It runs (its approval answered once: a replay finds it answered).
    for _ in 0..400 {
      if ran() >= n {
        break;
      }
      let q = records(&home, "question")
        .into_iter()
        .find(|q| refs(&home, q, "thread").as_deref() == Some(t.as_str()));
      if let Some(q) = q
        && records(&home, "answer").is_empty()
      {
        answer_when_known(&mut c, q.id.as_str()).await;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(ran(), n, "run {n}");
    // The node stops (cleanly) while it runs.
    c.request(&Request::Stop).await.unwrap();
    node.wait().await.unwrap();
  }
  let t = thread.unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let report = until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(t.as_str()))
  })
  .await;
  assert_eq!(ran(), 2, "not run a third time");
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&report.id)
    .unwrap()
    .body;
  // Its only call failed (not run again), so it reports not done.
  assert_eq!(report.fields["outcome"], "not-done", "{body}");
  assert!(body.contains("Done."), "{body}");
  let model_calls = sent.lock().unwrap().clone();
  assert_eq!(model_calls.len(), 2);
  let last = model_calls[1].to_string();
  assert!(last.contains("not run again"), "{last}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A step is written once its calls returned; meanwhile clients following
/// the thread (the TUI, the app, the web page) read its running call, with
/// its whole input and since when, from what the loop keeps
/// (2026-11-01: a 13-minute deploy showed only "a turn is running").
#[tokio::test(flavor = "multi_thread")]
async fn a_running_call_is_seen_in_flight_till_its_step_is_written() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let command = "sleep 2";
  let (base, _sent) = serve(vec![tool_use("t1", command), answer("Deployed.")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c.request(&detach("Deploy it", None, None)).await.unwrap() else {
    panic!()
  };
  let thread: stitch_records::RecordId = thread.parse().unwrap();
  // Read as a client reads it: its own connection to the index.
  let index = Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  let pid = stitch_node::node_pid(&data);
  assert!(pid.is_some());
  let step = until("the call in flight", || {
    stitch_node::chat::step_in_flight(&index, &thread, pid)
  })
  .await;
  let call = step.running().expect("its running call");
  assert_eq!(call.name, "shell_run");
  assert_eq!(call.input["command"], command);
  assert_eq!(call.summary, format!("$ {command}"));
  assert!(call.since.is_some_and(|t| t <= jiff::Timestamp::now()));
  assert_eq!(call.ok, None);
  // The same through the API (the app, a TUI on another node).
  let asked = |lines: Reply| match lines {
    Reply::Lines(l) => l
      .first()
      .map(|j| serde_json::from_str::<stitch_node::chat::StepInFlight>(j).unwrap()),
    other => panic!("{other:?}"),
  };
  let in_flight = Request::InFlight {
    thread: thread.to_string(),
  };
  let via_api = asked(c.request(&in_flight).await.unwrap());
  assert_eq!(via_api.as_ref().and_then(|s| s.running()), Some(call));
  // Another process's step (a node that stopped) is not in flight.
  assert_eq!(
    stitch_node::chat::step_in_flight(&index, &thread, pid.map(|p| p + 1)),
    None
  );
  // It asks first (this node asks for every command): in flight all along.
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  // The web client opening the thread now reads the step in its
  // transcript: the message, and the call without its result (it showed
  // none of the turn's calls till its step was written).
  let transcript = |r: Reply| match r {
    Reply::Lines(l) => serde_json::from_str::<serde_json::Value>(&l[0]).unwrap(),
    other => panic!("{other:?}"),
  };
  let read = Request::Transcript {
    thread: thread.to_string(),
    after: None,
    before: None,
    tail: None,
  };
  let mid = transcript(c.request(&read).await.unwrap());
  let entries = mid["entries"].as_array().unwrap();
  let user: Vec<_> = entries.iter().filter(|e| e["kind"] == "user").collect();
  assert_eq!(user.len(), 1, "{mid}");
  assert_eq!(user[0]["text"], "Deploy it");
  let tools: Vec<_> = entries.iter().filter(|e| e["kind"] == "tool").collect();
  assert_eq!(tools.len(), 1, "{mid}");
  assert_eq!(tools[0]["call"], format!("$ {command}"));
  assert!(tools[0]["result"].is_null(), "{mid}");
  answer_when_known(&mut c, q.id.as_str()).await;
  let still = stitch_node::chat::step_in_flight(&index, &thread, pid);
  assert_eq!(still.as_ref().and_then(|s| s.running()), Some(call));
  until("the step written", || {
    stitch_node::chat::step_in_flight(&index, &thread, pid)
      .is_none()
      .then_some(())
  })
  .await;
  until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  assert_eq!(
    stitch_node::chat::step_in_flight(&index, &thread, pid),
    None
  );
  assert_eq!(asked(c.request(&in_flight).await.unwrap()), None);
  // Written, it is there once, with its result.
  let after = transcript(c.request(&read).await.unwrap());
  let entries = after["entries"].as_array().unwrap();
  let tools: Vec<_> = entries.iter().filter(|e| e["kind"] == "tool").collect();
  assert_eq!(tools.len(), 1, "{after}");
  assert!(!tools[0]["result"].is_null(), "{after}");
  let user = entries.iter().filter(|e| e["kind"] == "user").count();
  assert_eq!(user, 1, "{after}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_from_a_node_whose_clock_is_behind_is_taken_up() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("skewed.txt");
  let command = format!("touch '{}'", made.display());
  let (base, _sent) = serve(vec![
    // A step written first, so the question comes after it.
    tool_use("t0", "echo first"),
    tool_use("t1", &command),
    // After the restart its waiting step is replayed: no model call.
    answer("Done, answered from afar."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&detach("Survive a skewed answer", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  // Answered while the node was down, by a node whose clock is an hour
  // behind: the answer looks older than the step before the question.
  let phone: stitch_records::NodeName = "phone".parse().unwrap();
  let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1);
  let answer = stitch_records::Draft::new("answer", stitch_records::Author::user(phone.clone()))
    .unwrap()
    .field("choice", "allow")
    .unwrap()
    .reference("answers", stitch_records::Ref::to(q.id.clone()))
    .unwrap()
    .into_record(stitch_records::RecordId::random(at, &phone), at);
  RecordRoot::open(&home)
    .unwrap()
    .publish_record(&answer)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let report = until("the report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(thread.as_str()))
  })
  .await;
  assert_eq!(report.fields["outcome"], "done");
  assert!(made.exists());
  let questions = records(&home, "question")
    .into_iter()
    .filter(|q| refs(&home, q, "thread").as_deref() == Some(thread.as_str()))
    .count();
  assert_eq!(questions, 1, "the answer was taken up, not asked again");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A report to a thread past its budget is kept in that thread, not only
/// in the thread's own over-budget report to its spawner: the thread may
/// be the one the user talks to (2026-10-19: a blocked report to a thread
/// past its deadline went to the thread above it and never showed where
/// the user was talking). No model call for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_report_to_a_thread_past_its_budget_is_kept_in_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // The parent: over budget after one step (70 tokens of 10).
    tool_use("t1", "echo allowed"),
    // The child it started.
    answer("Child done: CHILD-WORDS."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let Reply::Detached(parent) = c
    .request(&detach("Spend little", None, Some(10)))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the parent's budget report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(parent.as_str()))
  })
  .await;
  let Reply::Detached(child) = c
    .request(&detach("Report back", Some(&parent), None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = until("the child's report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  // The parent's turn does not run, and says so to its own spawner (the
  // user here) with what came for it...
  let refused = until("the parent's second report", || {
    let mine: Vec<IndexedRecord> = records(&home, "report")
      .into_iter()
      .filter(|r| refs(&home, r, "thread").as_deref() == Some(parent.as_str()))
      .collect();
    (mine.len() >= 2).then(|| mine.into_iter().max_by_key(|r| r.created).unwrap())
  })
  .await;
  assert_eq!(refused.fields["outcome"], "over-budget");
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&refused.id)
    .unwrap()
    .body;
  assert!(body.contains("CHILD-WORDS"), "{body}");
  // ...and the report is kept in the parent's own transcript, with
  // Stitch's line on why no turn ran.
  let transcript = until("the report kept in the parent", || {
    let dir = home.join("threads").join(parent.as_str());
    let all: String = std::fs::read_dir(&dir)
      .ok()?
      .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
      .collect();
    all
      .contains(&format!("[Report {}", report.id))
      .then_some(all)
  })
  .await;
  assert!(transcript.contains("CHILD-WORDS"), "{transcript}");
  assert!(
    transcript.contains("[Stitch: no turn ran for this: "),
    "{transcript}"
  );
  assert_eq!(
    sent.lock().unwrap().len(),
    2,
    "no model call for the parent"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A thread past its budget that its spawner's reply reaches (the reply
/// to a `send` it made while it ran) keeps it, and does not report it
/// back to that spawner: it would come back to the thread that wrote it as
/// a new report, which it answers, and so on (issue
/// 2026-11-11T154611Z-beast-5vp5).
#[tokio::test(flavor = "multi_thread")]
async fn a_spawners_reply_to_a_thread_past_its_budget_is_not_echoed_back() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // The parent's first turn.
    answer("Parent started."),
    // The child: over budget after one step (70 tokens of 10).
    tool_use("t1", "echo allowed"),
    // The parent, woken by the child's report.
    answer("Parent read it."),
    // The parent's reply to what the child sent it.
    answer("PARENT-REPLY-TO-CHILD"),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let reports_of = |thread: &str| -> Vec<IndexedRecord> {
    records(&home, "report")
      .into_iter()
      .filter(|r| refs(&home, r, "thread").as_deref() == Some(thread))
      .collect()
  };
  let Reply::Detached(parent) = c.request(&detach("Lead", None, None)).await.unwrap() else {
    panic!()
  };
  until("the parent's report", || {
    (!reports_of(&parent).is_empty()).then_some(())
  })
  .await;
  let Reply::Detached(child) = c
    .request(&detach("Spend little", Some(&parent), Some(10)))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the child's budget report and the parent's answer", || {
    (!reports_of(&child).is_empty() && reports_of(&parent).len() >= 2).then_some(())
  })
  .await;
  // The child (past its budget) had sent its parent a note: the reply is
  // reported to it.
  c.request(&Request::Send {
    thread: parent.clone(),
    text: "A progress note from the child.".into(),
    from: Some(child.clone()),
    keep: false,
    interject: false,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  // Kept in the child, with Stitch's line on why no turn ran...
  until("the reply kept in the child", || {
    let dir = home.join("threads").join(child.as_str());
    let all: String = std::fs::read_dir(&dir)
      .ok()?
      .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
      .collect();
    (all.contains("PARENT-REPLY-TO-CHILD") && all.contains("[Stitch: no turn ran for this: "))
      .then_some(())
  })
  .await;
  tokio::time::sleep(Duration::from_millis(1500)).await;
  // ...and not reported back to the parent, which wrote it.
  for r in reports_of(&child) {
    let body = RecordRoot::open(&home)
      .unwrap()
      .read_record(&r.id)
      .unwrap()
      .body;
    assert!(!body.contains("PARENT-REPLY-TO-CHILD"), "echoed: {body}");
  }
  assert_eq!(
    sent.lock().unwrap().len(),
    4,
    "no turn of the parent for its own reply"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The user writing in a spawned thread takes it over: its spawn budget is
/// lifted (a user's `set` of `budget`), so a report that comes for it later
/// runs a turn instead of being refused as over budget (2026-10-19: a
/// thread the user chatted with kept its spawner's deadline, and every
/// thread it spawned, and every report to it, died with it).
#[tokio::test(flavor = "multi_thread")]
async fn the_user_writing_in_a_spawned_thread_lifts_its_budget() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // The parent: over budget after one step (70 tokens of 10).
    tool_use("t1", "echo allowed"),
    // The user's chat in it.
    answer("Yours now."),
    // The child it started.
    answer("Child done: CHILD-WORDS."),
    // The parent, woken by the child's report: it runs.
    answer("Parent read CHILD-WORDS."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let Reply::Detached(parent) = c
    .request(&detach("Spend little", None, Some(10)))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the parent's budget report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(parent.as_str()))
  })
  .await;
  let done = c
    .chat(
      &Request::Chat {
        thread: Some(parent.clone()),
        text: "I'll take it from here".into(),
        attachments: Vec::new(),
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
      },
      |_| None,
    )
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  let lifted = records(&home, "set")
    .into_iter()
    .filter(|r| refs(&home, r, "target").as_deref() == Some(parent.as_str()))
    .filter(|r| r.fields.get("field").and_then(|v| v.as_str()) == Some("budget"))
    .count();
  assert_eq!(lifted, 1, "the spawn budget is lifted once");

  let Reply::Detached(child) = c
    .request(&detach("Report back", Some(&parent), None))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the child's report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  // The parent's turn runs on the report: a model call for it, and its
  // report is no longer over budget.
  let last = until("the parent's turn on the report", || {
    let mine: Vec<IndexedRecord> = records(&home, "report")
      .into_iter()
      .filter(|r| refs(&home, r, "thread").as_deref() == Some(parent.as_str()))
      .collect();
    (mine.len() >= 2).then(|| mine.into_iter().max_by_key(|r| r.created).unwrap())
  })
  .await;
  assert_eq!(last.fields["outcome"], "done");
  assert_eq!(sent.lock().unwrap().len(), 4);

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A report to a thread that was compacted since is the newest thread's
/// once it had it: a restarted node finds it there and does not deliver it
/// again (issue 2026-10-31T064612Z-gamer-evuq: reports to a compacted
/// chat were redelivered, "[Late: … held until now]", at every restart,
/// as its node looked for them only in the thread they were addressed to).
#[tokio::test(flavor = "multi_thread")]
async fn a_report_had_by_a_compacted_thread_is_not_delivered_again() {
  use stitch_records::{Author, Draft, Event, Msg, NodeName, Part, Ref, Role as Who, kinds};
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![answer("Read the new one.")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config.detached = Some(stitch_node::config::DetachedSettings {
    gather_secs: Some(1),
    ..Default::default()
  });
  config.save(&data).unwrap();

  let root = RecordRoot::open(&home).unwrap();
  let mbp: NodeName = "mbp".parse().unwrap();
  let msg = |id: &str, role: Who, text: &str| {
    Event::Msg(Msg {
      id: id.into(),
      at: jiff::Timestamp::now(),
      role,
      from: None,
      model: (role == Who::Assistant).then(|| "claude-opus-5-5".into()),
      parts: vec![Part::Text { text: text.into() }],
      native: None,
    })
  };
  let thread = |draft: Draft| root.publish(&mbp, draft).unwrap().id;
  // The user's chat, and a thread it started.
  let old = thread(
    Draft::new(kinds::THREAD, Author::user(mbp.clone()))
      .unwrap()
      .body("# a chat\n"),
  );
  let first = root
    .publish_segment(
      &mbp,
      &old,
      None,
      None,
      vec![
        msg("m1", Who::User, "spawn a helper"),
        msg("m2", Who::Assistant, "spawned"),
      ],
    )
    .unwrap()
    .header
    .segment;
  let child = thread(
    Draft::new(kinds::THREAD, Author::node(mbp.clone()))
      .unwrap()
      .body("# a helper\n"),
  );
  let report = |words: &str| {
    root
      .publish(
        &mbp,
        Draft::new(kinds::REPORT, Author::node(mbp.clone()))
          .unwrap()
          .field("final", true)
          .unwrap()
          .field("outcome", "done")
          .unwrap()
          .reference("thread", Ref::to(child.clone()))
          .unwrap()
          .reference("reports-to", Ref::to(old.clone()))
          .unwrap()
          .body(format!("# a helper: done\n\n{words}\n")),
      )
      .unwrap()
      .id
  };
  let had = report("HAD-WORDS");
  // Compacted, then the report came: the chat it continues in had it.
  let continues = stitch_node::chat::Continues {
    thread: old.clone(),
    at: stitch_node::chat::Upto::Segment(first),
    shape: stitch_node::chat::Shape {
      summary: Some("A chat that spawned a helper.".into()),
      ..Default::default()
    },
  };
  let new = thread(
    continues
      .write(
        Draft::new(kinds::THREAD, Author::user(mbp.clone()))
          .unwrap()
          .field("compacted", true)
          .unwrap(),
      )
      .unwrap()
      .body("# a chat\n"),
  );
  root
    .publish_segment(
      &mbp,
      &new,
      None,
      None,
      vec![
        msg(
          "m3",
          Who::User,
          &format!("[Report {had} from mbp: thread {child}]\nHAD-WORDS"),
        ),
        msg("m4", Who::Assistant, "noted"),
      ],
    )
    .unwrap();
  // One it never had, while the node was down.
  let fresh = report("FRESH-WORDS");

  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  until("the turn on the report it did not have", || {
    (!sent.lock().unwrap().is_empty()).then_some(())
  })
  .await;
  tokio::time::sleep(Duration::from_secs(3)).await;
  let calls = sent.lock().unwrap().clone();
  assert_eq!(calls.len(), 1, "one turn");
  let last = calls[0]["messages"]
    .as_array()
    .unwrap()
    .last()
    .unwrap()
    .to_string();
  assert!(last.contains(&format!("[Report {fresh}")), "{last}");
  assert!(
    !last.contains(&format!("[Report {had}")),
    "had already: {last}"
  );
  assert!(!last.contains("held until now"), "{last}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A message for a thread whose turn runs, past its compaction threshold,
/// waits for that turn before the thread is compacted: the thread it goes
/// on in runs no turn beside the one still running (issue
/// 2026-11-04T102014Z-gamer-mfnh: a child's note compacted its waiting
/// parent at once, and the compacted thread's turn ran beside the
/// original's, two supervisors on one goal).
#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_a_running_thread_compacts_it_only_after_its_turn() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // A first step, written: its prompt (50 tokens of a 60-token window)
    // is past the threshold.
    tool_use("t0", "touch /nonexistent/first"),
    // Then the turn waits for an approval, holding its thread.
    tool_use("t1", "touch /nonexistent/never"),
    answer("First done."),
    // The message's turn, after it: compacted first.
    answer("SUMMARY-WORDS"),
    answer("Second done."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(60);
  config.threads = Some(stitch_node::config::ThreadSettings {
    compact_at: Some(80),
    compact_keep: Some(2),
    titles: Some(false),
    ..Default::default()
  });
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(busy) = c.request(&detach("Hold on", None, None)).await.unwrap() else {
    panic!()
  };
  let asked = |n: usize| {
    let home = home.clone();
    let busy = busy.clone();
    move || {
      let mut qs: Vec<IndexedRecord> = records(&home, "question")
        .into_iter()
        .filter(|q| refs(&home, q, "thread").as_deref() == Some(busy.as_str()))
        .collect();
      qs.sort_by_key(|q| q.created);
      (qs.len() >= n).then(|| qs.swap_remove(n - 1))
    }
  };
  let first = until("the first question", asked(1)).await;
  answer_when_known(&mut c, &first.id.to_string()).await;
  let question = until("the second question", asked(2)).await;
  assert_eq!(sent.lock().unwrap().len(), 2);

  // A chat typed into it meanwhile waits for that turn (here interrupted,
  // unsent) instead of compacting it and running beside it.
  let mut chat = Client::connect(&data).await.unwrap();
  let req = Request::Chat {
    thread: Some(busy.clone()),
    text: "how is it going?".into(),
    attachments: Vec::new(),
    model: None,
    repo: None,
    cwd: None,
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
  };
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let done = tokio::time::timeout(
    Duration::from_secs(10),
    chat.chat_with(&req, Some(outgoing), |r| {
      if let Reply::Waiting(true) = r {
        let _ = to_chat.send(Request::Interrupt);
      }
      None
    }),
  )
  .await
  .expect("the interrupt ends the wait")
  .unwrap();
  assert!(matches!(done, Reply::Unsent), "{done:?}");

  c.request(&Request::Send {
    thread: busy.clone(),
    text: "NOTE-FROM-CHILD".into(),
    from: None,
    keep: false,
    interject: false,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  tokio::time::sleep(Duration::from_secs(2)).await;
  let calls = shown_calls(&sent);
  assert_eq!(
    calls.len(),
    2,
    "no model call while the turn runs: {calls:?}"
  );
  assert!(
    records(&home, "thread")
      .iter()
      .all(|t| t.fields.get("compacted").is_none()),
    "not compacted while its turn runs"
  );

  answer_when_known(&mut c, &question.id.to_string()).await;
  for _ in 0..200 {
    if sent.lock().unwrap().len() >= 5 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let calls = shown_calls(&sent);
  assert_eq!(calls.len(), 5, "{calls:?}");
  // The running turn ends first, then the message's: compacted (the
  // summary's call), then on the new thread.
  assert!(calls[2].contains("\"t1\""), "the turn first: {calls:?}");
  assert!(!calls[2].contains("NOTE-FROM-CHILD"), "{calls:?}");
  assert!(calls[4].contains("SUMMARY-WORDS"), "{calls:?}");
  assert!(calls[4].contains("NOTE-FROM-CHILD"), "{calls:?}");
  until("the compacted thread", || {
    records(&home, "thread")
      .into_iter()
      .find(|t| t.fields.get("compacted").is_some())
  })
  .await;

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The model calls' messages, as text.
fn shown_calls(sent: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
  sent
    .lock()
    .unwrap()
    .iter()
    .map(|b| b["messages"].to_string())
    .collect()
}

/// A turn that outgrows its model's window midway (its prompt under the
/// compaction threshold as it started, past the window steps later) is
/// compacted when the model refuses it as too long, and goes on in the new
/// thread, told it was compacted midway, instead of ending failed (issue
/// 2026-11-19T095120Z-gamer-rwg5).
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_too_long_for_the_model_midway_is_compacted_and_goes_on() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let too_long = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 1012940 tokens > 1000000 maximum\"}}".to_owned();
  let (base, sent) = serve(vec![
    // A first step, written, with its call's result.
    tool_use("t0", "echo STEP-ONE"),
    // The next one is too long for the model.
    too_long,
    answer("SUMMARY-WORDS"),
    answer("Finished after compaction."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  // Its window known (not looked up): the first step's prompt far under
  // its compaction threshold.
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach("THE-GOAL-WORDS", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = until("the report", || records(&home, "report").into_iter().next()).await;
  let index = {
    let root = RecordRoot::open(&home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    index
  };
  let new = index
    .compacted_into(&child.parse().unwrap())
    .unwrap_or_else(|| {
      panic!(
        "not compacted: {:?} {:?}",
        report.fields,
        shown_calls(&sent)
      )
    });
  assert_eq!(report.fields["outcome"], "done", "{:?}", report.fields);
  assert_eq!(
    refs(&home, &report, "thread").as_deref(),
    Some(new.as_str()),
    "reported by the thread it went on in"
  );
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&report.id)
    .unwrap()
    .body;
  assert!(body.contains("Finished after compaction."), "{body}");
  let calls = shown_calls(&sent);
  assert_eq!(calls.len(), 4, "{calls:?}");
  // On the new thread: the summary, and a note that it goes on, not the
  // goal sent again as a new message.
  assert!(calls[3].contains("SUMMARY-WORDS"), "{calls:?}");
  assert!(calls[3].contains("compacted"), "{}", calls[3]);
  assert_eq!(
    calls[3].matches("THE-GOAL-WORDS").count(),
    calls[3].contains("THE-GOAL-WORDS") as usize,
    "the goal at most once: {}",
    calls[3]
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The same for a turn sent to a thread that ran before (`turn_with`):
/// compacted midway, it goes on in the new thread within the same turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_sent_turn_too_long_for_the_model_midway_is_compacted_and_goes_on() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let too_long = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 1012940 tokens > 1000000 maximum\"}}".to_owned();
  let (base, sent) = serve(vec![
    answer("First turn done."),
    // The message's turn: a step written, then one too long.
    tool_use("t1", "echo STEP-TWO"),
    too_long,
    answer("SUMMARY-WORDS"),
    answer("Second finished after compaction."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach("THE-GOAL-WORDS", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the first report", || {
    records(&home, "report").into_iter().next()
  })
  .await;
  c.request(&Request::Send {
    thread: child.clone(),
    text: "SECOND-MESSAGE".into(),
    from: None,
    keep: false,
    interject: false,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  let report = until("the second report", || {
    let mut reports = records(&home, "report");
    reports.sort_by_key(|r| r.created);
    (reports.len() >= 2).then(|| reports.swap_remove(1))
  })
  .await;
  let index = {
    let root = RecordRoot::open(&home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    index
  };
  let new = index
    .compacted_into(&child.parse().unwrap())
    .unwrap_or_else(|| {
      panic!(
        "not compacted: {:?} {:?}",
        report.fields,
        shown_calls(&sent)
      )
    });
  assert_eq!(report.fields["outcome"], "done", "{:?}", report.fields);
  assert_eq!(
    refs(&home, &report, "thread").as_deref(),
    Some(new.as_str())
  );
  let calls = shown_calls(&sent);
  assert_eq!(calls.len(), 5, "{calls:?}");
  assert!(calls[4].contains("SUMMARY-WORDS"), "{}", calls[4]);
  assert!(calls[4].contains("compacted in the middle"), "{}", calls[4]);
  assert!(
    calls[4].matches("SECOND-MESSAGE").count() <= 1,
    "the message at most once: {}",
    calls[4]
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// `detach`, with a step limit and a budget in money.
fn detach_with(goal: &str, steps: Option<u64>, usd: Option<f64>) -> Request {
  let mut r = detach(goal, None, None);
  if let Request::Detach {
    max_steps,
    budget_usd,
    ..
  } = &mut r
  {
    *max_steps = steps;
    *budget_usd = usd;
  }
  r
}

/// A provider's refusal of a prompt too long for the model's window.
fn too_long() -> String {
  "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 1012940 tokens > 1000000 maximum\"}}".to_owned()
}

/// Its window known (not looked up): a first step's prompt far under the
/// compaction threshold.
fn known_window(data: &Path) {
  let mut config = Config::load(data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(data).unwrap();
}

fn compacted_into(home: &Path, thread: &str) -> Option<String> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index
    .compacted_into(&thread.parse().unwrap())
    .map(|t| t.to_string())
}

/// The `n`th report written (oldest first), once there is one, then a
/// moment for any model call after it.
async fn nth_report(home: &Path, n: usize) -> IndexedRecord {
  let report = until("the report", || {
    let mut reports = records(home, "report");
    reports.sort_by_key(|r| r.created);
    (reports.len() > n).then(|| reports.swap_remove(n))
  })
  .await;
  tokio::time::sleep(Duration::from_millis(1500)).await;
  report
}

/// More answers than a turn may take: what a continuation given more
/// than was left of its turn would go on with.
fn spare(n: usize) -> Vec<String> {
  (0..n)
    .map(|i| tool_use(&format!("x{i}"), &format!("echo SPARE-{i}")))
    .collect()
}

/// A sent turn (`turn_with`'s retry) compacted midway goes on with the
/// steps left of it: of 4 steps (and the wrap-up call the step limit
/// makes), 2 written and the refused call taken, the retry takes 1 and its
/// wrap-up, never 4 more.
#[tokio::test(flavor = "multi_thread")]
async fn a_retry_after_a_midway_compaction_takes_only_the_steps_left() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    answer("First turn done."),
    tool_use("t1", "echo ONE"),
    tool_use("t2", "echo TWO"),
    too_long(),
    answer("SUMMARY-WORDS"),
    tool_use("t3", "echo THREE"),
  ];
  responses.extend(spare(6));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", Some(4), None))
    .await
    .unwrap()
  else {
    panic!()
  };
  nth_report(&home, 0).await;
  c.request(&Request::Send {
    thread: child.clone(),
    text: "SECOND-MESSAGE".into(),
    from: None,
    keep: false,
    interject: false,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  let report = nth_report(&home, 1).await;
  let new = compacted_into(&home, &child).expect("compacted");
  assert_eq!(
    refs(&home, &report, "thread").as_deref(),
    Some(new.as_str())
  );
  assert_eq!(report.fields["outcome"], "stopped", "{:?}", report.fields);
  // The first turn's call; the sent turn's 2 steps, its refused call, the
  // summary; then 1 step and the wrap-up: 4 + 1 calls of the turn, as
  // many as it had without compacting (4 steps and the wrap-up).
  let calls = shown_calls(&sent);
  assert_eq!(calls.len(), 1 + 3 + 1 + 2, "{calls:#?}");
  // The retry, on the compacted thread.
  assert!(calls[5].contains("SUMMARY-WORDS"), "{}", calls[5]);
  assert!(calls[5].contains("compacted in the middle"), "{}", calls[5]);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// No step left when the refused call comes: no compaction and no step
/// after it; the report says the step limit stopped it.
#[tokio::test(flavor = "multi_thread")]
async fn no_step_left_at_the_overflow_means_no_continuation() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![tool_use("t1", "echo ONE"), too_long()];
  responses.push(answer("SUMMARY-WORDS"));
  responses.extend(spare(6));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", Some(2), None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = nth_report(&home, 0).await;
  assert_eq!(report.fields["outcome"], "stopped", "{:?}", report.fields);
  assert_eq!(compacted_into(&home, &child), None);
  assert_eq!(shown_calls(&sent).len(), 2);
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&report.id)
    .unwrap()
    .body;
  assert!(body.contains("most steps"), "{body}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A turn's money carries across a midway compaction: of $2.95 (the
/// summary's bound, about $1.92, left after the first step), a step of
/// about $1 spent before the refused call, the continuation has about
/// $1.95, so it stops over budget after its second step of $1 (with its
/// whole budget again, it would take a third).
#[tokio::test(flavor = "multi_thread")]
async fn a_continuation_after_a_midway_compaction_has_the_money_left() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // 250k prompt tokens of claude-opus-5-5: $1.00 a step.
  let mut responses = vec![
    tool_use_in("t1", "echo ONE", 250_000),
    too_long(),
    answer("SUMMARY-WORDS"),
    tool_use_in("t2", "echo TWO", 250_000),
    tool_use_in("t3", "echo THREE", 250_000),
  ];
  responses.extend((0..4).map(|i| tool_use_in(&format!("x{i}"), "echo SPARE", 250_000)));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(2.95)))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = nth_report(&home, 0).await;
  let new = compacted_into(&home, &child).expect("compacted");
  assert_eq!(
    refs(&home, &report, "thread").as_deref(),
    Some(new.as_str())
  );
  assert_eq!(
    report.fields["outcome"], "over-budget",
    "{:?}",
    report.fields
  );
  assert_eq!(shown_calls(&sent).len(), 5, "{:#?}", shown_calls(&sent));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A schedule's firing brings its own cap (here 3 steps), smaller than its
/// thread's (none): compacted midway, its turn goes on under what is left
/// of the firing's cap, not under the thread's.
#[tokio::test(flavor = "multi_thread")]
async fn a_firings_cap_holds_across_a_midway_compaction() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    tool_use("t1", "echo ONE"),
    too_long(),
    answer("SUMMARY-WORDS"),
    tool_use("t2", "echo TWO"),
  ];
  responses.extend(spare(6));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let said = match c
    .request(&Request::Schedule {
      input: json!({"action": "create", "name": "capped", "goal": "THE-GOAL-WORDS",
        "every": "@every 1h", "max_steps": 3})
      .to_string(),
      cwd: None,
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  };
  let id = said
    .strip_prefix("Made schedule ")
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{said}"))
    .to_owned();
  c.request(&Request::Schedule {
    input: json!({"action": "run_now", "schedule": id}).to_string(),
    cwd: None,
  })
  .await
  .unwrap();
  let report = nth_report(&home, 0).await;
  assert_eq!(report.fields["outcome"], "stopped", "{:?}", report.fields);
  let thread = refs(&home, &report, "thread").unwrap();
  let first = records(&home, "thread")
    .into_iter()
    .find(|t| compacted_into(&home, t.id.as_str()).as_deref() == Some(thread.as_str()))
    .expect("compacted midway");
  assert_eq!(
    refs(&home, &first, "scheduled-by").as_deref(),
    Some(id.as_str())
  );
  // 1 step and the refused call of 3; the summary; 1 step and the wrap-up.
  assert_eq!(shown_calls(&sent).len(), 5, "{:#?}", shown_calls(&sent));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A summary the money left cannot pay for: of $1.10, a step of $1.00
/// spent, the summary may cost more than the $0.10 left (up to 64k tokens out,
/// for each model it may ask), so the thread is not compacted and nothing
/// more is asked of any model; the report says over budget.
#[tokio::test(flavor = "multi_thread")]
async fn a_midway_compaction_the_money_left_cannot_pay_for_is_not_made() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![tool_use_in("t1", "echo ONE", 250_000), too_long()];
  responses.push(answer("SUMMARY-WORDS"));
  responses.extend(spare(4));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(1.1)))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = nth_report(&home, 0).await;
  assert_eq!(
    report.fields["outcome"], "over-budget",
    "{:?}",
    report.fields
  );
  assert_eq!(compacted_into(&home, &child), None);
  assert_eq!(shown_calls(&sent).len(), 2, "{:#?}", shown_calls(&sent));
  let body = RecordRoot::open(&home)
    .unwrap()
    .read_record(&report.id)
    .unwrap()
    .body;
  assert!(body.contains("summary"), "{body}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A summary the money left pays for is taken off it: of $2.95, a step of
/// $1.00 and a summary of $1.00 spent, the continuation has $0.95 and
/// stops over budget after one step of $1.00 (without the summary taken
/// off, $1.95 would take it a second). The $1.95 left after the first
/// step covers the summary's bound (each model may write a tenth of its
/// 1M window, at most 64k: about $1.92 for both).
#[tokio::test(flavor = "multi_thread")]
async fn a_midway_compactions_summary_is_taken_off_the_money_left() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    tool_use_in("t1", "echo ONE", 250_000),
    too_long(),
    answer_in("SUMMARY-WORDS", 250_000),
    tool_use_in("t2", "echo TWO", 250_000),
  ];
  responses.extend((0..4).map(|i| tool_use_in(&format!("x{i}"), "echo SPARE", 250_000)));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(2.95)))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = nth_report(&home, 0).await;
  let new = compacted_into(&home, &child).expect("compacted");
  assert_eq!(
    refs(&home, &report, "thread").as_deref(),
    Some(new.as_str())
  );
  assert_eq!(
    report.fields["outcome"], "over-budget",
    "{:?}",
    report.fields
  );
  assert_eq!(shown_calls(&sent).len(), 4, "{:#?}", shown_calls(&sent));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// What came after a turn's last written step, before a call refused as
/// too long midway, is written still: a message woven in meanwhile is not
/// lost to the compaction, the new thread has it (its sender was told it
/// went to the model).
#[tokio::test(flavor = "multi_thread")]
async fn a_message_woven_in_before_a_midway_compaction_is_kept() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("t1", "sleep 1"),
    too_long(),
    answer("SUMMARY-WORDS"),
    answer("Finished after compaction."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach("THE-GOAL-WORDS", None, None))
    .await
    .unwrap()
  else {
    panic!()
  };
  // Woven in while its first step's call runs (asking first: this node
  // asks for every command but a few).
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  c.request(&Request::Send {
    thread: child.clone(),
    text: "WOVEN-WORDS".into(),
    from: None,
    keep: false,
    interject: true,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  answer_when_known(&mut c, q.id.as_str()).await;
  let report = nth_report(&home, 0).await;
  assert_eq!(report.fields["outcome"], "done", "{:?}", report.fields);
  compacted_into(&home, &child).expect("compacted");
  let calls = shown_calls(&sent);
  assert_eq!(calls.len(), 4, "{calls:#?}");
  assert!(calls[1].contains("WOVEN-WORDS"), "woven in: {}", calls[1]);
  assert!(
    calls[2].contains("WOVEN-WORDS") || calls[3].contains("WOVEN-WORDS"),
    "kept across the compaction: {calls:#?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A step that said nothing, then told to reply (`NO_REPLY`, design §2.30:
/// one more model call), counts against the turn's steps when that call
/// is refused as too long: of 4 steps, 3 taken (a call, the empty step,
/// the refused one), the continuation has 1 and its wrap-up, not 2.
#[tokio::test(flavor = "multi_thread")]
async fn a_step_told_to_reply_counts_across_a_midway_compaction() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    tool_use("t1", "echo ONE"),
    answer(""),
    too_long(),
    answer("SUMMARY-WORDS"),
  ];
  responses.extend(spare(6));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", Some(4), None))
    .await
    .unwrap()
  else {
    panic!()
  };
  let report = nth_report(&home, 0).await;
  compacted_into(&home, &child).expect("compacted");
  assert_eq!(report.fields["outcome"], "stopped", "{:?}", report.fields);
  let calls = shown_calls(&sent);
  assert!(calls[2].contains("without a reply"), "told: {}", calls[2]);
  // 3 calls, the summary, then 1 step and the wrap-up.
  assert_eq!(calls.len(), 3 + 1 + 2, "{calls:#?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A thread's total budget holds across its compactions (issue
/// 2026-11-19T193029Z-beast-ijcq, decided with the user 2026-11-20): of
/// $2.50, three turns of $1 each in three threads of one chain (compacted
/// as each turn starts, past its share of a 300k window) spend it, and the
/// fourth turn neither runs nor compacts. Counted per thread, each
/// compaction gave the budget back whole.
#[tokio::test(flavor = "multi_thread")]
async fn a_budget_spent_across_compactions_stops_the_chain_at_its_total() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    answer_in("ONE", 250_000),
    answer("SUMMARY-ONE"),
    answer_in("TWO", 250_000),
    answer("SUMMARY-TWO"),
    answer_in("THREE", 250_000),
  ];
  responses.extend((0..4).map(|i| answer_in(&format!("SPARE-{i}"), 250_000)));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(300_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(2.5)))
    .await
    .unwrap()
  else {
    panic!()
  };
  nth_report(&home, 0).await;
  for n in 1..4 {
    c.request(&Request::Send {
      thread: child.clone(),
      text: format!("MESSAGE-{n}"),
      from: None,
      keep: false,
      interject: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap();
    nth_report(&home, n).await;
  }
  let chain = {
    let root = RecordRoot::open(&home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    index.compaction_lineage(&child.parse().unwrap())
  };
  assert_eq!(chain.len(), 3, "compacted twice: {chain:?}");
  let last = nth_report(&home, 3).await;
  assert_eq!(last.fields["outcome"], "over-budget", "{:?}", last.fields);
  // 3 turns and 2 summaries; nothing for the fourth.
  assert_eq!(shown_calls(&sent).len(), 5, "{:#?}", shown_calls(&sent));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A turn compacted midway (a step of $1, the summary $1, then a step of
/// $1), then a turn sent: how much the chain has left of `budget` for it.
async fn midway_then_sent(budget: f64) -> (usize, Value) {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    tool_use_in("t1", "echo ONE", 250_000),
    too_long(),
    answer_in("SUMMARY-WORDS", 250_000),
    answer_in("DONE", 250_000),
    answer("SECOND"),
  ];
  responses.extend((0..3).map(|i| answer(&format!("SPARE-{i}"))));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(budget)))
    .await
    .unwrap()
  else {
    panic!()
  };
  nth_report(&home, 0).await;
  compacted_into(&home, &child).expect("compacted midway");
  c.request(&Request::Send {
    thread: child.clone(),
    text: "SECOND-MESSAGE".into(),
    from: None,
    keep: false,
    interject: false,
    scopes: Vec::new(),
  })
  .await
  .unwrap();
  let second = nth_report(&home, 1).await;
  let calls = shown_calls(&sent).len();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  (calls, second.fields["outcome"].clone())
}

/// Spending across a chain is counted exactly once, a midway summary's
/// included (written on the thread compacted): the chain spent $3, so
/// with $3.50 a turn after it runs (counted twice, nothing would be left),
/// and with $2.50 none does (the summary not counted, $0.50 would be).
#[tokio::test(flavor = "multi_thread")]
async fn a_chains_spending_is_counted_once_its_midway_summary_included() {
  let (calls, outcome) = midway_then_sent(3.5).await;
  assert_eq!(outcome, "done");
  assert_eq!(calls, 5, "the second turn ran");
  // The least that leaves the summary's bound (about $1.92) after the
  // first step, and nothing after the three calls.
  let (calls, outcome) = midway_then_sent(3.0).await;
  assert_eq!(outcome, "over-budget");
  assert_eq!(calls, 4, "no second turn");
}

/// A schedule's daily cap holds across its threads' compactions (issue
/// 2026-11-19T193029Z-beast-ijcq): a firing compacted midway spent $3 over
/// its chain (a step, the summary, a step), past the $3 cap, so the
/// next firing today does not run. Read from the newest thread only (and
/// the summary nowhere), $1 was spent and it ran.
#[tokio::test(flavor = "multi_thread")]
async fn a_schedules_daily_cap_holds_across_a_midway_compaction() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    tool_use_in("t1", "echo ONE", 250_000),
    too_long(),
    answer_in("SUMMARY-WORDS", 250_000),
    answer_in("DONE", 250_000),
  ];
  responses.extend((0..3).map(|i| answer(&format!("SPARE-{i}"))));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let said = match c
    .request(&Request::Schedule {
      input: json!({"action": "create", "name": "capped", "goal": "THE-GOAL-WORDS",
        "every": "@every 1h", "budget_usd": 3.0, "daily_cap_usd": 3.0})
      .to_string(),
      cwd: None,
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  };
  let id = said
    .strip_prefix("Made schedule ")
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{said}"))
    .to_owned();
  let run_now = Request::Schedule {
    input: json!({"action": "run_now", "schedule": id}).to_string(),
    cwd: None,
  };
  c.request(&run_now).await.unwrap();
  nth_report(&home, 0).await;
  assert_eq!(shown_calls(&sent).len(), 4, "{:#?}", shown_calls(&sent));
  let again = c.request(&run_now).await;
  tokio::time::sleep(Duration::from_secs(3)).await;
  assert_eq!(
    shown_calls(&sent).len(),
    4,
    "no firing past the cap: {again:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A turn of $1 past its share of a 300k window, then two turns sent: the
/// first compacted as it starts (its summary $1) and nearly free, the
/// second nearly free too. Their outcomes and the calls made, with
/// `budget` for the chain.
async fn compacted_as_sent(budget: f64) -> (usize, Vec<Value>) {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let mut responses = vec![
    answer_in("ONE", 250_000),
    answer_in("SUMMARY-ONE", 250_000),
    answer("TWO"),
    answer("THREE"),
  ];
  responses.extend((0..3).map(|i| answer(&format!("SPARE-{i}"))));
  let (base, sent) = serve(responses).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(300_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&detach_with("THE-GOAL-WORDS", None, Some(budget)))
    .await
    .unwrap()
  else {
    panic!()
  };
  nth_report(&home, 0).await;
  let mut outcomes = Vec::new();
  for n in 1..3 {
    c.request(&Request::Send {
      thread: child.clone(),
      text: format!("MESSAGE-{n}"),
      from: None,
      keep: false,
      interject: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap();
    outcomes.push(nth_report(&home, n).await.fields["outcome"].clone());
  }
  compacted_into(&home, &child).expect("compacted as the second turn started");
  let calls = shown_calls(&sent).len();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  (calls, outcomes)
}

/// A compaction as a detached turn starts is spent on the chain, exactly
/// once: $2 spent (a turn, the summary), so with $2.50 both turns after it
/// run (counted twice, the second would not), and with $1.90 neither does
/// (not counted, $0.90 would be left for both).
#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_as_a_turn_starts_is_spent_on_the_chain_once() {
  let (calls, outcomes) = compacted_as_sent(2.5).await;
  assert_eq!(outcomes, ["done", "done"]);
  assert_eq!(calls, 4, "both turns ran");
  let (calls, outcomes) = compacted_as_sent(1.9).await;
  assert_eq!(outcomes, ["over-budget", "over-budget"]);
  assert_eq!(calls, 2, "a turn and the summary, nothing after");
}
