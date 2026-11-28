//! A thread's lifecycle on a running node, against a mock model that
//! answers each conversation from its own script: a `wait` cut short,
//! what a finished thread leaves behind (jobs, shells, notes), the steps a
//! turn takes, budgets across turns, and cancelling a thread woken again.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, DetachedSettings, ProviderConfig, ShellSettings};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, IndexedRecord, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A response that never comes.
const HANG: &str = "hang";
/// Put before a response: it comes after 2.5 seconds.
const SLOW: &str = "slow:";

type Sent = Arc<Mutex<Vec<Value>>>;
/// Each conversation's key, and what is left of its script.
type Scripts = Arc<Mutex<Vec<(String, VecDeque<String>)>>>;

/// The text of a request's first message: what tells its conversation.
fn first_text(body: &Value) -> String {
  let content = &body["messages"][0]["content"];
  match content {
    Value::String(s) => s.clone(),
    Value::Array(parts) => parts
      .iter()
      .filter_map(|p| p["text"].as_str())
      .collect::<Vec<_>>()
      .join("\n"),
    _ => String::new(),
  }
}

/// A model: each conversation (its first message starts with a script's
/// key) is answered from its script in order, then with "ok". Records
/// every request.
async fn model(scripts: Vec<(&str, Vec<String>)>) -> (String, Sent) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen: Sent = Arc::default();
  let log = seen.clone();
  let scripts: Scripts = Arc::new(Mutex::new(
    scripts
      .into_iter()
      .map(|(k, v)| (k.to_owned(), v.into()))
      .collect(),
  ));
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let (log, scripts) = (log.clone(), scripts.clone());
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let body: Value = loop {
          let n = sock.read(&mut chunk).await.unwrap();
          if n == 0 {
            return;
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
            break serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap();
          }
        };
        let first = first_text(&body);
        log.lock().unwrap().push(body);
        let response = scripts
          .lock()
          .unwrap()
          .iter_mut()
          .find(|(k, _)| first.starts_with(k.as_str()))
          .and_then(|(_, q)| q.pop_front())
          .unwrap_or_else(|| answer("ok"));
        if response == HANG {
          tokio::time::sleep(Duration::from_secs(120)).await;
          return;
        }
        if let Some(late) = response.strip_prefix(SLOW) {
          tokio::time::sleep(Duration::from_millis(2500)).await;
          let _ = sock.write_all(late.as_bytes()).await;
        } else {
          let _ = sock.write_all(response.as_bytes()).await;
        }
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

/// The requests of the conversation that starts with `key`.
fn requests_of(sent: &Sent, key: &str) -> Vec<Value> {
  sent
    .lock()
    .unwrap()
    .iter()
    .filter(|b| first_text(b).starts_with(key))
    .cloned()
    .collect()
}

/// The text of a request's last message.
fn last_text(body: &Value) -> String {
  let last = body["messages"].as_array().unwrap().last().unwrap().clone();
  first_text(&json!({"messages": [last]}))
}

