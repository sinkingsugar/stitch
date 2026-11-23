//! The web client's image blobs (design §2.24): the gate of `GET /_stitch/blob/<hash>` and
//! `POST /_stitch/blob`, images only, and a message's images in the transcript.
#![allow(dead_code)]

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

/// A tiny PNG: its signature and a few bytes are all the node reads.
fn png() -> Vec<u8> {
  let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
  b.extend_from_slice(b"\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0");
  b
}

/// One HTTP request to the apps port, with the headers given; the status,
/// the response's headers (lowercased names) and its body.
async fn http(
  port: u16,
  method: &str,
  host: &str,
  path: &str,
  headers: &[(&str, &str)],
  body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
  let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let mut req = format!(
    "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
    body.len()
  );
  for (k, v) in headers {
    req.push_str(&format!("{k}: {v}\r\n"));
  }
  req.push_str("\r\n");
  // A refused request is answered (and the socket closed) before its body
  // is read: writing it can fail with a reset, which on macOS also drops
  // what was received. Send both in one write, ignore its failure, and
  // keep what was read before any reset.
  let mut all = req.into_bytes();
  all.extend_from_slice(body);
  let _ = s.write_all(&all).await;
  let mut out = Vec::new();
  let mut chunk = [0u8; 8192];
  loop {
    match s.read(&mut chunk).await {
      Ok(0) | Err(_) => break,
      Ok(n) => out.extend_from_slice(&chunk[..n]),
    }
  }
  let end = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
  let head = String::from_utf8_lossy(&out[..end]).into_owned();
  let mut lines = head.lines();
  let status = lines
    .next()
    .unwrap()
    .split(' ')
    .nth(1)
    .unwrap()
    .parse()
    .unwrap();
  let hs = lines
    .filter_map(|l| l.split_once(": "))
    .map(|(k, v)| (k.to_ascii_lowercase(), v.to_owned()))
    .collect();
  (status, hs, out[end + 4..].to_vec())
}

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
  hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn the_clients_image_blobs_are_gated_and_images_only() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  std::fs::create_dir_all(d.join("work")).unwrap();
  let (base, _) = serve(vec![answer("Seen.")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

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
  let Reply::Apps { apps, .. } = c.request(&Request::Dashboard).await.unwrap() else {
    panic!("no dashboard")
  };
  let dash = format!(
    "{}.localhost:{port}",
    host_of(apps[0].url.as_deref().unwrap())
  );
  let token = std::fs::read_to_string(data.join("apps.token")).unwrap();
  let cookie = format!("stitch={}", token.trim());
  let ck = [("Cookie", cookie.as_str())];
  let post = [
    ("Cookie", cookie.as_str()),
    ("Origin", origin.as_str()),
    ("Sec-Fetch-Site", "same-origin"),
  ];

  // An upload: an image is stored and named by its hash.
  let (status, _, body) = http(port, "POST", &host, "/_stitch/blob", &post, &png()).await;
  assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
  let said: Value = serde_json::from_slice(&body).unwrap();
  let hash = said["hash"].as_str().unwrap().to_owned();
  assert!(hash.starts_with("blake3:"), "{said}");
  assert_eq!(said["type"], "image/png");

  // Any file is taken, but a label does not make an image: an SVG labelled
  // as a PNG is stored as what its name says, never as an image.
  let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#;
  let mut labelled = post.to_vec();
  labelled.push(("Content-Type", "image/png"));
  let (status, _, body) = http(
    port,
    "POST",
    &host,
    "/_stitch/blob?name=x.svg",
    &labelled,
    svg,
  )
  .await;
  assert_eq!(status, 200, "a file");
  let said: Value = serde_json::from_slice(&body).unwrap();
  assert_ne!(said["type"], "image/png", "{said}");
  let (status, hs, got) = http(
    port,
    "GET",
    &host,
    &format!(
      "/_stitch/blob/{}?name=x.svg",
      said["hash"].as_str().unwrap()
    ),
    &ck,
    b"",
  )
  .await;
  assert_eq!(status, 200);
  assert_eq!(got, svg);
  assert_eq!(
    header(&hs, "content-type"),
    Some("application/octet-stream")
  );
  assert!(
    header(&hs, "content-disposition").is_some_and(|d| d.starts_with("attachment")),
    "{hs:?}"
  );
  assert_eq!(header(&hs, "content-security-policy"), Some("sandbox"));
  assert!(header(&hs, "x-content-type-options").is_some_and(|v| v.contains("nosniff")));
  // Empty, or past the limit, is refused.
  let (status, ..) = http(port, "POST", &host, "/_stitch/blob", &post, b"").await;
  assert_eq!(status, 400, "an empty file");
  // Without an Origin, or from another page, a POST is refused.
  let (status, ..) = http(port, "POST", &host, "/_stitch/blob", &ck, &png()).await;
  assert_eq!(status, 403, "a POST with no Origin");
  let evil = [
    ("Cookie", cookie.as_str()),
    ("Origin", "http://evil.example"),
  ];
  let (status, ..) = http(port, "POST", &host, "/_stitch/blob", &evil, &png()).await;
  assert_eq!(status, 403, "a POST from another page");

  // Read back with its type, nosniff, and no sharing.
  let path = format!("/_stitch/blob/{hash}");
  let (status, hs, body) = http(port, "GET", &host, &path, &ck, b"").await;
  assert_eq!(status, 200);
  assert_eq!(body, png());
  assert_eq!(header(&hs, "content-type"), Some("image/png"));
  assert_eq!(header(&hs, "x-content-type-options"), Some("nosniff"));
  assert_eq!(
    header(&hs, "cross-origin-resource-policy"),
    Some("same-origin")
  );
  // The page's own fetch says so too.
  let own = [
    ("Cookie", cookie.as_str()),
    ("Sec-Fetch-Site", "same-origin"),
  ];
  let (status, ..) = http(port, "GET", &host, &path, &own, b"").await;
  assert_eq!(status, 200);

  // Refused: no cookie, another site's name, another page, other apps,
  // a version's origin.
  let (status, ..) = http(port, "GET", &host, &path, &[], b"").await;
  assert_eq!(status, 403, "no cookie");
  let (status, ..) = http(
    port,
    "GET",
    &format!("evil.example:{port}"),
    &path,
    &ck,
    b"",
  )
  .await;
  assert_eq!(status, 421, "DNS rebinding");
  let cross = [
    ("Cookie", cookie.as_str()),
    ("Sec-Fetch-Site", "cross-site"),
  ];
  let (status, ..) = http(port, "GET", &host, &path, &cross, b"").await;
  assert_eq!(status, 403, "a cross-site request");
  let other = [
    ("Cookie", cookie.as_str()),
    ("Origin", "http://evil.example"),
  ];
  let (status, ..) = http(port, "GET", &host, &path, &other, b"").await;
  assert_eq!(status, 403, "another origin");
  let (status, ..) = http(port, "GET", &dash, &path, &ck, b"").await;
  assert_eq!(status, 403, "another app (the dashboard)");
  let shown = format!("{}.{}.localhost:{port}", "a".repeat(32), "b".repeat(32));
  let (status, ..) = http(port, "GET", &shown, &path, &ck, b"").await;
  assert_eq!(status, 403, "a version's origin (show_app)");
  let (status, ..) = http(port, "POST", &dash, "/_stitch/blob", &post, &png()).await;
  assert_eq!(status, 403, "upload from another app");

  // A blob that is not an image is a download, never shown inline.
  let text = stitch_store::BlobStore::open(&data.join("blobs"))
    .unwrap()
    .put_bytes(b"<script>alert(1)</script>")
    .unwrap();
  let (status, ..) = http(
    port,
    "GET",
    &host,
    &format!("/_stitch/blob/{text}"),
    &ck,
    b"",
  )
  .await;
  assert_eq!(status, 200, "a blob that is not an image is a download");
  let (status, ..) = http(port, "GET", &host, "/_stitch/blob/nonsense", &ck, b"").await;
  assert_eq!(status, 404);

  // The uploaded image is an attachment of a chat by its hash, and shows
  // in the transcript with that hash, which the blob endpoint serves.
  let mut req = chat(None, "look at this", &d.join("work"));
  if let Request::Chat { attachments, .. } = &mut req {
    attachments.push(hash.clone());
  }
  let mut thread = None;
  c.chat(&req, |r| {
    if let Reply::Thread(t) = r {
      thread = Some(t.clone());
    }
    None
  })
  .await
  .unwrap();
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: thread.unwrap(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!("no transcript")
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  let user = &t["entries"][0];
  assert_eq!(user["kind"], "user");
  assert_eq!(user["images"][0]["hash"], hash.as_str(), "{t}");
  assert_eq!(user["images"][0]["type"], "image/png");
  node.stop().await.unwrap();
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

/// A node whose model views an image, edits and writes a file and shows an
/// app; the thread it made, and what a test needs to reach the client.
struct Rich {
  node: stitch_node::NodeHandle,
  c: Client,
  thread: String,
  port: u16,
  host: String,
  cookie: String,
  data: std::path::PathBuf,
  _dir: tempfile::TempDir,
}

async fn rich() -> Rich {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  std::fs::create_dir_all(&work).unwrap();
  image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(300, 180, |x, y| {
    image::Rgb([(x % 256) as u8, (y % 256) as u8, 150])
  }))
  .save(work.join("shot.png"))
  .unwrap();
  std::fs::write(work.join("conn.txt"), "one\ntwo\nthree\n").unwrap();
  std::fs::create_dir_all(work.join("demo")).unwrap();
  std::fs::write(
    work.join("demo/index.html"),
    "<!doctype html><body style=\"background:#10243a;color:#fff;font:16px sans-serif\">\
     <h2>Demo app</h2><p id=p>running on its own origin</p><div style=\"height:520px;background:#1e5a4a\">tall content</div>",
  )
  .unwrap();
  let p = |f: &str| work.join(f).display().to_string();
  let (base, _) = serve(vec![
    tool_call("v1", "view_file", json!({"path": p("shot.png")})),
    tool_call(
      "e1",
      "edit_file",
      json!({"path": p("conn.txt"), "old_string": "two\n", "new_string": "2\nTWO\n"}),
    ),
    tool_call(
      "w1",
      "write_file",
      json!({"path": p("notes.md"), "content": "# Notes\n- a\n- b\n"}),
    ),
    tool_call("a1", "show_app", json!({"path": p("demo"), "name": "Demo"})),
    tool_call(
      "r1",
      "suggest_replies",
      json!({"replies": [{"label": "Ship it"}, {"label": "Not yet", "text": "Not yet: add tests first."}]}),
    ),
    // Refused (no replies): shows none.
    tool_call("r2", "suggest_replies", json!({"replies": []})),
    answer("All done."),
  ])
  .await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut thread = None;
  // Fail fast: a tool waiting on an approval nobody answers is a failure.
  let Reply::ChatDone { .. } = tokio::time::timeout(
    std::time::Duration::from_secs(60),
    c.chat(&chat(None, "show me things", &work), |r| {
      if let Reply::Thread(t) = r {
        thread = Some(t.clone());
      }
      None
    }),
  )
  .await
  .expect("the turn finished in time (a tool waits for an approval?)")
  .unwrap() else {
    panic!("a turn")
  };
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
  let token = std::fs::read_to_string(data.join("apps.token")).unwrap();
  Rich {
    node,
    c,
    thread: thread.unwrap(),
    port,
    host,
    cookie: format!("stitch={}", token.trim()),
    data,
    _dir: dir,
  }
}

#[tokio::test]
async fn the_transcript_carries_what_tools_showed() {
  let mut r = rich().await;
  let Reply::Lines(l) = r
    .c
    .request(&Request::Transcript {
      thread: r.thread.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!("no transcript")
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  let entries = t["entries"].as_array().unwrap();
  let tool = |name: &str| {
    entries
      .iter()
      .find(|e| e["name"] == name)
      .unwrap_or_else(|| panic!("no {name} in {t:#}"))
  };
  // The tools' images come back as messages after the results: they are
  // on the call's entry, and those messages are not entries of their own.
  let view = tool("view_file");
  assert_eq!(view["images"][0]["type"], "image/png", "{view:#}");
  assert!(
    view["images"][0]["hash"]
      .as_str()
      .unwrap()
      .starts_with("blake3:")
  );
  assert_eq!(view["result"]["ok"], true);
  assert!(
    !view["result"]["output"].as_str().unwrap().starts_with('{'),
    "the output is the tool's text, not its stored JSON: {view}"
  );
  assert!(
    entries
      .iter()
      .filter(|e| e["kind"] == "user")
      .all(|e| e["text"].as_str().unwrap().starts_with("show me")),
    "no tool-image messages shown as the user's: {t:#}"
  );
  let edit = tool("edit_file");
  assert!(edit["call"].as_str().unwrap().starts_with("edit_file "));
  let diff = edit["diff"]["text"].as_str().unwrap();
  assert!(
    diff.starts_with("@@") && diff.contains("-two") && diff.contains("+TWO"),
    "{diff}"
  );
  let write = tool("write_file");
  assert!(write["diff"]["text"].as_str().unwrap().contains("+# Notes"));
  let app = tool("show_app");
  assert_eq!(app["app"]["name"], "Demo", "{app:#}");
  let manifest = app["app"]["manifest"].as_str().unwrap().to_owned();
  // The client resolves the app to its address with `AppOpen`.
  let Reply::App { url, .. } = r
    .c
    .request(&Request::AppOpen { what: manifest })
    .await
    .unwrap()
  else {
    panic!("an app address")
  };
  assert!(url.unwrap().contains("?k="));
  // Preset answers (design §2.30): on the call's entry, `text` defaulting
  // to the label; a refused call has none.
  let suggested: Vec<&Value> = entries
    .iter()
    .filter(|e| e["name"] == "suggest_replies")
    .collect();
  assert_eq!(suggested.len(), 2, "{t:#}");
  assert_eq!(
    suggested[0]["replies"],
    json!([
      {"label": "Ship it", "text": "Ship it"},
      {"label": "Not yet", "text": "Not yet: add tests first."}
    ])
  );
  assert_eq!(suggested[0]["call"], "suggest_replies: Ship it · Not yet");
  assert_eq!(suggested[0]["result"]["ok"], true);
  assert!(suggested[1].get("replies").is_none(), "{:#}", suggested[1]);
  assert_eq!(suggested[1]["result"]["ok"], false);
  // The same on `chat::Shown` (the TUI's and the app's transcript), and
  // live: nothing the user wrote after them.
  {
    let index = Mutex::new(Index::open(&r.data.join("index.sqlite")).unwrap());
    let blobs = stitch_store::BlobStore::open(&r.data.join("blobs")).unwrap();
    let id: stitch_records::RecordId = r.thread.parse().unwrap();
    let root = {
      let i = index.lock().unwrap();
      let repo = i.record(&id).unwrap().unwrap().repo;
      i.roots()
        .unwrap()
        .into_values()
        .filter_map(|d| RecordRoot::open(Path::new(&d)).ok())
        .find(|r| r.repo() == &repo)
        .unwrap()
    };
    let node_name = "gamer".parse().unwrap();
    let ctx = stitch_node::chat::ChatContext {
      node: &node_name,
      root: &root,
      index: &index,
      blobs: &blobs,
    };
    let shown = stitch_node::chat::lineage_shown(&ctx, &id, None).unwrap();
    let with: Vec<_> = shown.iter().filter(|e| !e.replies.is_empty()).collect();
    assert_eq!(with.len(), 1);
    assert_eq!(with[0].who, "tool");
    assert_eq!(with[0].replies[1].text, "Not yet: add tests first.");
    let live = stitch_node::replies::live(
      shown.iter().map(|e| (e.is_user(), !e.replies.is_empty())),
      false,
    );
    assert_eq!(
      live.map(|i| shown[i].replies[0].label.as_str()),
      Some("Ship it")
    );
  }
  // And the image is served at its hash.
  let path = format!(
    "/_stitch/blob/{}",
    view["images"][0]["hash"].as_str().unwrap()
  );
  let ck = [("Cookie", r.cookie.as_str())];
  let (status, hs, _) = http(r.port, "GET", &r.host, &path, &ck, b"").await;
  assert_eq!(status, 200);
  assert_eq!(header(&hs, "content-type"), Some("image/png"));
  r.node.stop().await.unwrap();
}

/// A node with that thread, kept up for a browser (`STITCH_DEMO_SECS`):
/// prints the client's address. Run by hand, never by CI.
#[tokio::test]
#[ignore = "keeps a node up for a browser"]
async fn a_node_for_a_browser() {
  let r = rich().await;
  let token = r.cookie.trim_start_matches("stitch=");
  println!("DEMO http://{}/?k={token}", r.host);
  let secs: u64 = std::env::var("STITCH_DEMO_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(600);
  tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
  r.node.stop().await.unwrap();
}

/// The client's pin is the node's own: the user's lists, unpin and the
/// phone's `apps::pinned` leave it out, while `AppOpen`, `Client` and the
/// node's serving find it (design §2.24).
#[tokio::test]
async fn the_clients_pin_is_not_the_users_but_is_found_and_served() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  std::fs::create_dir_all(d.join("work")).unwrap();
  let (base, _) = serve(vec![answer("Seen.")]).await;
  let (data, home, port) = (d.join("data"), d.join("home"), free_port());
  setup(&data, &home, &base, port);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let Reply::App { url: open_url, .. } = c
    .request(&Request::AppOpen {
      what: "Stitch".into(),
    })
    .await
    .unwrap()
  else {
    panic!("the client is pinned")
  };
  let Reply::App { url: web_url, .. } = c.request(&Request::Client).await.unwrap() else {
    panic!("`Client` answers with the client's address")
  };
  assert!(web_url.is_some());
  assert_eq!(web_url, open_url);

  // The user's list leaves it out; so does the phone's (`apps::pinned`).
  let Reply::Apps { apps, .. } = c.request(&Request::Apps).await.unwrap() else {
    panic!("no list")
  };
  assert!(apps.is_empty(), "{apps:?}");
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  assert!(stitch_node::apps::pinned(&index).unwrap().is_empty());
  let all = stitch_node::apps::pinned_all(&index).unwrap();
  assert_eq!(all.len(), 1);
  assert_eq!(
    all[0].builtin.as_ref().map(|b| b.0.as_str()),
    Some("client")
  );

  // The user's commands cannot unpin it.
  let r = c
    .request(&Request::AppUnpin {
      app: "Stitch".into(),
    })
    .await;
  assert!(
    format!("{r:?}").contains("built-in web client: it cannot be unpinned"),
    "{r:?}"
  );
  assert_eq!(stitch_node::apps::pinned_all(&index).unwrap().len(), 1);

  // The node serves it.
  let host = format!("{}.localhost:{port}", all[0].host);
  let token = std::fs::read_to_string(data.join("apps.token")).unwrap();
  let cookie = format!("stitch={}", token.trim());
  let (status, _, _) = http(port, "GET", &host, "/", &[("Cookie", cookie.as_str())], b"").await;
  assert_eq!(status, 200);
  drop(node);
}
