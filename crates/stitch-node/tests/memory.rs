//! Memory on a running node, with the real EmbeddingGemma 2 (set
//! `STITCH_EMBED_MODEL`; without it the test passes saying so) and a mock
//! model: a decision in one thread is recalled, unasked, into another that
//! asks about it in other words; the `recall` tool finds it too; the answer
//! that used it earns a credit; a thread is not given the same memory
//! twice; a thread out of memory is never recalled, and comes back; an
//! answer that dismisses its memories knocks them; a declined follow-up is
//! never shown; a restarted node folds the same memory from its records.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers model requests in order; a follow-up (memory's unprompted
/// turn) gets `follow_up` instead, without using up the list.
async fn serve(responses: Vec<String>, follow_up: String) -> (String, Arc<Mutex<Vec<Value>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    let mut responses = responses.into_iter();
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 65536];
      let body: Value = loop {
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
          break serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap();
        }
      };
      let is_follow_up = last_user_texts(&body)
        .iter()
        .any(|t| t.contains("the user has NOT sent a new message"));
      log.lock().unwrap().push(body);
      let response = if is_follow_up {
        follow_up.clone()
      } else {
        match responses.next() {
          Some(r) => r,
          None => return,
        }
      };
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
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

/// The model asks for one shell command.
#[allow(dead_code)]
fn tool_use(id: &str, command: &str) -> String {
  tool_call(id, "shell_run", json!({"command": command}))
}

fn tool_call(id: &str, name: &str, input: Value) -> String {
  let input = input.to_string();
  let (a, b) = input.split_at(input.len() / 2);
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Let me check."}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": a}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": b}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
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