fn sse(events: Vec<(&str, Value)>) -> String {
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// A step calling one tool: 70 tokens.
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

/// An answer: 85 tokens.
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

fn run(command: &str) -> Value {
  json!({"command": command})
}

fn background(command: &str) -> Value {
  json!({"command": command, "background": true})
}

/// A node's data and home workspace; `yolo` runs every command unasked.
fn setup(data: &Path, home: &Path, anthropic: &str, yolo: bool) {
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
  // Without yolo, the approvals tested here are of commands the sandbox
  // would let run: the allowlist's mode.
  c.shell = Some(ShellSettings {
    approval: Some(if yolo { "allow" } else { "ask" }.into()),
    ..ShellSettings::default()
  });
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

fn with_config(data: &Path, change: impl FnOnce(&mut Config)) {
  let mut c = Config::load(data).unwrap();
  change(&mut c);
  c.save(data).unwrap();
}

/// The reports on `thread`, oldest first, with their bodies.
fn reports(home: &Path, thread: &str) -> Vec<(IndexedRecord, String)> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  let mut out: Vec<(IndexedRecord, String)> = index
    .records(Some("report"))
    .unwrap()
    .into_iter()
    .filter(|r| {
      root
        .read_record(&r.id)
        .unwrap()
        .refs
        .first("thread")
        .is_some_and(|t| t.id.as_str() == thread)
    })
    .map(|r| {
      let body = root.read_record(&r.id).unwrap().body;
      (r, body)
    })
    .collect();
  out.sort_by_key(|r| r.0.created);
  out
}

fn questions(home: &Path, thread: &str) -> usize {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index
    .records(Some("question"))
    .unwrap()
    .into_iter()
    .filter(|q| {
      root
        .read_record(&q.id)
        .unwrap()
        .refs
        .first("thread")
        .is_some_and(|t| t.id.as_str() == thread)
    })
    .count()
}

/// Waits until `ok`.
async fn until(what: &str, mut ok: impl FnMut() -> bool) {
  for _ in 0..300 {
    if ok() {
      return;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

fn detach(goal: &str, tokens: Option<u64>, max_steps: Option<u64>) -> Request {
  Request::Detach {
    goal: goal.into(),
    model: None,
    repo: None,
    cwd: None,
    parent: None,
    budget_usd: None,
    budget_tokens: tokens,
    budget_minutes: None,
    max_steps,
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

fn send(thread: &str, text: &str) -> Request {
  Request::Send {
    thread: thread.into(),
    text: text.into(),
    from: None,
    keep: false,
    interject: false,
    scopes: Vec::new(),
  }
}

async fn detached(c: &mut Client, req: &Request) -> String {
  match c.request(req).await.unwrap() {
    Reply::Detached(id) => id,
    other => panic!("{other:?}"),
  }
}

/// What the node says runs on it, as a client of another version hears.
async fn busy(data: &Path) -> Vec<String> {
  let stream = tokio::net::UnixStream::connect(stitch_node::socket_path(data).unwrap())
    .await
    .unwrap();
  let (mut r, mut w) = stream.into_split();
  stitch_node::api::write(
    &mut w,
    &Request::Hello {
      version: "0.0.1/local-1".into(),
    },
  )
  .await
  .unwrap();
  match stitch_node::api::read::<Reply, _>(&mut r).await.unwrap() {
    Some(Reply::Hello { busy, .. }) => busy,
    other => panic!("{other:?}"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_cut_short_leaves_reports_to_wake_the_thread() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "PARENT",
      vec![
        tool_call("p1", "spawn", json!({"goal": "CHILD: answer late"})),
        tool_call("p2", "wait", json!({})),
        answer("Woken by the report."),
      ],
    ),
    (
      "CHILD",
      vec![format!("{SLOW}{}", answer("The child's late answer."))],
    ),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base, false);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Esc while it waits: the wait is dropped mid-call.
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let chat = Request::Chat {
    thread: None,
    text: "PARENT: spawn and wait".into(),
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
  let done = c
    .chat_with(&chat, Some(outgoing), |r| {
      if let Reply::ToolCall(call) = r
        && call.starts_with("wait")
      {
        let to_chat = to_chat.clone();
        tokio::spawn(async move {
          tokio::time::sleep(Duration::from_millis(300)).await;
          to_chat.send(Request::Interrupt).unwrap();
        });
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "interrupted");

  // The child's report comes after: the thread is no longer waiting, so it
  // is woken with it.
  until("the parent woken with the report", || {
    requests_of(&sent, "PARENT").len() >= 3
  })
  .await;
  let wake = last_text(&requests_of(&sent, "PARENT")[2]);
  assert!(wake.contains("The child's late answer."), "{wake}");
  // Its header: the report's ID first (how it is known as delivered), then
  // the thread it is from, by ID and title, to send to.
  let home = d.join("home");
  let root = RecordRoot::open(&home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  let child = index
    .records(Some("thread"))
    .unwrap()
    .into_iter()
    .find(|t| t.title.as_deref() == Some("CHILD: answer late"))
    .unwrap();
  let (report, _) = reports(&home, child.id.as_str()).remove(0);
  let header = wake.lines().next().unwrap();
  assert_eq!(
    header,
    format!(
      "[Report {} from mbp: thread {} \"CHILD: answer late\", started by this thread]",
      report.id, child.id
    )
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_thread_ends_its_jobs_and_shells_and_stays_finished() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "DONE-LEAVES",
    vec![
      tool_call("a1", "shell_run", background("sleep 300")),
      tool_call("a2", "shell_run", background("sleep 2.5; echo BG-DONE")),
      answer("Done; things left running."),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let t = detached(&mut c, &detach("DONE-LEAVES: start things", None, None)).await;
  until("its report", || !reports(&home, &t).is_empty()).await;
  // The job on `sleep 2.5` (it outlives the loop) would have fired by
  // now, and woken it.
  tokio::time::sleep(Duration::from_secs(4)).await;
  assert_eq!(
    requests_of(&sent, "DONE-LEAVES").len(),
    3,
    "no turn after its report: {:?}",
    requests_of(&sent, "DONE-LEAVES")
      .iter()
      .map(last_text)
      .collect::<Vec<_>>()
  );
  assert_eq!(reports(&home, &t).len(), 1);
  // Nothing of it runs: its sessions closed with what ran in them.
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  let left: Vec<_> = jobs
    .iter()
    .filter(|j| j.thread == t && j.kind != "thread")
    .collect();
  assert!(left.is_empty(), "{left:?}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_takes_its_steps_and_notes_left_at_the_limit_wake_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "TWO-STEPS",
      vec![
        // A job past its first second, and it fires while the next call
        // runs: its note waits for a step that never comes.
        tool_call("b1", "shell_run", background("sleep 1.3; echo NOTE")),
        tool_call("b2", "shell_run", run("sleep 2")),
        // Told it is at its limit: one last step, without tools.
        answer("WRAPPED: where it stands"),
      ],
    ),
    (
      "CONFIGURED",
      vec![
        tool_call("c1", "shell_run", run("true")),
        tool_call("c2", "shell_run", run("true")),
        tool_call("c3", "shell_run", run("true")),
        // Told to wrap up, it calls a tool anyway: the turn ends there.
        tool_call("c4", "shell_run", run("true")),
        answer("never asked"),
      ],
    ),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  with_config(&data, |c| {
    c.detached = Some(DetachedSettings {
      max_steps: Some(3),
      ..DetachedSettings::default()
    })
  });
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Its own limit (per spawn) over the node's.
  let t = detached(&mut c, &detach("TWO-STEPS: work", None, Some(2))).await;
  until("its report", || !reports(&home, &t).is_empty()).await;
  let (report, body) = &reports(&home, &t)[0];
  assert_eq!(report.fields["outcome"], "stopped");
  assert!(body.contains("Stopped after its most steps"), "{body}");
  assert!(body.contains("WRAPPED: where it stands"), "{body}");
  tokio::time::sleep(Duration::from_secs(2)).await;
  let asked = requests_of(&sent, "TWO-STEPS");
  assert!(
    last_text(&asked[2]).contains("reached its step limit (2 model calls)"),
    "the wrap-up is the last thing it reads: {}",
    last_text(&asked[2])
  );
  assert_eq!(
    requests_of(&sent, "TWO-STEPS").len(),
    3,
    "the note left in its queue did not start it again: {:?}",
    requests_of(&sent, "TWO-STEPS")
      .iter()
      .map(last_text)
      .collect::<Vec<_>>()
  );
  assert_eq!(reports(&home, &t).len(), 1);

  // The node's limit when the spawn sets none.
  let t = detached(&mut c, &detach("CONFIGURED: work", None, None)).await;
  until("its report", || !reports(&home, &t).is_empty()).await;
  assert_eq!(reports(&home, &t)[0].0.fields["outcome"], "stopped");
  assert_eq!(requests_of(&sent, "CONFIGURED").len(), 4);

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_gets_what_is_left_of_the_budget() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "BUDGET",
    vec![
      // 85 tokens of 100.
      answer("First turn."),
      // 70 more: past what was left.
      tool_call("d1", "shell_run", run("true")),
      answer("never asked"),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let t = detached(&mut c, &detach("BUDGET: spend", Some(100), None)).await;
  until("its report", || !reports(&home, &t).is_empty()).await;
  assert_eq!(reports(&home, &t)[0].0.fields["outcome"], "done");

  c.request(&send(&t, "More, please.")).await.unwrap();
  until("the second report", || reports(&home, &t).len() == 2).await;
  assert_eq!(reports(&home, &t)[1].0.fields["outcome"], "over-budget");
  assert_eq!(requests_of(&sent, "BUDGET").len(), 2, "one step, then over");

  // Nothing left: it does not run, and says so.
  c.request(&send(&t, "And more.")).await.unwrap();
  until("the third report", || reports(&home, &t).len() == 3).await;
  let (report, body) = &reports(&home, &t)[2];
  assert_eq!(report.fields["outcome"], "over-budget");
  assert!(
    body.contains("Not run") && body.contains("And more."),
    "{body}"
  );
  assert_eq!(requests_of(&sent, "BUDGET").len(), 2);

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_woken_again_is_busy_and_can_be_cancelled() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "ASK-THEN",
    vec![
      // Waits for an approval, and is cancelled meanwhile.
      tool_call("e1", "shell_run", run("touch /nonexistent/never")),
      // Sent to after: it works (a model slow to answer), and is
      // cancelled again.
      HANG.into(),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, false);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let t = detached(&mut c, &detach("ASK-THEN: try", None, None)).await;
  until("its question", || questions(&home, &t) == 1).await;
  c.request(&Request::Cancel { thread: t.clone() })
    .await
    .unwrap();
  until("the cancel report", || reports(&home, &t).len() == 1).await;

  // A message runs it (what is sent always does), and that loop is the
  // node's work, not an approval it waits on.
  c.request(&send(&t, "ASK-THEN: once more")).await.unwrap();
  until("the second turn", || {
    requests_of(&sent, "ASK-THEN").len() == 2
  })
  .await;
  let said = busy(&data).await;
  assert!(
    said.iter().any(|b| b.contains(&t) && b.contains("working")),
    "{said:?}"
  );
  // And it is cancelled like any running loop.
  c.request(&Request::Cancel { thread: t.clone() })
    .await
    .unwrap();
  until("the second cancel report", || reports(&home, &t).len() == 2).await;
  assert_eq!(reports(&home, &t)[1].0.fields["outcome"], "cancelled");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_background_start_stops_its_own_command() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = model(vec![(
    "BG-CUT",
    vec![tool_call(
      "g1",
      "shell_run",
      background("sleep 30; echo never"),
    )],
  )])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Stopped during the second a background command has to fail at once,
  // in a session of its own (`bg…`, not `main`).
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let chat = Request::Chat {
    thread: None,
    text: "BG-CUT: start it".into(),
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
  let done = c
    .chat_with(&chat, Some(outgoing), |r| {
      if let Reply::ToolCall(_) = r {
        let to_chat = to_chat.clone();
        tokio::spawn(async move {
          tokio::time::sleep(Duration::from_millis(300)).await;
          to_chat.send(Request::Interrupt).unwrap();
        });
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, thread, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "interrupted");
  // Its command got Ctrl+C: nothing of it runs on.
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  let left: Vec<_> = jobs
    .iter()
    .filter(|j| j.thread == thread && j.kind == "shell")
    .collect();
  assert!(left.is_empty(), "{left:?}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_says_when_the_work_was_not_done() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = model(vec![
    (
      "ALL-FAIL",
      vec![
        tool_call("f1", "shell_view", json!({"session": "nowhere"})),
        tool_call(
          "f2",
          "shell_send",
          json!({"session": "nowhere", "text": "x"}),
        ),
        answer("Here is what I found."),
      ],
    ),
    (
      "SAYS-BLOCKED",
      vec![answer("Blocked: the key it needs is not there.")],
    ),
    (
      "SOME-FAIL",
      vec![
        tool_call("s1", "shell_view", json!({"session": "nowhere"})),
        tool_call("s2", "shell_run", run("echo fine")),
        answer("All good."),
      ],
    ),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let failed = detached(&mut c, &detach("ALL-FAIL: try", None, None)).await;
  let blocked = detached(&mut c, &detach("SAYS-BLOCKED: try", None, None)).await;
  let fine = detached(&mut c, &detach("SOME-FAIL: try", None, None)).await;
  for t in [&failed, &blocked, &fine] {
    until("its report", || !reports(&home, t).is_empty()).await;
  }
  // Every call it made failed: the work never ran, whatever it said.
  let (r, body) = reports(&home, &failed).remove(0);
  assert_eq!(r.fields["outcome"], "not-done");
  assert!(
    body.contains("Every tool call it made failed (2 of 2)"),
    "{body}"
  );
  // Its own words.
  assert_eq!(reports(&home, &blocked)[0].0.fields["outcome"], "blocked");
  // A failed call among others that worked: done.
  assert_eq!(reports(&home, &fine)[0].0.fields["outcome"], "done");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_with_nothing_new_sends_no_report() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "TOP",
      vec![
        tool_call(
          "t1",
          "spawn",
          json!({"goal": "MIDDLE: spawn a child and report"}),
        ),
        answer("Spawned the middle one."),
        answer("Noted the middle one's report."),
      ],
    ),
    (
      "MIDDLE",
      vec![
        tool_call("m1", "spawn", json!({"goal": "LEAF: answer late"})),
        answer("The middle one's report."),
        answer("[nothing new]"),
      ],
    ),
    (
      "LEAF",
      vec![format!("{SLOW}{}", answer("The leaf's late answer."))],
    ),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let chat = Request::Chat {
    thread: None,
    text: "TOP: spawn".into(),
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
  c.chat_with(&chat, None, |_| None).await.unwrap();

  // The middle one's report wakes the top thread once.
  until("the top thread woken", || {
    requests_of(&sent, "TOP").len() >= 3
  })
  .await;
  let wake = last_text(&requests_of(&sent, "TOP")[2]);
  assert!(wake.contains("The middle one's report."), "{wake}");
  assert!(wake.contains(", started by this thread]"), "{wake}");
  // The leaf's report wakes the middle one, which is told it may have
  // nothing new to say, and says so.
  until("the middle one woken", || {
    requests_of(&sent, "MIDDLE").len() >= 3
  })
  .await;
  let woken = last_text(&requests_of(&sent, "MIDDLE")[2]);
  assert!(woken.contains("The leaf's late answer."), "{woken}");
  assert!(woken.contains("[nothing new]"), "{woken}");
  let root = RecordRoot::open(&home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  let middle = index
    .records(Some("thread"))
    .unwrap()
    .into_iter()
    .find(|t| t.title.as_deref() == Some("MIDDLE: spawn a child and report"))
    .unwrap();
  until("its second report", || {
    reports(&home, middle.id.as_str()).len() == 2
  })
  .await;
  let (quiet, _) = reports(&home, middle.id.as_str()).remove(1);
  assert_eq!(quiet.fields["news"], false);
  assert_eq!(quiet.fields["outcome"], "done");
  // Written, not delivered: the top thread is not woken again.
  tokio::time::sleep(Duration::from_secs(2)).await;
  assert_eq!(requests_of(&sent, "TOP").len(), 3);

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopping_node_starts_no_loop_for_notes_left_behind() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "STOPPING",
    vec![
      tool_call("s1", "shell_run", background("sleep 1.5; echo BG-DONE")),
      HANG.into(),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  detached(&mut c, &detach("STOPPING: start something", None, None)).await;
  // Its second step hangs; the job's note waits in its loop meanwhile.
  until("its second step", || {
    requests_of(&sent, "STOPPING").len() == 2
  })
  .await;
  tokio::time::sleep(Duration::from_secs(3)).await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  // The loop aborted leaves the note behind: it wakes nothing while the
  // node stops (the thread resumes at the next start).
  tokio::time::sleep(Duration::from_secs(2)).await;
  assert_eq!(requests_of(&sent, "STOPPING").len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_command_s_own_session_closes_when_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "BG-OWN",
    vec![
      tool_call("b1", "shell_run", background("sleep 1.5; echo BG-OUT")),
      answer("Started."),
      tool_call("b2", "shell_view", json!({"session": "bg1"})),
      answer("Seen."),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let chat = Request::Chat {
    thread: None,
    text: "BG-OWN: run it".into(),
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
  c.chat_with(&chat, None, |_| None).await.unwrap();
  // Its end wakes the thread, saying the session closed with it.
  until("woken by the job", || {
    requests_of(&sent, "BG-OWN").len() >= 4
  })
  .await;
  let reqs = requests_of(&sent, "BG-OWN");
  let note = last_text(&reqs[2]);
  assert!(
    note.contains("BG-OUT") && note.contains("Session \"bg1\" is closed with it"),
    "{note}"
  );
  // And it is gone.
  let view = reqs[3]["messages"].to_string();
  assert!(view.contains("no session bg1 here"), "{view}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn chat_request(text: &str) -> Request {
  Request::Chat {
    thread: None,
    text: text.into(),
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
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_for_an_idle_thread_wake_it_once_together() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "GATHER",
      vec![
        tool_call("g1", "spawn", json!({"goal": "ONE: answer"})),
        tool_call("g2", "spawn", json!({"goal": "TWO: answer"})),
        tool_call("g3", "spawn", json!({"goal": "THREE: answer"})),
        answer("Three of them are on it."),
        answer("All three are in."),
      ],
    ),
    ("ONE", vec![format!("{SLOW}{}", answer("One says 1."))]),
    ("TWO", vec![format!("{SLOW}{}", answer("Two says 2."))]),
    ("THREE", vec![format!("{SLOW}{}", answer("Three says 3."))]),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  with_config(&data, |c| {
    c.detached = Some(DetachedSettings {
      gather_secs: Some(2),
      ..DetachedSettings::default()
    });
  });
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat_with(&chat_request("GATHER: spawn three"), None, |_| None)
    .await
    .unwrap();
  // The three reports come one by one while it is idle: one turn, with
  // all of them.
  until("woken", || requests_of(&sent, "GATHER").len() >= 5).await;
  tokio::time::sleep(Duration::from_secs(4)).await;
  let reqs = requests_of(&sent, "GATHER");
  assert_eq!(reqs.len(), 5, "woken once");
  let wake = last_text(&reqs[4]);
  for said in ["One says 1.", "Two says 2.", "Three says 3."] {
    assert!(wake.contains(said), "{wake}");
  }

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_kept_for_the_user_wakes_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "KEEPER",
      vec![
        tool_call(
          "k1",
          "spawn",
          json!({"goal": "KEPT: answer", "wake": false}),
        ),
        answer("It runs; its report is yours."),
        tool_call("k2", "wait", json!({})),
        answer("Nothing to wait for."),
      ],
    ),
    ("KEPT", vec![answer("The kept answer.")]),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  with_config(&data, |c| {
    c.detached = Some(DetachedSettings {
      gather_secs: Some(1),
      ..DetachedSettings::default()
    });
  });
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let Reply::ChatDone { thread, .. } = c
    .chat_with(&chat_request("KEEPER: spawn one for me"), None, |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(results[0].contains("kept for the user"), "{results:?}");
  let root = RecordRoot::open(&home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  let kept = index
    .records(Some("thread"))
    .unwrap()
    .into_iter()
    .find(|t| t.title.as_deref() == Some("KEPT: answer"))
    .unwrap();
  until("its report", || {
    !reports(&home, kept.id.as_str()).is_empty()
  })
  .await;
  let (report, _) = reports(&home, kept.id.as_str()).remove(0);
  assert_eq!(report.fields["wake"], false);
  // It wakes nothing.
  tokio::time::sleep(Duration::from_secs(3)).await;
  assert_eq!(requests_of(&sent, "KEEPER").len(), 2);
  // The thread's next turn: `wait` with nothing out does not count it.
  let mut results = Vec::new();
  let chat = Request::Chat {
    thread: Some(thread),
    text: "and now?".into(),
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
  c.chat_with(&chat, None, |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(results[0].starts_with("Nothing to wait for"), "{results:?}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_ends_when_the_user_writes_not_on_a_guessed_time() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![
    (
      "WAITER",
      vec![
        tool_call("w1", "spawn", json!({"goal": "SLOWPOKE: take forever"})),
        tool_call("w2", "wait", json!({})),
        answer("Heard you."),
      ],
    ),
    ("SLOWPOKE", vec![HANG.into()]),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let mut results = Vec::new();
  let began = std::time::Instant::now();
  c.chat_with(
    &chat_request("WAITER: spawn and wait"),
    Some(outgoing),
    |r| {
      match r {
        Reply::ToolCall(call) if call.starts_with("wait") => {
          let to_chat = to_chat.clone();
          tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            to_chat
              .send(Request::Interject {
                id: 1,
                text: "Stop waiting, what's the status?".into(),
                attachments: vec![],
                scopes: vec![],
              })
              .unwrap();
          });
        }
        Reply::ToolResult { output, .. } => results.push(output.clone()),
        _ => {}
      }
      None
    },
  )
  .await
  .unwrap();
  // The wait ended because the user wrote, at once, with no time given.
  assert!(
    began.elapsed() < Duration::from_secs(20),
    "{:?}",
    began.elapsed()
  );
  assert!(results[1].starts_with("Cut short"), "{results:?}");
  let reqs = requests_of(&sent, "WAITER");
  assert!(
    last_text(&reqs[2]).contains("what's the status?"),
    "{}",
    last_text(&reqs[2])
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_is_told_once_after_a_restart_what_its_shells_ran_was_cut() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = model(vec![(
    "CUT-SHELLS",
    vec![
      tool_call(
        "c1",
        "shell_run",
        json!({"command": "sleep 600", "background": true, "session": "pt"}),
      ),
      tool_call(
        "c2",
        "shell_run",
        json!({"command": "sleep 700", "session": "loose", "timeout_secs": 1}),
      ),
      answer("Started."),
      answer("Told."),
    ],
  )])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat_with(&chat_request("CUT-SHELLS: start them"), None, |_| None)
    .await
    .unwrap();
  assert_eq!(requests_of(&sent, "CUT-SHELLS").len(), 3);
  // The node stops with both commands running, and starts again.
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  until("the thread told of what was cut", || {
    requests_of(&sent, "CUT-SHELLS").len() >= 4
  })
  .await;
  let note = last_text(&requests_of(&sent, "CUT-SHELLS")[3]);
  assert!(
    note.contains("job 1 `sleep 600` in session \"pt\"")
      && note.contains("`sleep 700` in session \"loose\"")
      && note.contains("start again what you still need"),
    "{note}"
  );
  // Told once: another restart tells nothing more.
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  tokio::time::sleep(Duration::from_secs(3)).await;
  assert_eq!(requests_of(&sent, "CUT-SHELLS").len(), 4);
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
