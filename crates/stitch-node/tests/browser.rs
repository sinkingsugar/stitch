//! Browser control through a thread (design §2.18), with a fake extension
//! on the node's socket standing in for Chrome: the questions the browser
//! tools ask go where the user is (the client, the question records, and the
//! page while the user watches, the first answer winning), carry a
//! screenshot when nobody watches, and "always" keeps a site for the place.
//! The real extension in a real Chrome is `stitch-cli`'s `tests/browser.rs`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{self, Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};

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

fn chat(thread: Option<&str>, text: &str) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: Vec::new(),
    model: None,
    repo: None,
    cwd: None,
    global: true,
    env: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
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

/// What the fake extension saw and how it behaves.
#[derive(Default)]
struct Fake {
  /// Every request, as `op` (and `msg.t` or `method`).
  ops: Vec<String>,
  /// The user is watching the agent's tab.
  watching: bool,
  /// Questions shown in the page: answered there with this choice.
  answer_in_page: Option<&'static str>,
  asks: Vec<Value>,
}

const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A fake extension: a browser link that answers like the real one.
async fn fake_extension(
  data: &Path,
  fake: Arc<Mutex<Fake>>,
) -> tokio::sync::mpsc::UnboundedSender<Value> {
  let stream = UnixStream::connect(stitch_node::socket_path(data).unwrap())
    .await
    .unwrap();
  let (mut r, mut w) = stream.into_split();
  api::write(
    &mut w,
    &Request::Hello {
      version: stitch_node::VERSION.into(),
    },
  )
  .await
  .unwrap();
  let _: Option<Reply> = api::read(&mut r).await.unwrap();
  let hello = json!({"t": "hello", "proto": 1, "version": "0.1.0",
    "profile": {"id": "p1", "name": "work", "enabled": true},
    "browser": {"brand": "Google Chrome 155"}});
  api::write(
    &mut w,
    &Request::BrowserLink {
      hello: hello.to_string(),
    },
  )
  .await
  .unwrap();
  // Events the test sends, as the user's doing.
  let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
  let w = Arc::new(tokio::sync::Mutex::new(w));
  let w2 = w.clone();
  tokio::spawn(async move {
    while let Some(e) = events_rx.recv().await {
      let mut w = w2.lock().await;
      api::write(&mut *w, &Request::BrowserMsg(e.to_string()))
        .await
        .unwrap();
    }
  });
  tokio::spawn(async move {
    while let Ok(Some(Reply::BrowserMsg(m))) = api::read::<Reply, _>(&mut r).await {
      let m: Value = serde_json::from_str(&m).unwrap();
      let op = m["op"].as_str().unwrap_or("").to_owned();
      let page = m["msg"]["t"].as_str().unwrap_or("");
      let mut events = Vec::new();
      let result = {
        let mut f = fake.lock().unwrap();
        f.ops
          .push(format!("{op} {page}{}", m["method"].as_str().unwrap_or("")));
        match (op.as_str(), page) {
          ("newTab", _) => json!({"tab": 7, "window": 1, "group": 3}),
          ("adopt", _) => {
            json!({"tab": m["tab"], "url": "https://news.example/a", "title": "A story"})
          }
          ("tabs", _) => json!([]),
          ("context", _) => {
            json!({"active": {"tab": if f.watching { 7 } else { 1 }, "url": "http://x/", "title": "x"},
            "focused": true, "idle": "active", "profile": {}})
          }
          ("cdp", _) if m["method"] == "Page.captureScreenshot" => json!({"data": PNG_1X1}),
          ("cdp", _) => json!({}),
          ("page", "info") => {
            json!({"ok": true, "result": {"doc": "d1", "url": "https://shop.example/cart", "title": "Cart",
            "ready": "complete", "scroll": {"x": 0, "y": 0}, "viewport": {"w": 800, "h": 600, "dpr": 1}}})
          }
          ("page", "snapshot") => {
            json!({"ok": true, "result": {"doc": "d1", "url": "https://shop.example/cart", "title": "Cart",
            "text": "button \"Delete everything\" [ref_1]\ntextbox \"Note\" [ref_2]", "truncated": false,
            "scroll": {"x": 0, "y": 0, "height": 600, "viewport": 600}}})
          }
          ("page", "resolve") => {
            let text = if m["msg"]["ref"] == "ref_1" {
              "Delete everything"
            } else {
              ""
            };
            json!({"ok": true, "result": {"x": 50, "y": 40, "rect": {"x": 10, "y": 30, "w": 80, "h": 20},
              "text": text, "tag": "button", "what": format!("button \"{text}\""), "sensitive": false}})
          }
          ("page", "ask") => {
            if m["msg"]["done"].is_null() {
              f.asks.push(m["msg"].clone());
              if let Some(choice) = f.answer_in_page {
                events.push(json!({"t": "event", "kind": "answer", "tab": 7, "id": m["msg"]["id"], "choice": choice}));
              }
            }
            json!({"ok": true, "result": {}})
          }
          ("page", _) => json!({"ok": true, "result": {}}),
          _ => json!({}),
        }
      };
      let res = json!({"t": "res", "id": m["id"], "ok": true, "result": result});
      api::write(&mut *w.lock().await, &Request::BrowserMsg(res.to_string()))
        .await
        .unwrap();
      for e in events {
        // As the user clicks a moment later.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        api::write(&mut *w.lock().await, &Request::BrowserMsg(e.to_string()))
          .await
          .unwrap();
      }
    }
  });
  events_tx
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_questions_go_where_the_user_is() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let delete = json!({"intent": "input", "actions": [{"type": "click", "ref": "ref_1"}]});
  let (base, sent) = serve(vec![
    tool_call(
      "b1",
      "browser_tabs",
      json!({"action": "new", "url": "https://shop.example/cart"}),
    ),
    tool_call("b2", "browser_act", delete.clone()),
    tool_call("b3", "browser_act", delete.clone()),
    answer("Done."),
  ])
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let fake = Arc::new(Mutex::new(Fake {
    watching: false,
    ..Fake::default()
  }));
  let _events = fake_extension(&data, fake.clone()).await;
  let mut c = Client::connect(&data).await.unwrap();
  let mut asked = Vec::new();
  let f2 = fake.clone();
  let done = c
    .chat(&chat(None, "Empty my cart"), |r| match r {
      Reply::Approve { command, grant, .. } => {
        asked.push((command.clone(), grant.clone()));
        let n = asked.len();
        let (tx, rx) = tokio::sync::oneshot::channel();
        match n {
          // Another site: "always", kept for the place.
          1 => {
            tx.send(stitch_node::tools::Decision::always()).unwrap();
            Some(rx)
          }
          // Unwatched, the look-alike delete asks here, with a screenshot:
          // allowed; then the user watches, and the next is answered in the
          // page while this client says nothing.
          2 => {
            let mut f = f2.lock().unwrap();
            f.watching = true;
            f.answer_in_page = Some("allow");
            tx.send(stitch_node::tools::Decision::allow()).unwrap();
            Some(rx)
          }
          _ => None,
        }
      }
      _ => None,
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert_eq!(asked.len(), 3, "{asked:?}");
  assert!(
    asked[0].0.contains("open https://shop.example/cart"),
    "{asked:?}"
  );
  assert_eq!(
    asked[0].1,
    vec!["shop.example".to_string()],
    "always grants the site"
  );
  assert!(asked[1].0.contains("looks like delete"), "{asked:?}");
  assert!(asked[1].1.is_empty(), "no always for the always-ask set");
  {
    let f = fake.lock().unwrap();
    // The unwatched question took a screenshot; the watched one asked in the page.
    let shot = f
      .ops
      .iter()
      .position(|o| o.contains("Page.captureScreenshot"))
      .expect("a screenshot");
    assert!(shot > 0);
    assert_eq!(f.asks.len(), 1, "{:?}", f.asks);
    assert!(
      f.asks[0]["text"]
        .as_str()
        .unwrap()
        .contains("Stitch wants to input on shop.example")
    );
  }
  {
    let bodies = sent.lock().unwrap();
    let last = bodies.last().unwrap().to_string();
    assert!(
      last.contains("This thread is out of memory now"),
      "a thread that used the browser leaves memory: {last}"
    );
    assert!(
      last.contains("done: click button"),
      "the action ran after the page's answer: {last}"
    );
  }
  // "Always" was kept for the place.
  let config = std::fs::read_to_string(data.join("config.toml")).unwrap();
  assert!(
    config.contains("[browser.places.") && config.contains("shop.example"),
    "{config}"
  );
  // The answers say where they came from.
  let home = dir.path().join("home");
  let answers: Vec<String> = walk(&home)
    .into_iter()
    .filter_map(|p| std::fs::read_to_string(p).ok())
    .filter(|t| t.contains("kind = \"answer\""))
    .collect();
  assert!(
    answers.iter().any(|a| a.contains("approved in the page")),
    "{answers:?}"
  );
  let questions: Vec<String> = walk(&home)
    .into_iter()
    .filter_map(|p| std::fs::read_to_string(p).ok())
    .filter(|t| t.contains("kind = \"question\"") && t.contains("In your browser"))
    .collect();
  assert!(
    questions.iter().any(|q| q.contains("tab.png")),
    "a question carries the screenshot: {questions:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
  let mut out = Vec::new();
  if let Ok(rd) = std::fs::read_dir(dir) {
    for e in rd.flatten() {
      let p = e.path();
      if p.is_dir() {
        out.extend(walk(&p));
      } else {
        out.push(p);
      }
    }
  }
  out
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_user_points_at_reaches_the_thread() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let (base, sent) = serve(vec![
    tool_call("c1", "browser_context", json!({})),
    tool_call("c2", "browser_tabs", json!({"action": "share", "tab": 42})),
    answer("It is about rockets."),
  ])
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let fake = Arc::new(Mutex::new(Fake::default()));
  let events = fake_extension(&data, fake.clone()).await;
  // The user right-clicks a paragraph: "Ask Stitch about this…".
  events
    .send(json!({"t": "event", "kind": "point", "tab": 42, "url": "https://news.example/a", "title": "A story",
      "how": "menu", "selection": "rockets are fast", "element": "text \"rockets\" [ref_9]", "ref": "ref_9", "doc": "d9", "session": null}))
    .unwrap();
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  let mut c = Client::connect(&data).await.unwrap();
  let done = c
    .chat(&chat(None, "What is this about?"), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  {
    let bodies = sent.lock().unwrap();
    let told = bodies[1].to_string();
    assert!(
      told.contains("The user pointed at the element text")
        && told.contains("rockets are fast")
        && told.contains("action share, tab 42"),
      "{told}"
    );
    let told = bodies[2].to_string();
    assert!(told.contains("tab 42 is now this session's"), "{told}");
    assert!(
      fake
        .lock()
        .unwrap()
        .ops
        .iter()
        .any(|o| o.starts_with("adopt"))
    );
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
