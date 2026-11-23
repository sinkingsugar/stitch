//! The proxy against a mock upstream speaking raw HTTP/1.1, so the test
//! decides when each chunk of a stream goes out.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use super::*;

/// What the mock upstream received: the request head (lower-cased header
/// names) and body.
#[derive(Debug)]
struct Seen {
  head: String,
  body: String,
}

/// How the mock answers one request: a status, a content type, and the
/// chunks of its body, each sent when its gate opens (`None`: at once).
struct Answer {
  status: u16,
  content_type: &'static str,
  chunks: Vec<(String, Option<oneshot::Receiver<()>>)>,
}

async fn read_request(s: &mut TcpStream) -> Seen {
  let mut buf = Vec::new();
  let mut b = [0u8; 4096];
  let head_end = loop {
    let n = s.read(&mut b).await.unwrap();
    assert!(n > 0, "the proxy closed before a request");
    buf.extend_from_slice(&b[..n]);
    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
      break p + 4;
    }
  };
  let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
  let len: usize = head
    .lines()
    .find_map(|l| l.strip_prefix("content-length:"))
    .map(|v| v.trim().parse().unwrap())
    .unwrap_or(0);
  while buf.len() < head_end + len {
    let n = s.read(&mut b).await.unwrap();
    buf.extend_from_slice(&b[..n]);
  }
  Seen {
    head,
    body: String::from_utf8_lossy(&buf[head_end..head_end + len]).into_owned(),
  }
}

/// A mock upstream: answers each connection's request with the next
/// answer, telling what it saw.
async fn upstream() -> (
  String,
  mpsc::UnboundedSender<Answer>,
  mpsc::UnboundedReceiver<Seen>,
) {
  let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("http://{}", l.local_addr().unwrap());
  let (atx, mut arx) = mpsc::unbounded_channel::<Answer>();
  let (stx, srx) = mpsc::unbounded_channel();
  tokio::spawn(async move {
    loop {
      let (mut s, _) = l.accept().await.unwrap();
      let seen = read_request(&mut s).await;
      let _ = stx.send(seen);
      let Some(a) = arx.recv().await else { return };
      let head = format!(
        "HTTP/1.1 {} X\r\ncontent-type: {}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
        a.status, a.content_type
      );
      s.write_all(head.as_bytes()).await.unwrap();
      for (chunk, gate) in a.chunks {
        if let Some(g) = gate {
          let _ = g.await;
        }
        let c = format!("{:x}\r\n{chunk}\r\n", chunk.len());
        s.write_all(c.as_bytes()).await.unwrap();
        s.flush().await.unwrap();
      }
      s.write_all(b"0\r\n\r\n").await.unwrap();
      let _ = s.shutdown().await;
    }
  });
  (url, atx, srx)
}

struct Running {
  url: String,
  dir: tempfile::TempDir,
  _task: tokio::task::JoinHandle<()>,
}

async fn proxy(upstream_url: &str, token: config::Token) -> (Running, String) {
  let dir = tempfile::tempdir().unwrap();
  let mut c = Config::default();
  c.upstreams.insert(
    "anthropic".into(),
    config::Upstream {
      kind: "anthropic".into(),
      base_url: upstream_url.into(),
      key: Some("real-anthropic".into()),
    },
  );
  c.upstreams.insert(
    "local".into(),
    config::Upstream {
      kind: "openai".into(),
      base_url: format!("{upstream_url}/v1"),
      key: None,
    },
  );
  c.save(dir.path()).unwrap();
  let clear = add_token(dir.path(), "bot", token).unwrap();
  let keys: Keys = Arc::new(|name: &str, _kind: &str| match name {
    "real-anthropic" => Some("sk-ant-REAL".into()),
    "local" => Some("sk-local-REAL".into()),
    _ => None,
  });
  let p = Proxy::new(dir.path(), keys).unwrap();
  let ls = bind(&["127.0.0.1:0".into()]).await.unwrap();
  let url = format!("http://{}", ls[0].local_addr().unwrap());
  let task = tokio::spawn(serve(p, ls));
  (
    Running {
      url,
      dir,
      _task: task,
    },
    clear,
  )
}

fn sse(event: &str, data: serde_json::Value) -> String {
  format!("event: {event}\ndata: {data}\n\n")
}

