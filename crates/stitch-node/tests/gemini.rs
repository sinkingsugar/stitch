//! Gemini through the node (design §2.8, Gemini): its models listed from
//! the API, audio given to it as the user attached it, and an image it
//! makes kept as the reply's attachment, which goes back to it, with its
//! signature, when the conversation goes on.

use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the mock was asked: the request line and the JSON body.
type Seen = Arc<Mutex<Vec<(String, Value)>>>;

/// A Gemini API: `models.list` and `models.get` answered from `models`,
/// each `streamGenerateContent` with the next of `streams`.
async fn serve(models: Value, streams: Vec<Vec<Value>>) -> (String, Seen) {
  serve_both(models, streams, Vec::new()).await
}

/// The same, and Anthropic's Messages API (`POST /v1/messages`) answering
/// with the next of `anthropic`, whole responses.
async fn serve_both(
  models: Value,
  streams: Vec<Vec<Value>>,
  anthropic: Vec<String>,
) -> (String, Seen) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen: Seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  let streams = Arc::new(Mutex::new(streams.into_iter()));
  let anthropic = Arc::new(Mutex::new(anthropic.into_iter()));
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let (models, streams, log) = (models.clone(), streams.clone(), log.clone());
      let anthropic = anthropic.clone();
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let (line, body) = loop {
          let n = sock.read(&mut chunk).await.unwrap();
          if n == 0 {
            return;
          }
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let len: usize = head
              .lines()
              .find_map(|l| {
                l.to_ascii_lowercase()
                  .strip_prefix("content-length:")
                  .map(|v| v.trim().parse().unwrap())
              })
              .unwrap_or(0);
            while buf.len() < end + 4 + len {
              let n = sock.read(&mut chunk).await.unwrap();
              buf.extend_from_slice(&chunk[..n]);
            }
            let body = serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap_or(Value::Null);
            break (head.lines().next().unwrap_or("").to_owned(), body);
          }
        };
        let response = if line.starts_with("GET /v1beta/models?") {
          json_reply(&models)
        } else if let Some(rest) = line.strip_prefix("GET /v1beta/models/") {
          let name = rest.split_whitespace().next().unwrap();
          let m = models["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == format!("models/{name}"))
            .cloned()
            .unwrap_or(json!({}));
          json_reply(&m)
        } else if line.starts_with("POST /v1/messages") {
          anthropic
            .lock()
            .unwrap()
            .next()
            .expect("an Anthropic response to answer")
        } else {
          let chunks = streams.lock().unwrap().next().expect("a stream to answer");
          let body: String = chunks
            .iter()
            .map(|c| format!("data: {c}\r\n\r\n"))
            .collect();
          format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
          )
        };
        log.lock().unwrap().push((line, body));
        sock.write_all(response.as_bytes()).await.unwrap();
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

fn json_reply(v: &Value) -> String {
  let b = v.to_string();
  format!(
    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
    b.len()
  )
}

fn setup(data: &Path, home: &Path, base: &str) {
  setup_with(data, home, base, false);
}

/// With `claude`, the default model is Anthropic's, served by the same mock.
fn setup_with(data: &Path, home: &Path, base: &str, claude: bool) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    default_model: Some(
      if claude {
        "anthropic/claude-opus-5-5"
      } else {
        "gemini/gemini-3.1-flash-image"
      }
      .into(),
    ),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  c.providers.insert(
    "gemini".into(),
    ProviderConfig {
      kind: "gemini".into(),
      base_url: Some(format!("{base}/v1beta")),
      ..ProviderConfig::default()
    },
  );
  if claude {
    c.providers.insert(
      "anthropic".into(),
      ProviderConfig {
        kind: "anthropic".into(),
        base_url: Some(base.into()),
        ..ProviderConfig::default()
      },
    );
    keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  }
  c.save(data).unwrap();
  keys::set(data, "gemini", "test-key", Store::File).unwrap();
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

