//! The web client's WebSocket (design §2.24): the gate at the upgrade (Host,
//! Origin, cookie, and the client's origin alone), the Hello rule, a
//! Request/Reply round trip, a chat's stream and a followed thread.

use std::path::Path;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig, ShellSettings};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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
    // Nothing asks: a throwaway node must not wait on an approval (or on
    // the macOS sandbox, which an agent sandbox refuses to nest).
    shell: Some(ShellSettings {
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

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

/// Tries the upgrade at `host`, with these headers; the socket, or the
/// status it was refused with.
async fn open(
  port: u16,
  host: &str,
  origin: Option<&str>,
  cookie: Option<&str>,
) -> Result<Ws, u16> {
  let mut req = format!("ws://127.0.0.1:{port}/_stitch/ws")
    .into_client_request()
    .unwrap();
  let h = req.headers_mut();
  h.insert("host", host.parse().unwrap());
  if let Some(o) = origin {
    h.insert("origin", o.parse().unwrap());
  }
  if let Some(c) = cookie {
    h.insert("cookie", c.parse().unwrap());
  }
  let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  match tokio_tungstenite::client_async(req, tcp).await {
    Ok((ws, _)) => Ok(ws),
    Err(tokio_tungstenite::tungstenite::Error::Http(r)) => Err(r.status().as_u16()),
    Err(e) => panic!("{e}"),
  }
}

async fn send(ws: &mut Ws, v: Value) {
  ws.send(Message::text(v.to_string())).await.unwrap();
}

async fn next(ws: &mut Ws) -> Value {
  loop {
    match tokio::time::timeout(std::time::Duration::from_secs(20), ws.next())
      .await
      .expect("a frame in time")
    {
      Some(Ok(Message::Text(t))) => return serde_json::from_str(t.as_str()).unwrap(),
      Some(Ok(Message::Close(_))) | None => return json!("closed"),
      Some(Ok(_)) => {}
      Some(Err(e)) => panic!("{e}"),
    }
  }
}

fn host_of(url: &str) -> String {
  url
    .strip_prefix("http://")
    .unwrap()
    .split('.')
    .next()
    .unwrap()
    .to_owned()
}

#[tokio::test]
async fn the_web_clients_websocket() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  std::fs::create_dir_all(d.join("work")).unwrap();
  let (base, _) = serve(vec![answer("Hi there.")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // The client is pinned at start, reached by name, and not in the app
  // list devices show; the dashboard is another pinned app.
  let Reply::Apps { apps, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!("no apps")
  };
  assert!(
    apps.iter().all(|a| a.builtin.as_deref() != Some("client")),
    "the client is not listed: {apps:?}"
  );
  let Reply::App { url, .. } = c
    .request(&Request::AppOpen {
      what: "Stitch".into(),
    })
    .await
    .unwrap()
  else {
    panic!("the client is pinned")
  };
  let client_host = host_of(url.as_deref().unwrap());
  let Reply::Apps { apps, .. } = c.request(&Request::Dashboard).await.unwrap() else {
    panic!("no dashboard")
  };
  let dash_host = host_of(apps[0].url.as_deref().unwrap());
  let token = std::fs::read_to_string(data.join("apps.token")).unwrap();
  let cookie = format!("stitch={}", token.trim());
  let host = format!("{client_host}.localhost:{port}");
  let origin = format!("http://{host}");

  // Refused at the upgrade.
  assert_eq!(
    open(
      port,
      &format!("evil.example:{port}"),
      Some("http://evil.example"),
      Some(&cookie)
    )
    .await
    .err(),
    Some(421),
    "another site's name (DNS rebinding)"
  );
  assert_eq!(
    open(port, &host, Some("http://evil.example"), Some(&cookie))
      .await
      .err(),
    Some(403),
    "a page of another site"
  );
  assert_eq!(
    open(port, &host, None, Some(&cookie)).await.err(),
    Some(403),
    "no Origin"
  );
  assert_eq!(
    open(port, &host, Some(&origin), None).await.err(),
    Some(403),
    "no cookie"
  );
  assert_eq!(
    open(port, &host, Some(&origin), Some("stitch=nope"))
      .await
      .err(),
    Some(403),
    "a wrong cookie"
  );
  let dash = format!("{dash_host}.localhost:{port}");
  assert_eq!(
    open(port, &dash, Some(&format!("http://{dash}")), Some(&cookie))
      .await
      .err(),
    Some(403),
    "an app that is not the client"
  );
  let shown = format!("{}.{}.localhost:{port}", "a".repeat(32), "b".repeat(32));
  assert_eq!(
    open(
      port,
      &shown,
      Some(&format!("http://{shown}")),
      Some(&cookie)
    )
    .await
    .err(),
    Some(403),
    "a version's origin (show_app)"
  );

  // Hello must come first, and match.
  let mut ws = open(port, &host, Some(&origin), Some(&cookie))
    .await
    .unwrap();
  send(&mut ws, json!({"id": 1, "request": "Status"})).await;
  assert!(
    next(&mut ws).await["error"]
      .as_str()
      .unwrap()
      .contains("Hello")
  );
  assert_eq!(next(&mut ws).await, json!("closed"));
  let mut ws = open(port, &host, Some(&origin), Some(&cookie))
    .await
    .unwrap();
  send(
    &mut ws,
    json!({"id": 1, "request": {"Hello": {"version": "0.0.0/local-0"}}}),
  )
  .await;
  let said = next(&mut ws).await;
  assert_eq!(said["id"], 1);
  assert_eq!(said["reply"]["Hello"]["version"], stitch_node::VERSION);
  assert_eq!(next(&mut ws).await, json!("closed"));

  // A request and its reply; ids correlate.
  let mut ws = open(port, &host, Some(&origin), Some(&cookie))
    .await
    .unwrap();
  send(
    &mut ws,
    json!({"id": 7, "request": {"Hello": {"version": stitch_node::VERSION}}}),
  )
  .await;
  let said = next(&mut ws).await;
  assert_eq!(
    (
      said["id"].as_u64(),
      said["reply"]["Hello"]["version"].as_str()
    ),
    (Some(7), Some(stitch_node::VERSION))
  );
  send(&mut ws, json!({"id": 8, "request": "Status"})).await;
  send(&mut ws, json!({"id": 9, "request": "Apps"})).await;
  let (a, b) = (next(&mut ws).await, next(&mut ws).await);
  assert_eq!(a["id"], 8);
  assert!(a["reply"]["Status"]["node"].is_string(), "{a}");
  assert_eq!(b["id"], 9, "{b}");
  send(&mut ws, json!({"id": 10, "request": {"Nonsense": 1}})).await;
  let bad = next(&mut ws).await;
  assert!(bad["error"].is_string() && bad["id"] == 10, "{bad}");

  // Requests in flight together are told apart by construction: a chat
  // that fails, and a request that answers, sent back to back.
  let bad = serde_json::to_value(chat(Some("no-such-thread"), "x", &d.join("work"))).unwrap();
  send(&mut ws, json!({"id": 40, "request": bad})).await;
  send(&mut ws, json!({"id": 41, "request": "Status"})).await;
  send(
    &mut ws,
    json!({"id": 42, "request": {"Transcript": {"thread": "nonsense"}}}),
  )
  .await;
  send(&mut ws, json!({"id": 43, "request": "Status"})).await;
  let mut got = std::collections::BTreeMap::new();
  while got.len() < 4 {
    let m = next(&mut ws).await;
    if let Some(id) = m["id"].as_u64() {
      assert!(got.insert(id, m).is_none(), "one frame per request");
    }
  }
  assert!(got[&40]["reply"]["Error"].is_string(), "{}", got[&40]);
  assert!(
    got[&41]["reply"]["Status"]["node"].is_string(),
    "{}",
    got[&41]
  );
  assert!(got[&42]["reply"]["Error"].is_string(), "{}", got[&42]);
  assert!(
    got[&43]["reply"]["Status"]["node"].is_string(),
    "{}",
    got[&43]
  );

  // A thread's turn streams: the chat's replies come with its id, and a
  // follower of the thread is told where the feed stands.
  let request = serde_json::to_value(chat(None, "hello", &d.join("work"))).unwrap();
  send(&mut ws, json!({"id": 20, "request": request})).await;
  let mut thread = None;
  let mut text = String::new();
  loop {
    let m = next(&mut ws).await;
    if m.get("event").is_some() || m.get("changed").is_some() {
      continue;
    }
    assert_eq!(m["id"], 20, "{m}");
    if let Some(t) = m["reply"]["Thread"].as_str() {
      thread = Some(t.to_owned());
      send(&mut ws, json!({"follow": {"thread": t}})).await;
    }
    if let Some(t) = m["reply"]["Text"].as_str() {
      text.push_str(t);
    }
    if m["reply"].get("ChatDone").is_some() {
      break;
    }
    assert!(m["reply"].get("Error").is_none(), "{m}");
  }
  assert_eq!(text, "Hi there.");
  let thread = thread.unwrap();
  // The list and the transcript are requests of the protocol.
  send(
    &mut ws,
    json!({"id": 30, "request": {"Threads": {"status": null, "since": null, "limit": null}}}),
  )
  .await;
  send(
    &mut ws,
    json!({"id": 31, "request": {"Transcript": {"thread": thread, "after": null}}}),
  )
  .await;
  let (mut listed, mut transcript, mut changed) = (None, None, false);
  while listed.is_none() || transcript.is_none() || !changed {
    let m = next(&mut ws).await;
    match m["id"].as_u64() {
      Some(30) => listed = Some(m),
      Some(31) => transcript = Some(m),
      _ if m.get("changed").is_some() => changed = true,
      _ => {}
    }
  }
  let one = |m: &Value| -> Value {
    serde_json::from_str(m["reply"]["Lines"][0].as_str().unwrap()).unwrap()
  };
  let listed = one(&listed.unwrap());
  assert!(
    listed["threads"]
      .as_array()
      .unwrap()
      .iter()
      .any(|t| t["id"] == thread),
    "{listed}"
  );
  let transcript = one(&transcript.unwrap());
  let entries = transcript["entries"].as_array().unwrap();
  assert_eq!(entries[0]["kind"], "user");
  assert_eq!(entries[0]["text"], "hello");
  assert_eq!(entries.last().unwrap()["kind"], "assistant");
  assert_eq!(entries.last().unwrap()["text"], "Hi there.");
  assert!(transcript["next"].as_u64().unwrap() >= 2);
  send(&mut ws, json!({"unfollow": {"thread": thread}})).await;
  node.stop().await.unwrap();
}

/// The model calls `shell_run` with `command`.
fn tool_use(id: &str, command: &str) -> String {
  let input = json!({"command": command}).to_string();
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": "shell_run", "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input}}),
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

fn git(dir: &Path, args: &[&str]) {
  let out = std::process::Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {args:?}: {}",
    String::from_utf8_lossy(&out.stderr)
  );
}

