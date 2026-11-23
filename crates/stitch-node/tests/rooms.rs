//! Rooms (design §2.27, docs/rooms.md §4): participants are threads, each
//! relayed only what it has not seen of the room (`Part::RoomTurn`), so its
//! earlier prompt stays byte for byte; sequential rounds with `[pass]`,
//! `@slug` targeting and a bounded `@handoff`; parallel (council) rounds
//! blind to each other; "Pull in model…" from a 1:1 thread; stopping a
//! round. Against a mock Anthropic endpoint that answers each participant
//! by the handle its room note gives it.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request, RoomParticipantSpec};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, DigestSettings, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What the mock answers: by participant handle (from the room note in
/// the last message), else `plain` (a 1:1 turn); `SLOW` holds the answer
/// for a minute.
type Scripts = Arc<Mutex<HashMap<String, VecDeque<String>>>>;
/// Each request: the handle it was for (`plain` without a room note) and
/// its body.
type Seen = Arc<Mutex<Vec<(String, Value)>>>;

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

/// The texts of a message's content.
fn texts(message: &Value) -> Vec<String> {
  match &message["content"] {
    Value::String(s) => vec![s.clone()],
    Value::Array(a) => a
      .iter()
      .filter_map(|b| b["text"].as_str().map(str::to_owned))
      .collect(),
    _ => Vec::new(),
  }
}

fn last_user(body: &Value) -> String {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .rev()
    .find(|m| m["role"] == "user")
    .map(|m| texts(m).join("\n"))
    .unwrap_or_default()
}

/// A tool call the mock makes (`TOOL:<name>:<input JSON>`), after some
/// text when given (`SAY:<text>|TOOL:<name>:<input JSON>`).
fn tool_use(name: &str, input: &str, text: Option<&str>) -> String {
  let id = format!("tu_{}", rand_id());
  let mut events = vec![(
    "message_start",
    json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 80}}}),
  )];
  let at = usize::from(text.is_some());
  if let Some(t) = text {
    events.extend([
      (
        "content_block_start",
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
      ),
      (
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}}),
      ),
      (
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
      ),
    ]);
  }
  events.extend([
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": at, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": at, "delta": {"type": "input_json_delta", "partial_json": input}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": at}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 5}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ]);
  sse(events)
}

fn rand_id() -> u64 {
  use std::sync::atomic::{AtomicU64, Ordering};
  static N: AtomicU64 = AtomicU64::new(0);
  N.fetch_add(1, Ordering::SeqCst)
}

/// Whether a user message holds only tool results (a step within a turn).
fn only_results(m: &Value) -> bool {
  m["content"]
    .as_array()
    .is_some_and(|a| !a.is_empty() && a.iter().all(|b| b["type"] == "tool_result"))
}

/// The handle a request's room note names: in the last user message, or,
/// within a turn (tool results since), the one that began it.
fn handle_of(body: &Value) -> Option<String> {
  let users: Vec<&Value> = body["messages"]
    .as_array()?
    .iter()
    .filter(|m| m["role"] == "user")
    .collect();
  let begun = users.iter().rev().find(|m| !only_results(m))?;
  let text = texts(begun).join("\n");
  let at = text.find("you are \"")? + 9;
  let end = text[at..].find('"')? + at;
  Some(text[at..end].to_owned())
}

async fn serve(scripts: Scripts) -> (String, Seen) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen: Seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    loop {
      let Ok((mut sock, _)) = listener.accept().await else {
        return;
      };
      let (log, scripts) = (log.clone(), scripts.clone());
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let body = loop {
          let Ok(n) = sock.read(&mut chunk).await else {
            return;
          };
          if n == 0 {
            return;
          }
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
            break serde_json::from_slice::<Value>(&buf[end + 4..end + 4 + len]).ok();
          }
        };
        let Some(body) = body else {
          let _ = sock
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
            .await;
          return;
        };
        let who = handle_of(&body).unwrap_or_else(|| "plain".into());
        let said = scripts
          .lock()
          .unwrap()
          .get_mut(&who)
          .and_then(VecDeque::pop_front)
          .unwrap_or_else(|| "ok".into());
        log.lock().unwrap().push((who, body));
        if said == "SLOW" {
          tokio::time::sleep(Duration::from_secs(60)).await;
        }
        // `LATE:<text>`: the text, after a moment to send something meanwhile.
        let said = match said.strip_prefix("LATE:") {
          Some(t) => {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            t.to_owned()
          }
          None => said,
        };
        let (text, call) = match said.strip_prefix("SAY:").and_then(|t| t.split_once('|')) {
          Some((t, call)) => (Some(t), call),
          None => (None, said.as_str()),
        };
        let reply = match call.strip_prefix("TOOL:").and_then(|t| t.split_once(':')) {
          Some((name, input)) => tool_use(name, input, text),
          None => answer(&said),
        };
        let _ = sock.write_all(reply.as_bytes()).await;
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

fn scripts(of: &[(&str, &[&str])]) -> Scripts {
  Arc::new(Mutex::new(
    of.iter()
      .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
      .collect(),
  ))
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
    digests: Some(DigestSettings {
      enabled: Some(false),
      ..Default::default()
    }),
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
  let root = RecordRoot::open(home)
    .or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home))
    .unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

struct Node {
  _dir: tempfile::TempDir,
  data: std::path::PathBuf,
  home: std::path::PathBuf,
  handle: Option<stitch_node::NodeHandle>,
}

impl Node {
  async fn start(base: &str) -> Node {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let (data, home) = (d.join("data"), d.join("home"));
    std::fs::create_dir_all(&home).unwrap();
    setup(&data, &home, base);
    let handle = stitch_node::start(&data).await.unwrap();
    Node {
      _dir: dir,
      data,
      home,
      handle: Some(handle),
    }
  }

  async fn client(&self) -> Client {
    Client::connect(&self.data).await.unwrap()
  }

  async fn stop(mut self) {
    self.handle.take().unwrap().stop().await.unwrap();
  }
}

fn model(m: &str) -> RoomParticipantSpec {
  RoomParticipantSpec {
    model: Some(format!("anthropic/{m}")),
    thread: None,
    name: None,
    slug: None,
    node: None,
    tools: false,
    read_only: true,
  }
}

fn chat(thread: Option<&str>, text: &str, model: Option<&str>) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: model.map(str::to_owned),
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