fn png() -> Vec<u8> {
  let img =
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([200, 0, 0])));
  let mut out = Cursor::new(Vec::new());
  img.write_to(&mut out, image::ImageFormat::Png).unwrap();
  out.into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn gemini_models_audio_in_and_an_image_out() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let made = png();
  let models = json!({"models": [
    {"name": "models/gemini-3.8-flash", "supportedGenerationMethods": ["generateContent"],
     "inputTokenLimit": 1_048_576, "outputTokenLimit": 65_536, "thinking": true},
    {"name": "models/gemini-3.1-flash-image", "supportedGenerationMethods": ["generateContent"],
     "inputTokenLimit": 65_536, "outputTokenLimit": 32_768, "thinking": true},
    {"name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent"]}
  ]});
  let (base, seen) = serve(
    models,
    vec![
      vec![json!({"candidates": [{"content": {"role": "model", "parts": [
        {"text": "A red square."},
        {"inlineData": {"mimeType": "image/png", "data": B64.encode(&made)}, "thoughtSignature": "c2ln"}
      ]}, "finishReason": "STOP"}],
        "usageMetadata": {"promptTokenCount": 300, "candidatesTokenCount": 1290}})],
      vec![json!({"candidates": [{"content": {"parts": [{"text": "Blue now."}]}, "finishReason": "STOP"}]})],
    ],
  )
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let tone = dir.path().join("tone.wav");
  std::fs::write(&tone, b"RIFF\0\0\0\0WAVEfmt fake").unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();

  // Listed from the API: chat and image models, not embeddings.
  let Reply::Models {
    models, problems, ..
  } = client
    .request(&Request::Models { node: None })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(
    models,
    ["gemini/gemini-3.1-flash-image", "gemini/gemini-3.8-flash"]
  );
  assert!(
    !problems.iter().any(|p| p.starts_with("gemini")),
    "{problems:?}"
  );

  let Reply::ChatDone { thread, .. } = client
    .chat(
      &chat(
        None,
        "Draw what this sounds like.",
        vec![tone.display().to_string()],
      ),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  client
    .chat(&chat(Some(&thread), "Make it blue.", vec![]), |_| None)
    .await
    .unwrap();
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  let seen = seen.lock().unwrap().clone();
  let posts: Vec<&Value> = seen
    .iter()
    .filter(|(line, _)| {
      line.starts_with("POST /v1beta/models/gemini-3.1-flash-image:streamGenerateContent")
    })
    .map(|(_, b)| b)
    .collect();
  assert_eq!(posts.len(), 2, "{seen:?}");
  // The audio, inline, as attached.
  let first = &posts[0]["contents"][0]["parts"];
  assert!(
    first.as_array().unwrap().iter().any(|p| p["inlineData"]
      == json!({"mimeType": "audio/wav", "data": B64.encode(b"RIFF\0\0\0\0WAVEfmt fake")})),
    "{first}"
  );
  assert_eq!(
    posts[0]["generationConfig"]["responseModalities"],
    json!(["TEXT", "IMAGE"])
  );
  // The image made comes back from the reply's attachment, signed.
  let contents = posts[1]["contents"].as_array().unwrap();
  let model_turn = contents.iter().find(|c| c["role"] == "model").unwrap();
  assert_eq!(
    model_turn["parts"],
    json!([
      {"text": "A red square."},
      {"inlineData": {"mimeType": "image/png", "data": B64.encode(&made)}, "thoughtSignature": "c2ln"}
    ])
  );
  // And it is a file in the blob store, for the user.
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  assert!(blobs.contains(&stitch_records::BlobHash::of(&made)));
}