/// The web client's socket on `data`'s node, greeted.
async fn client_socket(data: &Path, port: u16, c: &mut Client) -> Ws {
  let Reply::App { url, .. } = c
    .request(&Request::AppOpen {
      what: "Stitch".into(),
    })
    .await
    .unwrap()
  else {
    panic!("the client is pinned")
  };
  let host = format!("{}.localhost:{port}", host_of(url.as_deref().unwrap()));
  let origin = format!("http://{host}");
  let token = std::fs::read_to_string(data.join("apps.token")).unwrap();
  let cookie = format!("stitch={}", token.trim());
  let mut ws = open(port, &host, Some(&origin), Some(&cookie))
    .await
    .unwrap();
  send(
    &mut ws,
    json!({"id": 1, "request": {"Hello": {"version": stitch_node::VERSION}}}),
  )
  .await;
  assert_eq!(next(&mut ws).await["id"], 1);
  ws
}

/// The frame answering request `id`, skipping pushes and events.
async fn reply_of(ws: &mut Ws, id: u64) -> Value {
  loop {
    let m = next(ws).await;
    if m["id"] == id {
      return m;
    }
    assert_ne!(m, json!("closed"));
  }
}

/// `Reply::Lines([json])`, parsed.
fn one_line(m: &Value) -> Value {
  serde_json::from_str(
    m["reply"]["Lines"][0]
      .as_str()
      .unwrap_or_else(|| panic!("{m}")),
  )
  .unwrap()
}

