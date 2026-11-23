//! Scopes and rooms as the app sees them (design §2.27, docs/rooms.md
//! §3–§5): a room pulled out of a 1:1 chat ("Pull in model…"), listed with
//! its participants, a message to it with a scope chip and an `@mention`,
//! its reply with its speaker, and its roster and policy changed.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use stitch_ffi::{ChatListener, CompactionInfo, ParticipantSpec, ScopeRef, Stitch};
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

/// A thread's record file under `dir`, as text.
fn record_of(dir: &std::path::Path, id: &str) -> Option<String> {
  for e in std::fs::read_dir(dir).ok()?.flatten() {
    let p = e.path();
    if p.is_dir() {
      if let Some(r) = record_of(&p, id) {
        return Some(r);
      }
    } else if let Ok(text) = std::fs::read_to_string(&p)
      && text.contains(&format!("id = \"{id}\""))
      && text.contains("kind = \"thread\"")
    {
      return Some(text);
    }
  }
  None
}

#[test]
fn a_room_pulled_out_of_a_chat_reads_with_speakers_and_scopes_in_the_app() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let base = serve(vec![
    answer("Hi."),
    answer("Agree."),
    answer("Agree."),
    answer("Agree."),
    answer("Agree."),
  ]);
  let mut config = Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    apps: Some(AppsSettings { port: Some(0) }),
    default_model: Some("anthropic/claude-opus-5-5".into()),
    threads: Some(ThreadSettings {
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
      ..ProviderConfig::default()
    },
  );
  config.save(&data).unwrap();
  keys::set(&data, "anthropic", "sk-test", Store::File).unwrap();
  let stitch = Stitch::open(data.display().to_string(), "phone".into()).unwrap();
  let chat = |thread: Option<String>, text: &str, scopes: Vec<ScopeRef>| {
    let heard = Arc::new(Heard::default());
    let out = wait(stitch.chat(
      thread,
      None,
      None,
      text.into(),
      Vec::new(),
      None,
      false,
      scopes,
      heard.clone(),
    ))
    .unwrap();
    (out, heard)
  };
  let (first, _) = chat(None, "hello", Vec::new());
  let one = first.thread;

  // "Pull in model…": the chat joins first, a fresh model after it.
  let room = wait(stitch.room_create(
    Some("Review".into()),
    None,
    Some(one.clone()),
    vec![ParticipantSpec {
      model: Some("anthropic/claude-sonnet-5".into()),
      thread: None,
      name: Some("Sonnet".into()),
      slug: Some("sonnet".into()),
      node: None,
      tools: true,
      read_only: true,
    }],
    Vec::new(),
    None,
    None,
    None,
  ))
  .unwrap();
  assert_eq!(room.promoted_from.as_deref(), Some(one.as_str()));
  assert_eq!(room.mode, "sequential");
  assert_eq!(room.participants.len(), 2, "{:?}", room.participants);
  assert_eq!(room.participants[0].thread, one);
  let sonnet = room.participants[1].clone();
  assert_eq!(sonnet.slug, "sonnet");

  // Listed as a room, its participants with it.
  let threads = wait(stitch.clone().threads()).unwrap();
  let row = threads
    .iter()
    .find(|t| t.id == room.room)
    .expect("the room listed");
  assert!(row.room);
  assert_eq!(row.room_mode.as_deref(), Some("sequential"));
  assert_eq!(row.participants, room.participants);
  assert!(
    threads
      .iter()
      .filter(|t| t.id != room.room)
      .all(|t| !t.room)
  );

  // A message to it, @sonnet alone, the chat attached as a scope.
  let (done, heard) = chat(
    Some(room.room.clone()),
    "what do you think, @sonnet?",
    vec![ScopeRef {
      kind: "thread".into(),
      id: one.clone(),
    }],
  );
  assert_eq!(done.thread, room.room);
  let notes = heard.0.lock().unwrap().clone();
  assert!(
    notes.iter().any(|n| n.starts_with("note [Sonnet · ")),
    "{notes:?}"
  );
  let entries = wait(stitch.clone().transcript(room.room.clone())).unwrap();
  let asked = entries
    .iter()
    .find(|e| e.who == "you" && e.text.contains("@sonnet"))
    .expect("the message");
  assert_eq!(asked.scopes.len(), 1, "{:?}", asked.scopes);
  assert_eq!(asked.scopes[0].kind, "thread");
  assert_eq!(asked.scopes[0].id, one);
  assert!(asked.speaker.is_none());
  let reply = entries
    .iter()
    .find(|e| e.speaker.is_some())
    .expect("a participant's reply");
  let speaker = reply.speaker.clone().unwrap();
  assert_eq!(speaker.slug, "sonnet");
  assert_eq!(speaker.name, "Sonnet");
  assert_eq!(speaker.thread.as_deref(), Some(sonnet.thread.as_str()));
  assert!(reply.text.contains("Agree."), "{}", reply.text);

  // Added as the app's 3-way choice makes them (design §2.27): no tools,
  // and can write; each record says so (`tools = false`; no read-only).
  let spec = |model: &str, slug: &str, tools: bool, read_only: bool| ParticipantSpec {
    model: Some(model.into()),
    thread: None,
    name: None,
    slug: Some(slug.into()),
    node: None,
    tools,
    read_only,
  };
  let grown = wait(stitch.room_update(
    room.room.clone(),
    vec![
      spec("anthropic/claude-haiku-5", "haiku", false, true),
      spec("anthropic/claude-opus-5-5", "opus", true, false),
    ],
    Vec::new(),
    Vec::new(),
    Vec::new(),
    None,
    None,
  ))
  .unwrap();
  let of = |slug: &str| {
    let t = &grown
      .participants
      .iter()
      .find(|p| p.slug == slug)
      .unwrap()
      .thread;
    record_of(dir.path(), t).unwrap_or_else(|| panic!("no record of {t}"))
  };
  let (haiku, opus) = (of("haiku"), of("opus"));
  assert!(haiku.contains("tools = false"), "{haiku}");
  assert!(!haiku.contains("approval"), "{haiku}");
  assert!(!opus.contains("tools = false"), "{opus}");
  assert!(!opus.contains("approval = \"read-only\""), "{opus}");
  let row = |slug: &str| {
    let t = grown
      .participants
      .iter()
      .find(|p| p.slug == slug)
      .unwrap()
      .thread
      .clone();
    wait(stitch.clone().threads())
      .unwrap()
      .into_iter()
      .find(|r| r.id == t)
      .unwrap()
  };
  assert!(!row("opus").read_only);
  assert!(row("sonnet").read_only);
  wait(stitch.room_update(
    room.room.clone(),
    Vec::new(),
    vec!["haiku".into(), "opus".into()],
    Vec::new(),
    Vec::new(),
    None,
    None,
  ))
  .unwrap();

  // Muted, and a council: as the room now says.
  let after = wait(stitch.room_update(
    room.room.clone(),
    Vec::new(),
    Vec::new(),
    vec!["sonnet".into()],
    Vec::new(),
    Some("parallel".into()),
    Some(0),
  ))
  .unwrap();
  assert_eq!(after.mode, "parallel");
  assert_eq!(after.hop_cap, 0);
  assert!(
    after
      .participants
      .iter()
      .any(|p| p.slug == "sonnet" && p.muted)
  );
  assert_eq!(wait(stitch.room_info(room.room.clone())).unwrap(), after);
  // A 1:1 chat's transcript names no speakers.
  let plain = wait(stitch.clone().transcript(one)).unwrap();
  assert!(plain.iter().all(|e| e.speaker.is_none()));
  wait(stitch.stop()).unwrap();
}
