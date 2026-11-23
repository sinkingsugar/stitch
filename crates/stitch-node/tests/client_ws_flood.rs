//! The web client's WebSocket under a flood (design §2.24), in a test binary
//! of its own: it counts the OS threads of the whole process, so no other
//! test may share its process (their runtimes' threads would be counted).
//! Only where it can count them (Linux, macOS): built for iOS and visionOS
//! (`just check-apple`), the whole file is out, helpers and imports too.
#![cfg(any(target_os = "linux", target_os = "macos"))]

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

/// The OS threads of this process.
#[cfg(target_os = "linux")]
fn os_threads() -> usize {
  std::fs::read_dir("/proc/self/task").unwrap().count()
}

/// The same on macOS: the task info of this process (`proc_pidinfo`).
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn os_threads() -> usize {
  let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
  let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
  let n = unsafe {
    libc::proc_pidinfo(
      libc::getpid(),
      libc::PROC_PIDTASKINFO,
      0,
      (&mut info as *mut libc::proc_taskinfo).cast(),
      size,
    )
  };
  assert_eq!(n, size, "proc_pidinfo failed");
  info.pti_threadnum as usize
}

/// One WebSocket that floods the node with `Threads` requests (a client
/// refreshing on every change): the node keeps answering the Unix socket
/// and the same connection's cheap requests, the blocking pool does not
/// grow past a small bound, and what the connection cannot take is refused
/// with a `busy` error, never dropped silently.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flooding_websocket_does_not_jam_the_node() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![]).await;
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

  let before = os_threads();
  let peak = Arc::new(std::sync::atomic::AtomicUsize::new(before));
  let sampling = {
    let peak = peak.clone();
    tokio::spawn(async move {
      loop {
        peak.fetch_max(os_threads(), std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
      }
    })
  };
  const FLOOD: u64 = 2000;
  for id in 100..100 + FLOOD {
    let req = if id % 2 == 0 {
      json!({"Threads": {"status": null, "since": null, "limit": 300}})
    } else {
      json!({"Live": {"what": "attention", "query": null}})
    };
    send(&mut ws, json!({"id": id, "request": req})).await;
  }
  // Meanwhile the node answers on its socket, promptly.
  let mut slowest = std::time::Duration::ZERO;
  for _ in 0..20 {
    let t = std::time::Instant::now();
    let r = tokio::time::timeout(
      std::time::Duration::from_secs(10),
      c.request(&Request::Status),
    )
    .await
    .expect("Status answers while the socket is flooded")
    .unwrap();
    assert!(matches!(r, Reply::Status { .. }));
    slowest = slowest.max(t.elapsed());
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
  }
  assert!(
    slowest < std::time::Duration::from_secs(3),
    "Status took {slowest:?} during the flood"
  );
  // Every request is answered once: a reply, or a `busy` refusal.
  let (mut replies, mut busy) = (0, 0);
  while replies + busy < FLOOD {
    let m = next(&mut ws).await;
    if m.get("changed").is_some() {
      continue;
    }
    if m["reply"]["Lines"].is_array() {
      replies += 1;
    } else if m["error"].as_str().is_some_and(|e| e.starts_with("busy")) {
      busy += 1;
    } else {
      panic!("unexpected frame {m}");
    }
  }
  sampling.abort();
  let grown = peak
    .load(std::sync::atomic::Ordering::SeqCst)
    .saturating_sub(before);
  assert!(
    grown <= 40,
    "the process grew by {grown} OS threads ({replies} answered, {busy} refused)"
  );
  assert!(replies >= 1);
  // The connection still works after the flood.
  send(&mut ws, json!({"id": 9000, "request": "Status"})).await;
  loop {
    let m = next(&mut ws).await;
    if m["id"] == 9000 {
      assert!(m["reply"]["Status"]["node"].is_string(), "{m}");
      break;
    }
  }
  node.stop().await.unwrap();
}
