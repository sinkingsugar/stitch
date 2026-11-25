//! Calls (design §2.3, "Calls"): an agent on the Mac rings the phone with
//! `call_user`. The phone registered its PushKit token; the Mac sends the
//! call as a VoIP push through the relay (a mock here), sealed for the
//! phone; the phone, opening it, tells the Mac how it went
//! (`Request::RingOutcome` on its own node, passed on as
//! `Question::Ring`), and the waiting tool says so to the model. A second
//! call right after is refused by the node's limit.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{ApnsSettings, AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers HTTP requests in order (a model's, the relay's), recording
/// each one's path and JSON body.
async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
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
          let path = head
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
          log.lock().unwrap().push((
            path,
            serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap(),
          ));
          break;
        }
      }
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

const OK: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\nconnection: close\r\n\r\n{\"ok\":true}";

fn records(data: &Path, kind: &str) -> Vec<stitch_store::IndexedRecord> {
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .records(Some(kind))
    .unwrap()
}

/// What the model was given as the result of tool call `id`.
fn result_of(body: &Value, id: &str) -> String {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
    .find(|c| c["type"] == "tool_result" && c["tool_use_id"] == id)
    .map(|c| c["content"].to_string())
    .unwrap_or_else(|| panic!("no result of {id} in {body}"))
}

fn chat(text: &str) -> Request {
  Request::Chat {
    thread: None,
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

#[tokio::test(flavor = "multi_thread")]
async fn a_call_rings_the_phone_and_its_answer_reaches_the_agent() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, _) = serve(vec![]).await;
  let (mac_api, mac_sent) = serve(vec![
    tool_call(
      "c1",
      "call_user",
      json!({"reason": "the build on beast is stuck"}),
    ),
    tool_call("c2", "call_user", json!({"reason": "still stuck"})),
    answer("Calling you about the build."),
  ])
  .await;
  let (relay, relayed) = serve(vec![OK.to_owned()]).await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let mut c = Config::load(&mac).unwrap();
  c.apns = Some(ApnsSettings {
    relay: Some(relay.clone()),
    ..Default::default()
  });
  c.save(&mac).unwrap();
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let _phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let _mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();

  // The phone registers for calls: its PushKit token, its push key.
  let private = [9u8; 32];
  let public =
    base64::engine::general_purpose::STANDARD.encode(stitch_node::apns::public_of(&private));
  let voip = "cafe".repeat(16);
  let r = p
    .request(&Request::RegisterVoip {
      token: voip.clone(),
      env: "sandbox".into(),
      key: public,
    })
    .await
    .unwrap();
  assert!(matches!(r, Reply::Ok), "{r:?}");
  m.request(&Request::Sync { peer: None }).await.unwrap();
  until("the phone's registration on the Mac", || {
    records(&mac, "device")
      .iter()
      .any(|r| r.fields["voip"] == json!(voip))
  })
  .await;

  // The Mac's agent calls; the turn waits for the phone's word.
  let turn = tokio::spawn(async move { m.chat(&chat("watch the build"), |_| None).await });
  until("the VoIP push at the relay", || {
    !relayed.lock().unwrap().is_empty()
  })
  .await;
  let (path, body) = relayed.lock().unwrap()[0].clone();
  assert_eq!(path, "/voip");
  assert_eq!(body["token"], json!(voip));
  assert_eq!(body["env"], "sandbox");
  // Nothing in the clear but the token: the call is sealed for the phone.
  assert_eq!(body.as_object().unwrap().len(), 3, "{body}");
  let sealed = stitch_node::apns::open(&private, body["e"].as_str().unwrap()).expect("opens");
  let call: Value = serde_json::from_slice(&sealed).unwrap();
  assert_eq!(call["node"], "mac");
  assert_eq!(call["caller"], "Stitch: the build on beast is stuck");
  assert!(call["thread"].as_str().unwrap().contains("-mac-"), "{call}");
  let id = call["ring"].as_str().unwrap().to_owned();

  // An outcome for a call nobody rang is said, not taken.
  let r = p
    .request(&Request::RingOutcome {
      node: "mac".into(),
      call: id.clone(),
      outcome: "maybe".into(),
    })
    .await;
  assert!(!matches!(r, Ok(Reply::Ok)), "{r:?}");
  // The user answers on the phone.
  let r = p
    .request(&Request::RingOutcome {
      node: "mac".into(),
      call: id,
      outcome: "answered".into(),
    })
    .await
    .unwrap();
  assert!(matches!(r, Reply::Ok), "{r:?}");
  let done = tokio::time::timeout(Duration::from_secs(30), turn)
    .await
    .expect("the turn ends once the phone said")
    .unwrap()
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  let bodies = mac_sent.lock().unwrap();
  let first = result_of(&bodies[1].1, "c1");
  assert!(first.contains("The user answered"), "{first}");
  // A second call at once: the node's limit, said to the model.
  let second = result_of(&bodies[2].1, "c2");
  assert!(second.contains("not called"), "{second}");
  assert_eq!(relayed.lock().unwrap().len(), 1, "rung once");
}
