//! Rewinding a thread between two nodes (design §2.15, "Rewinding a
//! thread"): edit-and-resend goes on in the same thread from the message
//! before, the message it replaces and what followed set aside, out of
//! the transcript and the model's context, on both nodes alike.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers model requests in order, recording each.
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
      // A slow model: answers after a while (unless the loop is stopped).
      let response = match response.strip_prefix("SLOW ") {
        Some(r) => {
          tokio::time::sleep(Duration::from_secs(4)).await;
          r.to_owned()
        }
        None => response,
      };
      let _ = sock.write_all(response.as_bytes()).await;
      let _ = sock.shutdown().await;
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

/// A free UDP port on localhost, for a node to bind again.
fn free_port() -> u16 {
  std::net::UdpSocket::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn config(data: &Path, name: &str, port: u16, anthropic: &str) {
  let mut c = Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    port: Some(port),
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
}

/// Joins `data` (a node named `name` on `port`) to the inviter of
/// `ticket`, as `stitch join` does: trust, then the home workspace.
async fn join(data: &Path, name: &str, port: u16, ticket: &Ticket) {
  let key = stitch_net::load_or_create_key(&data.join("node.key")).unwrap();
  let index = || Index::open(&data.join("index.sqlite")).unwrap();
  let state = |roots| {
    let peers_path = data.join("peers.toml");
    NodeState::new(
      name.parse().unwrap(),
      roots,
      index(),
      BlobStore::open(&data.join("blobs")).unwrap(),
      Peers::load(&peers_path).unwrap(),
      Some(peers_path),
      Policy {
        blobs: BlobPolicy::Lazy,
      },
    )
  };
  let endpoint = bind(key.clone(), Discovery::Local, Some(port))
    .await
    .unwrap();
  let net = Net::spawn(endpoint, Arc::new(state(Default::default())));
  let joined = net.join(ticket).await.unwrap();
  net.shutdown().await.unwrap();
  let repo = joined.home.expect("the inviter holds the home workspace");
  let root = RecordRoot::init(&data.join("home"), repo.parse().unwrap(), Role::Home).unwrap();
  index().refresh(&root).unwrap();
  let roots = [(root.repo().clone(), root.clone())].into_iter().collect();
  let endpoint = bind(key, Discovery::Local, Some(port)).await.unwrap();
  let net = Net::spawn(endpoint, Arc::new(state(roots)));
  net
    .sync_with(joined.inviter.addr().unwrap(), root.repo())
    .await
    .unwrap();
  net.shutdown().await.unwrap();
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
  for _ in 0..300 {
    if ok() {
      return;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  panic!("timed out waiting for {what}");
}

/// Two nodes, the phone holding the home workspace and the Mac joined to
/// it, both running; each node's model answers from its own list.
async fn pair(
  d: &Path,
  phone_says: Vec<String>,
  mac_says: Vec<String>,
) -> (
  stitch_node::NodeHandle,
  stitch_node::NodeHandle,
  Client,
  Client,
  Arc<Mutex<Vec<Value>>>,
  Arc<Mutex<Vec<Value>>>,
) {
  let (phone_api, phone_sent) = serve(phone_says).await;
  let (mac_api, mac_sent) = serve(mac_says).await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let m = Client::connect(&mac).await.unwrap();
  (phone_node, mac_node, p, m, phone_sent, mac_sent)
}

fn global_chat(thread: Option<String>, text: &str) -> Request {
  Request::Chat {
    thread,
    text: text.into(),
    attachments: vec![],
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
async fn chat_thread(c: &mut Client, text: &str) -> String {
  match c.chat(&global_chat(None, text), |_| None).await.unwrap() {
    Reply::ChatDone { thread, .. } => thread,
    other => panic!("{other:?}"),
  }
}

fn texts(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| match &m["content"] {
      Value::String(s) => vec![s.clone()],
      Value::Array(c) => c
        .iter()
        .filter_map(|b| b["text"].as_str().map(str::to_owned))
        .collect(),
      _ => Vec::new(),
    })
    .collect()
}

/// A thread's transcript as a node reads it: (message ID, text).
fn shown(data: &Path, home: &Path, thread: &str) -> Vec<(String, String)> {
  let root = RecordRoot::open(home).unwrap();
  let index = Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  let blobs = BlobStore::open(&data.join("blobs")).unwrap();
  let node: stitch_records::NodeName = "x".parse().unwrap();
  let ctx = stitch_node::chat::ChatContext {
    node: &node,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  stitch_node::chat::transcript_shown(&ctx, &thread.parse().unwrap(), None)
    .unwrap()
    .into_iter()
    .map(|e| (e.id.unwrap_or_default(), e.text))
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_and_resend_rewinds_the_thread_on_both_nodes() {
  let d = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, mut m, phone_sent, _) = pair(
    d.path(),
    vec![
      answer("noted the giraffe"),
      answer("the walrus, then"),
      answer("the otter instead"),
    ],
    vec![],
  )
  .await;
  let thread = chat_thread(&mut p, "first: the giraffe").await;
  let chat = |text: &str| global_chat(Some(thread.clone()), text);
  match p.chat(&chat("second: the walrus"), |_| None).await.unwrap() {
    Reply::ChatDone { .. } => {}
    other => panic!("{other:?}"),
  }
  // Edit and resend the second message: rewound to the reply before it.
  assert!(matches!(
    p.request(&Request::Rewind {
      thread: thread.clone(),
      upto: Some("m2".into()),
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
  let (phone, mac) = (d.path().join("phone"), d.path().join("mac"));
  let phone_home = d.path().join("phone-home");
  let ids = |t: &[(String, String)]| t.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
  assert_eq!(ids(&shown(&phone, &phone_home, &thread)), ["m1", "m2"]);
  match p.chat(&chat("second: the otter"), |_| None).await.unwrap() {
    Reply::ChatDone { thread: t, .. } => assert_eq!(t, thread, "the same thread, not a fork"),
    other => panic!("{other:?}"),
  }
  // What the model was sent: the line, without the walrus.
  let sent = texts(phone_sent.lock().unwrap().last().unwrap()).join("\n");
  assert!(sent.contains("giraffe") && sent.contains("otter"), "{sent}");
  assert!(
    !sent.contains("walrus"),
    "the set-aside branch is not in the context: {sent}"
  );
  // The transcript: the line, its new messages numbered past the old.
  let line = shown(&phone, &phone_home, &thread);
  assert_eq!(ids(&line), ["m1", "m2", "m5", "m6"], "{line:?}");
  assert!(line.iter().all(|(_, t)| !t.contains("walrus")));
  // Search no longer finds what was set aside.
  let Reply::Hits(hits) = p
    .request(&Request::Search {
      query: "walrus".into(),
      here: false,
      cwd: None,
      limit: 10,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(hits.is_empty(), "{hits:?}");
  // The Mac reads the same line once it has the records.
  m.request(&Request::Sync { peer: None }).await.unwrap();
  let mac_home = mac.join("home");
  until("the Mac has the rewound line", || {
    mac_home.join("threads").join(&thread).exists()
      && std::panic::catch_unwind(|| shown(&mac, &mac_home, &thread))
        .is_ok_and(|t| ids(&t) == ["m1", "m2", "m5", "m6"])
  })
  .await;
  assert_eq!(shown(&mac, &mac_home, &thread), line);
  mac_node.stop().await.unwrap();
  phone_node.stop().await.unwrap();
}
