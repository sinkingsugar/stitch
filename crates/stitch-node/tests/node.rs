//! A running node with its socket, against mock providers: chats with
//! attachments, continuing a thread with the provider's own content, a
//! refusal, the single-node lock, and a second node resuming the thread on
//! another provider after sync.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::sync::{ItemSet, sync_roots};
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

fn sse(body: String) -> String {
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// An Anthropic reply: a thinking block, then `text`.
fn anthropic_reply(text: &str, signature: &str) -> String {
  let events = [
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1000, "output_tokens": 1}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Looking."}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": signature}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": text}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 500}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ];
  sse(
    events
      .iter()
      .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
      .collect(),
  )
}

fn anthropic_refusal() -> String {
  let events = [
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 10}}}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"category": "cyber"}}, "usage": {"output_tokens": 0}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ];
  sse(
    events
      .iter()
      .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
      .collect(),
  )
}

fn openai_reply(text: &str) -> String {
  sse(format!(
    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
    json!({"model": "qwen3", "choices": [{"index": 0, "delta": {"content": text}}]}),
    json!({"model": "qwen3", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 5, "completion_tokens": 2}}),
  ))
}

/// A data directory for a node named `name` holding the home workspace at
/// `home`, with an Anthropic mock and an OpenAI-compatible mock.
fn setup(data: &Path, name: &str, home: &Path, anthropic: &str, openai: &str) {
  let mut c = Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    default_model: Some("stitch-test-anthropic/claude-opus-5-5".into()),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  c.providers.insert(
    "stitch-test-anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(anthropic.into()),
      ..ProviderConfig::default()
    },
  );
  c.providers.insert(
    "local".into(),
    ProviderConfig {
      kind: "openai".into(),
      base_url: Some(format!("{openai}/v1")),
      ..ProviderConfig::default()
    },
  );
  c.save(data).unwrap();
  keys::set(data, "stitch-test-anthropic", "sk-test", Store::File).unwrap();
  let root = RecordRoot::open(home)
    .unwrap_or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap());
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn chat(
  thread: Option<&str>,
  text: &str,
  attachments: Vec<String>,
  model: Option<&str>,
) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments,
    model: model.map(str::to_owned),
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