/// A chat on the socket as the web client sends it; the thread and what
/// its shell commands printed.
async fn web_chat(
  ws: &mut Ws,
  id: u64,
  thread: Option<&str>,
  text: &str,
  place: Option<&str>,
) -> (String, String) {
  let req = json!({"Chat": {
    "thread": thread, "text": text, "attachments": [], "model": null, "repo": null, "cwd": null,
    "global": true, "env": [], "tools": [], "skills": [], "place": place, "memory": null, "end_user": null,
  }});
  send(ws, json!({"id": id, "request": req})).await;
  let (mut thread, mut out) = (None, String::new());
  loop {
    let m = next(ws).await;
    if m["id"] != id {
      assert_ne!(m, json!("closed"));
      continue;
    }
    assert!(
      m.get("error").is_none() && m["reply"].get("Error").is_none(),
      "{m}"
    );
    if let Some(t) = m["reply"]["Thread"].as_str() {
      thread = Some(t.to_owned());
    }
    if let Some(o) = m["reply"]["ToolResult"]["output"].as_str() {
      out.push_str(o);
    }
    if m["reply"].get("ChatDone").is_some() {
      return (thread.unwrap(), out);
    }
  }
}

/// A new thread started from the web client in a place its picker lists
/// (`Live places`, `Chat.place`) belongs to that place, and its shell
/// starts there: a project's checkout, a folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_thread_from_the_web_client_runs_in_the_place_picked() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  std::fs::create_dir_all(&app).unwrap();
  git(&app, &["init", "--quiet"]);
  git(
    &app,
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "first",
    ],
  );
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();
  let (base, _) = serve(vec![
    answer("in the project"),
    answer("in the folder"),
    tool_use("t1", "pwd"),
    answer("ran in the project"),
    tool_use("t2", "pwd"),
    answer("ran in the folder"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  // Threads from a terminal in each, so the picker lists them.
  for (cwd, text) in [(&app, "here"), (&plain, "there")] {
    let mut req = chat(None, text, cwd);
    if let Request::Chat { global, .. } = &mut req {
      *global = false;
    }
    let done = c.chat(&req, |_| None).await.unwrap();
    assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  }
  let mut ws = client_socket(&data, port, &mut c).await;
  send(
    &mut ws,
    json!({"id": 2, "request": {"Live": {"what": "places", "query": null}}}),
  )
  .await;
  let places = one_line(&reply_of(&mut ws, 2).await)["places"].clone();
  let key_of = |prefix: &str| -> (String, String) {
    let p = places
      .as_array()
      .unwrap()
      .iter()
      .find(|p| p["key"].as_str().unwrap().starts_with(prefix))
      .unwrap_or_else(|| panic!("no {prefix} place in {places}"));
    (
      p["key"].as_str().unwrap().to_owned(),
      p["label"].as_str().unwrap().to_owned(),
    )
  };
  let mut id = 10;
  for (prefix, dir) in [("project:", &app), ("folder:", &plain)] {
    let (key, label) = key_of(prefix);
    id += 1;
    let (thread, out) = web_chat(&mut ws, id, None, "pwd please", Some(&key)).await;
    assert!(
      out.trim().ends_with(&format!("\n{}", dir.display())),
      "the shell of a thread in {key} starts there: {out}"
    );
    id += 1;
    send(
      &mut ws,
      json!({"id": id, "request": {"Threads": {"status": null, "since": null, "limit": null}}}),
    )
    .await;
    let listed = one_line(&reply_of(&mut ws, id).await);
    let row = listed["threads"]
      .as_array()
      .unwrap()
      .iter()
      .find(|t| t["id"] == thread.as_str())
      .unwrap_or_else(|| panic!("{thread} not listed"))
      .clone();
    assert_eq!(row["place"], label.as_str(), "{row}");
  }
  node.stop().await.unwrap();
}

/// The next `turn` status of a followed thread (`{"running", "queued"}`).
async fn turn_status(ws: &mut Ws) -> Value {
  loop {
    let m = next(ws).await;
    assert_ne!(m, json!("closed"));
    if let Some(t) = m["event"].get("turn") {
      return t.clone();
    }
  }
}

/// A message sent from the web client shows at once where its turn
/// stands, from the node's own state: a follower is told the thread's
/// status when it follows and on each change (a turn accepted and not
/// begun is `queued`, as is one waiting for the loop running), the list
/// says the thread works from the moment the message is taken, and a
/// `Send` is never held or refused behind a flood of refreshes.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_sent_from_the_web_client_shows_its_turn_at_once() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![
    answer("first"),
    tool_use("t1", "sleep 2"),
    answer("one done"),
    answer("two done"),
    answer("three done"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut ws = client_socket(&data, port, &mut c).await;
  let (thread, _) = web_chat(&mut ws, 2, None, "hello", None).await;

  // Followed: where it stands, at once.
  send(&mut ws, json!({"follow": {"thread": thread}})).await;
  assert_eq!(
    turn_status(&mut ws).await,
    json!({"running": false, "queued": 0})
  );
  // A message: its turn is accepted at once (queued until its loop
  // begins), then runs.
  let sent = |id: u64, text: &str, interject: bool| {
    json!({"id": id, "request": {"Send": {
      "thread": thread, "text": text, "from": null, "keep": false, "interject": interject,
    }}})
  };
  send(&mut ws, sent(3, "one", true)).await;
  let (mut said, mut ran) = (None, false);
  while said.is_none() || !ran {
    let m = next(&mut ws).await;
    assert_ne!(m, json!("closed"));
    if m["id"] == 3 {
      said = Some(
        m["reply"]["Lines"][0]
          .as_str()
          .unwrap_or_else(|| panic!("{m}"))
          .to_owned(),
      );
    } else if let Some(t) = m["event"].get("turn") {
      assert!(
        t["running"] == true || t["queued"] == 1,
        "the turn is accepted, then runs: {t}"
      );
      ran = t["running"] == true;
    }
  }
  assert!(said.unwrap().contains("starts a turn"));
  // Another while it runs, as a turn of its own: queued behind it, and
  // the list says the thread works.
  send(&mut ws, sent(4, "two", false)).await;
  assert_eq!(
    turn_status(&mut ws).await,
    json!({"running": true, "queued": 1})
  );
  send(
    &mut ws,
    json!({"id": 5, "request": {"Threads": {"status": null, "since": null, "limit": null}}}),
  )
  .await;
  let listed = one_line(&reply_of(&mut ws, 5).await);
  let row = listed["threads"]
    .as_array()
    .unwrap()
    .iter()
    .find(|t| t["id"] == thread.as_str())
    .unwrap()
    .clone();
  assert_eq!(row["state"], "working", "{row}");
  // Both turns run, one after the other, and end.
  loop {
    let t = turn_status(&mut ws).await;
    if t == json!({"running": false, "queued": 0}) {
      break;
    }
  }
  send(
    &mut ws,
    json!({"id": 6, "request": {"Transcript": {"thread": thread, "after": null}}}),
  )
  .await;
  let said: Vec<String> = one_line(&reply_of(&mut ws, 6).await)["entries"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|e| e["kind"] == "user")
    .map(|e| e["text"].as_str().unwrap().to_owned())
    .collect();
  assert_eq!(said, ["hello", "one", "two"]);

  // A flood of refreshes, then a message: the reads past the bound are
  // refused, the message is not, nor held behind them.
  const FLOOD: u64 = 300;
  for id in 100..100 + FLOOD {
    send(
      &mut ws,
      json!({"id": id, "request": {"Threads": {"status": null, "since": null, "limit": 300}}}),
    )
    .await;
  }
  send(&mut ws, sent(7, "three", true)).await;
  let (mut reads, mut busy) = (0, 0);
  let answered = loop {
    let m = next(&mut ws).await;
    assert_ne!(m, json!("closed"));
    if m["id"] == 7 {
      break m;
    }
    if m["reply"]["Lines"].is_array() {
      reads += 1;
    } else if m["error"].as_str().is_some_and(|e| e.starts_with("busy")) {
      busy += 1;
    }
  };
  assert!(
    answered["reply"]["Lines"][0]
      .as_str()
      .is_some_and(|l| l.starts_with("sent to thread")),
    "{answered}"
  );
  assert!(busy > 0, "the flood passed the bound of reads");
  assert!(
    reads < FLOOD - busy,
    "the message was answered before the reads waiting ({reads} of them answered first)"
  );
  node.stop().await.unwrap();
}

/// `Threads { thread }` gives that thread's row whatever the limit: the
/// web client opening a thread older than its sidebar's window (a search
/// hit) shows it instead of falling back to the home view.
#[tokio::test(flavor = "multi_thread")]
async fn one_threads_row_is_read_past_the_lists_limit() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("one"), answer("two"), answer("three")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut threads = Vec::new();
  for text in ["first", "second", "third"] {
    let done = c.chat(&chat(None, text, &d), |_| None).await.unwrap();
    let Reply::ChatDone { thread, .. } = done else {
      panic!("{done:?}")
    };
    threads.push(thread);
  }
  let list = |limit: Option<u32>, thread: Option<&str>| Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit,
    thread: thread.map(str::to_owned),
    place: None,
  };
  let rows = |r: Reply| -> Value {
    let Reply::Lines(l) = r else { panic!("{r:?}") };
    serde_json::from_str(&l[0]).unwrap()
  };
  let ids = |v: &Value| -> Vec<String> {
    v["threads"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| t["id"].as_str().unwrap().to_owned())
      .collect()
  };
  // The window: the latest only; the first thread is past it.
  let window = rows(c.request(&list(Some(1), None)).await.unwrap());
  assert_eq!(ids(&window), [threads[2].clone()], "{window}");
  assert_eq!(window["more"], 2, "{window}");
  // Asked by its ID, its row comes, with the limit as low.
  let one = rows(c.request(&list(Some(1), Some(&threads[0]))).await.unwrap());
  assert_eq!(ids(&one), [threads[0].clone()], "{one}");
  assert_eq!(one["more"], 0, "{one}");
  assert_eq!(one["threads"][0]["goal"], "first", "{one}");
  // On the web client's socket too (as it sends it, JSON).
  let mut ws = client_socket(&data, port, &mut c).await;
  send(
    &mut ws,
    json!({"id": 2, "request": {"Threads": {"status": null, "since": null, "limit": 1, "thread": threads[1]}}}),
  )
  .await;
  let got = one_line(&reply_of(&mut ws, 2).await);
  assert_eq!(ids(&got), [threads[1].clone()], "{got}");
  // An unknown thread: no row.
  let none = rows(
    c.request(&list(None, Some("2020-01-01T000000Z-nowhere-zzzz")))
      .await
      .unwrap(),
  );
  assert!(ids(&none).is_empty(), "{none}");
  node.stop().await.unwrap();
}