#[tokio::test]
async fn streams_unbuffered_with_the_real_key_and_counts_usage() {
  let (up, answers, mut seen) = upstream().await;
  let (p, token) = proxy(&up, config::Token::default()).await;
  let (open, gate) = oneshot::channel();
  answers
    .send(Answer {
      status: 200,
      content_type: "text/event-stream",
      chunks: vec![
        (
          sse(
            "message_start",
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":500,"output_tokens":1}}}),
          ),
          None,
        ),
        (
          sse(
            "message_delta",
            json!({"type":"message_delta","usage":{"output_tokens":25}}),
          ),
          Some(gate),
        ),
      ],
    })
    .unwrap();
  let r = reqwest::Client::new()
    .post(format!("{}/anthropic/v1/messages", p.url))
    .header("x-api-key", &token)
    .header("anthropic-version", "2023-06-01")
    .header(END_USER, "discord:42")
    .header(THREAD, "t-1")
    .body(r#"{"model":"claude-x","stream":true}"#)
    .send()
    .await
    .unwrap();
  assert_eq!(r.status(), 200);
  let mut body = r.bytes_stream();
  // The first event arrives while the upstream holds the second back.
  let first = tokio::time::timeout(Duration::from_secs(5), body.next())
    .await
    .expect("the first chunk passes before the stream ends")
    .unwrap()
    .unwrap();
  assert!(String::from_utf8_lossy(&first).contains("message_start"));
  open.send(()).unwrap();
  let mut rest = Vec::new();
  while let Some(b) = body.next().await {
    rest.extend_from_slice(&b.unwrap());
  }
  assert!(String::from_utf8_lossy(&rest).contains("message_delta"));
  let s = seen.recv().await.unwrap();
  assert!(s.head.starts_with("post /v1/messages "), "{}", s.head);
  assert!(s.head.contains("x-api-key: sk-ant-real"), "{}", s.head);
  assert!(s.head.contains("anthropic-version: 2023-06-01"));
  assert!(
    !s.head.contains("stp_") && !s.head.contains("x-stitch-"),
    "{}",
    s.head
  );
  assert_eq!(s.body, r#"{"model":"claude-x","stream":true}"#);
  // Recorded once the stream ended, kept on disk.
  tokio::time::sleep(Duration::from_millis(100)).await;
  let book = Book::load(p.dir.path());
  let d = book.day(&usage::today(), "bot").unwrap();
  assert_eq!(
    d.total,
    Counts {
      requests: 1,
      input: 10,
      output: 25,
      cache_read: 500,
      cache_write: 0
    }
  );
  assert_eq!(d.users["discord:42"].output, 25);
  let log = std::fs::read_to_string(p.dir.path().join(usage::LOG)).unwrap();
  assert!(
    log.contains("\"thread\":\"t-1\"") && log.contains("\"model\":\"claude-x\""),
    "{log}"
  );
}

#[tokio::test]
async fn refuses_strangers_other_paths_and_spent_budgets() {
  let (up, answers, _seen) = upstream().await;
  let (p, token) = proxy(
    &up,
    config::Token {
      daily_tokens: Some(100),
      daily_tokens_per_user: Some(30),
      ..config::Token::default()
    },
  )
  .await;
  let c = reqwest::Client::new();
  let post = |path: &str, key: &str, user: &str| {
    c.post(format!("{}{path}", p.url))
      .header("x-api-key", key)
      .header(END_USER, user)
      .body("{}")
      .send()
  };
  assert_eq!(
    post("/anthropic/v1/messages", "stp_nope", "u")
      .await
      .unwrap()
      .status(),
    401
  );
  assert_eq!(
    post("/anthropic/v1/files", &token, "u")
      .await
      .unwrap()
      .status(),
    404
  );
  assert_eq!(
    post("/elsewhere/v1/messages", &token, "u")
      .await
      .unwrap()
      .status(),
    404
  );
  // A JSON answer's usage counts: 40 for user a, over its 30.
  answers
    .send(Answer {
      status: 200,
      content_type: "application/json",
      chunks: vec![(
        json!({"usage":{"input_tokens":30,"output_tokens":10}}).to_string(),
        None,
      )],
    })
    .unwrap();
  let r = post("/anthropic/v1/messages", &token, "a").await.unwrap();
  assert_eq!(r.status(), 200);
  r.bytes().await.unwrap();
  tokio::time::sleep(Duration::from_millis(100)).await;
  let r = post("/anthropic/v1/messages", &token, "a").await.unwrap();
  assert_eq!(r.status(), 403);
  assert!(r.text().await.unwrap().contains("your share"));
  // User b still has room; then the whole budget (100) is spent.
  answers
    .send(Answer {
      status: 200,
      content_type: "application/json",
      chunks: vec![(json!({"usage":{"input_tokens":70}}).to_string(), None)],
    })
    .unwrap();
  let r = post("/anthropic/v1/messages", &token, "b").await.unwrap();
  assert_eq!(r.status(), 200);
  r.bytes().await.unwrap();
  tokio::time::sleep(Duration::from_millis(100)).await;
  let r = post("/anthropic/v1/messages", &token, "c").await.unwrap();
  assert_eq!(r.status(), 403);
  assert!(
    r.text()
      .await
      .unwrap()
      .contains("today's token budget (100)")
  );
}

#[tokio::test]
async fn openai_kind_gets_a_bearer_key_and_rates_are_limited() {
  let (up, answers, mut seen) = upstream().await;
  let (p, token) = proxy(
    &up,
    config::Token {
      requests_per_minute: Some(2),
      ..config::Token::default()
    },
  )
  .await;
  let c = reqwest::Client::new();
  for _ in 0..2 {
    answers
      .send(Answer {
        status: 200,
        content_type: "application/json",
        chunks: vec![(json!({"data":[{"id":"m"}]}).to_string(), None)],
      })
      .unwrap();
    let r = c
      .get(format!("{}/local/models", p.url))
      .bearer_auth(&token)
      .send()
      .await
      .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.text().await.unwrap().contains("\"m\""));
    let s = seen.recv().await.unwrap();
    assert!(s.head.starts_with("get /v1/models "), "{}", s.head);
    assert!(
      s.head.contains("authorization: bearer sk-local-real"),
      "{}",
      s.head
    );
  }
  let r = c
    .get(format!("{}/local/models", p.url))
    .bearer_auth(&token)
    .send()
    .await
    .unwrap();
  assert_eq!(r.status(), 429);
  assert!(r.headers().contains_key("retry-after"));
  // A token added while it runs is taken at once.
  let other = add_token(p.dir.path(), "other", config::Token::default()).unwrap();
  answers
    .send(Answer {
      status: 200,
      content_type: "application/json",
      chunks: vec![(json!({"data":[]}).to_string(), None)],
    })
    .unwrap();
  let r = c
    .get(format!("{}/local/models", p.url))
    .bearer_auth(&other)
    .send()
    .await
    .unwrap();
  assert_eq!(r.status(), 200);
}