fn setup(data: &Path, home: &Path, anthropic: &str) {
  let mut c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
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
  let root = RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn chat(thread: Option<&str>, text: &str) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
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

async fn facts(c: &mut Client, at_least: u64) -> u64 {
  for _ in 0..200 {
    let ask = Request::Memory {
      thread: None,
      cwd: None,
      global: false,
    };
    if let Reply::Memory { facts, .. } = c.request(&ask).await.unwrap()
      && facts >= at_least
    {
      return facts;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  panic!("memory did not reach {at_least} facts");
}

/// Every text in a request's last user message.
fn last_user_texts(body: &Value) -> Vec<String> {
  let msgs = body["messages"].as_array().unwrap();
  let last = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
  last["content"]
    .as_array()
    .map(|c| {
      c.iter()
        .filter_map(|b| {
          b["text"]
            .as_str()
            .or(b["content"].as_str())
            .map(str::to_owned)
        })
        .collect()
    })
    .unwrap_or_else(|| vec![last["content"].as_str().unwrap_or("").to_owned()])
}

/// The requests that were not memory's follow-ups.
fn asked(sent: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
  sent
    .lock()
    .unwrap()
    .iter()
    .filter(|b| {
      !last_user_texts(b)
        .iter()
        .any(|t| t.contains("the user has NOT sent a new message"))
    })
    .cloned()
    .collect()
}

fn note_of(body: &Value) -> Option<String> {
  last_user_texts(body)
    .into_iter()
    .find(|t| t.starts_with("<recalled-memories>"))
}

fn memory_facts(data: &Path, key: &str) -> Vec<Value> {
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  index
    .records(Some("memory"))
    .unwrap()
    .iter()
    .flat_map(|r| r.fields["facts"].as_array().cloned().unwrap_or_default())
    .filter_map(|f| f.get(key).cloned())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn memories_cross_threads_and_survive_a_restart() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL") else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let (base, sent) = serve(
    vec![
      answer("Noted: chords above the lyrics, one page per song, exported as a PDF."),
      tool_call(
        "r1",
        "recall",
        json!({"query": "lead sheet export layout decision"}),
      ),
      answer(
        "As we decided, the export renders the chords above the lyrics, one page per song, as a PDF.",
      ),
      answer("Glad that helps."),
      answer("I have nothing on that."),
      answer("That is unrelated to what you asked.\u{2060}"),
    ],
    answer("\u{2060}"),
  )
  .await;
  setup(&data, &dir.path().join("home"), &base);
  std::fs::create_dir_all(data.join("models")).unwrap();
  std::os::unix::fs::symlink(&model, data.join("models/embeddinggemma-2-Q8_0.gguf")).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut afterthoughts = Vec::new();

  let decision = "We decided the lead sheet export renders chords above the lyrics, one page per song, as a PDF.";
  let Reply::ChatDone { thread: first, .. } =
    c.chat(&chat(None, decision), |_| None).await.unwrap()
  else {
    panic!()
  };
  // The user's message and the reply.
  facts(&mut c, 2).await;

  let question = "How did we settle the chord sheet layout for exporting our songs to PDF files?";
  let Reply::ChatDone { thread: second, .. } = c
    .chat(&chat(None, question), |r| {
      if let Reply::Afterthought(t) = r {
        afterthoughts.push(t.clone());
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(first, second);
  // The note keeps how it was chosen, and its outcome joins it: the answer
  // used the decision.
  let mut rows: Vec<String> = Vec::new();
  for _ in 0..100 {
    let Reply::Lines(lines) = c.request(&Request::MemoryNotes).await.unwrap() else {
      panic!()
    };
    rows = lines;
    if rows.iter().any(|r| r.ends_with("\thelped")) {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  assert!(
    rows[0].starts_with("note\thandle\traw\tcentered\t"),
    "{rows:?}"
  );
  let helped: Vec<&String> = rows.iter().filter(|r| r.ends_with("\thelped")).collect();
  assert!(!helped.is_empty(), "{rows:?}");
  let cols: Vec<&str> = helped[0].split('\t').collect();
  assert!(cols[0].contains(&format!("{second}#m")), "{cols:?}");
  let (raw, centered): (f64, f64) = (cols[2].parse().unwrap(), cols[3].parse().unwrap());
  assert!(
    raw > 0.0 && raw <= 0.374 && centered > 0.0 && centered <= 0.49,
    "{cols:?}"
  );
  assert!(cols[4].parse::<u32>().unwrap() >= 4, "{cols:?}");
  {
    let bodies = asked(&sent);
    let note = note_of(&bodies[1])
      .unwrap_or_else(|| panic!("no recalled note: {:?}", last_user_texts(&bodies[1])));
    assert!(note.contains("chords above the lyrics"), "{note}");
    // Provenance, and the invisible dismissal it invites.
    assert!(note.contains("[id "), "{note}");
    assert!(note.contains('\u{2060}'), "{note}");
    assert!(
      bodies[1]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "recall")
    );
    let result = bodies[2]["messages"]
      .as_array()
      .unwrap()
      .iter()
      .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
      .find(|b| b["type"] == "tool_result")
      .unwrap();
    let result = result["content"].as_str().unwrap();
    assert!(result.contains("Episodic memory"), "{result}");
    assert!(
      result.contains("Archive (full history keyword search)"),
      "{result}"
    );
    assert!(result.contains("chords above the lyrics"), "{result}");
  }
  // Its turn observed (question and answer) and credited.
  let n = facts(&mut c, 5).await;
  assert!(
    memory_facts(&data, "credit")
      .iter()
      .any(|used| !used.as_array().unwrap().is_empty()),
    "the answer used the memory"
  );

  // Asked again in the same thread: the memory is not given twice.
  c.chat(
    &chat(
      Some(&second),
      "And how did we settle the chord sheet layout for the PDF export of songs?",
    ),
    |r| {
      if let Reply::Afterthought(t) = r {
        afterthoughts.push(t.clone());
      }
      None
    },
  )
  .await
  .unwrap();
  {
    let bodies = asked(&sent);
    if let Some(note) = note_of(&bodies[3]) {
      assert!(
        !note.contains("We decided the lead sheet"),
        "given twice: {note}"
      );
    }
  }

  // The first thread out of memory: what it said is recalled nowhere.
  let Reply::Lines(lines) = c
    .request(&Request::Remember {
      thread: first.clone(),
      on: false,
      cwd: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("out of memory"), "{lines:?}");
  c.chat(
    &chat(
      None,
      "Remind me how the lead sheet export lays out chords and lyrics in the PDF?",
    ),
    |_| None,
  )
  .await
  .unwrap();
  {
    let bodies = asked(&sent);
    if let Some(note) = note_of(&bodies[4]) {
      assert!(!note.contains("We decided the lead sheet"), "{note}");
      assert!(!note.contains("Noted: chords"), "{note}");
    }
  }
  // Back in memory, it is recalled again; an answer that dismisses what
  // it was shown knocks it.
  c.request(&Request::Remember {
    thread: first.clone(),
    on: true,
    cwd: None,
  })
  .await
  .unwrap();
  c.chat(
    &chat(
      None,
      "What did we decide about the lead sheet export with chords above the lyrics?",
    ),
    |_| None,
  )
  .await
  .unwrap();
  {
    let bodies = asked(&sent);
    let note = note_of(&bodies[5]).expect("recalled again");
    assert!(note.contains("chords above the lyrics"), "{note}");
  }
  for _ in 0..200 {
    if !memory_facts(&data, "penalize").is_empty() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  assert!(
    !memory_facts(&data, "penalize").is_empty(),
    "the dismissal knocked what was shown"
  );
  let Reply::Lines(rows) = c.request(&Request::MemoryNotes).await.unwrap() else {
    panic!()
  };
  assert!(
    rows.iter().any(|r| r.ends_with("\tdismissed")),
    "the note's outcome is the dismissal: {rows:?}"
  );
  // Declined follow-ups are written, never shown.
  assert!(afterthoughts.is_empty(), "{afterthoughts:?}");

  // A restarted node folds the same memory from the records.
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  assert!(facts(&mut c, n).await >= n);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A spawned thread is not remembered, but its `recall` searches memory
/// (design §2.9): a decision of the user's comes back to it.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_thread_recalls_though_it_is_not_remembered() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL") else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let (base, sent) = serve(
    vec![
      answer("Noted: chords above the lyrics, one page per song, exported as a PDF."),
      tool_call(
        "r1",
        "recall",
        json!({"query": "lead sheet export layout decision"}),
      ),
      answer("The export puts the chords above the lyrics."),
    ],
    answer("\u{2060}"),
  )
  .await;
  setup(&data, &dir.path().join("home"), &base);
  std::fs::create_dir_all(data.join("models")).unwrap();
  std::os::unix::fs::symlink(&model, data.join("models/embeddinggemma-2-Q8_0.gguf")).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let decision = "We decided the lead sheet export renders chords above the lyrics, one page per song, as a PDF.";
  let Reply::ChatDone { thread: first, .. } =
    c.chat(&chat(None, decision), |_| None).await.unwrap()
  else {
    panic!()
  };
  facts(&mut c, 2).await;
  let Reply::Detached(spawned) = c
    .request(&Request::Detach {
      goal: "Find how the lead sheet export lays out chords.".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: Some(first.clone()),
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
      keep: true,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  for _ in 0..200 {
    if asked(&sent).len() >= 3 {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  let bodies = asked(&sent);
  assert!(bodies.len() >= 3, "{} requests", bodies.len());
  let result = bodies[2]["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
    .find(|b| b["type"] == "tool_result")
    .unwrap();
  let result = result["content"].as_str().unwrap();
  assert!(!result.contains("disabled"), "{result}");
  assert!(result.contains("Episodic memory"), "{result}");
  assert!(result.contains("chords above the lyrics"), "{result}");
  // Out of memory all the same, recalling.
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let fact = index
    .records(Some("memory"))
    .unwrap()
    .iter()
    .flat_map(|r| r.fields["facts"].as_array().cloned().unwrap_or_default())
    .find(|f| f.get("thread").and_then(Value::as_str) == Some(spawned.as_str()))
    .expect("its memory fact");
  assert_eq!(fact["remember"], json!(false));
  assert_eq!(fact["recall"], json!(true));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn what_an_exchange_surfaces_can_bring_an_afterthought() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL") else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  // A topic told over a few exchanges; asked about elsewhere, the note
  // covers the misting, and the answer brings up the reservoir, which is
  // worth an afterthought.
  let said = [
    "The aeroponic tower mists the roots for fifteen seconds every fifteen minutes.",
    "The aeroponic misting timer runs day and night, fifteen seconds on, fifteen minutes off.",
    "The aeroponic misting nozzles clog with salt, so we soak them in vinegar monthly.",
    "The aeroponic tower's roots turned brown when the misting timer stuck for a night.",
    "The aeroponic misting pump is a 24 volt diaphragm pump on a cycle timer relay.",
    "The aeroponic reservoir gets fresh nutrient solution every Sunday morning.",
    "We keep the aeroponic reservoir solution at pH 5.8 and EC 1.6 for the lettuce.",
    "We added an air stone to the aeroponic reservoir to keep the nutrient solution oxygenated.",
  ];
  let mut responses: Vec<String> = said
    .iter()
    .map(|s| answer(&format!("Got it: {s}")))
    .collect();
  responses.push(answer(
    "The tower mists for fifteen seconds every fifteen minutes. Separately, the reservoir's nutrient solution is replaced weekly, kept at pH 5.8 with an air stone for oxygen.",
  ));
  let (base, sent) = serve(
    responses,
    answer("One more thing: the misting nozzles clog with salt, so soak them in vinegar monthly."),
  )
  .await;
  setup(&data, &dir.path().join("home"), &base);
  std::fs::create_dir_all(data.join("models")).unwrap();
  std::os::unix::fs::symlink(&model, data.join("models/embeddinggemma-2-Q8_0.gguf")).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut afterthoughts = Vec::new();
  let mut thread: Option<String> = None;
  for (i, s) in said.iter().enumerate() {
    let Reply::ChatDone { thread: t, .. } = c
      .chat(&chat(thread.as_deref(), s), |r| {
        if let Reply::Afterthought(t) = r {
          afterthoughts.push(t.clone());
        }
        None
      })
      .await
      .unwrap()
    else {
      panic!()
    };
    thread = Some(t);
    facts(&mut c, 2 * (i as u64 + 1)).await;
  }
  c.chat(
    &chat(
      None,
      "How is the misting cycle of the aeroponic tower set up again?",
    ),
    |r| {
      if let Reply::Afterthought(t) = r {
        afterthoughts.push(t.clone());
      }
      None
    },
  )
  .await
  .unwrap();
  let follow_ups: Vec<Value> = {
    let all = sent.lock().unwrap().clone();
    all
      .into_iter()
      .filter(|b| {
        last_user_texts(b)
          .iter()
          .any(|t| t.contains("the user has NOT sent a new message"))
      })
      .collect()
  };
  eprintln!(
    "follow-ups: {}, afterthoughts: {afterthoughts:?}",
    follow_ups.len()
  );
  assert!(
    !follow_ups.is_empty(),
    "something surfaced worth a follow-up"
  );
  for f in &follow_ups {
    assert!(note_of(f).is_some(), "a follow-up carries its note");
  }
  assert_eq!(afterthoughts.len(), follow_ups.len());
  assert!(afterthoughts[0].contains("vinegar"));
  // The afterthought's memories are new to the thread: not the note's.
  let bodies = sent.lock().unwrap().clone();
  let asked_note = bodies
    .iter()
    .rev()
    .find(|b| {
      !last_user_texts(b)
        .iter()
        .any(|t| t.contains("the user has NOT sent a new message"))
    })
    .and_then(note_of)
    .unwrap_or_default();
  for line in note_of(&follow_ups[0])
    .unwrap()
    .lines()
    .filter(|l| l.starts_with("- [id "))
  {
    let id = &line[..22];
    assert!(!asked_note.contains(id), "{line} was in the note already");
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_generic_message_brings_no_unrelated_note() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL") else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  // What a field note wrongly brought up: another thread's chatty question,
  // close to any short request in the raw embedding space.
  let earlier = "hey claude, which tools are avail now in stitch when you run?";
  let generic =
    "Can you check what are last priorities and what was the last claude session at pls?";
  let (base, sent) = serve(
    vec![
      answer("Shell sessions, the web tools, view_file and recall."),
      answer("I will look."),
    ],
    answer("\u{2060}"),
  )
  .await;
  setup(&data, &dir.path().join("home"), &base);
  std::fs::create_dir_all(data.join("models")).unwrap();
  std::os::unix::fs::symlink(&model, data.join("models/embeddinggemma-2-Q8_0.gguf")).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, earlier), |_| None).await.unwrap();
  facts(&mut c, 2).await;
  c.chat(&chat(None, generic), |_| None).await.unwrap();
  {
    let bodies = asked(&sent);
    assert_eq!(
      note_of(&bodies[1]),
      None,
      "{:?}",
      last_user_texts(&bodies[1])
    );
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Whether a thread (or a new one there) is in memory, as the TUI's status
/// line asks.
async fn thread_on(
  c: &mut Client,
  thread: Option<&str>,
  cwd: Option<&Path>,
  global: bool,
) -> Option<bool> {
  let ask = Request::Memory {
    thread: thread.map(str::to_owned),
    cwd: cwd.map(|d| d.display().to_string()),
    global,
  };
  match c.request(&ask).await.unwrap() {
    Reply::Memory { thread_on, .. } => thread_on,
    other => panic!("{other:?}"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_place_out_of_memory_says_so_before_any_turn() {
  let Some(model) = std::env::var_os("STITCH_EMBED_MODEL") else {
    eprintln!("STITCH_EMBED_MODEL not set: skipped");
    return;
  };
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let data = d.join("data");
  let app = d.join("app");
  std::fs::create_dir_all(app.join(".stitch")).unwrap();
  let git = |args: &[&str]| {
    let ok = std::process::Command::new("git")
      .arg("-C")
      .arg(&app)
      .args(args)
      .output()
      .unwrap()
      .status
      .success();
    assert!(ok, "git {args:?}");
  };
  git(&["init", "--quiet"]);
  git(&[
    "-c",
    "user.name=T",
    "-c",
    "user.email=t@example.com",
    "commit",
    "--quiet",
    "--allow-empty",
    "-m",
    "first",
  ]);
  let too_long = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: \
                  close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\
                  \"message\":\"prompt is too long: 250000 tokens > 200000 maximum\"}}"
    .to_owned();
  let (base, _) = serve(
    vec![
      answer("Hello."),
      answer("Hello again."),
      too_long,
      answer("SUMMARY"),
      answer("Compacted."),
    ],
    answer("\u{2060}"),
  )
  .await;
  setup(&data, &d.join("home"), &base);
  // Its window known: compacting asks the mock nothing more.
  let mut config = stitch_node::config::Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(&data).unwrap();
  std::fs::create_dir_all(data.join("models")).unwrap();
  std::os::unix::fs::symlink(&model, data.join("models/embeddinggemma-2-Q8_0.gguf")).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(
      &Request::Chat {
        thread: None,
        text: "hi there".into(),
        attachments: vec![],
        model: None,
        repo: None,
        cwd: Some(app.display().to_string()),
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
      },
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(
    thread_on(&mut c, Some(&thread), None, false).await,
    Some(true)
  );
  assert_eq!(thread_on(&mut c, None, Some(&app), false).await, Some(true));

  // The place goes out of memory: a new thread there would be, and so is
  // the one it has, before its next turn takes it out; global ones are not.
  std::fs::write(app.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  assert_eq!(
    thread_on(&mut c, None, Some(&app), false).await,
    Some(false)
  );
  assert_eq!(thread_on(&mut c, None, Some(&app), true).await, Some(true));
  assert_eq!(
    thread_on(&mut c, Some(&thread), None, false).await,
    Some(false)
  );
  // Asked about nothing: nothing said.
  assert_eq!(thread_on(&mut c, None, None, false).await, None);
  // A checkout no thread started in yet: its settings are read all the
  // same.
  let other = d.join("other");
  std::fs::create_dir_all(other.join(".stitch")).unwrap();
  std::fs::write(other.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  for args in [
    &["init", "--quiet"][..],
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "another",
    ],
  ] {
    let ok = std::process::Command::new("git")
      .arg("-C")
      .arg(&other)
      .args(args)
      .status()
      .unwrap()
      .success();
    assert!(ok, "git {args:?}");
  }
  assert_eq!(
    thread_on(&mut c, None, Some(&other), false).await,
    Some(false)
  );
  // `/memory on` brings it back for good.
  let Reply::Lines(_) = c
    .request(&Request::Remember {
      thread: thread.clone(),
      on: true,
      cwd: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(
    thread_on(&mut c, Some(&thread), None, false).await,
    Some(true)
  );
  // Asked again: already so.
  let Reply::Lines(lines) = c
    .request(&Request::Remember {
      thread: thread.clone(),
      on: true,
      cwd: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].ends_with("is in memory"), "{lines:?}");
  // A new thread's memory chosen before its first message (the TUI's
  // `/memory` on a new thread) wins over its place's.
  let Reply::ChatDone { thread: chosen, .. } = c
    .chat(
      &Request::Chat {
        thread: None,
        text: "hello again".into(),
        attachments: vec![],
        model: None,
        repo: None,
        cwd: Some(app.display().to_string()),
        global: false,
        env: Vec::new(),
        tools: Vec::new(),
        skills: Vec::new(),
        place: None,
        memory: Some(true),
        end_user: None,
        node: None,
        spoken: false,
        scopes: Vec::new(),
      },
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(chosen, thread);
  assert_eq!(
    thread_on(&mut c, Some(&chosen), None, false).await,
    Some(true)
  );
  // Compacted, the thread the user brought back goes on in memory: the
  // new one keeps their switch, whatever the place says.
  let Reply::ChatDone {
    thread: compacted, ..
  } = c
    .chat(&chat(Some(&thread), "go on"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(compacted, thread);
  assert_eq!(
    thread_on(&mut c, Some(&compacted), None, false).await,
    Some(true)
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