/// A compaction on the detached path (a message the web client sends,
/// `Send`) reads as one conversation (design §2.15, "One conversation"): a
/// follower of the old thread is told (`Compacted`) and gets the new
/// thread's turn under the ID it followed; the list shows the chain as one
/// row (its newest, with the old one's pin and title, the old one in
/// `earlier`); the old ID resolves to the newest; the transcript marks the
/// compaction with a divider entry.
#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_reads_as_one_conversation() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![
    answer("Hi."),
    answer("SUMMARY-WORDS"),
    answer("Second."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  // Its first step sends 80 tokens of a 90-token window: the next turn
  // compacts first.
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(90);
  config.threads = Some(stitch_node::config::ThreadSettings {
    compact_at: Some(80),
    compact_keep: Some(2),
    titles: Some(false),
    ..Default::default()
  });
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut ws = client_socket(&data, port, &mut c).await;
  let (old, _) = web_chat(&mut ws, 2, None, "hello", None).await;
  // Pinned and named before it is compacted: both carry over.
  for r in [
    Request::Pin {
      thread: old.clone(),
      on: true,
    },
    Request::Rename {
      thread: old.clone(),
      title: "My talk".into(),
    },
  ] {
    assert!(matches!(c.request(&r).await.unwrap(), Reply::Ok));
  }

  send(&mut ws, json!({"follow": {"thread": old}})).await;
  send(
    &mut ws,
    json!({"id": 3, "request": {"Send": {
      "thread": old, "text": "again", "from": null, "keep": false, "interject": true,
    }}}),
  )
  .await;
  let (mut compacted, mut text, mut ended) = (None, String::new(), false);
  while !ended {
    let m = next(&mut ws).await;
    assert_ne!(m, json!("closed"));
    let Some(e) = m["event"].get("event") else {
      continue;
    };
    assert_eq!(m["event"]["thread"], old.as_str(), "said as followed: {m}");
    if let Some(c) = e.get("Compacted") {
      compacted = Some(c.clone());
    } else if let Some(t) = e["Text"].as_str() {
      text.push_str(t);
    } else if e == "Ended" && compacted.is_some() {
      ended = true;
    }
  }
  let compacted = compacted.unwrap();
  let new = compacted["to"].as_str().unwrap().to_owned();
  assert_ne!(new, old);
  assert_eq!(compacted["from"], old.as_str(), "{compacted}");
  assert_eq!(compacted["window"], 90, "{compacted}");
  assert!(compacted["tokens"].as_u64().is_some(), "{compacted}");
  assert_eq!(compacted["kept"], 0, "summarized whole: {compacted}");
  assert!(
    compacted["summary"]
      .as_str()
      .unwrap()
      .contains("SUMMARY-WORDS")
  );
  assert!(text.contains("Second."), "the new thread's turn: {text}");

  let list = |thread: Option<&str>| Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit: None,
    thread: thread.map(str::to_owned),
    place: None,
  };
  let rows = |r: Reply| -> Vec<Value> {
    let Reply::Lines(l) = r else { panic!("{r:?}") };
    let v: Value = serde_json::from_str(&l[0]).unwrap();
    v["threads"].as_array().unwrap().clone()
  };
  // One row: the newest, with the chain's pin and title.
  let all = rows(c.request(&list(None)).await.unwrap());
  assert_eq!(all.len(), 1, "{all:?}");
  assert_eq!(all[0]["id"], new.as_str());
  assert_eq!(all[0]["earlier"], json!([old]));
  assert_eq!(all[0]["pinned"], true);
  assert_eq!(all[0]["title"], "My talk");
  // The old ID gives the newest's row.
  let one = rows(c.request(&list(Some(&old))).await.unwrap());
  assert_eq!(one.len(), 1);
  assert_eq!(one[0]["id"], new.as_str());
  // Unpinned on the newest, it is unpinned; pinned by the old ID, pinned.
  for (on, thread) in [(false, &new), (true, &old)] {
    c.request(&Request::Pin {
      thread: thread.clone(),
      on,
    })
    .await
    .unwrap();
    let all = rows(c.request(&list(None)).await.unwrap());
    assert_eq!(all[0]["pinned"], on, "{all:?}");
  }
  // The transcript: the old thread's, a divider, the new one's.
  let transcript = |thread: &str| Request::Transcript {
    thread: thread.to_owned(),
    after: None,
    before: None,
    tail: None,
  };
  let Reply::Lines(l) = c.request(&transcript(&new)).await.unwrap() else {
    panic!()
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  assert_eq!(t["current"], new.as_str());
  let entries = t["entries"].as_array().unwrap();
  let kinds: Vec<&str> = entries
    .iter()
    .map(|e| e["kind"].as_str().unwrap())
    .collect();
  assert_eq!(
    kinds,
    ["user", "assistant", "compacted", "user", "assistant"],
    "{t}"
  );
  let divider = &entries[2];
  assert_eq!(divider["from"], old.as_str());
  assert_eq!(divider["to"], new.as_str());
  assert!(
    divider["said"]
      .as_str()
      .unwrap()
      .starts_with("compacted here ("),
    "{divider}"
  );
  assert!(
    entries[0].get("id").is_none_or(Value::is_null),
    "an earlier thread's message has no ID"
  );
  assert!(entries[4]["id"].is_string(), "the newest's has");
  assert!(t["forked_from"].is_null(), "a compaction is no fork: {t}");
  // In windows from the end (the web client opens a long conversation at
  // its end): cut between events, a divider staying with its event.
  let window = |before: Option<u64>, tail: u64| Request::Transcript {
    thread: new.clone(),
    after: None,
    before,
    tail: Some(tail),
  };
  let read = |r: Reply| -> Value {
    let Reply::Lines(l) = r else { panic!("{r:?}") };
    serde_json::from_str(&l[0]).unwrap()
  };
  let kinds_of = |t: &Value| -> Vec<String> {
    t["entries"]
      .as_array()
      .unwrap()
      .iter()
      .map(|e| e["kind"].as_str().unwrap().to_owned())
      .collect()
  };
  let last = read(c.request(&window(None, 1)).await.unwrap());
  assert_eq!(kinds_of(&last), ["assistant"], "{last}");
  assert_eq!(last["earlier"], true);
  let w = read(c.request(&window(None, 2)).await.unwrap());
  assert_eq!(kinds_of(&w), ["compacted", "user", "assistant"], "{w}");
  assert_eq!(w["start"], entries[2]["n"], "{w}");
  let before = read(c.request(&window(w["start"].as_u64(), 10)).await.unwrap());
  assert_eq!(kinds_of(&before), ["user", "assistant"], "{before}");
  assert_eq!(before["earlier"], false);
  let whole = read(c.request(&window(None, 10)).await.unwrap());
  assert_eq!(
    whole["entries"], t["entries"],
    "a window of it all is it all"
  );
  assert_eq!(whole["earlier"], false);
  // The old one, as it was: no divider, and where it goes on.
  let Reply::Lines(l) = c.request(&transcript(&old)).await.unwrap() else {
    panic!()
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  assert_eq!(t["current"], new.as_str());
  assert!(
    t["entries"]
      .as_array()
      .unwrap()
      .iter()
      .all(|e| e["kind"] != "compacted"),
    "{t}"
  );
  // A thread the old one started (its report goes to it) nests under the
  // newest, the row the list shows, not under the hidden old one. Written
  // while the node is stopped; it indexes it as it starts again.
  node.stop().await.unwrap();
  let child = {
    use stitch_records::{Author, Draft, NodeName, Ref, kinds};
    let node_name: NodeName = "mbp".parse().unwrap();
    RecordRoot::open(&home)
      .unwrap()
      .publish(
        &node_name,
        Draft::new(kinds::THREAD, Author::node(node_name.clone()))
          .unwrap()
          .field("detached", true)
          .unwrap()
          .reference("reports-to", Ref::to(old.parse().unwrap()))
          .unwrap()
          .body("# a helper\n"),
      )
      .unwrap()
      .id
      .to_string()
  };
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let one = rows(c.request(&list(Some(&child))).await.unwrap());
  assert_eq!(one.len(), 1, "{one:?}");
  assert_eq!(one[0]["id"], child.as_str());
  assert_eq!(one[0]["parent"], new.as_str(), "{one:?}");
  node.stop().await.unwrap();
}

/// The web client's sidebar sections: `places` counts every listed thread by
/// place, whatever the limit, and `place` lists one place's threads with the
/// limit applied after the filter (an old thread of a small place is reached).
#[tokio::test(flavor = "multi_thread")]
async fn the_threads_list_counts_places_and_filters_by_one() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("one"), answer("two"), answer("three")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let elsewhere = d.join("notes");
  std::fs::create_dir_all(elsewhere.join(".stitch")).unwrap();
  // The folder keeps its threads out of memory (design §2.9, "Off for a place").
  std::fs::write(elsewhere.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  // The oldest thread is in a folder; the two after it are global.
  let mut req = chat(None, "in a folder", &elsewhere);
  if let Request::Chat { global, .. } = &mut req {
    *global = false;
  }
  let mut ids = Vec::new();
  for r in [
    req,
    chat(None, "global one", &d),
    chat(None, "global two", &d),
  ] {
    let Reply::ChatDone { thread, .. } = c.chat(&r, |_| None).await.unwrap() else {
      panic!()
    };
    ids.push(thread);
  }
  let list = |limit: Option<u32>, place: Option<&str>| Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit,
    thread: None,
    place: place.map(str::to_owned),
  };
  let read = |r: Reply| -> Value {
    let Reply::Lines(l) = r else { panic!("{r:?}") };
    serde_json::from_str(&l[0]).unwrap()
  };
  let ids_of = |v: &Value| -> Vec<String> {
    v["threads"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| t["id"].as_str().unwrap().to_owned())
      .collect()
  };
  // The window holds one global thread, yet both places are counted.
  let window = read(c.request(&list(Some(1), None)).await.unwrap());
  assert_eq!(ids_of(&window), [ids[2].clone()], "{window}");
  let counts: Vec<(String, u64)> = window["places"]
    .as_array()
    .unwrap()
    .iter()
    .map(|p| {
      (
        p["place"].as_str().unwrap().to_owned(),
        p["count"].as_u64().unwrap(),
      )
    })
    .collect();
  assert_eq!(counts.len(), 2, "{window}");
  assert_eq!(counts.iter().map(|c| c.1).sum::<u64>(), 3, "{window}");
  let (small, _) = counts.iter().find(|c| c.1 == 1).expect("the folder");
  assert_ne!(small, "global", "{window}");
  assert!(counts.contains(&("global".to_owned(), 2)), "{window}");
  // `memory: false` says only where the place's settings do.
  for p in window["places"].as_array().unwrap() {
    if p["place"] == small.as_str() {
      assert_eq!(p["memory"], false, "{window}");
    } else {
      assert!(p.get("memory").is_none(), "{window}");
    }
  }
  // The folder's thread is past the window, and the place filter reaches it
  // with the limit as low.
  let only = read(c.request(&list(Some(1), Some(small))).await.unwrap());
  assert_eq!(ids_of(&only), [ids[0].clone()], "{only}");
  assert_eq!(only["more"], 0, "{only}");
  assert_eq!(only["places"], window["places"], "{only}");
  // The limit counts after the filter.
  let g = read(c.request(&list(Some(1), Some("global"))).await.unwrap());
  assert_eq!(ids_of(&g), [ids[2].clone()], "{g}");
  assert_eq!(g["more"], 1, "{g}");
  // A place nobody has lists nothing.
  let none = read(
    c.request(&list(None, Some("nowhere & ~/else")))
      .await
      .unwrap(),
  );
  assert!(ids_of(&none).is_empty(), "{none}");
  node.stop().await.unwrap();
}

