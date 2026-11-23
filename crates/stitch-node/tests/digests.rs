//! Digests (design §2.25, step 2) on a running node, against a mock model:
//! the first digest and its update (previous items kept or dropped, the cost
//! summed, anchors checked, an unreadable reply leaving the old one), the
//! row's `digest` and its `fresh`, the day's cap, the per-turn trigger and
//! its ten-minute rule, and a compacted thread starting from the earlier
//! one's digest.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{Config, DigestSettings, ProviderConfig, ThreadSettings};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves `responses` in order, one per connection; records request bodies.
async fn serve(
  responses: Vec<String>,
  threads: Arc<Mutex<Vec<String>>>,
) -> (String, Arc<Mutex<Vec<Value>>>) {
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
      // `@THREAD@` and `@FIRST@` are the newest and the first thread's IDs
      // once they are known.
      // `@HOLD@`: the answer comes 1.5 s late.
      if response.contains("@HOLD@") {
        tokio::time::sleep(Duration::from_millis(1500)).await;
      }
      let response = {
        let known = threads.lock().unwrap();
        response
          .replace("@THREAD@", known.last().map_or("", String::as_str))
          .replace("@FIRST@", known.first().map_or("", String::as_str))
          .replace("@HOLD@", "")
      };
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

fn answer(text: &str) -> String {
  let events = [
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
  ];
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// A node on `anthropic` (the mock), digests on `model`, capped at `cap`;
/// `window` makes it compact threads past that many tokens.
fn setup(data: &Path, home: &Path, anthropic: &str, cap: Option<f64>, window: Option<u64>) {
  setup_on(data, home, anthropic, cap, window, None)
}

fn setup_on(
  data: &Path,
  home: &Path,
  anthropic: &str,
  cap: Option<f64>,
  window: Option<u64>,
  port: Option<u16>,
) {
  let mut c = Config {
    port,
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(ThreadSettings {
      titles: Some(false),
      compact_at: window.map(|_| 80),
      compact_keep: window.map(|_| 2),
      ..Default::default()
    }),
    digests: Some(DigestSettings {
      // Priced, so the cap and the cost are real.
      model: Some("anthropic/claude-opus-5-5".into()),
      daily_cap_usd: cap,
      backfill_pause_secs: Some(0),
      ..Default::default()
    }),
    ..Config::default()
  };
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(anthropic.into()),
      context_window: window,
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

async fn digest(c: &mut Client, thread: &str, update: bool) -> Value {
  match c
    .request(&Request::Digest {
      thread: thread.into(),
      update: Some(update),
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => serde_json::from_str(&l[0]).unwrap(),
    other => panic!("{other:?}"),
  }
}

async fn row(c: &mut Client, thread: &str) -> Value {
  let Reply::Lines(l) = c
    .request(&Request::Threads {
      forest: None,
      status: None,
      since: None,
      limit: None,
      thread: Some(thread.into()),
      place: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let v: Value = serde_json::from_str(&l[0]).unwrap();
  v["threads"][0].clone()
}

fn reply(headline: &str, items: &[(&str, &str, String)]) -> String {
  json!({
    "headline": headline,
    "items": items.iter().map(|(k, t, a)| json!({"kind": k, "text": t, "at": a})).collect::<Vec<_>>(),
  })
  .to_string()
}

fn at(thread: &str, n: u32) -> String {
  format!("{thread}#m{n}")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_digest_is_written_updated_read_in_the_list_and_goes_stale() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let cell = Arc::new(Mutex::new(Vec::<String>::new()));
  let t = "@THREAD@";
  let (base, sent) = serve(
    vec![
      answer("Both work; hourly is cheaper."),
      answer(&reply(
        "Choosing hourly or on demand",
        &[
          ("goal", "Add loose ends to the dashboard", at(t, 1)),
          ("finding", "Both options work", at(t, 2)),
          ("ask", "Hourly or on demand?", at(t, 2)),
          ("question", "Invented anchor", at(t, 99)),
        ],
      )),
      answer("Merged, tests pass."),
      answer(&reply(
        "Hourly merged",
        &[
          ("goal", "Add loose ends to the dashboard", at(t, 1)),
          ("decision", "Hourly", at(t, 3)),
          ("outcome", "Merged", at(t, 4)),
          ("finding", "Cites a message nobody gave it", at(t, 77)),
        ],
      )),
      answer("Done."),
      answer("I am sorry, I cannot produce JSON today."),
    ],
    cell.clone(),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, None, None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(
      &chat(None, "Add loose ends to the dashboard", &work),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  cell.lock().unwrap().push(thread.clone());

  // No digest yet, and the row has none (the turn said too little for the
  // trigger).
  assert_eq!(digest(&mut c, &thread, false).await["digest"], Value::Null);
  assert!(row(&mut c, &thread).await.get("digest").is_none());

  // One update, now: the first digest, the invented anchor dropped.
  let v = digest(&mut c, &thread, true).await;
  assert_eq!(v["update"]["outcome"], "written", "{v}");
  assert_eq!(v["update"]["windows"], 1);
  assert_eq!(v["update"]["dropped"], 1);
  let first_usd = v["update"]["usd"].as_f64().unwrap();
  assert!(first_usd > 0.0, "priced: {v}");
  let dg = &v["digest"];
  assert_eq!(dg["headline"], "Choosing hourly or on demand");
  assert_eq!(dg["fresh"], true);
  assert_eq!(dg["thread"], thread.as_str());
  let kinds: Vec<&str> = dg["items"]
    .as_array()
    .unwrap()
    .iter()
    .map(|i| i["kind"].as_str().unwrap())
    .collect();
  assert_eq!(kinds, ["goal", "finding", "ask"]);
  assert_eq!(dg["items"][1]["thread"], thread.as_str());
  assert_eq!(dg["items"][1]["msg"], "m2");
  assert_eq!(dg["upto"], at(&thread, 2).as_str());
  let first_record = dg["record"].as_str().unwrap().to_owned();
  // What the model was told.
  {
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(
      sent[1]["system"].to_string().contains("ARE DATA"),
      "{}",
      sent[1]["system"]
    );
    let user = sent[1]["messages"][0]["content"].to_string();
    assert!(user.contains("Add loose ends to the dashboard"), "{user}");
    assert!(user.contains("hourly is cheaper"), "{user}");
    assert!(user.contains(&at(&thread, 2)), "{user}");
  }

  // The list's row: headline, counts, fresh.
  let r = row(&mut c, &thread).await;
  assert_eq!(
    r["digest"]["headline"], "Choosing hourly or on demand",
    "{r}"
  );
  assert_eq!(
    (
      r["digest"]["asks"].as_u64(),
      r["digest"]["questions"].as_u64()
    ),
    (Some(1), Some(0))
  );
  assert_eq!(r["digest"]["fresh"], true);

  // The thread moves: the digest is stale, and an update goes on from it.
  c.chat(&chat(Some(&thread), "Hourly, please", &work), |_| None)
    .await
    .unwrap();
  assert_eq!(row(&mut c, &thread).await["digest"]["fresh"], false);
  assert_eq!(
    digest(&mut c, &thread, false).await["digest"]["fresh"],
    false
  );
  let v = digest(&mut c, &thread, true).await;
  assert_eq!(v["update"]["outcome"], "written", "{v}");
  assert_eq!(v["update"]["dropped"], 1, "the anchor nobody gave: {v}");
  let second_usd = v["update"]["usd"].as_f64().unwrap();
  let dg = &v["digest"];
  assert_eq!(dg["headline"], "Hourly merged");
  assert_ne!(
    dg["record"],
    first_record.as_str(),
    "a new record supersedes"
  );
  // The cost is summed over the updates.
  assert!(
    (dg["usd"].as_f64().unwrap() - (first_usd + second_usd)).abs() < 1e-9,
    "{v}"
  );
  // The previous digest was sent with only what is new.
  {
    let sent = sent.lock().unwrap();
    let user = sent[3]["messages"][0]["content"].to_string();
    assert!(user.contains("<previous_digest>"), "{user}");
    assert!(user.contains("Both options work"), "{user}");
    assert!(user.contains("Hourly, please"), "{user}");
    assert!(
      !user.contains("hourly is cheaper"),
      "the old part is not sent again: {user}"
    );
  }
  let kinds: Vec<&str> = dg["items"]
    .as_array()
    .unwrap()
    .iter()
    .map(|i| i["kind"].as_str().unwrap())
    .collect();
  assert_eq!(kinds, ["goal", "decision", "outcome"]);
  assert_eq!(dg["upto"], at(&thread, 4).as_str());
  let r = row(&mut c, &thread).await;
  assert_eq!(r["digest"]["headline"], "Hourly merged");
  assert_eq!(r["digest"]["fresh"], true);

  // An unreadable reply: the old digest stands, nothing is retried.
  c.chat(&chat(Some(&thread), "Thanks", &work), |_| None)
    .await
    .unwrap();
  let v = digest(&mut c, &thread, true).await;
  assert_eq!(v["update"]["outcome"], "failed", "{v}");
  assert!(
    v["update"]["why"].as_str().unwrap().contains("unreadable"),
    "{v}"
  );
  assert_eq!(v["digest"]["headline"], "Hourly merged");
  assert_eq!(v["digest"]["fresh"], false);
  assert_eq!(sent.lock().unwrap().len(), 6, "asked once, not retried");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_days_cap_stops_updates() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let cell = Arc::new(Mutex::new(Vec::<String>::new()));
  let t = "@THREAD@";
  let (base, sent) = serve(
    vec![
      answer("First."),
      answer(&reply("First digest", &[("goal", "Say hi", at(t, 1))])),
      answer("Second."),
      answer(&reply("Never written", &[])),
    ],
    cell.clone(),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  // A cap one call overshoots.
  setup(&data, &home, &base, Some(0.000001), None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "Say hi", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  cell.lock().unwrap().push(thread.clone());
  let v = digest(&mut c, &thread, true).await;
  assert_eq!(v["update"]["outcome"], "written", "{v}");
  assert!(
    v["update"]["day_usd"].as_f64().unwrap() >= v["update"]["cap_usd"].as_f64().unwrap(),
    "{v}"
  );
  c.chat(&chat(Some(&thread), "Again", &work), |_| None)
    .await
    .unwrap();
  let v = digest(&mut c, &thread, true).await;
  assert_eq!(v["update"]["outcome"], "capped", "{v}");
  assert_eq!(v["digest"]["headline"], "First digest");
  assert_eq!(sent.lock().unwrap().len(), 3, "no model call past the cap");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// 350 words.
fn long() -> String {
  (0..350)
    .map(|i| format!("word{i}"))
    .collect::<Vec<_>>()
    .join(" ")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_said_enough_is_digested_in_the_background_once_in_ten_minutes() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let cell = Arc::new(Mutex::new(Vec::<String>::new()));
  let t = "@THREAD@";
  let (base, sent) = serve(
    vec![
      answer("Short."),
      answer(&long()),
      answer(&reply("Said a lot", &[("finding", "Said a lot", at(t, 4))])),
      answer(&long()),
    ],
    cell.clone(),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, None, None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  // A short turn: no digest, no model call after the turn's own.
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "Hi", &work), |_| None).await.unwrap()
  else {
    panic!()
  };
  cell.lock().unwrap().push(thread.clone());
  tokio::time::sleep(Duration::from_secs(5)).await;
  assert_eq!(sent.lock().unwrap().len(), 1);
  assert_eq!(digest(&mut c, &thread, false).await["digest"], Value::Null);
  // A long one: digested in the background, after the turn.
  c.chat(&chat(Some(&thread), "Tell me everything", &work), |_| None)
    .await
    .unwrap();
  let mut v = Value::Null;
  for _ in 0..40 {
    v = digest(&mut c, &thread, false).await;
    if !v["digest"].is_null() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
  }
  assert_eq!(v["digest"]["headline"], "Said a lot", "{v}");
  assert_eq!(v["digest"]["fresh"], true);
  assert_eq!(sent.lock().unwrap().len(), 3);
  // Another long turn inside the ten minutes: not digested now (the check
  // is deferred to when the gap is over, which a test does not wait for).
  c.chat(&chat(Some(&thread), "And more", &work), |_| None)
    .await
    .unwrap();
  tokio::time::sleep(Duration::from_secs(6)).await;
  assert_eq!(sent.lock().unwrap().len(), 4, "the turn's own call only");
  assert_eq!(
    digest(&mut c, &thread, false).await["digest"]["fresh"],
    false
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_compacted_thread_starts_from_the_earlier_threads_digest() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let cell = Arc::new(Mutex::new(Vec::<String>::new()));
  let (base, sent) = serve(
    vec![
      answer("Hi."),
      // The digest of the first thread.
      answer(&reply(
        "Greeting",
        &[("goal", "Say hello", at("@THREAD@", 1))],
      )),
      // Its last step sent 80 tokens of a 90-token window: compacted first.
      answer("SUMMARY-ONE"),
      answer("Second."),
      // The digest of the second: the goal still anchors in the first.
      answer(&reply(
        "Hello said twice",
        &[
          ("goal", "Say hello", "@FIRST@#m1".into()),
          ("outcome", "Said again", at("@THREAD@", 1)),
        ],
      )),
    ],
    cell.clone(),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, None, Some(90));
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread: first, .. } =
    c.chat(&chat(None, "hello", &work), |_| None).await.unwrap()
  else {
    panic!()
  };
  cell.lock().unwrap().push(first.clone());
  let v = digest(&mut c, &first, true).await;
  assert_eq!(v["update"]["outcome"], "written", "{v}");
  let Reply::ChatDone { thread: second, .. } = c
    .chat(&chat(Some(&first), "again", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(first, second);
  // The conversation's row shows the earlier thread's digest, stale: the
  // new thread moved.
  let r = row(&mut c, &second).await;
  assert_eq!(r["digest"]["headline"], "Greeting", "{r}");
  assert_eq!(r["digest"]["fresh"], false);
  // The first update of the new thread: from the earlier one's digest.
  cell.lock().unwrap().push(second.clone());
  let v = digest(&mut c, &second, true).await;
  assert_eq!(v["update"]["outcome"], "written", "{v}");
  let user = sent.lock().unwrap()[4]["messages"][0]["content"].to_string();
  assert!(
    user.contains("<previous_digest>") && user.contains("Greeting"),
    "{user}"
  );
  assert!(
    user.contains(&format!("{first}#m1")),
    "the anchor into the earlier thread: {user}"
  );
  let _ = (&base, &sent);
  // The conversation's transcript names each message durably, the earlier
  // thread's too (a digest's items open by it).
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: second.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  let mids: Vec<&str> = t["entries"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|e| e["mid"].as_str())
    .collect();
  assert!(mids.contains(&format!("{first}#m1").as_str()), "{mids:?}");
  assert!(
    mids.iter().any(|m| m.starts_with(&format!("{second}#"))),
    "{mids:?}"
  );
  let old = t["entries"]
    .as_array()
    .unwrap()
    .iter()
    .find(|e| e["mid"] == format!("{first}#m1").as_str())
    .unwrap();
  assert!(
    old["id"].is_null(),
    "an earlier thread's message still has no local id"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

async fn status(c: &mut Client, sweep: bool) -> Value {
  match c
    .request(&Request::DigestStatus { sweep: Some(sweep) })
    .await
    .unwrap()
  {
    Reply::Lines(l) => serde_json::from_str(&l[0]).unwrap(),
    other => panic!("{other:?}"),
  }
}

/// A new chat on `text`, the latest of the threads so far.
async fn start(c: &mut Client, text: &str, work: &Path) -> String {
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, text, work), |_| None).await.unwrap()
  else {
    panic!()
  };
  // Their latest steps differ by more than a clock tick.
  tokio::time::sleep(Duration::from_millis(1100)).await;
  thread
}

fn asked(sent: &Arc<Mutex<Vec<Value>>>, n: usize) -> String {
  sent.lock().unwrap()[n]["messages"][0]["content"].to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_digests_the_newest_first_short_threads_once_and_not_again_until_they_grow() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(
    vec![
      answer("Alpha answered."),
      answer("Beta answered."),
      answer("Gamma answered."),
      answer(&reply("Gamma digest", &[])),
      answer(&reply("Beta digest", &[])),
      answer(&reply("Alpha digest", &[])),
      answer("Alpha again."),
    ],
    Arc::new(Mutex::new(Vec::new())),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, None, None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let a = start(&mut c, "alpha topic", &work).await;
  let b = start(&mut c, "beta topic", &work).await;
  let g = start(&mut c, "gamma topic", &work).await;
  // Three conversations, none digested (each too short for the trigger).
  let v = status(&mut c, false).await;
  assert_eq!(
    (
      v["conversations"].as_u64(),
      v["none"].as_u64(),
      v["due_here"].as_u64()
    ),
    (Some(3), Some(3), Some(3)),
    "{v}"
  );
  assert_eq!(v["running"], false);
  assert_eq!(sent.lock().unwrap().len(), 3);

  // The sweep: newest activity first, one call each, all short threads.
  let v = status(&mut c, true).await;
  let swept = &v["sweep"];
  assert_eq!(swept["digested"], json!([g, b, a]), "{v}");
  assert_eq!(swept["due"], 3);
  assert_eq!(sent.lock().unwrap().len(), 6);
  assert!(asked(&sent, 3).contains("gamma topic"));
  assert!(asked(&sent, 4).contains("beta topic"));
  assert!(asked(&sent, 5).contains("alpha topic"));
  assert!(swept["usd"].as_f64().unwrap() > 0.0);
  assert_eq!(
    (
      v["fresh"].as_u64(),
      v["none"].as_u64(),
      v["due_here"].as_u64()
    ),
    (Some(3), Some(0), Some(0)),
    "{v}"
  );
  assert_eq!(v["last_sweep"]["digested"].as_array().unwrap().len(), 3);
  // The list shows them.
  assert_eq!(row(&mut c, &b).await["digest"]["headline"], "Beta digest");

  // Again: nothing due, no model asked.
  let v = status(&mut c, true).await;
  assert_eq!(v["sweep"]["due"], 0, "{v}");
  assert_eq!(sent.lock().unwrap().len(), 6);

  // A short thread that moved is stale but under the floor since its
  // digest: looked at, set aside, no call; and not read again until it
  // moves. (A node started since: a thread digested minutes ago is the
  // trigger's, not the sweep's.)
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(Some(&a), "more on alpha", &work), |_| None)
    .await
    .unwrap();
  assert_eq!(sent.lock().unwrap().len(), 7);
  let v = status(&mut c, false).await;
  assert_eq!(
    (v["stale"].as_u64(), v["due_here"].as_u64()),
    (Some(1), Some(1)),
    "{v}"
  );
  let v = status(&mut c, true).await;
  assert_eq!(
    (
      v["sweep"]["skipped"].as_u64(),
      v["sweep"]["digested"].as_array().unwrap().len()
    ),
    (Some(1), 0),
    "{v}"
  );
  assert_eq!(sent.lock().unwrap().len(), 7, "under the floor: no call");
  assert_eq!(v["due_here"], 0, "set aside at this step: not due again");
  assert_eq!(v["stale"], 1);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cap_stops_the_sweep() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(
    vec![
      answer("One."),
      answer("Two."),
      answer(&reply("Two digest", &[])),
      answer(&reply("One digest", &[])),
    ],
    Arc::new(Mutex::new(Vec::new())),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, Some(0.000001), None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let one = start(&mut c, "first topic", &work).await;
  let two = start(&mut c, "second topic", &work).await;
  let v = status(&mut c, true).await;
  assert_eq!(v["sweep"]["digested"], json!([two]), "{v}");
  assert_eq!(v["sweep"]["capped"], true);
  assert_eq!(sent.lock().unwrap().len(), 3, "the second call never made");
  assert_eq!(
    (v["fresh"].as_u64(), v["none"].as_u64()),
    (Some(1), Some(1))
  );
  // Still capped: nothing more today.
  let v = status(&mut c, true).await;
  assert_eq!(v["sweep"]["capped"], true);
  assert_eq!(sent.lock().unwrap().len(), 3);
  let _ = one;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_being_updated_is_not_taken_by_the_sweep() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(
    vec![
      answer("One."),
      answer("Two."),
      // The update's call, held: the sweep runs meanwhile.
      answer(&format!("@HOLD@{}", reply("One digest", &[]))),
      answer(&reply("Two digest", &[])),
    ],
    Arc::new(Mutex::new(Vec::new())),
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base, None, None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let one = start(&mut c, "first topic", &work).await;
  let two = start(&mut c, "second topic", &work).await;
  let mut c2 = Client::connect(&data).await.unwrap();
  let o2 = one.clone();
  let update = tokio::spawn(async move { digest(&mut c2, &o2, true).await });
  // Its call is out (held).
  for _ in 0..40 {
    if sent.lock().unwrap().len() == 3 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  assert_eq!(sent.lock().unwrap().len(), 3);
  let v = status(&mut c, true).await;
  assert_eq!(
    v["sweep"]["digested"],
    json!([two]),
    "the sweep leaves the one being updated: {v}"
  );
  assert_eq!(v["sweep"]["due"], 1, "{v}");
  let u = update.await.unwrap();
  assert_eq!(u["update"]["outcome"], "written", "{u}");
  assert_eq!(sent.lock().unwrap().len(), 4, "one call each, none twice");
  assert_eq!(row(&mut c, &one).await["digest"]["headline"], "One digest");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn free_port() -> u16 {
  std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

/// A node stopped while digest work is pending (a turn just ended: its
/// settle wait; a long one inside the ten minutes: its deferred check) must
/// let go of its sockets: started again at once on the same port, it binds.
/// (The digest tasks held the node's `Arc<Shared>` through their sleeps, so
/// the endpoint was not freed and the next start failed with "Failed to
/// bind sockets": CI's weave test.)
#[tokio::test(flavor = "multi_thread")]
async fn a_node_stopped_with_digest_work_pending_starts_again_on_its_port() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let cell = Arc::new(Mutex::new(Vec::<String>::new()));
  let (base, sent) = serve(
    vec![
      answer(&long()),
      answer(&reply("Said a lot", &[])),
      answer(&long()),
      answer("Short."),
      answer("Short."),
      answer("Short."),
      answer("Short."),
    ],
    cell,
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup_on(&data, &home, &base, None, None, Some(free_port()));
  // First: a long turn, digested; a second long one inside the ten minutes
  // leaves a deferred check pending (and its settle wait ran first).
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "Tell me", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  for _ in 0..40 {
    if sent.lock().unwrap().len() >= 2 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
  }
  tokio::time::sleep(Duration::from_millis(500)).await;
  c.chat(&chat(Some(&thread), "And more", &work), |_| None)
    .await
    .unwrap();
  tokio::time::sleep(Duration::from_secs(5)).await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  // Then: turns that end just before the stop, again and again.
  for i in 0..4 {
    let node = stitch_node::start(&data)
      .await
      .unwrap_or_else(|e| panic!("start {i}: {e}"));
    let mut c = Client::connect(&data).await.unwrap();
    c.chat(&chat(Some(&thread), "Short", &work), |_| None)
      .await
      .unwrap();
    c.request(&Request::Stop).await.unwrap();
    node.wait().await.unwrap();
  }
}
