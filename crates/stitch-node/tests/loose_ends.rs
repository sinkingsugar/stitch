//! Loose ends (design §2.10): a conversation that stopped is judged once
//! by the quick model, cached by its latest step (the gate asks nothing
//! more until it moves), shown by the dashboard's live call, dismissed
//! until it moves again, and judged again once it did.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::apps::serve::{Request as AppRequest, Response};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, LooseEndsSettings, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
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

/// Gemini's Decisions API: each call answers the `verdict` choice with the
/// next of `answers` (the option, its probability); the inputs kept.
async fn serve_decisions(answers: Vec<(&'static str, f64)>) -> (String, Arc<Mutex<Vec<String>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}/v1beta", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    for (choice, p) in answers {
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 65536];
      let body: Value = loop {
        let n = sock.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
          let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
          assert!(head.starts_with("post /v1beta/decisions"), "{head}");
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
      assert_eq!(body["questions"]["verdict"]["type"], "choice");
      log
        .lock()
        .unwrap()
        .push(body["input"].as_str().unwrap_or_default().to_owned());
      let reply = json!({
        "answers": {"verdict": {"type": "choice", "choice": choice,
          "probabilities": {choice: p}, "confidence": p}},
        "usage": {"total_input_tokens": 400, "total_output_tokens": 0}
      })
      .to_string();
      let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{reply}",
        reply.len()
      );
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

fn free_port() -> u16 {
  std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn setup(data: &Path, home: &Path, anthropic: &str, port: u16) {
  setup_with(data, home, anthropic, None, port)
}

fn setup_with(data: &Path, home: &Path, anthropic: &str, gemini: Option<&str>, port: u16) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(port) }),
    // No timer: the test asks for every pass.
    loose_ends: Some(LooseEndsSettings {
      every_minutes: Some(0),
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
  if let Some(g) = gemini {
    c.providers.insert(
      "gemini".into(),
      ProviderConfig {
        kind: "gemini".into(),
        base_url: Some(g.into()),
        ..ProviderConfig::default()
      },
    );
  }
  c.save(data).unwrap();
  keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  if gemini.is_some() {
    keys::set(data, "gemini", "g-test", Store::File).unwrap();
  }
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

fn request(host: &str, method: &str, path: &str) -> AppRequest {
  AppRequest {
    host: host.into(),
    path: path.into(),
    method: method.into(),
    body: Vec::new(),
  }
}

fn json_of(r: &Response) -> Value {
  serde_json::from_slice(&r.body)
    .unwrap_or_else(|_| panic!("{} {}", r.status, String::from_utf8_lossy(&r.body)))
}

async fn loose(c: &mut Client, check: bool, dismiss: Option<&str>) -> String {
  match c
    .request(&Request::LooseEnds {
      check,
      dismiss: dismiss.map(str::to_owned),
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_waiting_on_the_user_is_judged_once_and_shown() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(vec![
    // The conversation's turn.
    answer("Both work. Should the dashboard check hourly, or only on demand?"),
    // The pass: the quick model's judgement.
    answer("{\"verdict\": \"waiting-on-you\", \"line\": \"Pick hourly or on demand\"}"),
    // The user answers: a turn, then the pass judges the new step.
    answer("Done: hourly it is, merged."),
    answer("{\"verdict\": \"done\", \"line\": \"Hourly checks merged\"}"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
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

  // Not judged yet, and read: the client watched its turn end.
  let listed = loose(&mut c, false, None).await;
  assert!(listed.contains("No loose ends"), "{listed}");

  // On demand: judged by the model, once.
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Judged 1 of 1"), "{said}");
  assert!(said.contains("waiting-on-you"), "{said}");
  assert!(said.contains("Pick hourly or on demand"), "{said}");
  assert_eq!(sent.lock().unwrap().len(), 2);
  let asked = sent.lock().unwrap()[1].to_string();
  assert!(asked.contains("Add loose ends to the dashboard"), "{asked}");
  assert!(asked.contains("only on demand?"), "{asked}");

  // The gate: judged at its step, so no model is asked again.
  let again = loose(&mut c, true, None).await;
  assert!(again.contains("Nothing to judge"), "{again}");
  assert_eq!(sent.lock().unwrap().len(), 2);

  // The dashboard reads it live.
  let Reply::Apps { apps, .. } = c.request(&Request::Dashboard).await.unwrap() else {
    panic!()
  };
  let host = stitch_node::apps::pinned_host(&apps[0].id.parse().unwrap());
  let live = json_of(
    &node
      .serve_app(request(&host, "GET", "/_stitch/live/loose-ends"))
      .await,
  );
  let items = live["items"].as_array().unwrap();
  assert_eq!(items.len(), 1, "{live}");
  assert_eq!(items[0]["thread"], thread.as_str());
  assert_eq!(items[0]["verdict"], "waiting-on-you");
  assert_eq!(items[0]["line"], "Pick hourly or on demand");
  assert_eq!(items[0]["unread"], false);
  assert_eq!(live["checking"], false);
  // The last pass was the gate's alone: nothing due, no model.
  assert_eq!(live["last_pass"]["due"], 0, "{live}");

  // Dismissed from the dashboard: gone until it moves.
  let r = node
    .serve_app(request(
      &host,
      "POST",
      &format!("/_stitch/live/loose-ends?dismiss={thread}"),
    ))
    .await;
  assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
  let live = json_of(
    &node
      .serve_app(request(&host, "GET", "/_stitch/live/loose-ends"))
      .await,
  );
  assert!(live["items"].as_array().unwrap().is_empty(), "{live}");
  // Other live state stays read-only.
  let r = node
    .serve_app(request(&host, "POST", "/_stitch/live/threads"))
    .await;
  assert_eq!(r.status, 405);

  // It moves: judged again, at its new step; done and read, no loose end.
  c.chat(&chat(Some(&thread), "Hourly, please", &work), |_| None)
    .await
    .unwrap();
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Judged 1 of 1"), "{said}");
  assert!(said.contains("No loose ends"), "{said}");
  assert_eq!(sent.lock().unwrap().len(), 4);
  // Opened again: nothing new to keep, and fine to say.
  assert!(matches!(
    c.request(&Request::Opened {
      thread: thread.clone()
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_decision_model_judges_first_and_the_quick_model_when_it_is_unsure() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(vec![
    answer("Both work. Should the dashboard check hourly, or only on demand?"),
    // The user answers; the decision model is unsure, the quick model judges.
    answer("Merged the hourly check."),
    answer("{\"verdict\": \"done\", \"line\": \"Hourly checks merged\"}"),
  ])
  .await;
  let (gemini, inputs) = serve_decisions(vec![("waiting-on-you", 0.93), ("done", 0.52)]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup_with(&data, &home, &base, Some(&gemini), port);
  let _node = stitch_node::start(&data).await.unwrap();
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
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Judged 1 of 1"), "{said}");
  assert!(said.contains("by the decision model"), "{said}");
  assert!(said.contains("waiting-on-you"), "{said}");
  // The line is the agent's own question; no quick model asked.
  assert!(
    said.contains("Should the dashboard check hourly, or only on demand?"),
    "{said}"
  );
  assert_eq!(sent.lock().unwrap().len(), 1, "only the turn");
  assert!(inputs.lock().unwrap()[0].contains("Add loose ends to the dashboard"));
  // Recorded for calibration, on the node's own day.
  let Reply::Lines(log) = c
    .request(&Request::Decide {
      input: json!({"action": "log", "profile": "loose-ends"}).to_string(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(log.len(), 1, "{log:?}");
  assert!(log[0].contains("\"calibrated\":true"), "{log:?}");
  let Reply::Lines(profiles) = c
    .request(&Request::Decide {
      input: json!({"action": "profiles"}).to_string(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let all = profiles.join("\n");
  assert!(all.contains("loose-ends:"), "{all}");
  assert!(all.contains("the node's own decisions today: 1"), "{all}");
  // The user's thresholds, synced, over the defaults, the guard's as any
  // profile's; unknown questions, and a fallback for what gates authority,
  // refused.
  let set = |input: Value| Request::Decide {
    input: input.to_string(),
  };
  let Reply::Lines(now) = c
    .request(&set(json!({"action": "set", "profile": "triage",
      "thresholds": ["duplicate=0.75"], "fallback": "none"})))
    .await
    .unwrap()
  else {
    panic!()
  };
  let now = now.join("\n");
  assert!(now.contains("duplicate 0.75"), "{now}");
  assert!(now.contains("synced settings"), "{now}");
  assert!(
    c.request(&set(
      json!({"action": "set", "profile": "triage", "thresholds": ["nope=0.5"]})
    ))
    .await
    .is_err()
  );
  assert!(
    c.request(&set(
      json!({"action": "set", "profile": "guard.fixer", "fallback": "anthropic/claude-haiku-4-5"})
    ))
    .await
    .is_err()
  );
  let Reply::Lines(g) = c
    .request(&set(json!({"action": "set", "profile": "guard.fixer",
      "thresholds": ["authority=0.4"]})))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(g.join("\n").contains("authority 0.40"), "{g:?}");
  // It moves; the decision model is unsure (done at 0.52 < 0.8): the quick model.
  c.chat(&chat(Some(&thread), "Hourly, please", &work), |_| None)
    .await
    .unwrap();
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Judged 1 of 1"), "{said}");
  assert!(!said.contains("by the decision model"), "{said}");
  assert_eq!(sent.lock().unwrap().len(), 3);
  assert_eq!(inputs.lock().unwrap().len(), 2);
}

/// Setting the auto-handling is the user's alone, from no sandbox: inside
/// one (an agent's `just quick` on macOS) the node rightly refuses the
/// test's own requests, so the tests that set it are not run there. On
/// Linux the node's sense of "confined" is `no_new_privs` (`NoNewPrivs:` in
/// `/proc/<pid>/status`, as `peer::confined` reads it), which the fixers'
/// sandbox sets.
fn outside_any_sandbox() -> bool {
  let ok = if cfg!(target_os = "macos") {
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
    eprintln!("inside a sandbox: only the user sets the auto-handling; not run");
  }
  ok
}

async fn auto(c: &mut Client, mode: Option<&str>, rules: Option<&str>) -> Value {
  match c
    .request(&Request::LooseEndsAuto {
      mode: mode.map(str::to_owned),
      rules: rules.map(str::to_owned),
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => serde_json::from_str(&l[0]).unwrap(),
    other => panic!("{other:?}"),
  }
}

const RULES: &str = "When the agent laid out a plan in the stitch repo and only waits for a go-ahead, tell it to go ahead.";

/// A dry run: the judge reads the rules and proposes, the default model
/// confirms, and nothing is sent: only recorded, and shown.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_records_what_it_would_do() {
  if !outside_any_sandbox() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(vec![
    answer("Plan: 1. the gate, 2. the judge. Shall I go ahead?"),
    answer(
      "{\"verdict\": \"waiting-on-you\", \"line\": \"Say go ahead\", \"act\": \"continue\", \"message\": \"Go ahead with the plan.\"}",
    ),
    answer("{\"confirm\": true, \"why\": \"the rule covers a go-ahead\"}"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Off by default, no rules.
  let a = auto(&mut c, None, None).await;
  assert_eq!(a["mode"], "off");
  assert_eq!(a["rules"], "");
  let a = auto(&mut c, Some("dry-run"), Some(RULES)).await;
  assert_eq!(a["mode"], "dry-run");
  assert_eq!(a["rules"], RULES);
  // Bad mode: refused.
  let bad = c
    .request(&Request::LooseEndsAuto {
      mode: Some("maybe".into()),
      rules: None,
    })
    .await;
  assert!(
    matches!(&bad, Ok(Reply::Error(e)) if e.contains("no mode")) || bad.is_err(),
    "{bad:?}"
  );

  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "Plan the loose ends gate", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Judged 1 of 1"), "{said}");
  assert!(said.contains("would-continue"), "{said}");
  assert!(
    said.contains("would continue: “Go ahead with the plan.”"),
    "{said}"
  );
  // The rules reached the judge, with what it may do.
  let asked = sent.lock().unwrap()[1].to_string();
  assert!(asked.contains("tell it to go ahead"), "{asked}");
  assert!(asked.contains("\\\"act\\\""), "{asked}");
  // The default model confirmed, reading the message proposed.
  let confirm = sent.lock().unwrap()[2].to_string();
  assert!(confirm.contains("Go ahead with the plan."), "{confirm}");
  assert!(confirm.contains("tell it to go ahead"), "{confirm}");
  // Nothing sent: no turn ran.
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert_eq!(sent.lock().unwrap().len(), 3);

  // Shown live: the setting, and the item's would-be action.
  let Reply::Apps { apps, .. } = c.request(&Request::Dashboard).await.unwrap() else {
    panic!()
  };
  let host = stitch_node::apps::pinned_host(&apps[0].id.parse().unwrap());
  let live = json_of(
    &node
      .serve_app(request(&host, "GET", "/_stitch/live/loose-ends"))
      .await,
  );
  assert_eq!(live["auto"]["mode"], "dry-run", "{live}");
  let items = live["items"].as_array().unwrap();
  assert_eq!(items[0]["thread"], thread.as_str());
  assert_eq!(items[0]["auto"]["outcome"], "would-continue", "{live}");
  assert_eq!(items[0]["auto"]["message"], "Go ahead with the plan.");

  // Off: the judge is asked nothing of the rules any more.
  let a = auto(&mut c, Some("off"), None).await;
  assert_eq!(a["mode"], "off");
  assert_eq!(a["rules"], RULES, "the rules stay");
}

/// On: confirmed, the message goes into the thread as from loose ends (a
/// turn runs); its next step, produced by that, is never auto-continued
/// again without a word of the user's.
#[tokio::test(flavor = "multi_thread")]
async fn on_it_continues_once_then_waits_for_the_user() {
  if !outside_any_sandbox() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  let (base, sent) = serve(vec![
    answer("Plan: 1. the gate, 2. the judge. Shall I go ahead?"),
    answer(
      "{\"verdict\": \"waiting-on-you\", \"line\": \"Say go ahead\", \"act\": \"continue\", \"message\": \"Go ahead with the plan.\"}",
    ),
    answer("{\"confirm\": true, \"why\": \"covered\"}"),
    // The turn loose ends started.
    answer("Step 1 done. Shall I do step 2 as well?"),
    // Judged again: the judge proposes again; the guard holds it.
    answer(
      "{\"verdict\": \"waiting-on-you\", \"line\": \"Say go ahead\", \"act\": \"continue\", \"message\": \"Go ahead.\"}",
    ),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let _node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  auto(&mut c, Some("on"), Some(RULES)).await;
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "Plan the loose ends gate", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let said = loose(&mut c, true, None).await;
  assert!(said.contains("Auto: continued"), "{said}");

  // The turn it started: the message, said to be loose ends'.
  let mut ran = false;
  for _ in 0..100 {
    if sent.lock().unwrap().len() >= 4 {
      ran = true;
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  assert!(ran, "no turn ran");
  let turn = sent.lock().unwrap()[3].to_string();
  assert!(turn.contains("Go ahead with the plan."), "{turn}");
  assert!(turn.contains("the user did not type it"), "{turn}");

  // Once the turn is over: judged again, the guard holds the next one
  // (no confirmation asked, nothing sent).
  let mut said = String::new();
  for _ in 0..100 {
    said = loose(&mut c, true, None).await;
    if said.contains("Judged 1 of 1") {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  assert!(said.contains("Auto: held"), "{said}");
  assert!(said.contains("waits for one of yours"), "{said}");
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert_eq!(sent.lock().unwrap().len(), 5);
  // It waits on the user, shown as such.
  assert!(said.contains(&thread), "{said}");
}