/// The map's whole forest (`Threads { forest }`, design §2.25): the same
/// listing as the client's own, as lean rows (`in_flight` and `app` left out: null), with
/// a limit that goes past the usual 500 (unit-tested in `live::tests`).
#[tokio::test(flavor = "multi_thread")]
async fn the_forest_lists_every_conversation_as_lean_rows() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("one"), answer("two")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut ids = Vec::new();
  for r in [chat(None, "first", &d), chat(None, "second", &d)] {
    let Reply::ChatDone { thread, .. } = c.chat(&r, |_| None).await.unwrap() else {
      panic!()
    };
    ids.push(thread);
  }
  let list = |forest: Option<bool>, limit: u32| Request::Threads {
    forest,
    status: None,
    since: None,
    limit: Some(limit),
    thread: None,
    place: None,
  };
  let read = |r: Reply| -> Value {
    let Reply::Lines(l) = r else { panic!("{r:?}") };
    serde_json::from_str(&l[0]).unwrap()
  };
  let forest = read(c.request(&list(Some(true), 5000)).await.unwrap());
  let rows = forest["threads"].as_array().unwrap();
  let mut got: Vec<&str> = rows.iter().map(|t| t["id"].as_str().unwrap()).collect();
  got.sort();
  let mut want: Vec<&str> = ids.iter().map(String::as_str).collect();
  want.sort();
  assert_eq!(got, want, "{forest}");
  assert_eq!(forest["more"], 0, "{forest}");
  for t in rows {
    assert!(t["in_flight"].is_null() && t["app"].is_null(), "{t}");
  }
  // Without the flag the same threads are listed (the rows carry the costly fields when there are any).
  let plain = read(c.request(&list(None, 5000)).await.unwrap());
  assert_eq!(plain["threads"].as_array().unwrap().len(), 2, "{plain}");
  node.stop().await.unwrap();
}

