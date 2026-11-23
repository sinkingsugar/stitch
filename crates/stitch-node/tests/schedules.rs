//! Schedules on a running node, against a mock model (design §2.3): a
//! schedule fires a detached thread and continues it at the next firing,
//! a check that finds nothing wakes nothing, failing firings pause it, and
//! a thread's wake-up continues it.

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
    shell: Some(stitch_node::config::ShellSettings {
      approval: Some("ask".into()),
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

fn refs(home: &Path, r: &IndexedRecord, rel: &str) -> Option<String> {
  let root = RecordRoot::open(home).unwrap();
  root
    .read_record(&r.id)
    .unwrap()
    .refs
    .first(rel)
    .map(|x| x.id.to_string())
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

/// The ID in "Made schedule <id> …".
fn made(said: &str) -> String {
  said
    .strip_prefix("Made schedule ")
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{said}"))
    .to_owned()
}

fn scheduled_threads(home: &Path, schedule: &str) -> Vec<IndexedRecord> {
  records(home, "thread")
    .into_iter()
    .filter(|t| refs(home, t, "scheduled-by").as_deref() == Some(schedule))
    .collect()
}

fn reports_of(home: &Path, thread: &str) -> Vec<IndexedRecord> {
  records(home, "report")
    .into_iter()
    .filter(|r| refs(home, r, "thread").as_deref() == Some(thread))
    .collect()
}

fn last_user_text(body: &Value) -> String {
  let last = body["messages"].as_array().unwrap().last().unwrap();
  match &last["content"] {
    Value::String(s) => s.clone(),
    Value::Array(parts) => parts
      .iter()
      .filter_map(|p| p["text"].as_str())
      .collect::<Vec<_>>()
      .join("\n"),
    _ => String::new(),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_fire_continue_check_and_pause() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    answer("First run: CI is green."),
    answer("Looked at the red CI."),
    answer("Looked again."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Once, two seconds from now: it starts a detached thread.
  let at = (jiff::Timestamp::now() + jiff::SignedDuration::from_secs(2)).to_string();
  let said = schedule(
    &mut c,
    json!({"action": "create", "name": "ci", "goal": "Look at CI", "at": at}),
  )
  .await;
  let once = made(&said);
  let thread = until("the scheduled thread", || {
    scheduled_threads(&home, &once).into_iter().next()
  })
  .await;
  assert_eq!(thread.fields["detached"], true);
  let report = until("its report", || {
    reports_of(&home, thread.id.as_str()).into_iter().next()
  })
  .await;
  assert_eq!(report.fields["outcome"], "done");
  // Kept for the user: nothing to wake.
  assert_eq!(report.fields["wake"], false);
  let first = last_user_text(&sent.lock().unwrap()[0]);
  assert!(
    first.starts_with(&format!("[Schedule {once} \"ci\"")) && first.contains("Look at CI"),
    "{first}"
  );
  // One-shot: it ended once it fired.
  let listed = until("ended", || {
    let l = futures_lite_block(&data, json!({"action": "list", "all": true}));
    l.contains("[ended]").then_some(l)
  })
  .await;
  assert!(listed.contains(&once), "{listed}");

  // A check that finds nothing wakes nothing.
  let quiet = made(
    &schedule(
      &mut c,
      json!({"action": "create", "name": "quiet", "goal": "Never", "every": "@every 1h", "check": "false"}),
    )
    .await,
  );
  let ran = schedule(&mut c, json!({"action": "run_now", "schedule": quiet})).await;
  assert!(ran.contains("found nothing"), "{ran}");
  assert!(scheduled_threads(&home, &quiet).is_empty());

  // A check that finds something: its output is in the message, and the
  // next firing continues the same thread.
  let red = made(
    &schedule(
      &mut c,
      json!({"action": "create", "name": "red", "goal": "Fix CI", "every": "0 9 * * *", "tz": "Europe/Rome", "check": "\"CI is red\""}),
    )
    .await,
  );
  let ran = schedule(&mut c, json!({"action": "run_now", "schedule": red})).await;
  assert!(ran.contains("woke thread"), "{ran}");
  let red_thread = until("the red thread", || {
    scheduled_threads(&home, &red).into_iter().next()
  })
  .await;
  until("its first report", || {
    (reports_of(&home, red_thread.id.as_str()).len() == 1).then_some(())
  })
  .await;
  let msg = last_user_text(&sent.lock().unwrap()[1]);
  assert!(msg.contains("What its check found:\nCI is red"), "{msg}");
  let ran = schedule(&mut c, json!({"action": "run_now", "schedule": red})).await;
  assert!(ran.contains(red_thread.id.as_str()), "{ran}");
  until("its second report", || {
    (reports_of(&home, red_thread.id.as_str()).len() == 2).then_some(())
  })
  .await;
  assert_eq!(scheduled_threads(&home, &red).len(), 1);
  // The second turn had the first in its history.
  let body = sent.lock().unwrap()[2].clone();
  assert!(body.to_string().contains("Looked at the red CI."));
  let shown = schedule(&mut c, json!({"action": "show", "schedule": "red"})).await;
  assert!(shown.contains("turns (latest last):"), "{shown}");
  assert_eq!(shown.matches("  done  ").count(), 2, "{shown}");

  // A check that does not parse is refused as it is set (`let` is not
  // Otter), not found out at each firing.
  // (An error ends its client's connection: one of its own.)
  let mut other = Client::connect(&data).await.unwrap();
  let refused = other
    .request(&Request::Schedule {
      input: json!({"action": "create", "name": "unparsed", "goal": "x", "every": "@every 1h",
        "check": "let r = Shell.Run(\"stitch --version\")\nr"})
      .to_string(),
      cwd: None,
    })
    .await;
  assert!(!matches!(refused, Ok(Reply::Lines(_))), "{refused:?}");
  let listed = schedule(&mut c, json!({"action": "list", "all": true})).await;
  assert!(!listed.contains("unparsed"), "{listed}");

  // Three failing checks in a row pause it.
  let failing = made(
    &schedule(
      &mut c,
      json!({"action": "create", "name": "failing", "goal": "x", "every": "@every 1h", "check": "\"boom\" | Fail"}),
    )
    .await,
  );
  for _ in 0..3 {
    let ran = schedule(&mut c, json!({"action": "run_now", "schedule": failing})).await;
    assert!(ran.contains("failed"), "{ran}");
  }
  let shown = schedule(&mut c, json!({"action": "show", "schedule": failing})).await;
  assert!(shown.contains("[paused]"), "{shown}");
  // Resumed by the user.
  let said = schedule(&mut c, json!({"action": "resume", "schedule": failing})).await;
  assert!(said.contains("resumed"), "{said}");
  let shown = schedule(&mut c, json!({"action": "show", "schedule": failing})).await;
  assert!(shown.contains("[active]"), "{shown}");
  let next = schedule(&mut c, json!({"action": "next"})).await;
  assert!(
    next.contains("\"red\"") && !next.contains("\"ci\""),
    "{next}"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The threads whose latest memory fact says `remember = false`, with its
/// `recall`.
fn out_of_memory(data: &Path) -> Vec<(String, Option<bool>)> {
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let mut off: Vec<(String, Option<bool>)> = Vec::new();
  for r in index.records(Some("memory")).unwrap() {
    for f in r.fields["facts"].as_array().cloned().unwrap_or_default() {
      let Some(t) = f.get("thread").and_then(Value::as_str) else {
        continue;
      };
      off.retain(|(o, _)| o != t);
      if f.get("remember") == Some(&json!(false)) {
        off.push((t.to_owned(), f.get("recall").and_then(Value::as_bool)));
      }
    }
  }
  off
}

/// A scheduled thread is remembered, but in a place whose settings say
/// `memory = false`, which wins (design §2.9): out of memory, recalling.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_thread_in_a_place_out_of_memory_is_not_remembered() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  std::fs::create_dir_all(app.join(".stitch")).unwrap();
  for args in [
    &["init", "-q"][..],
    &[
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@t",
      "commit",
      "-q",
      "--allow-empty",
      "-m",
      "first",
    ],
  ] {
    let ok = std::process::Command::new("git")
      .arg("-C")
      .arg(&app)
      .args(args)
      .status()
      .unwrap()
      .success();
    assert!(ok, "git {args:?}");
  }
  std::fs::write(app.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  let (base, _sent) = serve(vec![answer("Looked."), answer("Looked globally.")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let quiet = fire_new(&mut c, Some(app.display().to_string()), "quiet").await;
  let kept = fire_new(&mut c, None, "kept").await;
  let off = out_of_memory(&data);
  assert!(
    off.contains(&(quiet.clone(), Some(true))),
    "out of memory, recalling: {off:?}"
  );
  assert!(
    !off.iter().any(|(t, _)| *t == kept),
    "remembered elsewhere: {off:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Makes a schedule where `cwd` is, fires it now; the thread it started.
async fn fire_new(c: &mut Client, cwd: Option<String>, name: &str) -> String {
  let input = json!({"action": "create", "name": name, "goal": "Look at CI", "every": "@every 1h"});
  let said = match c
    .request(&Request::Schedule {
      input: input.to_string(),
      cwd,
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  };
  let id = made(&said);
  let ran = schedule(c, json!({"action": "run_now", "schedule": id})).await;
  ran
    .split("woke thread ")
    .nth(1)
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{ran}"))
    .trim_end_matches(['.', ','])
    .to_owned()
}

/// `list` from a client of its own (the closure of `until` cannot await).
fn futures_lite_block(data: &Path, input: Value) -> String {
  let data = data.to_owned();
  std::thread::spawn(move || {
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap()
      .block_on(async move {
        let mut c = Client::connect(&data).await.unwrap();
        schedule(&mut c, input).await
      })
  })
  .join()
  .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_schedule_asks_the_user() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_use(
      "s1",
      "schedule",
      json!({"action": "create", "name": "nightly", "goal": "Run the benchmarks", "every": "0 3 * * *"}),
    ),
    answer("Scheduled."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&Request::Detach {
      goal: "Set up nightly benchmarks".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: None,
      budget_usd: None,
      budget_tokens: None,
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
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let q = until("the question", || {
    records(&home, "question")
      .into_iter()
      .find(|q| refs(&home, q, "thread").as_deref() == Some(child.as_str()))
  })
  .await;
  assert!(records(&home, "schedule").is_empty());
  let title = q.title.clone().unwrap_or_default();
  assert!(title.contains("schedule \"nightly\""), "{title}");
  c.request(&Request::Answer {
    question: q.id.to_string(),
    allow: true,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  let sch = until("the schedule", || {
    records(&home, "schedule").into_iter().next()
  })
  .await;
  assert_eq!(sch.fields["every"], "0 3 * * *");
  assert!(!sch.by_user);
  // Its reports go to the thread that made it.
  assert_eq!(
    refs(&home, &sch, "reports-to").as_deref(),
    Some(child.as_str())
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_schedule_under_yolo_says_it_did_not_ask() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use(
      "s1",
      "schedule",
      json!({"action": "create", "name": "nightly", "goal": "Run the benchmarks", "every": "0 3 * * *"}),
    ),
    answer("Scheduled."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = stitch_node::config::Config::load(&data).unwrap();
  config.shell.as_mut().unwrap().approval = Some("allow".into());
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(child) = c
    .request(&Request::Detach {
      goal: "Set up nightly benchmarks".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: None,
      budget_usd: None,
      budget_tokens: None,
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
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let sch = until("the schedule", || {
    records(&home, "schedule").into_iter().next()
  })
  .await;
  assert_eq!(sch.fields["every"], "0 3 * * *");
  assert!(!sch.by_user);
  // Its reports go to the thread that made it.
  assert_eq!(
    refs(&home, &sch, "reports-to").as_deref(),
    Some(child.as_str())
  );
  // No question was asked, and the tool's result said so.
  assert!(records(&home, "question").is_empty());
  let result = until("the tool's result", || {
    let sent = sent.lock().unwrap();
    (sent.len() >= 2).then(|| sent[1].to_string())
  })
  .await;
  assert!(result.contains("The user was not asked"), "{result}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_wakes_itself() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use(
      "w1",
      "wake_at",
      json!({"minutes": 0.03, "note": "See whether the build finished."}),
    ),
    answer("I will look again shortly."),
    answer("Woke up: the build finished."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let chat = Request::Chat {
    thread: None,
    text: "Watch the build".into(),
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
  let Reply::ChatDone { .. } = c.chat(&chat, |_| None).await.unwrap() else {
    panic!()
  };
  // The tool's result said it was set.
  let result = sent.lock().unwrap()[1].to_string();
  assert!(result.contains("Set wake-up"), "{result}");
  until("the wake-up's turn", || {
    (sent.lock().unwrap().len() >= 3).then_some(())
  })
  .await;
  let woke = last_user_text(&sent.lock().unwrap()[2]);
  assert!(
    woke.contains("the wake-up this thread set")
      && woke.contains("See whether the build finished."),
    "{woke}"
  );
  // One-shot: ended once fired.
  let sch = until("ended", || records(&home, "schedule").into_iter().next()).await;
  let listed = until("its end", || {
    let l = futures_lite_block(&data, json!({"action": "list", "all": true}));
    l.contains("[ended]").then_some(l)
  })
  .await;
  assert!(listed.contains(sch.id.as_str()), "{listed}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Wake-ups outlive the node: one whose deadline passed while the node was
/// down (after a poll) fires at once as the deadline, one cut short in the
/// middle of its firing fires again, and one whose check fails continues
/// its thread with the error instead of being polled (and paused) quietly.
#[tokio::test(flavor = "multi_thread")]
async fn wake_ups_survive_a_restart() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let deadline_at = (jiff::Timestamp::now() + jiff::SignedDuration::from_secs(5)).to_string();
  let far = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1)).to_string();
  let (base, sent) = serve(vec![
    tool_use(
      "w1",
      "wake_at",
      json!({"check": "false", "every": "1m", "until": deadline_at, "note": "DEADLINE-NOTE"}),
    ),
    answer("Waiting for the deadline."),
    tool_use(
      "w2",
      "wake_at",
      json!({"check": "\"built\"", "every": "1m", "until": far, "note": "CUT-NOTE"}),
    ),
    answer("Waiting for the build."),
    tool_use(
      "w3",
      "wake_at",
      json!({"check": "\"boom\" | Fail", "every": "1m", "until": far, "note": "FAIL-NOTE"}),
    ),
    answer("Waiting on a broken check."),
    answer("Woke."),
    answer("Woke."),
    answer("Woke."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  for text in ["Wait for the deadline", "Wait for the build", "Wait on it"] {
    let chat = Request::Chat {
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
    };
    let Reply::ChatDone { .. } = c.chat(&chat, |_| None).await.unwrap() else {
      panic!()
    };
  }
  assert_eq!(sent.lock().unwrap().len(), 6);
  assert!(sent.lock().unwrap()[1].to_string().contains("Set wake-up"));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  // While the node is down: the deadline wake-up had polled once (its
  // next poll before its deadline), the build's was in the middle of a
  // firing, and so was the failing one. The deadline passes meanwhile.
  let schedules = records(&home, "schedule");
  assert_eq!(schedules.len(), 3);
  let id = |note: &str| {
    schedules
      .iter()
      .find(|s| s.fields["goal"] == note)
      .unwrap()
      .id
      .to_string()
  };
  let deadline: jiff::Timestamp = deadline_at.parse().unwrap();
  let polled = deadline - jiff::SignedDuration::from_secs(90);
  let begun = jiff::Timestamp::now();
  let state = json!({
    id("DEADLINE-NOTE"): {"last": polled.to_string()},
    id("CUT-NOTE"): {"last": begun.to_string(), "pending": begun.to_string()},
    id("FAIL-NOTE"): {"last": begun.to_string(), "pending": begun.to_string()},
  });
  std::fs::write(
    data.join("schedules.json"),
    serde_json::to_vec_pretty(&state).unwrap(),
  )
  .unwrap();
  let left = deadline.duration_since(jiff::Timestamp::now());
  if left.is_positive() {
    tokio::time::sleep(left.unsigned_abs() + Duration::from_secs(1)).await;
  }

  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  until("the three wake-ups' turns", || {
    (sent.lock().unwrap().len() >= 9).then_some(())
  })
  .await;
  let woke: Vec<String> = sent.lock().unwrap()[6..]
    .iter()
    .map(last_user_text)
    .collect();
  let find = |note: &str| {
    woke
      .iter()
      .find(|w| w.contains(note))
      .unwrap_or_else(|| panic!("{note} did not wake: {woke:?}"))
      .clone()
  };
  let w = find("DEADLINE-NOTE");
  assert!(w.contains("its deadline"), "{w}");
  let w = find("CUT-NOTE");
  assert!(
    w.contains("its check found something") && w.contains("built"),
    "{w}"
  );
  let w = find("FAIL-NOTE");
  assert!(w.contains("its check failed") && w.contains("boom"), "{w}");
  // All three ended once fired, none paused.
  let listed = until("all ended", || {
    let l = futures_lite_block(&data, json!({"action": "list", "all": true}));
    (l.matches("[ended]").count() == 3).then_some(l)
  })
  .await;
  assert!(!listed.contains("[paused]"), "{listed}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn chat_in(cwd: &Path, text: &str) -> Request {
  Request::Chat {
    thread: None,
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

/// A chat's thread.
async fn chat(c: &mut Client, req: &Request) -> String {
  let done = c.chat(req, |_| None).await.unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  thread
}

/// A folder's schedule whose folder became a project: its record stays in
/// home, tied to the folder, and the thread a firing starts is the
/// project's (in its record store), not a global one with no folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_folders_schedule_fires_in_the_project_it_became() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();
  let (base, _sent) = serve(vec![answer("Here."), answer("Folded."), answer("Looked.")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  // A folder with a thread of its own, and a schedule made there.
  let first = chat(&mut c, &chat_in(&plain, "hello")).await;
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let r = index.record(&first.parse().unwrap()).unwrap().unwrap();
  assert_eq!(r.repo.as_str(), "home");
  assert!(r.fields.get("folder").is_some(), "{:?}", r.fields);
  let input =
    json!({"action": "create", "name": "old", "goal": "Look at CI", "every": "@every 1h"});
  let said = match c
    .request(&Request::Schedule {
      input: input.to_string(),
      cwd: Some(plain.display().to_string()),
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  };
  let old = made(&said);
  for args in [
    &["init", "-q"][..],
    &[
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@t",
      "commit",
      "-q",
      "--allow-empty",
      "-m",
      "first of plain",
    ],
  ] {
    let ok = std::process::Command::new("git")
      .arg("-C")
      .arg(&plain)
      .args(args)
      .status()
      .unwrap()
      .success();
    assert!(ok, "git {args:?}");
  }
  // Seen as a checkout now: the folder is folded into its project.
  let new = chat(&mut c, &chat_in(&plain, "and now?")).await;
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let repo = index.record(&new.parse().unwrap()).unwrap().unwrap().repo;
  assert!(repo.as_str().starts_with("p-"), "{repo}");
  let shown = schedule(&mut c, json!({"action": "show", "schedule": old})).await;
  assert!(shown.contains(&format!("place: project:{repo}")), "{shown}");
  let ran = schedule(&mut c, json!({"action": "run_now", "schedule": old})).await;
  let t = ran
    .split("woke thread ")
    .nth(1)
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{ran}"))
    .trim_end_matches(['.', ','])
    .to_owned();
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let r = index.record(&t.parse().unwrap()).unwrap().unwrap();
  assert_eq!(r.repo, repo, "the project's thread, not a global one");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