#[tokio::test(flavor = "multi_thread")]
async fn chats_run_on_the_node_and_resume_on_another() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (anthropic, sent) = serve(vec![
    anthropic_reply("A cat on a mat.", "sig-1"),
    anthropic_reply("Still a cat.", "sig-2"),
    anthropic_refusal(),
  ])
  .await;
  let (openai, sent_openai) = serve(vec![openai_reply("Hello from qwen")]).await;
  let data = d.join("mbp");
  setup(&data, "mbp", &d.join("home-mbp"), &anthropic, &openai);
  let notes = d.join("notes.md");
  std::fs::write(&notes, "Buy milk.").unwrap();
  let photo = d.join("cat.png");
  std::fs::write(&photo, [137, 80, 78, 71]).unwrap();

  let node = stitch_node::start(&data).await.unwrap();
  assert!(matches!(
    stitch_node::start(&data).await,
    Err(stitch_node::Error::AlreadyRunning(_))
  ));
  let mut c = Client::connect(&data).await.unwrap();
  assert_eq!(c.node, "mbp");

  // A new thread, with a text file and an image.
  let mut streamed = String::new();
  let done = c
    .chat(
      &chat(
        None,
        "What is in the photo?",
        vec![notes.display().to_string(), photo.display().to_string()],
        None,
      ),
      |r| {
        if let Reply::Text(t) = r {
          streamed.push_str(t)
        }
        None
      },
    )
    .await
    .unwrap();
  assert_eq!(streamed, "A cat on a mat.");
  let Reply::ChatDone {
    thread, model, usd, ..
  } = done
  else {
    panic!()
  };
  assert_eq!(model, "claude-opus-5-5");
  // 1000 input at $4/M and 500 output at $20/M.
  assert!((usd.unwrap() - 0.014).abs() < 1e-9);
  {
    let body = &sent.lock().unwrap()[0];
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["text"], "What is in the photo?");
    assert!(content[1]["text"].as_str().unwrap().contains("Buy milk."));
    // The image, after its ID for zoom_image.
    assert!(
      content[2]["text"]
        .as_str()
        .unwrap()
        .starts_with("[image img:")
    );
    assert_eq!(content[3]["type"], "image");
  }

  // Continuing: the first reply goes back as it came, thinking included.
  c.chat(&chat(Some("last"), "And now?", vec![], None), |_| None)
    .await
    .unwrap();
  {
    let body = &sent.lock().unwrap()[1];
    let m = &body["messages"];
    assert_eq!(m.as_array().unwrap().len(), 3);
    assert_eq!(
      m[1]["content"][0],
      json!({"type": "thinking", "thinking": "Looking.", "signature": "sig-1"})
    );
    assert_eq!(
      m[0]["content"][3]["type"], "image",
      "attachments are sent again, with their IDs"
    );
  }

  // A refusal is recorded, and the declined reply is not part of history.
  let Reply::ChatDone { stop, .. } = c
    .chat(
      &chat(Some(&thread), "Something declined", vec![], None),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(stop, "refusal: cyber");
  assert!(
    matches!(c.request(&Request::Status).await.unwrap(), Reply::Status { repos, .. } if repos == ["home"])
  );
  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
  assert!(Client::connect(&data).await.is_err());

  // A second node gets the thread by record sync, but not the attachments.
  let data2 = d.join("nas");
  setup(&data2, "nas", &d.join("home-nas"), &anthropic, &openai);
  let a = RecordRoot::open(&d.join("home-mbp")).unwrap();
  let b = RecordRoot::open(&d.join("home-nas")).unwrap();
  sync_roots(
    &a,
    &ItemSet::from_root(&a).unwrap(),
    &b,
    &ItemSet::from_root(&b).unwrap(),
  )
  .unwrap();
  let node2 = stitch_node::start(&data2).await.unwrap();
  let mut c2 = Client::connect(&data2).await.unwrap();
  let err = c2
    .chat(
      &chat(Some(&thread), "Hi", vec![], Some("local/qwen3")),
      |_| None,
    )
    .await
    .unwrap_err();
  assert!(err.to_string().contains("not on this node"), "{err}");

  // With them there, it continues on another provider.
  std::fs::create_dir_all(data2.join("blobs")).unwrap();
  let store = stitch_store::BlobStore::open(&data2.join("blobs")).unwrap();
  store.put_bytes(&[137, 80, 78, 71]).unwrap();
  store.put_bytes(b"Buy milk.").unwrap();
  // And the thread's system prompt, a blob too (it syncs as its record's
  // attachment).
  let system: stitch_records::BlobHash = b
    .read_record(&thread.parse().unwrap())
    .unwrap()
    .field_str("system_prompt")
    .unwrap()
    .parse()
    .unwrap();
  let mbp_blobs = stitch_store::BlobStore::open(&d.join("mbp").join("blobs")).unwrap();
  store
    .put_bytes(&std::fs::read(mbp_blobs.path(&system)).unwrap())
    .unwrap();
  let done = c2
    .chat(
      &chat(Some(&thread), "Hi", vec![], Some("local/qwen3")),
      |_| None,
    )
    .await
    .unwrap();
  assert!(
    matches!(&done, Reply::ChatDone { thread: t, model, usd: None, .. } if t == &thread && model == "qwen3")
  );
  {
    let body = &sent_openai.lock().unwrap()[0];
    let m = body["messages"].as_array().unwrap();
    // The thread's prompt, three user turns (one declined, so two in a
    // row), two replies, plus this one.
    let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
    assert_eq!(
      roles,
      [
        "system",
        "user",
        "assistant",
        "user",
        "assistant",
        "user",
        "user"
      ]
    );
    assert!(
      m[0]["content"]
        .as_str()
        .unwrap()
        .starts_with("You are Stitch")
    );
    assert_eq!(m[2]["content"], "A cat on a mat.");
    assert!(
      m[1]["content"][3]["image_url"]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/png;base64,")
    );
  }
  c2.request(&Request::Stop).await.unwrap();
  node2.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_in_process_is_served_as_one_on_the_socket() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (anthropic, _) = serve(vec![anthropic_reply("Hello in process.", "sig-1")]).await;
  let data = d.join("phone");
  setup(
    &data,
    "phone",
    &d.join("home"),
    &anthropic,
    "http://127.0.0.1:9",
  );
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = node.client().await.unwrap();
  assert_eq!(c.node, "phone");
  let mut streamed = String::new();
  let done = c
    .chat(&chat(None, "Hi", Vec::new(), None), |r| {
      if let Reply::Text(t) = r {
        streamed.push_str(t)
      }
      None
    })
    .await
    .unwrap();
  assert_eq!(streamed, "Hello in process.");
  assert!(matches!(done, Reply::ChatDone { .. }));
  // Another at once, while the first stays open.
  let mut other = node.client().await.unwrap();
  assert!(matches!(
    other.request(&Request::Status).await.unwrap(),
    Reply::Status { .. }
  ));
  node.stop().await.unwrap();
}

/// A request whose handler fails is answered with its error, and the
/// connection serves the next one: a failure is not the node closing it
/// ("the node closed the connection" on the phone, for a project's
/// fetch).
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_request_is_answered_and_the_connection_goes_on() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let data = d.join("phone");
  setup(
    &data,
    "phone",
    &d.join("home"),
    "http://127.0.0.1:9",
    "http://127.0.0.1:9",
  );
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = node.client().await.unwrap();
  let failing = [
    Request::ProjectFetch {
      repo: "not a repo".into(),
    },
    Request::ProjectSwitch {
      repo: "not a repo".into(),
      branch: "main".into(),
    },
    Request::Rename {
      thread: "nothing".into(),
      title: "x".into(),
    },
  ];
  for req in &failing {
    match c.request(req).await {
      Err(stitch_node::Error::Remote(_)) => {}
      other => panic!("{req:?}: {other:?}"),
    }
    assert!(
      matches!(
        c.request(&Request::Status).await.unwrap(),
        Reply::Status { .. }
      ),
      "the connection goes on after {req:?}"
    );
  }
  node.stop().await.unwrap();
}
