//! App threads (design §2.22): an app runs threads of its own, a family,
//! within what its manifest (`stitch-app.json`) declares and the user
//! granted once; it reaches none of the user's other threads; its threads
//! never run as yolo, have only the tools granted, and what stays the
//! user's (a push) still asks the user.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::apps::serve::{Request as AppRequest, Response};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig, ShellSettings};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A mock Anthropic: each connection gets the next response; the bodies
/// sent are kept.
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
      // A response marked so is held a moment: a turn stays at work.
      let response = match response.strip_prefix(SLOW) {
        Some(r) => {
          tokio::time::sleep(Duration::from_millis(1500)).await;
          r.to_owned()
        }
        None => response,
      };
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

/// Marks a mock response held 1.5 s before it is written.
const SLOW: &str = "SLOW:";

fn sse(events: Vec<(&str, Value)>) -> String {
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

fn tool_use(id: &str, name: &str, input: Value) -> String {
  tool_use_of(id, name, input, 50)
}

/// A tool call whose step read `tokens` tokens (claude-opus-5-5: $4 a
/// million).
fn tool_use_of(id: &str, name: &str, input: Value, tokens: u64) -> String {
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": tokens}}}),
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

fn free_port() -> u16 {
  std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

/// A node `mbp`; `yolo`: its shell runs everything without asking.
fn setup(data: &Path, home: &Path, anthropic: &str, port: u16, yolo: bool) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(port) }),
    shell: yolo.then(|| ShellSettings {
      approval: Some("allow".into()),
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

fn get(host: &str, path: &str) -> AppRequest {
  AppRequest {
    host: host.into(),
    path: path.into(),
    method: "GET".into(),
    body: Vec::new(),
  }
}

fn post(host: &str, path: &str, body: Value) -> AppRequest {
  AppRequest {
    host: host.into(),
    path: path.into(),
    method: "POST".into(),
    body: body.to_string().into_bytes(),
  }
}

fn text(r: &Response) -> String {
  String::from_utf8_lossy(&r.body).into_owned()
}

fn json_of(r: &Response) -> Value {
  serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("{} {}", r.status, text(r)))
}

async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..400 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

/// The version of the app a thread showed last, as its host: pinned a
/// moment to learn it.
async fn shown_host(c: &mut Client, thread: &str) -> String {
  let Reply::Apps { apps, .. } = c
    .request(&Request::AppPin {
      what: thread.into(),
      name: Some("Probe".into()),
      icon: None,
      fixed: Some(true),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  c.request(&Request::AppUnpin {
    app: apps[0].id.clone(),
  })
  .await
  .unwrap();
  stitch_node::apps::version_host(&apps[0].version.parse().unwrap())
}

fn write_app(dir: &Path, needs: Value) {
  std::fs::create_dir_all(dir).unwrap();
  std::fs::write(dir.join("index.html"), "<p>atelier</p>").unwrap();
  std::fs::write(dir.join("stitch-app.json"), needs.to_string()).unwrap();
}

/// The user grants an app, as `stitch app allow-threads` does: shown
/// first (nothing written), then granted with the hash shown. What the
/// grant said.
async fn grant(c: &mut Client, thread: &str) -> String {
  let Reply::Lines(shown) = c
    .request(&Request::AppThreads {
      app: "Atelier".into(),
      thread: Some(thread.into()),
      allow: true,
      needs: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let shown = shown.join("\n");
  assert!(shown.contains("Nothing was granted"), "{shown}");
  let hash = shown.rsplit("--needs ").next().unwrap().trim().to_owned();
  // Another hash: refused, nothing written.
  assert!(
    c.request(&Request::AppThreads {
      app: "Atelier".into(),
      thread: Some(thread.into()),
      allow: true,
      needs: Some("0000".into()),
    })
    .await
    .is_err()
  );
  let Reply::Lines(said) = c
    .request(&Request::AppThreads {
      app: "Atelier".into(),
      thread: Some(thread.into()),
      allow: true,
      needs: Some(hash),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  said.join("\n")
}

/// The tool names a model request offered.
fn offered(body: &Value) -> Vec<String> {
  body["tools"]
    .as_array()
    .map(|t| {
      t.iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
    })
    .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_runs_threads_of_its_own_within_its_grant() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  write_app(
    &work.join("atelier"),
    json!({"threads": {
      "places": ["global"],
      "tools": ["viewer", "web"],
      "models": ["default"],
      "budget": {"usd": 1},
      "daily_usd": 5,
      "concurrent": 2,
    }}),
  );
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("atelier").display().to_string(), "name": "Atelier"}),
    ),
    answer("Shown."),
    // The app's thread: it reads only in its place, and no address here.
    tool_use(
      "v1",
      "view_file",
      json!({"path": d.join("data/config.toml").display().to_string()}),
    ),
    tool_use("w1", "browse_web", json!({"url": "http://localhost:7177/"})),
    tool_use("w2", "browse_web", json!({"url": "http://[::ffff:127.0.0.1]:7177/"})),
    // A blob of the user's (the app's own page, stored when shown): not an
    // image of this thread's.
    tool_use(
      "z1",
      "zoom_image",
      json!({"image": stitch_records::BlobHash::of(b"<p>atelier</p>").to_string(), "left": 0, "top": 0, "right": 1, "bottom": 1}),
    ),
    tool_use("e1", "web_eval", json!({"url": "https://example.com", "code": "1"})),
    answer("A red dress, then."),
    answer("Hemmed."),
    // The app changed: other needs.
    tool_use(
      "c2",
      "show_app",
      json!({"path": work.join("atelier").display().to_string(), "name": "Atelier"}),
    ),
    answer("Shown again."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port, false);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "an atelier app", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let host = shown_host(&mut c, &thread).await;

  // Not granted yet: refused, saying how to grant it and what it asks.
  let r = node.serve_app(get(&host, "/_stitch/threads")).await;
  assert_eq!(r.status, 403);
  let refused = json_of(&r);
  assert_eq!(refused["consent"], "ask", "{refused}");
  assert!(
    refused["allow"].as_str().unwrap().starts_with(&format!(
      "stitch app allow-threads \"Atelier\" --thread {thread} --needs "
    )),
    "{refused}"
  );
  assert!(
    refused["needs"].to_string().contains("tools: viewer"),
    "{refused}"
  );
  let r = node
    .serve_app(post(&host, "/_stitch/threads", json!({"goal": "x"})))
    .await;
  assert_eq!(r.status, 403);
  let page = text(&node.serve_app(get(&host, "/")).await);
  assert!(page.contains("\"threads\":{"), "{page}");
  assert!(page.contains("\"consent\":\"ask\""), "{page}");

  // The user grants it, seeing the list first.
  let said = grant(&mut c, &thread).await;
  assert!(said.contains("may now run threads"), "{said}");
  assert!(
    said.contains("place global: each thread's own folder"),
    "{said}"
  );
  assert!(
    said.contains("places: global") && said.contains("tools: viewer"),
    "{said}"
  );
  assert_eq!(
    json_of(&node.serve_app(get(&host, "/_stitch/threads/grant")).await)["consent"],
    "allowed"
  );

  // What the grant does not cover is refused before anything is written.
  for (body, says) in [
    (
      json!({"goal": "x", "model": "anthropic/claude-opus-5-5"}),
      "model",
    ),
    (
      json!({"goal": "x", "place": d.display().to_string()}),
      "place",
    ),
    (json!({"goal": "x", "budget_usd": 3}), "budget_usd"),
  ] {
    let r = node.serve_app(post(&host, "/_stitch/threads", body)).await;
    assert_eq!(r.status, 403, "{}", text(&r));
    assert!(text(&r).contains(says), "{}", text(&r));
  }
  let r = node
    .serve_app(post(
      &host,
      "/_stitch/threads",
      json!({"goal": "x", "images": ["data:image/svg+xml;base64,PHN2Zz4="]}),
    ))
    .await;
  assert_eq!(r.status, 400, "{}", text(&r));

  // Created: its first turn runs with only what was granted.
  let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
  let made = json_of(
    &node
      .serve_app(post(
        &host,
        "/_stitch/threads",
        json!({"goal": "Design a red dress", "images": [{"data": png, "name": "sketch.png"}]}),
      ))
      .await,
  );
  let id = made["id"].as_str().unwrap().to_owned();
  assert_eq!(made["place"], "global");
  let mut read = Value::Null;
  for _ in 0..400 {
    read = json_of(
      &node
        .serve_app(get(&host, &format!("/_stitch/threads/{id}/messages")))
        .await,
    );
    if read["state"]["state"] == "idle" && read.to_string().contains("A red dress, then.") {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let list = read["messages"].as_array().unwrap();
  assert_eq!(list[0]["role"], "user", "{read}");
  assert_eq!(list[0]["text"], "Design a red dress");
  assert_eq!(list[0]["from"], "app \"Atelier\"");
  let image = list[0]["images"][0]["url"].as_str().unwrap().to_owned();
  let r = node.serve_app(get(&host, &image)).await;
  assert_eq!(r.status, 200);
  assert_eq!(r.get("Content-Type"), Some("image/png"));
  assert_eq!(r.get("Cross-Origin-Resource-Policy"), Some("same-origin"));
  assert!(
    list
      .iter()
      .any(|m| m["role"] == "assistant" && m["text"] == "A red dress, then."),
    "{read}"
  );
  {
    let bodies = sent.lock().unwrap();
    let tools = offered(&bodies[2]);
    assert!(tools.contains(&"view_file".to_owned()), "{tools:?}");
    for t in [
      "shell_run",
      "spawn",
      "send",
      "recall",
      "write_file",
      "show_app",
    ] {
      assert!(!tools.contains(&t.to_owned()), "{t} offered: {tools:?}");
    }
    // The image went to the model.
    assert!(
      bodies[2]["messages"].to_string().contains("\"image\""),
      "image sent"
    );
    let read = bodies[3]["messages"].to_string();
    assert!(read.contains("outside this thread's place"), "{read}");
    let last = |i: usize| {
      bodies[i]["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string()
    };
    let web = last(4);
    assert!(web.contains("reaches no address of this machine"), "{web}");
    let mapped = last(5);
    assert!(
      mapped.contains("reaches no address of this machine"),
      "{mapped}"
    );
    let zoom = last(6);
    assert!(zoom.contains("its own images only"), "{zoom}");
    let eval = last(7);
    assert!(eval.contains("web_eval is off"), "{eval}");
    // Its prompt is its own: nothing of the user's home, nodes or paths.
    let system = bodies[2]["system"].to_string();
    assert!(system.contains("untrusted page"), "{system}");
    for leak in [
      "# This node",
      "home workspace",
      "SSH hosts",
      "Where this turn runs",
    ] {
      assert!(!system.contains(leak), "{leak} in {system}");
    }
  }
  // Its record names its app.
  {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let r = index.record(&id.parse().unwrap()).unwrap().unwrap();
    assert_eq!(r.fields["app"], thread.as_str());
    assert_eq!(r.fields["app_name"], "Atelier");
  }

  // The family: its own thread, never the user's.
  let all = json_of(&node.serve_app(get(&host, "/_stitch/threads")).await);
  let rows = all["threads"].as_array().unwrap();
  assert_eq!(rows.len(), 1, "{all}");
  assert_eq!(rows[0]["id"], id.as_str());
  assert_eq!(rows[0]["state"], "idle");
  assert_eq!(all["daily_usd"], 5.0);
  for path in [
    format!("/_stitch/threads/{thread}"),
    format!("/_stitch/threads/{thread}/messages"),
    "/_stitch/threads/not-an-id".to_owned(),
  ] {
    assert_eq!(
      node.serve_app(get(&host, &path)).await.status,
      404,
      "{path}"
    );
  }
  let r = node
    .serve_app(post(
      &host,
      &format!("/_stitch/threads/{thread}/send"),
      json!({"text": "hi"}),
    ))
    .await;
  assert_eq!(r.status, 404, "the user's thread is not the app's");
  let r = node
    .serve_app(post(
      &host,
      &format!("/_stitch/threads/{thread}/cancel"),
      json!({}),
    ))
    .await;
  assert_eq!(r.status, 404);

  // A message: a new turn, from the app.
  let r = json_of(
    &node
      .serve_app(post(
        &host,
        &format!("/_stitch/threads/{id}/send"),
        json!({"text": "Hem it"}),
      ))
      .await,
  );
  assert_eq!(r["ok"], true, "{r}");
  let after = list.last().unwrap()["n"].as_i64().unwrap();
  until("the reply", || {
    (sent.lock().unwrap().len() >= 9).then_some(())
  })
  .await;
  let mut newer = Value::Null;
  for _ in 0..400 {
    newer = json_of(
      &node
        .serve_app(get(
          &host,
          &format!("/_stitch/threads/{id}/messages?after={after}"),
        ))
        .await,
    );
    if newer.to_string().contains("Hemmed.") && newer["state"]["state"] == "idle" {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let newer_list = newer["messages"].as_array().unwrap();
  assert_eq!(newer_list[0]["text"], "Hem it", "{newer}");
  assert_eq!(newer_list[0]["from"], "app \"Atelier\"");
  assert!(newer_list.iter().all(|m| m["n"].as_i64().unwrap() > after));

  // The app changes what it asks for: a new grant is needed to create;
  // what it made stays readable.
  write_app(
    &work.join("atelier"),
    json!({"threads": {"tools": ["viewer"]}}),
  );
  let Reply::ChatDone { .. } = c
    .chat(&chat(Some(&thread), "let it browse", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let host2 = shown_host(&mut c, &thread).await;
  assert_ne!(host, host2);
  let r = node
    .serve_app(post(&host2, "/_stitch/threads", json!({"goal": "x"})))
    .await;
  assert_eq!(r.status, 403);
  assert_eq!(json_of(&r)["consent"], "changed");
  let still = json_of(&node.serve_app(get(&host2, "/_stitch/threads")).await);
  assert_eq!(
    still["threads"].as_array().unwrap().len(),
    1,
    "same family: {still}"
  );

  // Denied: nothing more.
  c.request(&Request::AppThreads {
    app: "Atelier".into(),
    thread: Some(thread.clone()),
    allow: false,
    needs: None,
  })
  .await
  .unwrap();
  let r = node.serve_app(get(&host, "/_stitch/threads")).await;
  assert_eq!(r.status, 403);
  assert_eq!(json_of(&r)["consent"], "denied");
  drop(node);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_thread_never_runs_as_yolo_and_what_is_the_users_still_asks() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  write_app(
    &work.join("atelier"),
    json!({"threads": {"tools": ["files"], "concurrent": 1}}),
  );
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("atelier").display().to_string(), "name": "Atelier"}),
    ),
    answer("Shown."),
    // Inside its own folder, but a place's skill: the user's to approve.
    tool_use(
      "p1",
      "write_file",
      json!({"path": ".stitch/skills/evil/tools/run.otr", "content": "1"}),
    ),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  // The node runs everything without asking: its own threads would push.
  setup(&data, &home, &base, port, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "an atelier app", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let host = shown_host(&mut c, &thread).await;
  grant(&mut c, &thread).await;
  let made = json_of(
    &node
      .serve_app(post(
        &host,
        "/_stitch/threads",
        json!({"goal": "Publish the pattern"}),
      ))
      .await,
  );
  let id = made["id"].as_str().unwrap().to_owned();

  // Writing what would steer the user's agents waits on the user, in its
  // place, yolo or not; the question names the app.
  let asked = until("the question", || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let items = index.attention().unwrap();
    (!items.is_empty()).then(|| format!("{items:?}"))
  })
  .await;
  assert!(asked.contains("Atelier"), "{asked}");
  let row = json_of(
    &node
      .serve_app(get(&host, &format!("/_stitch/threads/{id}")))
      .await,
  );
  assert_eq!(row["state"], "approval", "{row}");
  {
    let bodies = sent.lock().unwrap();
    let tools = offered(&bodies[2]);
    assert!(tools.contains(&"write_file".to_owned()), "{tools:?}");
    assert!(!tools.contains(&"shell_run".to_owned()), "{tools:?}");
    assert!(!tools.contains(&"spawn".to_owned()), "{tools:?}");
  }

  // One at work at most: a second is refused.
  tokio::time::sleep(Duration::from_millis(2100)).await;
  let r = node
    .serve_app(post(&host, "/_stitch/threads", json!({"goal": "Another"})))
    .await;
  assert_eq!(r.status, 429, "{}", text(&r));

  // The app cancels its own.
  let r = node
    .serve_app(post(
      &host,
      &format!("/_stitch/threads/{id}/cancel"),
      json!({}),
    ))
    .await;
  assert_eq!(r.status, 200, "{}", text(&r));
  let mut state = Value::Null;
  for _ in 0..200 {
    state = json_of(
      &node
        .serve_app(get(&host, &format!("/_stitch/threads/{id}")))
        .await,
    );
    if state["state"] == "cancelled" {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  assert_eq!(state["state"], "cancelled", "{state}");
  drop(node);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_apps_money_is_counted_at_every_step_and_its_slots_under_a_lock() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  write_app(
    &work.join("atelier"),
    json!({"threads": {"tools": ["viewer"], "daily_usd": 5, "budget": {"usd": 5}, "concurrent": 1}}),
  );
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("atelier").display().to_string(), "name": "Atelier"}),
    ),
    answer("Shown."),
    // A's first step reads a million tokens ($4): its next step would pass
    // the family's $5 a day, so there is none.
    tool_use_of("v1", "view_file", json!({"path": "."}), 1_000_000),
    answer("never asked for"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port, false);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "an atelier app", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let host = shown_host(&mut c, &thread).await;
  grant(&mut c, &thread).await;
  let made = json_of(
    &node
      .serve_app(post(
        &host,
        "/_stitch/threads",
        json!({"goal": "Cut the pattern", "title": "Pattern"}),
      ))
      .await,
  );
  let a = made["id"].as_str().unwrap().to_owned();
  let mut row = Value::Null;
  for _ in 0..400 {
    row = json_of(
      &node
        .serve_app(get(&host, &format!("/_stitch/threads/{a}")))
        .await,
    );
    if (row["state"] == "done" || row["state"] == "idle") && row["title"] == "Pattern" {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  assert_eq!(row["title"], "Pattern", "{row}");
  assert_eq!(row["outcome"], "over-budget", "{row}");
  assert_eq!(
    sent.lock().unwrap().len(),
    3,
    "no step past the day's money"
  );
  let list = json_of(&node.serve_app(get(&host, "/_stitch/threads")).await);
  assert!(list["spent_today"].as_f64().unwrap() >= 4.0, "{list}");
  drop(node);
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_sends_do_not_pass_the_apps_concurrency_and_a_denial_stops_its_turns() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  write_app(
    &work.join("atelier"),
    json!({"threads": {"tools": ["viewer"], "concurrent": 1}}),
  );
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("atelier").display().to_string(), "name": "Atelier"}),
    ),
    answer("Shown."),
    answer("A."),
    answer("B."),
    format!("{SLOW}{}", answer("Slow.")),
    format!("{SLOW}{}", answer("Slow too.")),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port, false);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "an atelier app", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let host = shown_host(&mut c, &thread).await;
  grant(&mut c, &thread).await;
  let mut ids = Vec::new();
  for goal in ["A", "B"] {
    let made = json_of(
      &node
        .serve_app(post(&host, "/_stitch/threads", json!({"goal": goal})))
        .await,
    );
    let id = made["id"].as_str().unwrap().to_owned();
    for _ in 0..400 {
      let r = json_of(
        &node
          .serve_app(get(&host, &format!("/_stitch/threads/{id}")))
          .await,
      );
      if r["state"] == "idle" {
        break;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ids.push(id);
    tokio::time::sleep(Duration::from_millis(2100)).await;
  }
  assert_eq!(sent.lock().unwrap().len(), 4);
  // Both at once: one is admitted, the other refused.
  let (r1, r2) = tokio::join!(
    node.serve_app(post(
      &host,
      &format!("/_stitch/threads/{}/send", ids[0]),
      json!({"text": "again"}),
    )),
    node.serve_app(post(
      &host,
      &format!("/_stitch/threads/{}/send", ids[1]),
      json!({"text": "again"}),
    )),
  );
  let mut statuses = [r1.status, r2.status];
  statuses.sort();
  assert_eq!(statuses, [200, 429], "{} / {}", text(&r1), text(&r2));
  // The user takes the grant back: the turn at work is cancelled.
  until("the turn at work", || {
    (sent.lock().unwrap().len() >= 5).then_some(())
  })
  .await;
  let Reply::Lines(said) = c
    .request(&Request::AppThreads {
      app: "Atelier".into(),
      thread: Some(thread.clone()),
      allow: false,
      needs: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(said.join("\n").contains("1 at work cancelled"), "{said:?}");
  drop(node);
}
