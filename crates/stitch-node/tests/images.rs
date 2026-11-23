//! Images too large for a model never break a thread (design §2.3,
//! "Seeing and asking"): a tool's image is scaled down where it enters the
//! thread, its original kept for `zoom_image`; an image in the history
//! still too large (older records, the user's attachments) is fitted when
//! sent; and a provider refusing images over its many-image limit gets
//! them fitted to the limit it names, once.

use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
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

/// Anthropic's refusal of a request with many images, one over `side`.
fn too_many_pixels(side: u32) -> String {
  let body = json!({
    "type": "error",
    "error": {
      "type": "invalid_request_error",
      "message": format!(
        "messages.0.content.2.image.source.base64.data: At least one of the image dimensions \
         exceed max allowed size for many-image requests: {side} pixels"
      ),
    },
  })
  .to_string();
  format!(
    "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
    body.len()
  )
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
      approval: Some("off".into()),
      ..Default::default()
    }),
    // Apps not served: a test node never takes the fixed port 7177.
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

fn chat(thread: Option<&str>, text: &str, attachments: Vec<String>) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments,
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

/// A PNG of `w`×`h`, at `path`.
fn png(path: &Path, w: u32, h: u32) {
  let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
    image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
  }));
  img.save(path).unwrap();
}

fn blocks(body: &Value) -> Vec<Value> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
    .collect()
}

/// The sizes of the images a request sent.
fn sizes(body: &Value) -> Vec<(u32, u32)> {
  blocks(body)
    .iter()
    .filter(|b| b["type"] == "image")
    .map(|b| {
      let data = base64::engine::general_purpose::STANDARD
        .decode(b["source"]["data"].as_str().unwrap())
        .unwrap();
      image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .unwrap()
        .into_dimensions()
        .unwrap()
    })
    .collect()
}

fn texts(body: &Value) -> String {
  blocks(body)
    .iter()
    .filter_map(|b| match b["type"].as_str() {
      Some("text") => b["text"].as_str().map(str::to_owned),
      Some("tool_result") => Some(b["content"].to_string()),
      _ => None,
    })
    .collect::<Vec<_>>()
    .join("\n")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tools_oversized_image_is_scaled_where_it_enters_and_zoomed_from_the_original() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let shot = dir.path().join("shot.png");
  png(&shot, 3000, 1000);
  // The original's ID, as the node will store it.
  let original = stitch_store::BlobStore::open(&dir.path().join("probe"))
    .unwrap()
    .put_bytes(&std::fs::read(&shot).unwrap())
    .unwrap();
  let original = format!("img:{}", &original.hex()[..12]);
  let (base, sent) = serve(vec![
    tool_call(
      "v1",
      "view_file",
      json!({"path": shot.display().to_string()}),
    ),
    answer("A wide screenshot."),
    tool_call(
      "z1",
      "zoom_image",
      json!({"image": original, "left": 0.0, "top": 0.0, "right": 0.5, "bottom": 1.0}),
    ),
    answer("Zoomed."),
  ])
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = client
    .chat(&chat(None, "Look at the screenshot", vec![]), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(sizes(&bodies[1]), vec![(2000, 667)], "scaled at the source");
    let told = texts(&bodies[1]);
    assert!(
      told.contains(&format!(
        "the image was 3000×1000 scaled to 2000×667 (×0.67) for the model; its original is {original}"
      )),
      "the result says so: {told}"
    );
  }
  client
    .chat(
      &chat(Some(&thread), "Zoom into the left half", vec![]),
      |_| None,
    )
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    let told = texts(&bodies[2]);
    assert!(
      !told.contains("to be sent"),
      "the stored image fits as it is: {told}"
    );
    assert_eq!(sizes(&bodies[2]), vec![(2000, 667)]);
    let told = texts(&bodies[3]);
    assert!(
      told.contains("1500×1000 pixels of the 3000×1000 original"),
      "zoomed from the original: {told}"
    );
    assert_eq!(sizes(&bodies[3]), vec![(2000, 667), (1500, 1000)]);
  }
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_image_in_the_history_is_fitted_when_sent() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  // Over the many-image limit Anthropic stated (3000), under the 4000
  // fitted before: what broke a thread.
  let photo = dir.path().join("wide.png");
  png(&photo, 3200, 1600);
  let (base, sent) = serve(vec![answer("Wide."), answer("Still wide.")]).await;
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = client
    .chat(
      &chat(None, "What is this?", vec![photo.display().to_string()]),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  client
    .chat(&chat(Some(&thread), "And again?", vec![]), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    for body in bodies.iter() {
      assert_eq!(sizes(body), vec![(2000, 1000)]);
      let told = texts(body);
      assert!(
        told.contains("`wide.png`, 3200×1600 scaled to 2000×1000 (×0.62) to be sent"),
        "{told}"
      );
    }
  }
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_over_the_many_image_limit_is_sent_again_fitted_once() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let photo = dir.path().join("photo.png");
  png(&photo, 1800, 900);
  let (base, sent) = serve(vec![
    too_many_pixels(1000),
    answer("Seen."),
    too_many_pixels(1000),
    too_many_pixels(500),
  ])
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let done = client
    .chat(
      &chat(None, "What is this?", vec![photo.display().to_string()]),
      |_| None,
    )
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(bodies.len(), 2, "sent again once");
    assert_eq!(sizes(&bodies[0]), vec![(1800, 900)]);
    assert_eq!(sizes(&bodies[1]), vec![(1000, 500)], "fitted to the limit");
  }
  // Only once: a second refusal fails the turn.
  let failed = client
    .chat(&chat(Some(&thread), "Again?", vec![]), |_| None)
    .await;
  assert!(
    format!("{failed:?}").contains("many-image requests: 500 pixels"),
    "{failed:?}"
  );
  assert_eq!(sent.lock().unwrap().len(), 4);
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
