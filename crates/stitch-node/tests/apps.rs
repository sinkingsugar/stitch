//! Apps (design §2.10): an agent shows one, the node serves it (to the
//! phone's web view and to a browser here), its state is kept as records
//! of its thread, it can be pinned, and it reports to its thread.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::apps::serve::{Request as AppRequest, Response};
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

/// The model calls one tool.
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

/// The pinned apps but the built-in web client (pinned at start).
fn user_apps(all: Vec<stitch_node::apps::Pinned>) -> Vec<stitch_node::apps::Pinned> {
  all
    .into_iter()
    .filter(|p| p.builtin.as_ref().is_none_or(|(k, _)| k != "client"))
    .collect()
}

fn free_port() -> u16 {
  std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn setup(data: &Path, home: &Path, anthropic: &str, port: u16) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(port) }),
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

async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..200 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_is_shown_served_kept_pinned_and_reports() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("work/apps/todo");
  std::fs::create_dir_all(app.join("js")).unwrap();
  std::fs::write(
    app.join("index.html"),
    "<!doctype html><html><head><title>Todo</title></head><body><script src=\"js/main.js\"></script></body></html>",
  )
  .unwrap();
  std::fs::write(app.join("js/main.js"), "console.log('hi')").unwrap();

  let (base, sent) = serve(vec![
    tool_use("c1", "read_skill", json!({"name": "apps"})),
    tool_use(
      "c0",
      "read_skill",
      json!({"name": "leadsheet", "file": "assets/player.html"}),
    ),
    tool_use(
      "c2",
      "show_app",
      json!({"path": "apps/todo", "name": "Todo"}),
    ),
    answer("Here it is."),
    answer("Refreshed."),
    answer("Got your list."),
    tool_use(
      "c3",
      "show_app",
      json!({"path": "apps/todo", "name": "Todo"}),
    ),
    answer("Changed."),
    tool_use(
      "c4",
      "show_app",
      json!({"path": "apps/todo", "name": "Todo"}),
    ),
    answer("Changed again."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let mut told = Vec::new();
  let done = c
    .chat(&chat(None, "make me a todo app", &d.join("work")), |r| {
      if let Reply::App { name, url } = r {
        told.push((name.clone(), url.clone()));
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  let (manifest, address) = {
    let bodies = sent.lock().unwrap();
    let system = bodies[0]["system"][0]["text"].as_str().unwrap().to_owned();
    assert!(
      system.contains("- apps: Making apps")
        && system.contains("- leadsheet: Compose multitrack music")
        && system.contains("(built in: read_skill)"),
      "{system}"
    );
    assert!(
      bodies[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "show_app"),
      "show_app is offered"
    );
    // The skill came back whole.
    let skill = bodies[1]["messages"].to_string();
    assert!(skill.contains("Bundled libraries"), "{skill}");
    // The model hears the app was shown, by name and version.
    // A built-in skill's file, by its path.
    let file = bodies[2]["messages"].to_string();
    assert!(file.contains("[leadsheet assets/player.html]"), "{file}");
    assert!(file.contains("/_lib/leadsheet.js"), "{file}");
    let shown = bodies[3]["messages"].to_string();
    assert!(shown.contains("Shown: app \\\"Todo\\\""), "{shown}");
    assert!(shown.contains("[App `Todo`, version "), "{shown}");
    let url = shown
      .split("it is at ")
      .nth(1)
      .unwrap()
      .split(' ')
      .next()
      .unwrap()
      .to_owned();
    (
      shown
        .split("Shown: app \\\"Todo\\\", version ")
        .nth(1)
        .unwrap()[..8]
        .to_owned(),
      url,
    )
  };
  let Reply::Apps {
    launcher: Some(launcher),
    ..
  } = c.request(&Request::Apps).await.unwrap()
  else {
    panic!("no apps listing")
  };

  // The version, as the phone's web view gets it.
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let version = {
    let dir = std::fs::read_dir(data.join("blobs/blake3"))
      .unwrap()
      .flatten()
      .flat_map(|a| std::fs::read_dir(a.path()).unwrap().flatten())
      .flat_map(|b| std::fs::read_dir(b.path()).unwrap().flatten())
      .map(|f| f.file_name().to_string_lossy().into_owned())
      .find(|n| n.starts_with(&manifest))
      .unwrap();
    format!("blake3:{dir}").parse().unwrap()
  };
  let m = stitch_node::apps::Manifest::read(&blobs, &version).unwrap();
  assert_eq!(m.name, "Todo");
  assert_eq!(
    m.thread.as_ref().map(|t| t.to_string()),
    Some(thread.clone())
  );
  let host = stitch_node::apps::version_host(&version);
  // The chat's client is told of it, with a link its browser gets in
  // with (the token, never in the transcript); the model's has none.
  assert!(!address.contains("k="), "{address}");
  let link = format!("http://{host}.localhost:{port}/?k=");
  assert!(
    matches!(&told[..], [(n, Some(u))] if n == "Todo" && u.starts_with(&link)),
    "{told:?}"
  );
  // `stitch app open <thread>`: the same link.
  let Reply::App { name, url } = c
    .request(&Request::AppOpen {
      what: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!("no app of the thread")
  };
  assert_eq!((name.as_str(), &url), ("Todo", &told[0].1));
  let page = node.serve_app(get(&host, "/")).await;
  assert_eq!(page.status, 200);
  let html = text(&page);
  assert!(html.contains("window.__stitch="), "{html}");
  assert!(html.contains("<script src=\"js/main.js\">"), "{html}");
  assert!(
    page
      .get("Content-Security-Policy")
      .is_some_and(|c| c.contains("stitch-app:")),
    "no networking: a CSP"
  );
  assert_eq!(
    text(&node.serve_app(get(&host, "/js/main.js")).await),
    "console.log('hi')"
  );
  assert_eq!(node.serve_app(get(&host, "/nope.js")).await.status, 404);
  assert_eq!(
    node.serve_app(get(&host, "/../index.html")).await.status,
    404
  );
  assert!(text(&node.serve_app(get(&host, "/_stitch/bridge.js")).await).contains("window.stitch"));
  assert!(
    node
      .serve_app(get(&host, "/_lib/chart.min.js"))
      .await
      .body
      .len()
      > 100_000
  );
  assert_eq!(node.serve_app(get("0123", "/")).await.status, 404);

  // State: written by the page, read at once, then as records of the
  // thread; a page waiting for changes hears of it.
  let before = node.serve_app(get(&host, "/_stitch/state")).await;
  let generation = serde_json::from_slice::<Value>(&before.body).unwrap()["generation"]
    .as_u64()
    .unwrap();
  let waiting = tokio::spawn(node.serve_app(get(
    &host,
    &format!("/_stitch/events?generation={generation}"),
  )));
  let w = node
    .serve_app(post(
      &host,
      "/_stitch/state",
      json!({"set": {"todos": ["milk", "eggs"], "tab": "all"}}),
    ))
    .await;
  assert_eq!(w.status, 200, "{}", text(&w));
  let heard: Value = serde_json::from_slice(&waiting.await.unwrap().body).unwrap();
  assert_eq!(heard["state"]["todos"], "[\"milk\",\"eggs\"]");
  let page = text(&node.serve_app(get(&host, "/")).await);
  assert!(page.contains("milk"), "a page starts with the state");
  until("the state written as records", || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let t = thread.parse().unwrap();
    let s = stitch_node::apps::stored(&index, &t).unwrap();
    (s.get("tab").map(String::as_str) == Some("\"all\"")).then_some(())
  })
  .await;
  node
    .serve_app(post(&host, "/_stitch/state", json!({"remove": ["tab"]})))
    .await;
  let bad = node
    .serve_app(post(&host, "/_stitch/state", json!({"set": {"": 1}})))
    .await;
  assert_eq!(bad.status, 400);

  // Pinned: an origin of its own, the same state.
  let Reply::Apps { apps, .. } = c
    .request(&Request::AppPin {
      what: thread.clone(),
      name: None,
      icon: Some("checklist".into()),
      fixed: None,
    })
    .await
    .unwrap()
  else {
    panic!("not pinned")
  };
  assert_eq!(apps.len(), 1);
  assert_eq!(apps[0].name, "Todo");
  assert!(!apps[0].fixed, "pinned from a thread, it follows it");
  assert_eq!(apps[0].version, version.to_string());
  let pinned = {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    user_apps(stitch_node::apps::pinned(&index).unwrap()).remove(0)
  };
  let page = node.serve_app(get(&pinned.host, "/")).await;
  assert_eq!(page.status, 200);
  assert!(text(&page).contains("milk"));
  // Pinned again from the same thread, at a version: the same app, still
  // one, fixed there.
  let Reply::Apps { apps: again, .. } = c
    .request(&Request::AppPin {
      what: version.to_string(),
      name: None,
      icon: None,
      fixed: None,
    })
    .await
    .unwrap()
  else {
    panic!("not pinned again")
  };
  assert_eq!(again[0].id, apps[0].id);
  assert_eq!(again[0].icon.as_deref(), Some("checklist"));
  assert!(again[0].fixed, "pinned at a version, it stays");
  let Reply::Apps { apps: all, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!()
  };
  assert_eq!(all.iter().filter(|a| a.builtin.is_none()).count(), 1);

  // A pinned app reports to the thread it was pinned from, as its version.
  let r = node
    .serve_app(post(&pinned.host, "/_stitch/report", json!({"refresh": 1})))
    .await;
  assert_eq!(r.status, 200, "{}", text(&r));
  let told = until("the pinned app's report", || {
    let bodies = sent.lock().unwrap();
    (bodies.len() == 5).then(|| bodies[4]["messages"].to_string())
  })
  .await;
  assert!(
    told.contains(&format!(
      "[Pinned app \\\"Todo\\\" (version {manifest}) sent you data]"
    )),
    "{told}"
  );
  // The same rate limit, per thread, pinned or not.
  for h in [&pinned.host, &host] {
    let again = node
      .serve_app(post(h, "/_stitch/report", json!("again")))
      .await;
    assert_eq!(again.status, 429, "{}", text(&again));
  }

  // A pin with no thread, or whose thread is not here: a clear 409.
  for (name, t, says) in [
    ("Loose", None, "no conversation to report to"),
    (
      "Orphan",
      Some(
        "2026-01-01T000000Z-mbp-zzzz"
          .parse::<stitch_records::RecordId>()
          .unwrap(),
      ),
      "is not on this device",
    ),
  ] {
    let v = stitch_node::apps::snapshot(&blobs, &app, name, false, t.as_ref()).unwrap();
    let Reply::Apps { apps: p, .. } = c
      .request(&Request::AppPin {
        what: v.manifest.to_string(),
        name: None,
        icon: None,
        fixed: None,
      })
      .await
      .unwrap()
    else {
      panic!("{name} not pinned")
    };
    let host = {
      let index = Index::open(&data.join("index.sqlite")).unwrap();
      let all = user_apps(stitch_node::apps::pinned(&index).unwrap());
      stitch_node::apps::find_pinned(&all, &p[0].id)
        .unwrap()
        .host
        .clone()
    };
    let r = node
      .serve_app(post(&host, "/_stitch/report", json!("x")))
      .await;
    assert_eq!(r.status, 409, "{name}");
    assert!(text(&r).contains(says), "{name}: {}", text(&r));
    assert!(matches!(
      c.request(&Request::AppUnpin { app: name.into() })
        .await
        .unwrap(),
      Reply::Ok
    ));
  }

  // The browser on this machine: let in by the link's token only.
  let http = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();
  assert!(
    !address.contains("k="),
    "no token in a transcript: {address}"
  );
  let token = launcher.split("k=").nth(1).unwrap();
  let app_url = format!("{address}?k={token}");
  let origin = address.trim_end_matches('/').to_owned();
  let out = http.get(&origin).send().await.unwrap();
  assert_eq!(out.status(), 403, "no token, no cookie");
  let first = http.get(&app_url).send().await.unwrap();
  assert_eq!(first.status(), 303);
  let cookie = first.headers()["set-cookie"]
    .to_str()
    .unwrap()
    .split(';')
    .next()
    .unwrap()
    .to_owned();
  let page = http
    .get(format!("{origin}/"))
    .header("cookie", &cookie)
    .send()
    .await
    .unwrap();
  assert_eq!(page.status(), 200);
  assert!(page.text().await.unwrap().contains("window.__stitch="));
  let rebound = http
    .get(format!("http://127.0.0.1:{port}/"))
    .header("host", "evil.example:80")
    .send()
    .await
    .unwrap();
  assert_eq!(rebound.status(), 421, "another site's name");
  let forged = http
    .post(format!("{origin}/_stitch/state"))
    .header("cookie", &cookie)
    .header("origin", "http://other.localhost")
    .body(json!({"set": {"x": 1}}).to_string())
    .send()
    .await
    .unwrap();
  assert_eq!(forged.status(), 403, "a POST from another origin");

  // A report wakes the thread with the data (past the rate limit).
  tokio::time::sleep(Duration::from_secs(2)).await;
  let r = node
    .serve_app(post(&host, "/_stitch/report", json!({"done": ["milk"]})))
    .await;
  assert_eq!(r.status, 200, "{}", text(&r));
  until("the report's turn", || {
    let bodies = sent.lock().unwrap();
    (bodies.len() == 6).then(|| bodies[5]["messages"].to_string())
  })
  .await
  .contains("[App \\\"Todo\\\" sent you data]")
  .then_some(())
  .expect("the report reached the model");
  // Its turn ends.
  tokio::time::sleep(Duration::from_secs(1)).await;

  // The thread shows a newer version: a pin fixed at a version stays.
  let pin_now = || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    user_apps(stitch_node::apps::pinned(&index).unwrap()).remove(0)
  };
  let show_again = |marker: &str| {
    std::fs::write(
      app.join("index.html"),
      format!("<!doctype html><html><head></head><body>{marker}</body></html>"),
    )
    .unwrap();
  };
  show_again("v2");
  let done = c
    .chat(&chat(Some(&thread), "change it", &d.join("work")), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  let shown = |n: usize| {
    let bodies = sent.lock().unwrap();
    let m = bodies[n]["messages"].to_string();
    let last = m
      .rsplit("Shown: app \\\"Todo\\\", version ")
      .next()
      .unwrap();
    (last[..8].to_owned(), last.to_owned())
  };
  let (v2, said) = shown(7);
  assert!(!said.contains("moved to this version"), "{said}");
  let p = pin_now();
  assert_eq!(p.manifest, version, "a pin fixed at a version stays");
  assert!(p.fixed);

  // Pinned from the thread again, it follows it: at its latest, then
  // moved by the next one, with its ID (its origin, its data), name and
  // icon.
  c.request(&Request::AppPin {
    what: thread.clone(),
    name: None,
    icon: None,
    fixed: None,
  })
  .await
  .unwrap();
  let p = pin_now();
  assert_eq!(&p.manifest.hex()[..8], v2);
  assert!(!p.fixed);
  show_again("v3");
  c.chat(&chat(Some(&thread), "again", &d.join("work")), |_| None)
    .await
    .unwrap();
  let (v3, said) = shown(9);
  assert!(
    said.contains("The pinned app \\\"Todo\\\" moved to this version."),
    "{said}"
  );
  let moved = pin_now();
  assert_eq!(&moved.manifest.hex()[..8], v3);
  assert_eq!(moved.id, pinned.id);
  assert_eq!(moved.host, pinned.host);
  assert_eq!(moved.name, "Todo");
  assert_eq!(moved.icon.as_deref(), Some("checklist"));
  assert!(!moved.fixed);
  let page = text(&node.serve_app(get(&pinned.host, "/")).await);
  assert!(page.contains("v3") && page.contains("milk"), "{page}");

  // Unpinned: gone from the list, its versions kept.
  assert!(matches!(
    c.request(&Request::AppUnpin { app: "todo".into() })
      .await
      .unwrap(),
    Reply::Ok
  ));
  let Reply::Apps { apps: none, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!()
  };
  assert!(none.iter().all(|a| a.builtin.as_deref() == Some("client")));
  assert_eq!(node.serve_app(get(&pinned.host, "/")).await.status, 404);
  assert_eq!(node.serve_app(get(&host, "/")).await.status, 200);

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A spawned thread that showed an app and reported is woken by that app's
/// report (pinned, as the user's Refresh sends it): a message from the
/// user, not a late note, which would wake nothing once it reported.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_report_wakes_a_spawned_thread_that_reported() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(work.join("dash")).unwrap();
  std::fs::write(work.join("dash/index.html"), "<p>dash</p>").unwrap();
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("dash").display().to_string(), "name": "Dash"}),
    ),
    answer("Shown; done."),
    answer("Refreshed the data."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&Request::Detach {
      goal: "Make a dashboard".into(),
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
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!("not detached")
  };
  // It shows the app and reports.
  until("its report", || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    index
      .records(Some("report"))
      .unwrap()
      .into_iter()
      .any(|r| {
        index
          .edges_from(&r.id)
          .unwrap()
          .iter()
          .any(|e| e.rel == "thread" && e.dst.id.as_str() == thread)
      })
      .then_some(())
  })
  .await;
  tokio::time::sleep(Duration::from_millis(500)).await;
  let pinned = c
    .request(&Request::AppPin {
      what: thread.clone(),
      name: None,
      icon: None,
      fixed: None,
    })
    .await
    .unwrap();
  let Reply::Apps { apps, .. } = pinned else {
    panic!("not pinned: {pinned:?}")
  };
  let host = {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let all = user_apps(stitch_node::apps::pinned(&index).unwrap());
    stitch_node::apps::find_pinned(&all, &apps[0].id)
      .unwrap()
      .host
      .clone()
  };
  let r = node
    .serve_app(post(&host, "/_stitch/report", json!({"refresh": true})))
    .await;
  assert_eq!(r.status, 200, "{}", text(&r));
  assert!(text(&r).contains("starts a turn"), "{}", text(&r));
  let told = until("the report's turn", || {
    let bodies = sent.lock().unwrap();
    (bodies.len() == 3).then(|| bodies[2]["messages"].to_string())
  })
  .await;
  assert!(told.contains("[Pinned app \\\"Dash\\\""), "{told}");
  tokio::time::sleep(Duration::from_secs(1)).await;
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

async fn pin_following(c: &mut Client, what: String) -> String {
  let Reply::Apps { apps, .. } = c
    .request(&Request::AppPin {
      what,
      name: None,
      icon: None,
      fixed: Some(false),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  apps[0].id.clone()
}

async fn listed(c: &mut Client) -> Vec<stitch_node::api::AppInfo> {
  let Reply::Apps { apps, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!()
  };
  apps
    .into_iter()
    .filter(|a| a.builtin.as_deref() != Some("client"))
    .collect()
}

/// Two pinned apps, one following its thread to a newer version:
/// unpinning either leaves the other pinned (the phone once unpinned the
/// other one).
#[tokio::test(flavor = "multi_thread")]
async fn unpinning_one_app_leaves_the_others() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(work.join("dash")).unwrap();
  std::fs::create_dir_all(work.join("mount")).unwrap();
  std::fs::write(work.join("dash/index.html"), "<p>v1</p>").unwrap();
  std::fs::write(work.join("mount/index.html"), "<p>mount</p>").unwrap();
  let show = |id: &str| {
    tool_use(
      id,
      "show_app",
      json!({"path": work.join("dash").display().to_string(), "name": "Dashboard"}),
    )
  };
  let (base, _sent) = serve(vec![
    show("c1"),
    answer("Shown."),
    show("c2"),
    answer("Changed."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "a dashboard", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let mount = stitch_node::apps::snapshot(
    &blobs,
    &work.join("mount"),
    "Camera wedge mount",
    false,
    None,
  )
  .unwrap();
  let mount_id = pin_following(&mut c, mount.manifest.to_string()).await;
  let dash_id = pin_following(&mut c, thread.clone()).await;
  let v1 = listed(&mut c)
    .await
    .into_iter()
    .find(|a| a.id == dash_id)
    .unwrap()
    .version;
  // The thread shows a newer version: the dashboard's pin follows it.
  std::fs::write(work.join("dash/index.html"), "<p>v2</p>").unwrap();
  c.chat(&chat(Some(&thread), "change it", &work), |_| None)
    .await
    .unwrap();
  let apps = listed(&mut c).await;
  assert_eq!(apps.len(), 2, "{apps:?}");
  let dash = apps.iter().find(|a| a.id == dash_id).unwrap();
  assert!(!dash.fixed);
  assert_ne!(dash.version, v1, "the pin followed the newer version");

  // Unpinned by ID, the dashboard goes and the mount stays.
  assert!(matches!(
    c.request(&Request::AppUnpin {
      app: dash_id.clone()
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
  let apps = listed(&mut c).await;
  assert_eq!(
    apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
    [mount_id.as_str()],
    "{apps:?}"
  );
  // Pinned again (a new app), then the mount unpinned: the dashboard stays.
  let dash_id = pin_following(&mut c, thread.clone()).await;
  assert!(matches!(
    c.request(&Request::AppUnpin {
      app: mount_id.clone()
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
  let apps = listed(&mut c).await;
  assert_eq!(
    apps.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
    [dash_id.as_str()],
    "{apps:?}"
  );

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

fn json_of(r: &Response) -> Value {
  serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("{} {}", r.status, text(r)))
}

/// Live state for apps (design §2.10): refused until the user lets the app
/// read it; then threads (blocked with its exact command), attention and
/// jobs; the answer kept per app in its conversation, and per pin; change
/// events on a new question and on its answer; no model call while a page
/// listens.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_reads_live_state_once_the_user_lets_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(work.join("dash")).unwrap();
  std::fs::write(work.join("dash/index.html"), "<p>dash</p>").unwrap();
  let (base, sent) = serve(vec![
    tool_use(
      "c1",
      "show_app",
      json!({"path": work.join("dash").display().to_string(), "name": "Dash"}),
    ),
    answer("Shown."),
    tool_use(
      "p1",
      "shell_run",
      json!({"command": "git push origin main"}),
    ),
    answer("Not pushed: declined."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "a dashboard", &work), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  // The version the thread showed: pinned a moment to learn it.
  let version = {
    let Reply::Apps { apps, .. } = c
      .request(&Request::AppPin {
        what: thread.clone(),
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
    apps[0].version.parse::<stitch_records::BlobHash>().unwrap()
  };
  let host = stitch_node::apps::version_host(&version);

  // Not allowed yet: refused, saying the user may be asked.
  let r = node.serve_app(get(&host, "/_stitch/live/threads")).await;
  assert_eq!(r.status, 403);
  let refused = json_of(&r);
  assert_eq!(refused["consent"], "ask", "{refused}");
  assert!(
    refused["allow"]
      .as_str()
      .unwrap()
      .contains(&format!("stitch app allow-live \"Dash\" --thread {thread}")),
    "{refused}"
  );
  let page = text(&node.serve_app(get(&host, "/")).await);
  assert!(page.contains("\"consent\":\"ask\""), "{page}");

  // A detached thread blocked on an approval.
  let Reply::Detached(pusher) = c
    .request(&Request::Detach {
      goal: "Push it".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: Some(thread.clone()),
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
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the question", || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    (!index.attention().unwrap().is_empty()).then_some(())
  })
  .await;

  // The user lets it (the app in its conversation).
  let Reply::Lines(said) = c
    .request(&Request::AppLive {
      app: host.clone(),
      thread: None,
      allow: true,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(said[0].contains("may now see"), "{said:?}");
  let page = text(&node.serve_app(get(&host, "/")).await);
  assert!(page.contains("\"consent\":\"allowed\""), "{page}");

  let threads = json_of(&node.serve_app(get(&host, "/_stitch/live/threads")).await);
  let listed = threads["threads"].as_array().unwrap();
  assert_eq!(listed.len(), 1, "the app's thread's own: {threads}");
  let t = &listed[0];
  assert_eq!(t["id"], pusher.as_str());
  assert_eq!(t["state"], "blocked");
  assert_eq!(t["question"]["command"], "git push origin main");
  assert_eq!(t["parent"], thread.as_str());
  assert_eq!(t["node"], "mbp");
  let all = json_of(
    &node
      .serve_app(get(&host, "/_stitch/live/threads?all=1&status=blocked"))
      .await,
  );
  assert_eq!(all["threads"].as_array().unwrap().len(), 1, "{all}");
  let none = json_of(
    &node
      .serve_app(get(&host, "/_stitch/live/threads?all=1&status=working"))
      .await,
  );
  assert!(none["threads"].as_array().unwrap().is_empty(), "{none}");
  // Without `conversations`, `all` lists the detached threads alone: the
  // conversation that spawned it is not there, and its child's `parent`
  // names a row that is not listed.
  let all = json_of(
    &node
      .serve_app(get(&host, "/_stitch/live/threads?all=1"))
      .await,
  );
  let rows = all["threads"].as_array().unwrap();
  assert_eq!(rows.len(), 1, "{all}");
  assert_eq!(rows[0]["detached"], true);
  // With it: the whole forest, the parent a listed row, the same child.
  let forest = json_of(
    &node
      .serve_app(get(
        &host,
        "/_stitch/live/threads?all=1&conversations=1&limit=2000",
      ))
      .await,
  );
  let rows = forest["threads"].as_array().unwrap();
  let chat_row = rows
    .iter()
    .find(|r| r["id"] == thread.as_str())
    .unwrap_or_else(|| panic!("the conversation is listed: {forest}"));
  assert_eq!(chat_row["detached"], false);
  assert!(chat_row["parent"].is_null());
  let child = rows.iter().find(|r| r["id"] == pusher.as_str()).unwrap();
  assert_eq!(child["detached"], true);
  assert_eq!(child["parent"], thread.as_str(), "{forest}");
  assert_eq!(child["state"], "blocked");
  assert_eq!(child["question"]["command"], "git push origin main");
  for k in [
    "title",
    "goal",
    "node",
    "place",
    "started",
    "last_step",
    "model_calls",
    "cost",
    "last_words",
    "unread",
    "pinned",
  ] {
    assert!(child.get(k).is_some(), "{k}: {child}");
  }
  assert!(child["in_flight"].is_null(), "left out of this listing");
  assert_eq!(forest["more"], 0);
  let bad = node
    .serve_app(get(&host, "/_stitch/live/threads?status=asleep"))
    .await;
  assert_eq!(bad.status, 400);
  let attention = json_of(&node.serve_app(get(&host, "/_stitch/live/attention")).await);
  let q = &attention["questions"][0];
  assert_eq!(q["command"], "git push origin main", "{attention}");
  assert_eq!(q["thread"], pusher.as_str());
  assert_eq!(q["approval"], true);
  let question = q["id"].as_str().unwrap().to_owned();
  let jobs = json_of(&node.serve_app(get(&host, "/_stitch/live/jobs")).await);
  assert!(jobs["jobs"].is_array(), "{jobs}");
  // Schedules: what fires next, and on its own node what its timer did;
  // paused by the user is not paused by the node.
  let schedule = |input: Value| Request::Schedule {
    input: input.to_string(),
    cwd: None,
  };
  let Reply::Lines(made) = c
    .request(&schedule(json!({"action": "create", "name": "nightly", "goal": "Run the benchmarks", "every": "0 3 * * *"})))
    .await
    .unwrap()
  else {
    panic!("not made");
  };
  let id = made[0]
    .strip_prefix("Made schedule ")
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{made:?}"))
    .to_owned();
  let read = json_of(&node.serve_app(get(&host, "/_stitch/live/schedules")).await);
  let x = &read["schedules"][0];
  assert_eq!(x["id"], id.as_str(), "{read}");
  assert_eq!(x["name"], "nightly");
  assert_eq!(x["state"], "active");
  assert_eq!(x["every"], "0 3 * * *");
  assert_eq!(x["self_wake"], false);
  assert_eq!(x["next"].as_array().unwrap().len(), 3, "{read}");
  assert_eq!(x["here"]["fired"], 0, "{read}");
  c.request(&schedule(json!({"action": "pause", "schedule": id})))
    .await
    .unwrap();
  let read = json_of(&node.serve_app(get(&host, "/_stitch/live/schedules")).await);
  let x = &read["schedules"][0];
  assert_eq!(x["state"], "paused", "{read}");
  assert_eq!(x["paused_by_node"], false);
  assert_eq!(x["next"], json!([]));
  let post = node
    .serve_app(post(&host, "/_stitch/live/threads", json!({})))
    .await;
  assert_eq!(post.status, 405, "read-only");

  // Pinned, it takes its conversation's answer; denied on the pin, the
  // pin is refused, the version not; allowed again, kept.
  let Reply::Apps { apps, .. } = c
    .request(&Request::AppPin {
      what: thread.clone(),
      name: None,
      icon: None,
      fixed: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(apps[0].live, Some(true));
  let pin_host = stitch_node::apps::pinned_host(&apps[0].id.parse().unwrap());
  assert_eq!(
    node
      .serve_app(get(&pin_host, "/_stitch/live/attention"))
      .await
      .status,
    200
  );
  c.request(&Request::AppLive {
    app: "Dash".into(),
    thread: None,
    allow: false,
  })
  .await
  .unwrap();
  let r = node
    .serve_app(get(&pin_host, "/_stitch/live/attention"))
    .await;
  assert_eq!(r.status, 403);
  assert_eq!(json_of(&r)["consent"], "denied");
  assert_eq!(
    node
      .serve_app(get(&host, "/_stitch/live/attention"))
      .await
      .status,
    200
  );
  c.request(&Request::AppLive {
    app: "Dash".into(),
    thread: None,
    allow: true,
  })
  .await
  .unwrap();
  let Reply::Apps { apps, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!()
  };
  assert_eq!(apps[0].live, Some(true), "kept on the pin");

  // A page left listening: no model call.
  let calls = sent.lock().unwrap().len();
  let state = json_of(&node.serve_app(get(&host, "/_stitch/state")).await);
  let page = text(&node.serve_app(get(&host, "/")).await);
  let live_generation: u64 = page
    .split("\"live\":{")
    .nth(1)
    .and_then(|r| r.split("\"generation\":").nth(1))
    .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
    .and_then(|n| n.parse().ok())
    .unwrap_or_else(|| panic!("{page}"));
  let events = |generation: u64, live: u64| {
    node.serve_app(get(
      &host,
      &format!("/_stitch/events?generation={generation}&live={live}"),
    ))
  };
  let quiet = tokio::time::timeout(
    Duration::from_secs(3),
    events(state["generation"].as_u64().unwrap(), live_generation),
  )
  .await;
  if let Ok(r) = quiet {
    // Something moved (the watch's first look); nothing called a model.
    assert_eq!(r.status, 200);
  }
  assert_eq!(sent.lock().unwrap().len(), calls, "no model call");

  // Answered: attention changes, and the page hears of it.
  let now = json_of(&events(u64::MAX, u64::MAX).await);
  let before = now["live"].clone();
  let waiting = tokio::spawn(events(
    now["generation"].as_u64().unwrap(),
    before["generation"].as_u64().unwrap(),
  ));
  tokio::time::sleep(Duration::from_millis(300)).await;
  c.request(&Request::Answer {
    question: question.clone(),
    allow: false,
    reason: Some("not now".into()),
    always: false,
  })
  .await
  .unwrap();
  // Woken by the answer's records (state) or by live state: until live
  // state moves.
  let mut heard = json_of(
    &tokio::time::timeout(Duration::from_secs(10), waiting)
      .await
      .unwrap()
      .unwrap(),
  );
  for _ in 0..20 {
    if heard["live"]["attention"] != before["attention"] {
      break;
    }
    heard = json_of(
      &tokio::time::timeout(
        Duration::from_secs(10),
        events(
          heard["generation"].as_u64().unwrap(),
          heard["live"]["generation"].as_u64().unwrap(),
        ),
      )
      .await
      .unwrap(),
    );
  }
  let after = &heard["live"];
  assert_ne!(after["attention"], before["attention"], "{before} {after}");
  let attention = json_of(&node.serve_app(get(&host, "/_stitch/live/attention")).await);
  assert!(attention["questions"].as_array().unwrap().is_empty());

  tokio::time::sleep(Duration::from_secs(1)).await;
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}
