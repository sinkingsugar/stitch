//! A compaction as the app sees it (design §2.15, "One conversation"): the
//! chat's listener told, one row in the list, the old ID resolving to the
//! newest, and the transcript read over the chain with its divider.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use stitch_ffi::{ChatListener, CompactionInfo, Stitch};
use stitch_node::config::{AppsSettings, Config, ProviderConfig, ThreadSettings};
use stitch_node::keys::{self, Store};

fn wait<T>(f: impl std::future::Future<Output = T>) -> T {
  tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap()
    .block_on(f)
}

/// A provider answering each request with the next response, on a
/// thread of its own.
fn serve(responses: Vec<String>) -> String {
  let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  std::thread::spawn(move || {
    for response in responses {
      let (mut sock, _) = listener.accept().unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 65536];
      loop {
        let n = sock.read(&mut chunk).unwrap();
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
            let n = sock.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
          }
          break;
        }
      }
      sock.write_all(response.as_bytes()).unwrap();
      let _ = sock.shutdown(std::net::Shutdown::Both);
    }
  });
  base
}

/// An Anthropic stream answering `text`, its prompt 80 tokens.
fn answer(text: &str) -> String {
  let events = [
    (
      "message_start",
      r#"{"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":80}}}"#
        .to_owned(),
    ),
    (
      "content_block_start",
      r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#
        .to_owned(),
    ),
    (
      "content_block_delta",
      format!(
        r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{text}"}}}}"#
      ),
    ),
    (
      "content_block_stop",
      r#"{"type":"content_block_stop","index":0}"#.to_owned(),
    ),
    (
      "message_delta",
      r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#
        .to_owned(),
    ),
    ("message_stop", r#"{"type":"message_stop"}"#.to_owned()),
  ];
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// What a chat streamed, in order.
#[derive(Default)]
struct Heard(Mutex<Vec<String>>, Mutex<Option<CompactionInfo>>);

impl ChatListener for Heard {
  fn thread(&self, id: String) {
    self.0.lock().unwrap().push(format!("thread {id}"));
  }
  fn text(&self, _: String) {}
  fn thinking(&self, _: String) {}
  fn tool_call(&self, _: String) {}
  fn tool_result(&self, _: bool, _: String) {}
  fn note(&self, text: String) {
    self.0.lock().unwrap().push(format!("note {text}"));
  }
  fn compacted(&self, c: CompactionInfo) {
    self.0.lock().unwrap().push(format!("compacted {}", c.to));
    *self.1.lock().unwrap() = Some(c);
  }
  fn retry(&self, _: String) {}
  fn woven(&self, _: Vec<u64>) {}
}

#[test]
fn a_compacted_chat_reads_as_one_conversation_in_the_app() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let base = serve(vec![
    answer("Hi."),
    answer("SUMMARY-WORDS"),
    answer("Second."),
  ]);
  // Its first step sends 80 tokens of a 90-token window: the next turn
  // compacts first.
  let mut config = Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    apps: Some(AppsSettings { port: Some(0) }),
    default_model: Some("anthropic/claude-opus-5-5".into()),
    threads: Some(ThreadSettings {
      compact_at: Some(80),
      compact_keep: Some(2),
      titles: Some(false),
      ..Default::default()
    }),
    ..Config::default()
  };
  config.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(base),
      context_window: Some(90),
      ..ProviderConfig::default()
    },
  );
  config.save(&data).unwrap();
  keys::set(&data, "anthropic", "sk-test", Store::File).unwrap();
  let stitch = Stitch::open(data.display().to_string(), "phone".into()).unwrap();

  let chat = |thread: Option<String>, text: &str| {
    let heard = Arc::new(Heard::default());
    let out = wait(stitch.chat(
      thread,
      None,
      None,
      text.into(),
      Vec::new(),
      None,
      false,
      Vec::new(),
      heard.clone(),
    ))
    .unwrap();
    (out, heard)
  };
  let (first, _) = chat(None, "hello");
  let old = first.thread;
  wait(stitch.pin(old.clone(), true)).unwrap();
  wait(stitch.rename(old.clone(), "My talk".into())).unwrap();

  let (second, heard) = chat(Some(old.clone()), "again");
  let new = second.thread;
  assert_ne!(new, old);
  let c = heard
    .1
    .lock()
    .unwrap()
    .clone()
    .expect("told of the compaction");
  assert_eq!(
    (c.from.as_str(), c.to.as_str()),
    (old.as_str(), new.as_str())
  );
  assert_eq!(c.window, 90);
  assert!(c.said.starts_with("compacted here ("), "{}", c.said);
  assert!(c.summary.contains("SUMMARY-WORDS"), "{}", c.summary);
  // The divider before the thread it goes on in, and no note for it.
  let events = heard.0.lock().unwrap().clone();
  let at = events
    .iter()
    .position(|e| e.starts_with("compacted"))
    .unwrap();
  assert!(
    events[at + 1..].contains(&format!("thread {new}")),
    "{events:?}"
  );
  assert!(!events.iter().any(|e| e.contains("── ")), "{events:?}");

  // One row: the newest, the chain's pin and title, the old one earlier.
  let threads = wait(stitch.clone().threads()).unwrap();
  assert_eq!(
    threads.len(),
    1,
    "{:?}",
    threads.iter().map(|t| &t.id).collect::<Vec<_>>()
  );
  assert_eq!(threads[0].id, new);
  assert!(threads[0].pinned);
  assert_eq!(threads[0].title, "My talk");
  assert_eq!(threads[0].earlier, vec![old.clone()]);

  // Whatever the app kept, it opens the newest.
  assert_eq!(stitch.newest(old.clone()), new);
  assert_eq!(stitch.newest(new.clone()), new);
  assert_eq!(stitch.newest("not-an-id".into()), "not-an-id");

  // The transcript over the chain: the old thread's, the divider with its
  // compaction, the new one's; the old messages with no IDs.
  let entries = wait(stitch.clone().transcript(new.clone())).unwrap();
  let d = entries
    .iter()
    .position(|e| e.who == "compacted")
    .expect("a divider");
  assert_eq!(entries[d].compaction.as_ref(), Some(&c));
  assert_eq!(entries[d].text, c.said);
  assert!(
    entries[..d]
      .iter()
      .any(|e| e.text == "hello" && e.id.is_none())
  );
  assert!(entries[d + 1..].iter().any(|e| e.text.contains("Second.")));
  assert!(entries.iter().filter(|e| e.compaction.is_some()).count() == 1);
  // The old thread as it was: no divider, nothing after it.
  let before = wait(stitch.clone().transcript(old)).unwrap();
  assert!(before.iter().all(|e| e.who != "compacted"));
  assert!(!before.iter().any(|e| e.text.contains("Second.")));
  wait(stitch.stop()).unwrap();
}
