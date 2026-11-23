//! The bridge's node side, end to end without Discord: `LocalNode` asks a
//! real node for a turn on behalf of a Discord user; the node's provider
//! is the key-holding proxy (in process), which forwards to a mock
//! Anthropic with the real key and counts the turn for that user and
//! thread.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::json;
use stitch_discord::LocalNode;
use stitch_discord::bridge::Node;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A mock Anthropic: every POST gets a short streamed answer, every GET
/// a model; the request heads it saw are kept.
async fn upstream() -> (String, Arc<Mutex<Vec<String>>>) {
  let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("http://{}", l.local_addr().unwrap());
  let heads = Arc::new(Mutex::new(Vec::new()));
  let seen = heads.clone();
  tokio::spawn(async move {
    loop {
      let (mut s, _) = l.accept().await.unwrap();
      let seen = seen.clone();
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut b = [0u8; 65536];
        let end = loop {
          let n = s.read(&mut b).await.unwrap();
          if n == 0 {
            return;
          }
          buf.extend_from_slice(&b[..n]);
          if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
          }
        };
        let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
        let len: usize = head
          .lines()
          .find_map(|l| l.strip_prefix("content-length:"))
          .map(|v| v.trim().parse().unwrap())
          .unwrap_or(0);
        while buf.len() < end + len {
          let n = s.read(&mut b).await.unwrap();
          buf.extend_from_slice(&b[..n]);
        }
        seen.lock().unwrap().push(head.clone());
        let reply = if head.starts_with("post") {
          let events = [
            json!({"type": "message_start", "message": {"model": "claude-mock", "usage": {"input_tokens": 100, "output_tokens": 1}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello there."}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 7}}),
            json!({"type": "message_stop"}),
          ];
          let body: String = events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
            .collect();
          format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
          )
        } else {
          let body = json!({"id": "claude-mock", "max_input_tokens": 200000}).to_string();
          format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
          )
        };
        let _ = s.write_all(reply.as_bytes()).await;
        let _ = s.shutdown().await;
      });
    }
  });
  (url, heads)
}

fn setup_node(data: &Path, home: &Path, proxy: &str, token: &str) {
  let mut c = Config {
    node: Some("discordagent".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    default_model: Some("anthropic/claude-mock".into()),
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(format!("{proxy}/anthropic")),
      ..ProviderConfig::default()
    },
  );
  c.save(data).unwrap();
  // The node's key is the proxy's token, nothing more.
  keys::set(data, "anthropic", token, Store::File).unwrap();
  let root = RecordRoot::open(home)
    .unwrap_or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap());
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_discord_turn_goes_through_the_proxy_and_is_counted_for_its_user() {
  let dir = tempfile::tempdir().unwrap();
  let (up, heads) = upstream().await;
  // The proxy, holding the real key.
  let pdir = dir.path().join("proxy");
  let mut pc = stitch_proxy::Config::default();
  pc.upstreams.insert(
    "anthropic".into(),
    stitch_proxy::config::Upstream {
      kind: "anthropic".into(),
      base_url: up.clone(),
      key: None,
    },
  );
  pc.save(&pdir).unwrap();
  let token = stitch_proxy::add_token(&pdir, "discord-agent", Default::default()).unwrap();
  let keys: stitch_proxy::Keys =
    Arc::new(|name: &str, _: &str| (name == "anthropic").then(|| "sk-ant-REAL".to_owned()));
  let proxy = stitch_proxy::Proxy::new(&pdir, keys).unwrap();
  let ls = stitch_proxy::bind(&["127.0.0.1:0".into()]).await.unwrap();
  let purl = format!("http://{}", ls[0].local_addr().unwrap());
  tokio::spawn(stitch_proxy::serve(proxy, ls));

  let data = dir.path().join("node");
  setup_node(&data, &dir.path().join("home"), &purl, &token);
  let node = stitch_node::start(&data).await.unwrap();
  let local = LocalNode {
    data: data.clone(),
    model: None,
  };
  let turn = local
    .chat(None, "Thrall: hi".into(), "discord:42".into())
    .await
    .unwrap();
  assert_eq!(turn.text, "Hello there.");
  assert!(turn.declined.is_empty());
  // Continuing the thread.
  let again = local
    .chat(
      Some(turn.thread.clone()),
      "Jaina: and?".into(),
      "discord:43".into(),
    )
    .await
    .unwrap();
  assert_eq!(again.thread, turn.thread);

  let heads = heads.lock().unwrap().clone();
  let posts: Vec<&String> = heads
    .iter()
    .filter(|h| h.starts_with("post /v1/messages"))
    .collect();
  assert_eq!(posts.len(), 2, "{heads:?}");
  for h in &heads {
    assert!(h.contains("x-api-key: sk-ant-real"), "{h}");
    assert!(!h.contains("stp_") && !h.contains("x-stitch-"), "{h}");
  }
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  let book = stitch_proxy::usage::Book::load(&pdir);
  let day = book
    .day(&stitch_proxy::usage::today(), "discord-agent")
    .unwrap();
  assert_eq!(day.users["discord:42"].requests, 1);
  assert_eq!(day.users["discord:42"].output, 7);
  assert_eq!(day.users["discord:43"].requests, 1);
  let log = std::fs::read_to_string(pdir.join(stitch_proxy::usage::LOG)).unwrap();
  assert!(
    log.contains(&format!("\"thread\":\"{}\"", turn.thread)),
    "{log}"
  );
  drop(node);
}