/// Timing of the thread list with many threads (run by hand:
/// `cargo test -p stitch-node --test client_ws list_timing -- --ignored --nocapture`).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn list_timing() {
  let n: usize = std::env::var("N")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(600);
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("x"); n]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  for i in 0..n {
    c.chat(&chat(None, &format!("t{i}"), &d), |_| None)
      .await
      .unwrap();
  }
  let req = Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit: Some(300),
    thread: None,
    place: None,
  };
  let mut ms = Vec::new();
  for _ in 0..5 {
    let t = std::time::Instant::now();
    c.request(&req).await.unwrap();
    ms.push(t.elapsed().as_millis());
  }
  eprintln!("LIST_TIMING n={n} ms={ms:?}");
  node.stop().await.unwrap();
}

/// The thread list of a node already running on real data, read only (run
/// by hand: `STITCH_BENCH_DATA=~/.stitch cargo test -p stitch-node --test
/// client_ws list_timing_running -- --ignored --nocapture`).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn list_timing_running() {
  let data =
    std::path::PathBuf::from(std::env::var("STITCH_BENCH_DATA").expect("STITCH_BENCH_DATA"));
  let limit: u32 = std::env::var("LIMIT")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(500);
  let mut c = Client::connect(&data).await.unwrap();
  let req = Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit: Some(limit),
    thread: None,
    place: None,
  };
  let mut ms = Vec::new();
  for _ in 0..5 {
    let t = std::time::Instant::now();
    let r = c.request(&req).await.unwrap();
    let Reply::Lines(l) = r else { panic!() };
    let v: Value = serde_json::from_str(&l[0]).unwrap();
    ms.push((
      t.elapsed().as_millis(),
      v["threads"].as_array().unwrap().len(),
      l[0].len(),
    ));
  }
  eprintln!("LIST_TIMING_RUNNING limit={limit} (ms, rows, bytes)={ms:?}");
}