fn anthropic_sse(events: Vec<Value>) -> String {
  let body: String = events
    .iter()
    .map(|d| format!("event: {}\ndata: {d}\n\n", d["type"].as_str().unwrap()))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// Claude calling a tool with `input`, or answering `text`.
fn claude(call: Option<(&str, Value)>, text: &str) -> String {
  let mut events = vec![
    json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
    json!({"type": "content_block_stop", "index": 0}),
  ];
  if let Some((name, input)) = &call {
    events.extend([
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_img", "name": name, "input": {}}}),
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
      json!({"type": "content_block_stop", "index": 1}),
    ]);
  }
  events.extend([
    json!({"type": "message_delta", "delta": {"stop_reason": if call.is_some() {"tool_use"} else {"end_turn"}}, "usage": {"output_tokens": 9}}),
    json!({"type": "message_stop"}),
  ]);
  anthropic_sse(events)
}

/// The image blocks of an Anthropic request, by message: (role, the
/// texts and images in order, as `text:` / `image:<bytes>`).
fn anthropic_shape(body: &Value) -> Vec<(String, Vec<String>)> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|m| {
      let blocks = m["content"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| match b["type"].as_str().unwrap() {
          "image" => format!(
            "image:{}",
            B64
              .decode(b["source"]["data"].as_str().unwrap())
              .unwrap()
              .len()
          ),
          "text" => format!("text:{}", b["text"].as_str().unwrap()),
          other => other.to_owned(),
        })
        .collect();
      (m["role"].as_str().unwrap().to_owned(), blocks)
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn any_model_makes_images_with_generate_image_and_sees_them_after() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let made = png();
  let models = json!({"models": [
    {"name": "models/gemini-3.1-flash-image", "supportedGenerationMethods": ["generateContent"],
     "inputTokenLimit": 65_536, "outputTokenLimit": 32_768, "thinking": true}
  ]});
  let (base, seen) = serve_both(
    models,
    vec![vec![json!({"candidates": [{"content": {"role": "model", "parts": [
      {"inlineData": {"mimeType": "image/png", "data": B64.encode(&made)}, "thoughtSignature": "c2ln"}
    ]}, "finishReason": "STOP"}]})]],
    vec![
      claude(
        Some((
          "generate_image",
          json!({"prompt": "a red square", "aspect_ratio": "1:1"}),
        )),
        "Drawing it.",
      ),
      claude(None, "Here is your red square."),
      claude(None, "It is red."),
    ],
  )
  .await;
  setup_with(&data, &dir.path().join("home"), &base, true);
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = client
    .chat(&chat(None, "Draw a red square.", vec![]), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  client
    .chat(&chat(Some(&thread), "What colour is it?", vec![]), |_| None)
    .await
    .unwrap();
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();

  let seen = seen.lock().unwrap().clone();
  let claude_bodies: Vec<&Value> = seen
    .iter()
    .filter(|(l, _)| l.starts_with("POST /v1/messages"))
    .map(|(_, b)| b)
    .collect();
  assert_eq!(claude_bodies.len(), 3);
  // Offered to Claude.
  assert!(
    claude_bodies[0]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .any(|t| t["name"] == "generate_image")
  );
  // The image model was asked with the prompt and the ratio, no tools.
  let gemini = seen
    .iter()
    .find(|(l, _)| l.contains(":streamGenerateContent"))
    .map(|(_, b)| b)
    .unwrap();
  assert_eq!(
    gemini["contents"][0]["parts"],
    json!([{"text": "a red square"}])
  );
  assert_eq!(
    gemini["generationConfig"]["imageConfig"],
    json!({"aspectRatio": "1:1"})
  );
  assert!(gemini.get("tools").is_none());
  // Claude sees the image after the result, in that step and in the
  // next turn, from the reply's attachment.
  for body in &claude_bodies[1..] {
    let shape = anthropic_shape(body);
    let (role, blocks) = &shape[2];
    assert_eq!(role, "user", "{shape:?}");
    assert_eq!(blocks[0], "tool_result");
    assert_eq!(
      blocks[1],
      "text:[The images of the assistant's turn above, made for it:]"
    );
    assert!(blocks[2].starts_with("text:[image img:"), "{blocks:?}");
    assert_eq!(blocks[3], format!("image:{}", made.len()));
  }
  let result = claude_bodies[1]["messages"][2]["content"][0]["content"]
    .as_str()
    .unwrap()
    .to_owned();
  assert!(
    result.starts_with("Made with gemini/gemini-3.1-flash-image: img:"),
    "{result}"
  );
  // The result says where the image's file is on disk: the node's stored copy.
  let path = result
    .split(", file ")
    .nth(1)
    .and_then(|r| r.split(" (").next())
    .unwrap_or_else(|| panic!("no file path in: {result}"));
  let path = std::path::Path::new(path);
  assert!(path.starts_with(&data), "{path:?} not under {data:?}");
  assert_eq!(std::fs::read(path).unwrap(), made, "{path:?}");
}