async fn json_of(c: &mut Client, r: &Request) -> Value {
  match c.request(r).await.unwrap() {
    Reply::Lines(l) => serde_json::from_str(&l[0]).unwrap(),
    other => panic!("{other:?}"),
  }
}

async fn room(c: &mut Client, models: &[&str], mode: Option<&str>, hop_cap: Option<u32>) -> Value {
  json_of(
    c,
    &Request::RoomCreate {
      title: Some("Review".into()),
      place: None,
      from_thread: None,
      participants: models.iter().map(|m| model(m)).collect(),
      scopes: Vec::new(),
      mode: mode.map(str::to_owned),
      hop_cap,
      budget_usd: None,
    },
  )
  .await
}

/// A chat in a room, to its end: the notes it streamed, and its stop.
async fn say(c: &mut Client, room: &str, text: &str) -> (Vec<String>, String) {
  let mut notes = Vec::new();
  let done = c
    .chat(&chat(Some(room), text, None), |r| {
      if let Reply::Note(n) = r {
        notes.push(n.clone());
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = done else {
    panic!("{done:?}")
  };
  (notes, stop)
}

async fn entries(c: &mut Client, thread: &str) -> (Vec<Value>, Value) {
  let v = json_of(
    c,
    &Request::Transcript {
      thread: thread.into(),
      after: None,
      before: None,
      tail: None,
    },
  )
  .await;
  (v["entries"].as_array().unwrap().clone(), v)
}

fn of(seen: &Seen, who: &str) -> Vec<Value> {
  seen
    .lock()
    .unwrap()
    .iter()
    .filter(|(w, _)| w == who)
    .map(|(_, b)| b.clone())
    .collect()
}

fn order(seen: &Seen) -> Vec<String> {
  seen
    .lock()
    .unwrap()
    .iter()
    .map(|(w, _)| w.clone())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sequential_room_relays_only_what_is_new_and_passes_quietly() {
  let s = scripts(&[
    ("opus", &["Opus says ship it.", "Opus round two."]),
    ("sonnet", &["Sonnet adds tests.", "[pass]"]),
    ("haiku", &["[PASS]", "Haiku has a new point."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
    None,
    None,
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let slugs: Vec<&str> = info["participants"]
    .as_array()
    .unwrap()
    .iter()
    .map(|p| p["slug"].as_str().unwrap())
    .collect();
  assert_eq!(slugs, ["opus", "sonnet", "haiku"]);
  assert_eq!(info["mode"], "sequential");
  assert_eq!(info["hop_cap"], 2);

  let (notes, stop) = say(&mut c, &r, "What now?").await;
  assert_eq!(stop, "end_turn");
  assert!(notes.iter().any(|n| n == "[Haiku passed]"), "{notes:?}");
  assert_eq!(order(&seen), ["opus", "sonnet", "haiku"]);
  // The first speaker always answers; the next may pass.
  let opus1 = of(&seen, "opus")[0].clone();
  assert!(!last_user(&opus1).contains("[pass]"));
  assert!(last_user(&opus1).contains("[User]: What now?"));
  // The second sees the first's reply of this round.
  let sonnet1 = last_user(&of(&seen, "sonnet")[0]);
  assert!(sonnet1.contains("[User]: What now?"), "{sonnet1}");
  assert!(
    sonnet1.contains("[Opus · anthropic/claude-opus-5-5]: Opus says ship it."),
    "{sonnet1}"
  );
  assert!(sonnet1.contains("reply with exactly [pass]"));
  assert!(
    last_user(&of(&seen, "haiku")[0])
      .contains("[Sonnet · anthropic/claude-sonnet-5-5]: Sonnet adds tests.")
  );

  // The room shows the user and two replies, with their speakers; the
  // pass wrote nothing.
  let (es, v) = entries(&mut c, &r).await;
  let shown: Vec<(String, String)> = es
    .iter()
    .map(|e| {
      (
        e["kind"].as_str().unwrap().to_owned(),
        e["text"].as_str().unwrap().to_owned(),
      )
    })
    .collect();
  assert_eq!(
    shown,
    [
      ("user".into(), "What now?".into()),
      ("assistant".into(), "Opus says ship it.".into()),
      ("assistant".into(), "Sonnet adds tests.".into()),
    ]
  );
  assert_eq!(es[1]["speaker"]["slug"], "opus");
  assert_eq!(es[1]["speaker"]["name"], "Opus");
  assert_eq!(es[1]["speaker"]["model"], "anthropic/claude-opus-5-5");
  assert_eq!(
    es[1]["speaker"]["thread"],
    info["participants"][0]["thread"]
  );
  assert_eq!(v["room"]["participants"].as_array().unwrap().len(), 3);

  // A participant's own thread shows where the relay stood, not the note.
  let opus_t = info["participants"][0]["thread"]
    .as_str()
    .unwrap()
    .to_owned();
  let (pe, _) = entries(&mut c, &opus_t).await;
  assert_eq!(pe[0]["room_turn"]["room"], json!(r));
  assert_eq!(pe[0]["room_turn"]["slug"], "opus");
  assert!(!pe[0]["text"].as_str().unwrap().contains("Stitch: you are"));

  // Round two: each gets only what came after its last turn; its earlier
  // prompt is sent again byte for byte.
  let (_, _) = say(&mut c, &r, "And then?").await;
  let opus2 = of(&seen, "opus")[1].clone();
  assert_eq!(
    opus2["messages"][0], opus1["messages"][0],
    "the prefix holds"
  );
  let delta = last_user(&opus2);
  assert!(delta.contains("[Sonnet · anthropic/claude-sonnet-5-5]: Sonnet adds tests."));
  assert!(delta.contains("[User]: And then?"));
  assert!(!delta.contains("What now?"), "{delta}");
  assert!(
    !delta.contains("Opus says ship it."),
    "its own reply is not relayed"
  );
  // Sonnet passes this time; Haiku, after it, sees Opus's new reply.
  let haiku2 = last_user(&of(&seen, "haiku")[1]);
  assert!(haiku2.contains("[Opus · anthropic/claude-opus-5-5]: Opus round two."));
  assert!(
    !haiku2.contains("Sonnet adds tests."),
    "seen in its last turn"
  );
  let (es, _) = entries(&mut c, &r).await;
  let last: Vec<&str> = es.iter().map(|e| e["text"].as_str().unwrap()).collect();
  assert_eq!(
    last[3..],
    ["And then?", "Opus round two.", "Haiku has a new point."]
  );

  // The thread list says it is a room, with its participants.
  let list = json_of(
    &mut c,
    &Request::Threads {
      status: None,
      since: None,
      limit: None,
      thread: Some(r.clone()),
      place: None,
      forest: None,
    },
  )
  .await;
  let row = &list["threads"][0];
  assert_eq!(row["room"], true);
  assert_eq!(row["room_mode"], "sequential");
  assert_eq!(row["participants"].as_array().unwrap().len(), 3);
  let plain = json_of(
    &mut c,
    &Request::Threads {
      status: None,
      since: None,
      limit: None,
      thread: Some(opus_t.clone()),
      place: None,
      forest: None,
    },
  )
  .await;
  assert!(plain["threads"][0].get("room").is_none());
  assert_eq!(plain["threads"][0]["parent"], json!(r));
  node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mentions_pick_the_speakers_and_handoffs_stop_at_the_cap() {
  let s = scripts(&[
    ("sonnet", &["Ask @haiku, then @opus."]),
    ("haiku", &["Over to @opus."]),
    ("opus", &["never asked"]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
    None,
    Some(1),
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  say(&mut c, &r, "@Sonnet what do you think?").await;
  // Only Sonnet was asked; it handed to Haiku (the first it named), and
  // the cap of one stopped Haiku's handoff to Opus.
  assert_eq!(order(&seen), ["sonnet", "haiku"]);
  let haiku = last_user(&of(&seen, "haiku")[0]);
  assert!(!haiku.contains("[pass]"), "addressed: it answers");
  assert!(haiku.contains("[Sonnet · anthropic/claude-sonnet-5-5]: Ask @haiku, then @opus."));
  // Muted participants are skipped unless named.
  let v = json_of(
    &mut c,
    &Request::RoomUpdate {
      thread: r.clone(),
      add: Vec::new(),
      remove: Vec::new(),
      mute: vec!["opus".into(), "haiku".into()],
      unmute: Vec::new(),
      mode: None,
      hop_cap: Some(0),
    },
  )
  .await;
  assert_eq!(v["participants"][0]["muted"], true);
  assert_eq!(v["hop_cap"], 0);
  let (_, _) = say(&mut c, &r, "Anyone?").await;
  assert_eq!(order(&seen), ["sonnet", "haiku", "sonnet"]);
  node.stop().await;
}

/// The room of 2026-11-14 (gemini, fable, gpt): agreeing with an earlier
/// speaker by `@handle` handed the floor back before the third had spoken.
/// Now everyone speaks once first, and a handoff goes to the back.
#[tokio::test(flavor = "multi_thread")]
async fn everyone_speaks_once_before_a_handoff_comes_back() {
  let s = scripts(&[
    ("opus", &["Opus: plan.", "Opus: answering Haiku."]),
    ("sonnet", &["Broadly agree with @opus."]),
    ("haiku", &["@opus, why that order?"]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
    None,
    None,
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  say(&mut c, &r, "Opinions?").await;
  assert_eq!(order(&seen), ["opus", "sonnet", "haiku", "opus"]);
  // Opus, back for its second turn, has both replies since its first.
  let opus2 = last_user(&of(&seen, "opus")[1]);
  assert!(opus2.contains("Broadly agree with @opus."));
  assert!(opus2.contains("@opus, why that order?"));
  assert!(!opus2.contains("[pass]"), "asked: it answers");
  assert!(opus2.contains("Refer to other participants by name"));
  node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_council_answers_blind_then_everyone_sees_the_round() {
  let s = scripts(&[
    ("opus", &["Opus: plan A.", "Opus: B is fine too."]),
    ("sonnet", &["Sonnet: plan B.", "Sonnet: A works."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5"],
    Some("council"),
    None,
  )
  .await;
  assert_eq!(info["mode"], "parallel");
  let r = info["room"].as_str().unwrap().to_owned();
  say(&mut c, &r, "Which plan?").await;
  let o1 = last_user(&of(&seen, "opus")[0]);
  let s1 = last_user(&of(&seen, "sonnet")[0]);
  assert!(!o1.contains("plan B") && !s1.contains("plan A"), "blind");
  assert!(!o1.contains("[pass]") && !s1.contains("[pass]"));
  let (es, _) = entries(&mut c, &r).await;
  assert_eq!(es.len(), 3);
  say(&mut c, &r, "Now compare.").await;
  let o2 = last_user(&of(&seen, "opus")[1]);
  let s2 = last_user(&of(&seen, "sonnet")[1]);
  assert!(
    o2.contains("Sonnet: plan B.") && !o2.contains("Opus: plan A."),
    "{o2}"
  );
  assert!(
    s2.contains("Opus: plan A.") && !s2.contains("Sonnet: plan B."),
    "{s2}"
  );
  assert!(o2.contains("[User]: Now compare."));
  node.stop().await;
}

/// A room with a participant that has tools, and another without.
async fn room_with_tools(c: &mut Client) -> (String, String) {
  let mut opus = model("claude-opus-5-5");
  opus.tools = true;
  let info = json_of(
    c,
    &Request::RoomCreate {
      title: Some("Plans".into()),
      place: None,
      from_thread: None,
      participants: vec![opus, model("claude-sonnet-5-5")],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  (
    info["room"].as_str().unwrap().to_owned(),
    info["participants"][0]["thread"]
      .as_str()
      .unwrap()
      .to_owned(),
  )
}

/// A participant's whole turn reaches the room: what it wrote before a
/// tool call as well as after (issue 2026-11-21T070219Z-iphone-yxp6: only
/// the turn's last message did; the rest never reached the room, nor the
/// others through it).
#[tokio::test(flavor = "multi_thread")]
async fn a_participants_whole_turn_reaches_the_room() {
  let call = format!(
    "SAY:Here is the plan: A, then B.|TOOL:suggest_replies:{}",
    json!({"replies": [{"label": "Do it"}]})
  );
  let s = scripts(&[
    ("opus", &[&call, "Or I can leave things as they are."]),
    ("sonnet", &["Sonnet agrees."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let (r, _) = room_with_tools(&mut c).await;
  say(&mut c, &r, "What now?").await;
  assert_eq!(of(&seen, "opus").len(), 2, "a step, then its last");
  let whole = "Here is the plan: A, then B.\n\nOr I can leave things as they are.";
  let (es, _) = entries(&mut c, &r).await;
  let opus: Vec<&str> = es
    .iter()
    .filter(|e| e["speaker"]["slug"] == "opus")
    .map(|e| e["text"].as_str().unwrap())
    .collect();
  assert_eq!(opus, [whole]);
  let sonnet1 = last_user(&of(&seen, "sonnet")[0]);
  assert!(
    sonnet1.contains(&format!("[Opus · anthropic/claude-opus-5-5]: {whole}")),
    "{sonnet1}"
  );
  node.stop().await;
}

/// `suggest_replies` and `offer_actions` render from a thread's own
/// transcript, which nobody reads for a room participant: refused there,
/// so nothing claims buttons were shown (issue 2026-11-21T070413Z-gamer-58iv).
#[tokio::test(flavor = "multi_thread")]
async fn a_participants_suggest_replies_is_refused_in_a_room() {
  let call = format!(
    "TOOL:suggest_replies:{}",
    json!({"replies": [{"label": "Do it"}]})
  );
  let s = scripts(&[
    ("opus", &[&call, "Shall I go ahead?"]),
    ("sonnet", &["Sonnet agrees."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let (r, _) = room_with_tools(&mut c).await;
  say(&mut c, &r, "What now?").await;
  let after = of(&seen, "opus")[1].to_string();
  assert!(
    after.contains("does not show in a room"),
    "refused, not shown"
  );
  assert!(!after.contains("Shown:"), "shown in a room");
  node.stop().await;
}

/// A participant woken by a report (a thread it spawned in the room, done
/// later) answers in the room: its reply is written there as its room
/// replies are, relayed to the others in their next turn and never back
/// to it (issue 2026-11-21T070219Z-iphone-yxp6).
#[tokio::test(flavor = "multi_thread")]
async fn a_participants_reply_to_a_report_reaches_the_room() {
  let spawn = format!(
    "TOOL:spawn:{}",
    json!({"goal": "Check the build.", "model": "anthropic/claude-haiku-5-5", "tools": false})
  );
  let s = scripts(&[
    (
      "opus",
      &[&spawn, "I asked a thread to check.", "Opus round two."],
    ),
    ("sonnet", &["Sonnet waits.", "Sonnet saw it."]),
    // The spawned thread's turn, then the participant's woken by its report.
    (
      "plain",
      &["The build is green.", "The check says the build is green."],
    ),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let (r, opus_t) = room_with_tools(&mut c).await;
  say(&mut c, &r, "Is the build fine?").await;
  let woken = "The check says the build is green.";
  let mut posted = None;
  for _ in 0..120 {
    let (es, _) = entries(&mut c, &r).await;
    if let Some(e) = es.iter().find(|e| e["text"] == woken) {
      posted = Some(e.clone());
      break;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
  }
  let posted = posted.expect("the woken reply reached the room");
  assert_eq!(posted["kind"], "assistant");
  assert_eq!(posted["speaker"]["slug"], "opus");
  assert_eq!(posted["speaker"]["thread"], json!(opus_t));
  assert_eq!(posted["speaker"]["model"], "anthropic/claude-opus-5-5");
  // Its participant's thread has it as its own reply, as before.
  let (pe, _) = entries(&mut c, &opus_t).await;
  assert!(pe.iter().any(|e| e["text"] == woken));

  // Next round: the others are relayed it, its author is not.
  say(&mut c, &r, "And now?").await;
  let opus2 = last_user(&of(&seen, "opus").last().unwrap().clone());
  assert!(opus2.contains("[User]: And now?"), "{opus2}");
  assert!(!opus2.contains(woken), "not relayed back: {opus2}");
  let sonnet2 = last_user(&of(&seen, "sonnet")[1]);
  assert!(
    sonnet2.contains(&format!("[Opus · anthropic/claude-opus-5-5]: {woken}")),
    "{sonnet2}"
  );
  node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pulling_in_a_model_keeps_the_thread_and_briefs_the_newcomer() {
  let s = scripts(&[
    ("plain", &["We cut the release on Friday."]),
    ("opus", &["As I said: Friday."]),
    ("sonnet", &["Friday is risky."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let done = c
    .chat(
      &chat(None, "Plan the release", Some("anthropic/claude-opus-5-5")),
      |_| None,
    )
    .await
    .unwrap();
  let Reply::ChatDone { thread: t, .. } = done else {
    panic!()
  };
  let (before, _) = entries(&mut c, &t).await;
  let first = of(&seen, "plain")[0].clone();

  let mut spec = model("claude-sonnet-5-5");
  spec.tools = true;
  let info = json_of(
    &mut c,
    &Request::RoomCreate {
      title: None,
      place: None,
      from_thread: Some(t.clone()),
      participants: vec![spec],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  assert_eq!(info["promoted_from"], json!(t));
  assert_eq!(info["title"], "Plan the release · room");
  let ps = info["participants"].as_array().unwrap();
  assert_eq!(ps[0]["thread"], json!(t), "the thread itself joins first");
  assert_eq!(ps[0]["slug"], "opus");
  assert_eq!(ps[1]["slug"], "sonnet");
  // Its untouched history, so far.
  assert_eq!(entries(&mut c, &t).await.0, before);
  // The newcomer is read-only, under the room.
  let newcomer = ps[1]["thread"].as_str().unwrap().to_owned();
  let row = json_of(
    &mut c,
    &Request::Threads {
      status: None,
      since: None,
      limit: None,
      thread: Some(newcomer.clone()),
      place: None,
      forest: None,
    },
  )
  .await;
  assert_eq!(row["threads"][0]["read_only"], true);
  assert_eq!(row["threads"][0]["parent"], json!(r));
  // The room's seed: a chip back to the thread.
  let (es, _) = entries(&mut c, &r).await;
  assert_eq!(es[0]["from"], "room:seed");
  assert_eq!(es[0]["scopes"][0]["id"], json!(t));

  say(&mut c, &r, "Thoughts, both?").await;
  // The thread goes on from its own history, with the room's new message
  // only (not the seed: it knows itself).
  let opus = of(&seen, "opus")[0].clone();
  assert_eq!(opus["messages"][0], first["messages"][0]);
  assert_eq!(opus["messages"][1]["role"], "assistant");
  assert!(
    texts(&opus["messages"][1])
      .join("")
      .contains("We cut the release on Friday.")
  );
  let mine = last_user(&opus);
  assert!(mine.contains("[User]: Thoughts, both?"));
  assert!(!mine.contains("<scope>"), "{mine}");
  // The newcomer gets the briefing on the thread, and the thread's reply.
  let sonnet = last_user(&of(&seen, "sonnet")[0]);
  assert!(sonnet.contains("<scope>"), "{sonnet}");
  assert!(sonnet.contains(&t));
  assert!(sonnet.contains("[Stitch]: This room continues the conversation \"Plan the release\""));
  assert!(sonnet.contains("[Opus · anthropic/claude-opus-5-5]: As I said: Friday."));
  // The thread's earlier entries are as they were; the room turn follows.
  let (after, _) = entries(&mut c, &t).await;
  assert_eq!(after[..before.len()], before[..]);
  assert!(after[before.len()]["room_turn"].is_object());
  node.stop().await;
}

/// A message sent while a participant speaks waits for its reply, is
/// written after it (that participant never saw it), and starts a round of
/// its own: the rest of the old round does not run.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_mid_round_is_written_after_the_reply_being_written() {
  let s = scripts(&[
    ("opus", &["LATE:Opus first.", "Opus on the new one."]),
    ("sonnet", &["Sonnet on the new one."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5"],
    None,
    None,
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let data = node.data.clone();
  let r2 = r.clone();
  let running = tokio::spawn(async move {
    let mut c = Client::connect(&data).await.unwrap();
    say(&mut c, &r2, "First?").await
  });
  for _ in 0..200 {
    if !of(&seen, "opus").is_empty() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  let sent = c
    .request(&Request::Send {
      thread: r.clone(),
      text: "Actually, second?".into(),
      from: None,
      keep: false,
      interject: true,
      scopes: Vec::new(),
    })
    .await
    .unwrap();
  // Woven into the round's steer (`weave::send` finds the room working),
  // which, before 2026-11-14, the round never read: the message waited
  // for the whole round to end.
  let sent = format!("{sent:?}");
  assert!(
    sent.contains("woven into its running turn") || sent.contains("taken by the round running"),
    "{sent}"
  );
  // Not written yet: Opus is still speaking.
  let (es, _) = entries(&mut c, &r).await;
  assert_eq!(es.len(), 1);
  tokio::time::timeout(Duration::from_secs(20), running)
    .await
    .expect("ended")
    .unwrap();
  assert_eq!(order(&seen), ["opus", "opus", "sonnet"]);
  let (es, _) = entries(&mut c, &r).await;
  let texts: Vec<&str> = es.iter().map(|e| e["text"].as_str().unwrap()).collect();
  assert_eq!(
    texts,
    [
      "First?",
      "Opus first.",
      "Actually, second?",
      "Opus on the new one.",
      "Sonnet on the new one."
    ]
  );
  let opus2 = last_user(&of(&seen, "opus")[1]);
  assert!(opus2.contains("[User]: Actually, second?"), "{opus2}");
  node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_room_stops_its_round() {
  let s = scripts(&[("opus", &["SLOW"]), ("sonnet", &["never"])]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let info = room(
    &mut c,
    &["claude-opus-5-5", "claude-sonnet-5-5"],
    None,
    None,
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let data = node.data.clone();
  let r2 = r.clone();
  let running = tokio::spawn(async move {
    let mut c = Client::connect(&data).await.unwrap();
    say(&mut c, &r2, "Take your time.").await
  });
  for _ in 0..200 {
    if !of(&seen, "opus").is_empty() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  assert_eq!(of(&seen, "opus").len(), 1);
  let row = json_of(
    &mut c,
    &Request::Threads {
      status: None,
      since: None,
      limit: None,
      thread: Some(r.clone()),
      place: None,
      forest: None,
    },
  )
  .await;
  assert_eq!(row["threads"][0]["state"], "working");
  assert!(matches!(
    c.request(&Request::Cancel { thread: r.clone() })
      .await
      .unwrap(),
    Reply::Ok
  ));
  let (_, stop) = tokio::time::timeout(Duration::from_secs(20), running)
    .await
    .expect("stopped")
    .unwrap();
  assert_eq!(stop, "interrupted");
  // Nobody else spoke, and nothing was written for the stopped one.
  assert_eq!(order(&seen), ["opus"]);
  let (es, _) = entries(&mut c, &r).await;
  assert_eq!(es.len(), 1);
  // Nothing runs now: a second cancel says so.
  assert!(
    c.request(&Request::Cancel { thread: r.clone() })
      .await
      .is_err()
  );
  node.stop().await;
}

/// The questions written in the home workspace, read from its files.
fn questions(home: &Path) -> Vec<stitch_store::IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index.records(Some("question")).unwrap()
}

/// The question whose command names `what`, once it is asked.
async fn question_on(home: &Path, what: &str) -> stitch_store::IndexedRecord {
  for _ in 0..400 {
    if let Some(q) = questions(home).into_iter().find(|q| {
      q.fields["command"]
        .as_str()
        .is_some_and(|c| c.contains(what))
    }) {
      return q;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("no question on {what}");
}

/// Answers a question, once the node's index has it.
async fn answer_q(c: &mut Client, q: &str, allow: bool, always: bool, reason: Option<&str>) {
  for _ in 0..200 {
    match c
      .request(&Request::Answer {
        question: q.into(),
        allow,
        reason: reason.map(str::to_owned),
        always,
      })
      .await
    {
      Ok(_) => return,
      Err(e) if e.to_string().contains("no thread") => {
        tokio::time::sleep(Duration::from_millis(50)).await
      }
      Err(e) => panic!("{e}"),
    }
  }
  panic!("the node never knew {q}");
}

/// Whether a thread is read-only, as the list says.
async fn read_only_of(c: &mut Client, t: &str) -> Value {
  let req = Request::Threads {
    status: None,
    since: None,
    limit: None,
    thread: Some(t.to_owned()),
    place: None,
    forest: None,
  };
  json_of(c, &req).await["threads"][0]["read_only"].clone()
}

/// A file named `name` anywhere under `dir`.
fn found(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
  for e in std::fs::read_dir(dir).ok()?.flatten() {
    let p = e.path();
    if p.is_dir() {
      if let Some(f) = found(&p, name) {
        return Some(f);
      }
    } else if p.file_name().is_some_and(|n| n == name) {
      return Some(p);
    }
  }
  None
}

fn write(path: &str) -> String {
  format!(
    "TOOL:write_file:{}",
    json!({"path": path, "content": format!("{path}!")})
  )
}

/// A read-only room participant asks the user what the read-only mode
/// refuses (design §2.27, 2026-11-14): allowed once, the write runs and
/// the next asks again; declined, the model gets the refusal with the
/// reason; "always", the thread is writable from then on (its record's
/// `approval` switched, for the user) and asks no more. A participant
/// without tools has none.
/// Answering a question is the user's alone, from no sandbox: inside one
/// (an agent's `just quick`) the node rightly refuses the test's own
/// answer, so the tests that answer as the user are not run there. On
/// Linux the node's sense of "confined" is `no_new_privs` (`NoNewPrivs:` in
/// `/proc/<pid>/status`, as `peer::confined` reads it), which the fixers'
/// sandbox sets. With `CI` set (GitHub Actions does) it never skips: the
/// tests must run there, so a sandboxed CI fails them loudly.
fn outside_any_sandbox() -> bool {
  let ok = if std::env::var_os("CI").is_some() {
    true
  } else if cfg!(target_os = "macos") {
    stitch_shell::Sandbox::available(None).is_ok()
  } else if cfg!(target_os = "linux") {
    std::fs::read_to_string("/proc/self/status").is_ok_and(|s| {
      s.lines()
        .find_map(|l| l.strip_prefix("NoNewPrivs:"))
        .is_some_and(|v| v.trim() == "0")
    })
  } else {
    true
  };
  if !ok {
    eprintln!("inside a sandbox: only the user answers questions; not run");
  }
  ok
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_participant_asks_instead_of_being_refused() {
  if !outside_any_sandbox() {
    return;
  }
  let (a, b, c1, d) = (
    write("a.txt"),
    write("b.txt"),
    write("c.txt"),
    write("d.txt"),
  );
  let s = scripts(&[
    (
      "opus",
      &[
        &a,
        &b,
        "Wrote a; b was declined.",
        &c1,
        &d,
        "Wrote c and d.",
      ],
    ),
    ("sonnet", &["An opinion.", "Another."]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let mut writer = model("claude-opus-5-5");
  writer.tools = true;
  let info = json_of(
    &mut c,
    &Request::RoomCreate {
      title: Some("Pot feet".into()),
      place: None,
      from_thread: None,
      participants: vec![writer, model("claude-sonnet-5-5")],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let opus_t = info["participants"][0]["thread"]
    .as_str()
    .unwrap()
    .to_owned();
  assert_eq!(read_only_of(&mut c, &opus_t).await, true);

  // Round one: a.txt allowed once, b.txt declined.
  let mut talker = node.client().await;
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Make the files").await });
  let qa = question_on(&node.home, "a.txt").await;
  let text = qa.fields["command"].as_str().unwrap();
  assert_eq!(
    text, "Opus (read-only, in room “Pot feet”) wants to write a.txt",
    "{text}"
  );
  assert_eq!(qa.fields["choices"], json!(["allow", "always", "decline"]));
  assert_eq!(qa.fields["grant"], json!(["make Opus writable"]));
  assert_eq!(qa.fields["user_only"], true);
  answer_q(&mut c, qa.id.as_str(), true, false, None).await;
  let qb = question_on(&node.home, "b.txt").await;
  answer_q(&mut c, qb.id.as_str(), false, false, Some("not that one")).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(60), round)
    .await
    .expect("round one ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  let a_file = found(&node.data, "a.txt").expect("a.txt written");
  assert_eq!(std::fs::read_to_string(a_file).unwrap(), "a.txt!");
  assert!(found(&node.data, "b.txt").is_none(), "b.txt declined");
  let after_b = of(&seen, "opus")[2].to_string();
  assert!(
    after_b.contains(
      "this thread is read-only: write_file is refused (the user declined: not that one)"
    ),
    "{after_b}"
  );
  assert_eq!(
    read_only_of(&mut c, &opus_t).await,
    true,
    "allowed once only"
  );
  // The one without tools has none.
  let sonnet = &of(&seen, "sonnet")[0];
  assert!(
    sonnet["tools"].as_array().is_none_or(Vec::is_empty),
    "{}",
    sonnet["tools"]
  );

  // Round two: c.txt "always", then d.txt asks nothing.
  let mut talker = node.client().await;
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Two more").await });
  let qc = question_on(&node.home, "c.txt").await;
  answer_q(&mut c, qc.id.as_str(), true, true, None).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(60), round)
    .await
    .expect("round two ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  assert!(found(&node.data, "c.txt").is_some());
  assert!(found(&node.data, "d.txt").is_some());
  assert!(
    !questions(&node.home).iter().any(|q| q.fields["command"]
      .as_str()
      .is_some_and(|c| c.contains("d.txt"))),
    "writable: d.txt asked nothing"
  );
  assert_eq!(read_only_of(&mut c, &opus_t).await, false, "made writable");
  node.stop().await;
}

/// The room's notes, as its transcript has them (`kind: "note"`), once
/// there are `n`.
async fn notes(c: &mut Client, room: &str, n: usize) -> Vec<String> {
  let mut got = Vec::new();
  for _ in 0..600 {
    let (es, _) = entries(c, room).await;
    got = es
      .iter()
      .filter(|e| e["kind"] == "note")
      .map(|e| e["text"].as_str().unwrap().to_owned())
      .collect();
    if got.len() >= n {
      return got;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  panic!("{n} notes expected in the room, got {got:?}");
}

/// A participant's question settled is noted in the room (design §2.27,
/// 2026-11-21): allowed, declined with the reason, "always" (made
/// writable), withdrawn when the round is stopped; a note, not a message,
/// so the next speaker's relay never carries it, nor the asker's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_participants_question_is_noted_in_the_room_and_not_relayed() {
  if !outside_any_sandbox() {
    return;
  }
  let (a, b, c1, s1) = (
    write("a.txt"),
    write("b.txt"),
    write("c.txt"),
    write("s.txt"),
  );
  let s = scripts(&[
    ("opus", &[&a, &b, "Wrote a.", &c1, "Wrote c."]),
    ("sonnet", &["An opinion.", &s1, "never"]),
  ]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let mut opus = model("claude-opus-5-5");
  opus.tools = true;
  let mut sonnet = model("claude-sonnet-5-5");
  sonnet.tools = true;
  let info = json_of(
    &mut c,
    &Request::RoomCreate {
      title: Some("Pot feet".into()),
      place: None,
      from_thread: None,
      participants: vec![opus, sonnet],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();

  // Round one: a.txt allowed, b.txt declined; then Sonnet speaks.
  let mut talker = node.client().await;
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Make the files").await });
  let qa = question_on(&node.home, "a.txt").await;
  answer_q(&mut c, qa.id.as_str(), true, false, None).await;
  let qb = question_on(&node.home, "b.txt").await;
  answer_q(&mut c, qb.id.as_str(), false, false, Some("not that one")).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(60), round)
    .await
    .expect("round one ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  let said = notes(&mut c, &r, 2).await;
  assert_eq!(
    said,
    [
      "[Opus was allowed to write a.txt]",
      "[Opus was not allowed to write b.txt: not that one]"
    ]
  );
  // In the room's order: the notes, then Opus's reply, then Sonnet's.
  let (es, _) = entries(&mut c, &r).await;
  let kinds: Vec<String> = es
    .iter()
    .map(|e| match e["speaker"]["slug"].as_str() {
      Some(slug) => slug.to_owned(),
      None => e["kind"].as_str().unwrap().to_owned(),
    })
    .collect();
  assert_eq!(kinds, ["user", "note", "note", "opus", "sonnet"], "{es:?}");
  let sonnet1 = last_user(&of(&seen, "sonnet")[0]);
  assert!(sonnet1.contains("Wrote a."), "{sonnet1}");
  assert!(!sonnet1.contains("[Opus was"), "not relayed: {sonnet1}");

  // Round two: c.txt "always"; Sonnet's s.txt is withdrawn by the stop.
  let mut talker = node.client().await;
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Two more").await });
  let qc = question_on(&node.home, "c.txt").await;
  answer_q(&mut c, qc.id.as_str(), true, true, None).await;
  question_on(&node.home, "s.txt").await;
  let opus2 = last_user(&of(&seen, "opus")[3]);
  assert!(opus2.contains("An opinion."), "{opus2}");
  assert!(
    !opus2.contains("[Opus was"),
    "not relayed to its asker: {opus2}"
  );
  let sonnet2 = last_user(&of(&seen, "sonnet")[1]);
  assert!(sonnet2.contains("Wrote c."), "{sonnet2}");
  assert!(!sonnet2.contains("made writable"), "not relayed: {sonnet2}");
  assert!(matches!(
    c.request(&Request::Cancel { thread: r.clone() })
      .await
      .unwrap(),
    Reply::Ok
  ));
  let _ = tokio::time::timeout(Duration::from_secs(30), round)
    .await
    .expect("round two stops");
  let said = notes(&mut c, &r, 4).await;
  assert_eq!(
    said[2..],
    [
      "[Opus was allowed to write c.txt, and made writable]",
      "[Sonnet no longer asks to write s.txt]"
    ]
  );
  node.stop().await;
}

/// A read-only thread out of a room (an agent's survey thread) is still
/// refused at once, never asking.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_thread_out_of_a_room_is_refused_without_asking() {
  let e = write("e.txt");
  let s = scripts(&[("plain", &[&e, "Could not write."])]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let Reply::Detached(_) = c
    .request(&Request::Detach {
      goal: "Write e.txt".into(),
      model: Some("anthropic/claude-opus-5-5".into()),
      repo: None,
      cwd: None,
      parent: None,
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: None,
      global: true,
      node: None,
      context: None,
      no_tools: false,
      tools: Vec::new(),
      skills: Vec::new(),
      keep: false,
      memory: false,
      read_only: true,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  for _ in 0..400 {
    if of(&seen, "plain").len() >= 2 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  let after = of(&seen, "plain")
    .get(1)
    .expect("a second call, with the refusal")
    .to_string();
  assert!(
    after.contains("this thread is read-only: write_file is refused"),
    "{after}"
  );
  assert!(questions(&node.home).is_empty(), "nothing asked");
  assert!(found(&node.data, "e.txt").is_none());
  node.stop().await;
}

/// A read-only participant's command the read-only rules refuse asks too:
/// allowed once, it runs in a session started writable; the next asks
/// again.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_participants_command_asks_and_runs_once() {
  if !outside_any_sandbox() {
    return;
  }
  let run = |cmd: &str| format!("TOOL:shell_run:{}", json!({"command": cmd}));
  let (s1, s2) = (run("touch s.txt"), run("touch t.txt"));
  let s = scripts(&[("opus", &[&s1, &s2, "Done."])]);
  let (base, seen) = serve(s).await;
  let node = Node::start(&base).await;
  let mut c = node.client().await;
  let mut writer = model("claude-opus-5-5");
  writer.tools = true;
  let info = json_of(
    &mut c,
    &Request::RoomCreate {
      title: Some("Shell".into()),
      place: None,
      from_thread: None,
      participants: vec![writer],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let mut talker = node.client().await;
  let round = tokio::spawn(async move { say(&mut talker, &r, "Touch them").await });
  let q = question_on(&node.home, "s.txt").await;
  assert_eq!(
    q.fields["command"],
    "Opus (read-only, in room “Shell”) wants to run `touch s.txt`"
  );
  answer_q(&mut c, q.id.as_str(), true, false, None).await;
  let q = question_on(&node.home, "t.txt").await;
  answer_q(&mut c, q.id.as_str(), false, false, None).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(60), round)
    .await
    .expect("the round ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  let calls = of(&seen, "opus");
  assert!(found(&node.data, "s.txt").is_some(), "{}", calls[1]);
  assert!(found(&node.data, "t.txt").is_none());
  assert!(
    calls[2]
      .to_string()
      .contains("this thread is read-only: `touch t.txt` is refused (the user declined)"),
    "{}",
    calls[2]
  );
  node.stop().await;
}

/// A UDP port no other test is about to use (as tests/weave.rs picks them).
fn free_port() -> u16 {
  use std::sync::atomic::{AtomicU16, Ordering};
  static NEXT: AtomicU16 = AtomicU16::new(0);
  const BASE: u32 = 32_000;
  const SPAN: u32 = 8_000;
  let start = std::process::id().wrapping_mul(2_654_435_761) % SPAN;
  loop {
    let n = NEXT.fetch_add(1, Ordering::SeqCst) as u32;
    let port = (BASE + (start + n) % SPAN) as u16;
    let v4 = std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok();
    let v6 = std::net::UdpSocket::bind(("::", port)).is_ok();
    if v4 && (v6 || std::net::UdpSocket::bind(("::1", 0)).is_err()) {
      return port;
    }
  }
}

fn paired_config(data: &Path, name: &str, port: u16, anthropic: &str) {
  let mut c = Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    digests: Some(DigestSettings {
      enabled: Some(false),
      ..Default::default()
    }),
    port: Some(port),
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

/// Joins `data` (a node `name` on `port`) to the inviter of `ticket`, as
/// `stitch join` does: trust, then the home workspace.
async fn join(data: &Path, name: &str, port: u16, ticket: &stitch_net::Ticket) {
  use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, bind};
  let key = stitch_net::load_or_create_key(&data.join("node.key")).unwrap();
  let index = || Index::open(&data.join("index.sqlite")).unwrap();
  let state = |roots| {
    let peers_path = data.join("peers.toml");
    NodeState::new(
      name.parse().unwrap(),
      roots,
      index(),
      stitch_store::BlobStore::open(&data.join("blobs")).unwrap(),
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

/// A read-only participant whose room runs on another node (a room on the
/// phone, the participant a thread of the Mac's, its turn relayed with
/// `room_turn`) asks the user too, from the node its turn runs on (live:
/// room `2026-11-17T143401Z-gamer-dp9v`, Fable refused `write_file` twice
/// with no question): allowed once (answered on the phone), the write runs;
/// "always", the thread is made writable and asks no more.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_participant_relayed_to_another_node_asks_too() {
  if !outside_any_sandbox() {
    return;
  }
  let (a, b, c1) = (write("ra.txt"), write("rb.txt"), write("rc.txt"));
  let mac_scripts = scripts(&[("opus", &[&a, "Wrote ra.", &b, &c1, "Wrote rb and rc."])]);
  let (phone_api, _phone_seen) = serve(scripts(&[])).await;
  let (mac_api, mac_seen) = serve(mac_scripts).await;
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  paired_config(&phone, "phone", phone_port, &phone_api);
  paired_config(&mac, "mac", mac_port, &mac_api);
  let phone_home = d.join("phone-home");
  let home = RecordRoot::init(&phone_home, "home".parse().unwrap(), Role::Home).unwrap();
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
  let mut m = Client::connect(&mac).await.unwrap();

  let mut writer = model("claude-opus-5-5");
  writer.tools = true;
  writer.node = Some("mac".into());
  let info = json_of(
    &mut p,
    &Request::RoomCreate {
      title: Some("Relayed".into()),
      place: None,
      from_thread: None,
      participants: vec![writer],
      scopes: Vec::new(),
      mode: None,
      hop_cap: None,
      budget_usd: None,
    },
  )
  .await;
  let r = info["room"].as_str().unwrap().to_owned();
  let opus_t = info["participants"][0]["thread"]
    .as_str()
    .unwrap()
    .to_owned();
  assert_eq!(read_only_of(&mut p, &opus_t).await, true);

  // Round one: ra.txt asked on the Mac, allowed once from the phone.
  let mut talker = Client::connect(&phone).await.unwrap();
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Make ra").await });
  let mac_home = mac.join("home");
  let qa = question_on(&mac_home, "ra.txt").await;
  assert_eq!(
    qa.fields["command"],
    "Opus (read-only, in room “Relayed”) wants to write ra.txt"
  );
  assert_eq!(qa.fields["choices"], json!(["allow", "always", "decline"]));
  assert_eq!(qa.fields["grant"], json!(["make Opus writable"]));
  assert_eq!(qa.fields["user_only"], true);
  let told = of(&mac_seen, "opus")[0].to_string();
  assert!(
    told.contains("is not refused but asked of the user"),
    "the model is told it asks: {told}"
  );
  answer_q(&mut p, qa.id.as_str(), true, false, None).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(90), round)
    .await
    .expect("round one ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  let ra = found(&d, "ra.txt").expect("ra.txt written");
  assert_eq!(std::fs::read_to_string(ra).unwrap(), "ra.txt!");
  assert_eq!(
    read_only_of(&mut m, &opus_t).await,
    true,
    "allowed once only"
  );
  // Noted by the Mac, which asked, into the room the phone runs.
  assert_eq!(
    notes(&mut p, &r, 1).await,
    ["[Opus was allowed to write ra.txt]"]
  );

  // Round two: rb.txt "always", then rc.txt asks nothing.
  let mut talker = Client::connect(&phone).await.unwrap();
  let room = r.clone();
  let round = tokio::spawn(async move { say(&mut talker, &room, "Two more").await });
  let qb = question_on(&mac_home, "rb.txt").await;
  answer_q(&mut p, qb.id.as_str(), true, true, None).await;
  let (_, stop) = tokio::time::timeout(Duration::from_secs(90), round)
    .await
    .expect("round two ends")
    .unwrap();
  assert_eq!(stop, "end_turn");
  assert!(found(&d, "rb.txt").is_some());
  assert!(found(&d, "rc.txt").is_some());
  assert!(
    !questions(&mac_home).iter().any(|q| q.fields["command"]
      .as_str()
      .is_some_and(|c| c.contains("rc.txt"))),
    "writable: rc.txt asked nothing"
  );
  assert_eq!(read_only_of(&mut m, &opus_t).await, false, "made writable");
  assert_eq!(
    notes(&mut p, &r, 2).await[1],
    "[Opus was allowed to write rb.txt, and made writable]"
  );
  // The room's line holds the notes and every reply: not forked.
  let (es, _) = entries(&mut p, &r).await;
  let said: Vec<&str> = es.iter().filter_map(|e| e["text"].as_str()).collect();
  for want in ["Wrote ra.", "Wrote rb and rc.", "Make ra", "Two more"] {
    assert!(said.iter().any(|t| t.contains(want)), "{want}: {said:?}");
  }
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}