/// The read cursor of a conversation (design §2.25): never read is unread,
/// reading up to its latest activity clears it, newer activity brings it
/// back, an older `upto` does not move the cursor back, and a `Read` that
/// moves nothing writes nothing (these records sync to every node).
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_is_unread_after_its_cursor() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("one"), answer("two")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "first", &d), |_| None).await.unwrap()
  else {
    panic!()
  };
  let r0 = row(&mut c).await;
  assert_eq!(r0["unread"], true, "never read is unread: {r0}");
  read(&mut c, &thread, None).await.unwrap();
  let r1 = row(&mut c).await;
  assert_eq!(r1["unread"], false, "{r1}");
  // Moving nothing writes nothing.
  let files = |p: &Path| walkdir_count(p);
  let before = files(&d);
  read(&mut c, &thread, None).await.unwrap();
  read(&mut c, &thread, r1["last_step"].as_str())
    .await
    .unwrap();
  read(&mut c, &thread, Some("2001-01-01T00:00:00Z"))
    .await
    .unwrap();
  assert_eq!(files(&d), before, "no new record");
  // More activity: unread again; an `upto` short of it keeps it so.
  c.chat(&chat(Some(&thread), "second", &d), |_| None)
    .await
    .unwrap();
  let r2 = row(&mut c).await;
  assert_eq!(r2["unread"], true, "{r2}");
  read(&mut c, &thread, r1["last_step"].as_str())
    .await
    .unwrap();
  assert_eq!(row(&mut c).await["unread"], true);
  read(&mut c, &thread, r2["last_step"].as_str())
    .await
    .unwrap();
  assert_eq!(row(&mut c).await["unread"], false);
  assert!(read(&mut c, &thread, Some("tomorrow")).await.is_err());
  node.stop().await.unwrap();
}

async fn row(c: &mut Client) -> Value {
  let req = Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit: None,
    thread: None,
    place: None,
  };
  let Reply::Lines(l) = c.request(&req).await.unwrap() else {
    panic!()
  };
  let v: Value = serde_json::from_str(&l[0]).unwrap();
  v["threads"][0].clone()
}

