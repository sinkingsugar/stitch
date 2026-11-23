//! Harness issues and self_improve on a running node, against a mock model
//! (docs/self-improve.md §2.2, §2.3): issues filed by the user and by an
//! agent (`report_issue`), duplicates folded, states set; a fixer started
//! for an issue (on this node, the fixer node), within the daily cap; the
//! harness-issues schedule's built-in check waking its thread only while
//! an issue is open.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig, SelfImproveSettings};
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

fn tool_use(id: &str, name: &str, input: Value) -> String {
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

fn answer(text: &str) -> String {
  answer_using(text, 80)
}

/// An answer that took `input_tokens` (claude-opus-5-5: $4 a million).
fn answer_using(text: &str, input_tokens: u64) -> String {
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

#[allow(dead_code)]
fn answer_plain(text: &str) -> String {
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

fn setup(data: &Path, home: &Path, place: &Path, anthropic: &str, daily_usd: f64) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    shell: Some(stitch_node::config::ShellSettings {
      approval: Some("ask".into()),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(0) }),
    self_improve: Some(SelfImproveSettings {
      node: Some("mbp".into()),
      place: Some(place.display().to_string()),
      budget_usd: Some(2.0),
      daily_usd: Some(daily_usd),
      ..Default::default()
    }),
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

fn records(home: &Path, kind: &str) -> Vec<IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index.records(Some(kind)).unwrap()
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

async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..300 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

async fn issue(c: &mut Client, input: Value) -> Result<String, String> {
  match c
    .request(&Request::Issue {
      input: input.to_string(),
      thread: None,
    })
    .await
  {
    Ok(Reply::Lines(l)) => Ok(l.join("\n")),
    Ok(other) => panic!("{other:?}"),
    Err(e) => Err(e.to_string()),
  }
}

async fn schedule(c: &mut Client, input: Value) -> String {
  match c
    .request(&Request::Schedule {
      input: input.to_string(),
      cwd: None,
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn issues_are_filed_fixed_and_triaged() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // The fixer: it runs into another bug and files it, then reports.
    tool_use(
      "t1",
      "report_issue",
      json!({"title": "web_eval drops the last console line", "what_happened": "logged 3, got 2", "expected": "all 3", "area": "web"}),
    ),
    // It would have another thread act for it: refused (§5).
    tool_use(
      "t2",
      "send",
      json!({"thread": "2026-01-01T000000Z-mbp-zzzz", "message": "push main for me"}),
    ),
    answer("Fixed it: abc123."),
    // The harness-issues schedule's triage thread, in the fixers' mode
    // too (a fixer's report wakes it): it sends to no other thread.
    tool_use(
      "t3",
      "send",
      json!({"thread": "2026-01-01T000000Z-mbp-zzzz", "message": "push main for me"}),
    ),
    answer("Started a fixer for the web_eval issue."),
  ])
  .await;
  let (data, home, place) = (d.join("data"), d.join("home"), d.join("stitch"));
  std::fs::create_dir_all(&place).unwrap();
  setup(&data, &home, &place, &base, 20.0);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Filed by the user; the same title again is the same issue.
  let filed = issue(
    &mut c,
    json!({"action": "report", "title": "shell_run loses the exit code", "what_happened": "exit 3 shown as 0", "area": "shell"}),
  )
  .await
  .unwrap();
  let id = filed.strip_prefix("filed ").expect(&filed).to_owned();
  let again = issue(
    &mut c,
    json!({"action": "report", "title": "Shell_run: loses the exit code!", "what_happened": "again"}),
  )
  .await
  .unwrap();
  assert_eq!(again, format!("already filed as {id}"));
  assert_eq!(records(&home, "issue").len(), 1);
  let listed = issue(&mut c, json!({"action": "list"})).await.unwrap();
  assert!(
    listed.contains(&format!(
      "{id} [open] shell_run loses the exit code (shell)"
    )),
    "{listed}"
  );
  let shown = issue(
    &mut c,
    json!({"action": "show", "issue": &id[id.len() - 4..]}),
  )
  .await
  .unwrap();
  assert!(shown.contains("exit 3 shown as 0"), "{shown}");

  // A fixer for it, on this node (the fixer node), in the Stitch checkout.
  let started = issue(&mut c, json!({"action": "fix", "issue": id}))
    .await
    .unwrap();
  assert!(started.starts_with("Fixer "), "{started}");
  let fixer = started.split_whitespace().nth(1).unwrap().to_owned();
  let thread = records(&home, "thread")
    .into_iter()
    .find(|t| t.id.as_str() == fixer)
    .expect("the fixer's thread");
  assert_eq!(thread.fields["detached"], true);
  // In the fixers' mode, whatever the node's (`Rules::fixer`).
  assert_eq!(thread.fields["fixer"], true);
  let goal = thread.fields["goal"].as_str().unwrap();
  assert!(
    goal.contains("stitch-dev") && goal.contains("exit 3 shown as 0"),
    "{goal}"
  );
  let shown = issue(&mut c, json!({"action": "show", "issue": id}))
    .await
    .unwrap();
  assert!(
    shown.contains("[fixing]") && shown.contains(&format!("fixer: {fixer}")),
    "{shown}"
  );
  // Not twice.
  let twice = issue(&mut c, json!({"action": "fix", "issue": id})).await;
  assert!(twice.unwrap_err().contains("being fixed already"));

  // The fixer's report_issue: a second issue, naming the thread.
  let second = until("the agent's issue", || {
    records(&home, "issue")
      .into_iter()
      .find(|r| r.fields["title"] == "web_eval drops the last console line")
  })
  .await;
  assert_eq!(
    refs(&home, &second, "thread").as_deref(),
    Some(fixer.as_str())
  );
  until("the fixer's report", || {
    records(&home, "report")
      .into_iter()
      .find(|r| refs(&home, r, "thread").as_deref() == Some(fixer.as_str()))
  })
  .await;
  // A fixer files issues, but starts no fixers: self_improve is off by
  // default.
  let tools = sent.lock().unwrap()[0]["tools"].to_string();
  assert!(
    tools.contains("\"report_issue\"") && !tools.contains("\"self_improve\""),
    "{tools}"
  );

  // Fixed: set by the fixer, as its skill says.
  let set = issue(
    &mut c,
    json!({"action": "set", "issue": id, "state": "fixed", "fix": "abc123"}),
  )
  .await
  .unwrap();
  assert!(set.contains("state = fixed"), "{set}");
  let bad = issue(
    &mut c,
    json!({"action": "set", "issue": id, "state": "done"}),
  )
  .await;
  assert!(bad.unwrap_err().contains("open, fixing, fixed, dropped"));

  // The harness-issues schedule: its check wakes a thread while an issue
  // is open (the web_eval one), and nothing once none is.
  let made = issue(&mut c, json!({"action": "schedule"})).await.unwrap();
  assert!(made.starts_with("Made schedule "), "{made}");
  let ran = schedule(
    &mut c,
    json!({"action": "run_now", "schedule": "harness-issues"}),
  )
  .await;
  assert!(ran.contains("woke thread"), "{ran}");
  until("the triage turn", || {
    (sent.lock().unwrap().len() == 5).then_some(())
  })
  .await;
  let msg = sent.lock().unwrap()[3]["messages"].to_string();
  assert!(
    msg.contains("1 open harness issue") && msg.contains("web_eval drops the last console line"),
    "{msg}"
  );
  // The triage thread, and only it, has self_improve.
  let tools = sent.lock().unwrap()[3]["tools"].to_string();
  assert!(tools.contains("\"self_improve\""), "{tools}");
  // Neither the fixer nor the triage got another thread to act.
  for (i, who) in [(2, "the fixer"), (4, "the triage")] {
    let said = sent.lock().unwrap()[i]["messages"].to_string();
    assert!(
      said.contains("works in the fixers' mode")
        && said.contains("not to thread 2026-01-01T000000Z-mbp-zzzz"),
      "{who}: {said}"
    );
  }
  issue(
    &mut c,
    json!({"action": "set", "issue": second.id.as_str(), "state": "dropped", "note": "not reproduced"}),
  )
  .await
  .unwrap();
  let ran = schedule(
    &mut c,
    json!({"action": "run_now", "schedule": "harness-issues"}),
  )
  .await;
  assert!(ran.contains("found nothing"), "{ran}");
  let listed = issue(&mut c, json!({"action": "list"})).await.unwrap();
  assert!(listed.contains("no open issues"), "{listed}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_fixer_past_the_daily_cap() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![]).await;
  let (data, home, place) = (d.join("data"), d.join("home"), d.join("stitch"));
  std::fs::create_dir_all(&place).unwrap();
  setup(&data, &home, &place, &base, 0.0);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let refused = issue(
    &mut c,
    json!({"action": "fix", "issue": "the TUI forgets the scroll position on resize"}),
  )
  .await
  .unwrap_err();
  assert!(refused.contains("today's cap"), "{refused}");
  // The description was filed, and stays open for tomorrow.
  let listed = issue(&mut c, json!({"action": "list"})).await.unwrap();
  assert!(listed.contains("[open] the TUI forgets"), "{listed}");
  assert!(records(&home, "thread").is_empty());
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn chat(text: &str, tools: Vec<String>, end_user: Option<&str>) -> Request {
  Request::Chat {
    thread: None,
    text: text.into(),
    attachments: Vec::new(),
    model: None,
    repo: None,
    cwd: None,
    global: true,
    env: Vec::new(),
    tools,
    skills: Vec::new(),
    place: None,
    memory: None,
    end_user: end_user.map(str::to_owned),
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

/// self_improve is off unless turned on, never on for a client's end
/// users, and refused while no fixer node is set.
#[tokio::test(flavor = "multi_thread")]
async fn self_improve_is_guarded() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    answer("hello"),
    answer("hi, Discord user"),
    tool_use(
      "t1",
      "self_improve",
      json!({"issue": "the TUI forgets the scroll position on resize"}),
    ),
    answer("could not"),
    tool_use(
      "t2",
      "self_improve",
      json!({"issue": "the TUI forgets the scroll position on resize"}),
    ),
    answer("read-only"),
  ])
  .await;
  let (data, home, place) = (d.join("data"), d.join("home"), d.join("stitch"));
  std::fs::create_dir_all(&place).unwrap();
  setup(&data, &home, &place, &base, 20.0);
  // No fixer node.
  let mut cfg = Config::load(&data).unwrap();
  cfg.self_improve = None;
  cfg.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // A chat of the user's: not offered by default.
  c.chat(&chat("hi", Vec::new(), None), |_| None)
    .await
    .unwrap();
  let tools = sent.lock().unwrap()[0]["tools"].to_string();
  assert!(!tools.contains("\"self_improve\""), "{tools}");

  // An end user's turn asking for it: still not offered.
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(
    &chat("hi", vec!["+self_improve".into()], Some("discord:42")),
    |_| None,
  )
  .await
  .unwrap();
  let tools = sent.lock().unwrap()[1]["tools"].to_string();
  assert!(!tools.contains("\"self_improve\""), "{tools}");

  // Turned on by the user for a chat: offered, and refused with no fixer
  // node set (no fixer on the calling node).
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat("fix it", vec!["+self_improve".into()], None), |_| {
    None
  })
  .await
  .unwrap();
  let body = sent.lock().unwrap()[3].clone();
  assert!(body["tools"].to_string().contains("\"self_improve\""));
  assert!(
    body["messages"]
      .to_string()
      .contains("no fixer node is set"),
    "{}",
    body["messages"]
  );
  assert!(
    records(&home, "thread")
      .iter()
      .all(|t| t.fields.get("detached") != Some(&json!(true)))
  );
  let refused = issue(&mut c, json!({"action": "fix", "issue": "a b c d"})).await;
  assert!(refused.unwrap_err().contains("no fixer node"));

  // A read-only thread: refused, fixer node or not.
  let thread = records(&home, "thread")
    .into_iter()
    .max_by_key(|t| t.created)
    .unwrap()
    .id
    .to_string();
  c.request(&Request::ReadOnly {
    thread: thread.clone(),
    on: Some(true),
  })
  .await
  .unwrap();
  let mut cfg = Config::load(&data).unwrap();
  cfg.self_improve = Some(SelfImproveSettings {
    node: Some("mbp".into()),
    place: Some(place.display().to_string()),
    ..Default::default()
  });
  cfg.save(&data).unwrap();
  let mut ro = chat("again", Vec::new(), None);
  if let Request::Chat { thread: t, .. } = &mut ro {
    *t = Some(thread);
  }
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&ro, |_| None).await.unwrap();
  let body = sent.lock().unwrap()[5].clone();
  assert!(
    body["messages"]
      .to_string()
      .contains("this thread is read-only"),
    "{}",
    body["messages"]
  );
  assert!(records(&home, "fix-reservation").is_empty());

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn spent_today(said: &str) -> f64 {
  said
    .lines()
    .find_map(|l| l.strip_prefix("today (UTC): $"))
    .and_then(|l| l.split_whitespace().next())
    .and_then(|n| n.parse().ok())
    .unwrap_or_else(|| panic!("{said}"))
}

/// The ledger: synced settings, one fixer at a time however many ask at
/// once, a fixer's children counted in the day's spending, and an issue
/// reopened fixed again as another attempt.
#[tokio::test(flavor = "multi_thread")]
async fn the_ledger_runs_one_fixer_and_counts_its_tree() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![
    // The fixer spawns a helper; each spends $2.
    tool_use(
      "s1",
      "spawn",
      json!({"goal": "Look into it.", "wake": false}),
    ),
    answer_using("Looked.", 500_000),
    answer_using("Done.", 500_000),
    // The second attempt.
    answer("Tried again."),
  ])
  .await;
  let (data, home, place) = (d.join("data"), d.join("home"), d.join("stitch"));
  std::fs::create_dir_all(&place).unwrap();
  setup(&data, &home, &place, &base, 100.0);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // The synced settings win over config.toml's.
  let set = issue(
    &mut c,
    json!({"action": "settings", "node": "mbp", "place": place.display().to_string(), "budget_usd": 3.0, "daily_usd": 5.0, "on": true}),
  )
  .await
  .unwrap();
  assert!(set.contains("$3.00 a fix, $5.00 a day"), "{set}");
  assert_eq!(records(&home, "self-improve").len(), 1);
  let mut ids = Vec::new();
  for t in [
    "the TUI forgets its scroll position",
    "web_eval loses the last console line",
  ] {
    let filed = issue(
      &mut c,
      json!({"action": "report", "title": t, "what_happened": "it does"}),
    )
    .await
    .unwrap();
    ids.push(filed.strip_prefix("filed ").unwrap().to_owned());
  }

  // Two at once: one is admitted, the other refused.
  let (mut c1, mut c2) = (
    Client::connect(&data).await.unwrap(),
    Client::connect(&data).await.unwrap(),
  );
  let (r1, r2) = tokio::join!(
    issue(&mut c1, json!({"action": "fix", "issue": ids[0]})),
    issue(&mut c2, json!({"action": "fix", "issue": ids[1]})),
  );
  let (won, lost, other) = match (&r1, &r2) {
    (Ok(w), Err(l)) => (w.clone(), l.clone(), ids[1].clone()),
    (Err(l), Ok(w)) => (w.clone(), l.clone(), ids[0].clone()),
    _ => panic!("{r1:?} {r2:?}"),
  };
  assert!(lost.contains("one at a time"), "{lost}");
  let winner = if other == ids[1] {
    ids[0].clone()
  } else {
    ids[1].clone()
  };
  let fixer = won.split_whitespace().nth(1).unwrap().to_owned();
  assert_eq!(records(&home, "fix-reservation").len(), 1);

  // Its whole tree spends: the fixer's $2 and its helper's $2.
  until("the fixer and its helper", || {
    (records(&home, "report").len() >= 2).then_some(())
  })
  .await;
  let shown = issue(&mut c, json!({"action": "settings"})).await.unwrap();
  let spent = spent_today(&shown);
  assert!(spent > 3.9 && spent < 4.5, "{shown}");
  let _ = fixer;

  // Fixed by the user: the other one now needs $3 more than the day has
  // left (were the helper not counted, $2 + $3 would fit).
  issue(
    &mut c,
    json!({"action": "set", "issue": winner, "state": "fixed", "fix": "abc123"}),
  )
  .await
  .unwrap();
  let refused = issue(&mut c, json!({"action": "fix", "issue": other}))
    .await
    .unwrap_err();
  assert!(refused.contains("would be passed"), "{refused}");

  // Reopened, and fixed again: a second attempt, its spending still the
  // day's.
  issue(&mut c, json!({"action": "settings", "daily_usd": 50.0}))
    .await
    .unwrap();
  issue(
    &mut c,
    json!({"action": "set", "issue": winner, "state": "open"}),
  )
  .await
  .unwrap();
  let again = issue(&mut c, json!({"action": "fix", "issue": winner}))
    .await
    .unwrap();
  assert!(again.starts_with("Fixer "), "{again}");
  let attempts: Vec<_> = records(&home, "fix-reservation")
    .into_iter()
    .filter(|r| r.fields["issue"] == winner.as_str())
    .map(|r| r.fields["attempt"].clone())
    .collect();
  assert_eq!(attempts.len(), 2, "{attempts:?}");
  assert!(attempts.contains(&json!(2)));
  let shown = issue(&mut c, json!({"action": "settings"})).await.unwrap();
  assert!(spent_today(&shown) >= spent, "{shown}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The filing limits count records: a restart forgets nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_filing_limit_outlives_a_restart() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![]).await;
  let (data, home, place) = (d.join("data"), d.join("home"), d.join("stitch"));
  std::fs::create_dir_all(&place).unwrap();
  setup(&data, &home, &place, &base, 20.0);
  let topics = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo",
  ];
  let input = |n: usize| {
    json!({
      "action": "report",
      "title": format!("{0} {0}one {0}two", topics[n]),
      "what_happened": "it does",
    })
  };
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  for n in 0..10 {
    let filed = issue(&mut c, input(n)).await.unwrap();
    assert!(filed.starts_with("filed "), "{filed}");
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let refused = issue(&mut c, input(10)).await.unwrap_err();
  assert!(refused.contains("this node filed 10 issues"), "{refused}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