async fn read(c: &mut Client, thread: &str, upto: Option<&str>) -> stitch_node::Result<Reply> {
  c.request(&Request::Read {
    thread: thread.to_owned(),
    upto: upto.map(str::to_owned),
  })
  .await
}

fn walkdir_count(p: &Path) -> usize {
  std::fs::read_dir(p).map_or(0, |rd| {
    rd.flatten()
      .map(|e| {
        let p = e.path();
        if p.is_dir() { walkdir_count(&p) } else { 1 }
      })
      .sum()
  })
}

/// Typing into a conversation says the user read what was there: the cursor
/// advances to its activity before the message (one record, and only when
/// that moves it), and the reply after it is unread.
#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_conversation_reads_what_was_there_but_not_the_reply() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![answer("one"), answer("two"), answer("three")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "first", &d), |_| None).await.unwrap()
  else {
    panic!()
  };
  let r1 = row(&mut c).await;
  assert_eq!(r1["unread"], true, "{r1}");
  // The user answers without having marked anything: the first reply is
  // read, the second is not.
  c.chat(&chat(Some(&thread), "second", &d), |_| None)
    .await
    .unwrap();
  let r2 = row(&mut c).await;
  assert_eq!(r2["unread"], true, "the reply after the message: {r2}");
  // The cursor already covers the first reply: reading up to it writes nothing.
  let before = walkdir_count(&d);
  read(&mut c, &thread, r1["last_step"].as_str())
    .await
    .unwrap();
  assert_eq!(walkdir_count(&d), before, "the cursor was already there");
  // Reading to the end, then typing again: the message adds no unread of its own
  // (its reply does), and a send into it writes the cursor once.
  read(&mut c, &thread, None).await.unwrap();
  assert_eq!(row(&mut c).await["unread"], false);
  let Reply::Lines(_) = c
    .request(&Request::Send {
      thread: thread.clone(),
      text: "third".into(),
      from: None,
      keep: false,
      interject: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  // Its turn runs on its own; the thread ends up with the reply unread.
  for _ in 0..100 {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let r = row(&mut c).await;
    if r["unread"] == true && r["state"] != "working" {
      break;
    }
  }
  let r3 = row(&mut c).await;
  assert_eq!(r3["unread"], true, "{r3}");
  node.stop().await.unwrap();
}

/// `ReadAll`: every listed conversation, or one place's, in one pass; it
/// writes only where a cursor moves.
#[tokio::test(flavor = "multi_thread")]
async fn read_all_reads_every_conversation_or_one_places() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![
    answer("one"),
    answer("two"),
    answer("three"),
    answer("four"),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let elsewhere = d.join("notes");
  std::fs::create_dir_all(&elsewhere).unwrap();
  let mut req = chat(None, "in a folder", &elsewhere);
  if let Request::Chat { global, .. } = &mut req {
    *global = false;
  }
  for r in [req, chat(None, "g1", &d), chat(None, "g2", &d)] {
    c.chat(&r, |_| None).await.unwrap();
  }
  let v = list_all(&mut c).await;
  let unread = |v: &Value| -> Vec<(String, bool)> {
    let mut o: Vec<_> = v["threads"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| (t["place"].as_str().unwrap().to_owned(), t["unread"] == true))
      .collect();
    o.sort();
    o
  };
  assert!(unread(&v).iter().all(|(_, u)| *u), "{v}");
  let folder = v["places"]
    .as_array()
    .unwrap()
    .iter()
    .map(|p| p["place"].as_str().unwrap().to_owned())
    .find(|p| p != "global")
    .unwrap();
  let files0 = walkdir_count(&d);
  let Reply::Lines(n) = read_all(&mut c, Some(&folder)).await.unwrap() else {
    panic!()
  };
  assert_eq!(n, ["1"]);
  assert_eq!(
    walkdir_count(&d),
    files0 + 1,
    "one floor record for the place"
  );
  let v = list_all(&mut c).await;
  assert_eq!(
    unread(&v),
    [
      (folder.clone(), false),
      ("global".into(), true),
      ("global".into(), true)
    ],
    "{v}"
  );
  let files1 = walkdir_count(&d);
  let Reply::Lines(n) = read_all(&mut c, None).await.unwrap() else {
    panic!()
  };
  assert_eq!(n, ["2"]);
  assert_eq!(
    walkdir_count(&d),
    files1 + 1,
    "one record for all, not one per conversation"
  );
  let before = walkdir_count(&d);
  let Reply::Lines(n) = read_all(&mut c, None).await.unwrap() else {
    panic!()
  };
  assert_eq!(n, ["0"]);
  assert_eq!(walkdir_count(&d), before, "nothing moved, nothing written");
  assert!(unread(&list_all(&mut c).await).iter().all(|(_, u)| !*u));
  // A step that lands after the floor stays unread.
  let g1 = list_all(&mut c).await["threads"]
    .as_array()
    .unwrap()
    .iter()
    .find(|t| t["place"] == "global")
    .unwrap()["id"]
    .as_str()
    .unwrap()
    .to_owned();
  c.chat(&chat(Some(&g1), "more", &d), |_| None)
    .await
    .unwrap();
  let v = list_all(&mut c).await;
  assert_eq!(unread(&v).iter().filter(|(_, u)| *u).count(), 1, "{v}");
  node.stop().await.unwrap();
}

async fn list_all(c: &mut Client) -> Value {
  let req = Request::Threads {
    forest: None,
    status: None,
    since: None,
    limit: None,
    thread: None,
    place: None,
  };
  let Reply::Lines(l) = c.request(&req).await.unwrap() else {
    panic!()
  };
  serde_json::from_str(&l[0]).unwrap()
}

async fn read_all(c: &mut Client, place: Option<&str>) -> stitch_node::Result<Reply> {
  c.request(&Request::ReadAll {
    place: place.map(str::to_owned),
  })
  .await
}
