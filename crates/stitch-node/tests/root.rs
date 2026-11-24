//! The root agent's loop on a running node: the model (a mock) calls the
//! shell, results go back to it, commands outside the rules wait for the
//! client's approval, and everything lands in the thread's transcript.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::chat::{CUT_OFF, SUSPENDED, Unfinished};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, ChatSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

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
      if let Some(late) = response.strip_prefix(SLOW) {
        // A model slow to answer: written after a while.
        let late = late.to_owned();
        tokio::spawn(async move {
          tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
          sock.write_all(late.as_bytes()).await.unwrap();
          sock.shutdown().await.unwrap();
        });
        continue;
      }
      if response == HANG {
        // A model that never answers: held open, the next one served.
        tokio::spawn(async move {
          tokio::time::sleep(std::time::Duration::from_secs(120)).await;
          drop(sock);
        });
        continue;
      }
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

/// A response that never comes.
const HANG: &str = "hang";
/// Put before a response: it comes after 2.5 seconds.
const SLOW: &str = "slow:";

fn sse(events: Vec<(&str, Value)>) -> String {
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// The model asks for one shell command.
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
    // The approvals tested here are of commands the sandbox would let run.
    shell: Some(stitch_node::config::ShellSettings {
      approval: Some("ask".into()),
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
async fn the_root_works_through_the_shell_and_asks_before_writing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let approved = d.join("approved.txt");
  let declined = d.join("declined.txt");
  let (base, sent) = serve(vec![
    tool_use("t1", "echo hi-from-the-shell"),
    answer("The shell says hi."),
    tool_use("t2", &format!("touch '{}'", approved.display())),
    tool_use("t3", &format!("touch '{}'", declined.display())),
    answer("One file made, one refused."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // An allowed command runs without asking; its output reaches the model.
  let mut seen: Vec<String> = Vec::new();
  let done = c
    .chat(&chat(None, "Say hi from the shell"), |r| {
      match r {
        Reply::ToolCall(line) => seen.push(format!("call {line}")),
        Reply::ToolResult { ok, output } => seen.push(format!("result {ok} {output}")),
        Reply::Text(t) => seen.push(format!("text {t}")),
        Reply::Approve { .. } => panic!("echo needs no approval"),
        _ => {}
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!()
  };
  assert_eq!(seen[0], "text Let me check.");
  assert_eq!(seen[1], "call $ echo hi-from-the-shell");
  // The first command makes the session, and says so.
  assert!(
    seen[2].starts_with("result true (New session \"main\": a fresh shell")
      && seen[2].contains("exit 0")
      && seen[2].contains("hi-from-the-shell"),
    "{seen:?}"
  );
  assert_eq!(seen[3], "text The shell says hi.");
  {
    let bodies = sent.lock().unwrap();
    assert!(
      bodies[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "shell_run")
    );
    assert!(
      bodies[0]["system"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("You are Stitch")
    );
    let last = bodies[1]["messages"]
      .as_array()
      .unwrap()
      .last()
      .unwrap()
      .clone();
    assert_eq!(last["content"][0]["type"], "tool_result");
    assert!(
      last["content"][0]["content"]
        .as_str()
        .unwrap()
        .contains("hi-from-the-shell")
    );
  }

  // A write waits for the client: approved once, declined once.
  let mut asked = Vec::new();
  let done = c
    .chat(&chat(Some(&thread), "Make two files"), |r| match r {
      Reply::Approve { command, .. } => {
        asked.push(command.clone());
        let (answer, decided) = tokio::sync::oneshot::channel();
        let decision = if command.contains("approved") {
          stitch_node::tools::Decision::allow()
        } else {
          stitch_node::tools::Decision::decline(Some("not in my home directory".into()))
        };
        answer.send(decision).unwrap();
        Some(decided)
      }
      _ => None,
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  assert_eq!(asked.len(), 2);
  assert!(approved.exists(), "the approved command ran");
  assert!(!declined.exists(), "the declined command did not");
  {
    let bodies = sent.lock().unwrap();
    // The continued thread sent the earlier tool exchange back, and the
    // declined call came back as an error.
    let msgs = bodies[4]["messages"].as_array().unwrap();
    let results: Vec<Value> = msgs
      .iter()
      .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
      .filter(|b| b["type"] == "tool_result")
      .collect();
    assert_eq!(results.len(), 3);
    // Allowed by the rules: no note (but the session's start). Approved:
    // marked. Declined: the reason.
    assert!(
      results[0]["content"].as_str().unwrap().starts_with(
        "(New session \"main\": a fresh shell; nothing from before is in it.)\nexit 0"
      )
    );
    assert!(
      results[1]["content"]
        .as_str()
        .unwrap()
        .starts_with("(approved by the user)\nexit 0")
    );
    assert_eq!(results[2]["is_error"], true);
    assert_eq!(
      results[2]["content"],
      "the user declined this command: not in my home directory"
    );
    assert_eq!(
      bodies[0]["system"], bodies[4]["system"],
      "the thread keeps its prompt"
    );
  }

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A chat at its step limit asks the user for as many again, yolo or not:
/// granted, it goes on; declined, it answers once more without tools,
/// told where it stopped, and the user is told how to go on.
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_at_its_step_limit_asks_for_more_then_wraps_up() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("s1", "true"),
    tool_use("s2", "true"),
    // Granted 2 more.
    tool_use("s3", "true"),
    tool_use("s4", "true"),
    // Declined: told to wrap up.
    answer("Stopped at the step limit: half done. Say go on."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let mut c = Config::load(&data).unwrap();
  c.chat = Some(ChatSettings { max_steps: Some(2) });
  c.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Yolo { on: Some(true) }).await.unwrap();

  let (mut asked, mut notes) = (Vec::new(), Vec::new());
  let done = c
    .chat(&chat(None, "Work a while"), |r| match r {
      Reply::Approve {
        session, command, ..
      } => {
        assert_eq!(session, stitch_node::tools::STEPS_SESSION);
        asked.push(command.clone());
        let (answer, decided) = tokio::sync::oneshot::channel();
        let decision = if asked.len() == 1 {
          stitch_node::tools::Decision::allow()
        } else {
          stitch_node::tools::Decision::decline(None)
        };
        answer.send(decision).unwrap();
        Some(decided)
      }
      Reply::Note(n) => {
        notes.push(n.clone());
        None
      }
      _ => None,
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  assert_eq!(
    asked,
    [
      "Continue for 2 more steps? This turn has taken 2 and is not done",
      "Continue for 2 more steps? This turn has taken 4 and is not done",
    ]
  );
  assert!(
    notes.iter().any(|n| n.contains("2 more steps: going on")),
    "{notes:?}"
  );
  assert!(
    notes.last().unwrap().contains("say \"go on\" to continue"),
    "{notes:?}"
  );
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(bodies.len(), 5);
    let last = bodies[4]["messages"]
      .as_array()
      .unwrap()
      .last()
      .unwrap()
      .clone();
    assert!(
      last
        .to_string()
        .contains("reached its step limit (4 model calls)"),
      "{last}"
    );
  }
  c.request(&Request::Yolo { on: Some(false) }).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A tool call alone: no text with it.
fn silent_call(id: &str, name: &str, input: Value) -> String {
  let input = input.to_string();
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
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input}}),
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

/// A turn of tool calls that ends saying nothing (issue
/// 2026-11-20T065841Z-mbp-s6e3, after `suggest_replies`) is told once to
/// write its reply; the note is not shown, so the turn's preset answers
/// stay live (design §2.30). A model's empty turn is never sent back.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_ends_with_no_reply_is_told_once_to_write_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let offer = || json!({"replies": [{"label": "Ship it"}, {"label": "Wait"}]});
  let (base, sent) = serve(vec![
    // Silent calls, then nothing: told, it answers.
    silent_call("r1", "suggest_replies", offer()),
    answer(""),
    answer("Ship it now, or wait?"),
    // Told, and nothing again: the turn ends (once a turn).
    silent_call("r2", "suggest_replies", offer()),
    answer(""),
    answer(""),
    // It said something with its calls: nothing to tell.
    tool_call("r3", "suggest_replies", offer()),
    answer(""),
    answer("Fine."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut thread = None;
  let done = c
    .chat(&chat(None, "Ready?"), |r| {
      if let Reply::Thread(t) = r {
        thread = Some(t.clone());
      }
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  let thread = thread.unwrap();
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: thread.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!("no transcript")
  };
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  // The steps' empty messages aside (the calls' own, the one with nothing).
  let shown: Vec<&Value> = t["entries"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|e| e["kind"] != "assistant" || e["text"] != "")
    .collect();
  let kinds: Vec<&str> = shown.iter().map(|e| e["kind"].as_str().unwrap()).collect();
  assert_eq!(kinds, ["user", "tool", "assistant"], "{shown:#?}");
  assert!(shown[1]["replies"].is_array());
  assert_eq!(shown[2]["text"], "Ship it now, or wait?");
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(bodies.len(), 3);
    let msgs = bodies[2]["messages"].as_array().unwrap();
    let last = msgs.last().unwrap().to_string();
    assert!(last.contains("ended without a reply"), "{last}");
  }
  for text in ["Again?", "And now?"] {
    let done = c.chat(&chat(Some(&thread), text), |_| None).await.unwrap();
    assert!(matches!(done, Reply::ChatDone { .. }));
  }
  let bodies = sent.lock().unwrap().clone();
  assert_eq!(
    bodies.len(),
    8,
    "told once a turn, and only when nothing was said"
  );
  let told = |b: &Value| b.to_string().matches("ended without a reply").count();
  assert_eq!(
    told(&bodies[5]),
    2,
    "the second turn's note, besides the first's"
  );
  assert_eq!(told(&bodies[7]), 2, "nothing to tell the third");
  // No model turn without content goes back: the empty ones are dropped.
  for (n, b) in bodies.iter().enumerate() {
    for m in b["messages"].as_array().unwrap() {
      let empty = m["content"].as_array().is_some_and(|c| {
        c.iter()
          .all(|b| b["type"] == "text" && b["text"].as_str().is_some_and(str::is_empty))
      });
      assert!(!empty, "body {n}: {m}");
    }
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_take_the_clients_environment_and_yolo_asks_nothing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, _) = serve(vec![
    tool_use("t1", "echo client:$FROM_THE_CLIENT"),
    answer("It sees it."),
    tool_use("t2", &format!("touch '{}'", made.display())),
    answer("Made it."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // The shell has the environment of the client that chats.
  let mut env = stitch_node::client::client_env();
  env.push(("FROM_THE_CLIENT".into(), "yes".into()));
  let mut outputs = Vec::new();
  let req = Request::Chat {
    thread: None,
    text: "What does the shell see?".into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: Some(d.display().to_string()),
    global: true,
    env,
    tools: Vec::new(),
    skills: Vec::new(),
    place: None,
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  };
  c.chat(&req, |r| {
    if let Reply::ToolResult { output, .. } = r {
      outputs.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(outputs[0].contains("client:yes"), "{outputs:?}");

  // Yolo: what needed approval runs without asking, and it is saved.
  let Reply::Lines(lines) = c.request(&Request::Yolo { on: Some(true) }).await.unwrap() else {
    panic!()
  };
  assert!(lines[0].starts_with("yolo:"), "{lines:?}");
  assert!(
    std::fs::read_to_string(data.join("config.toml"))
      .unwrap()
      .contains("approval = \"allow\"")
  );
  c.chat(&chat(None, "Make the file"), |r| {
    assert!(!matches!(r, Reply::Approve { .. }), "yolo asks nothing");
    None
  })
  .await
  .unwrap();
  assert!(made.exists());
  let Reply::Lines(lines) = c.request(&Request::Yolo { on: Some(false) }).await.unwrap() else {
    panic!()
  };
  assert!(lines[0].starts_with("ask:"), "{lines:?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A place's `approval` (its `.stitch/settings.toml`) is followed there
/// instead of the node's mode, looser or stricter; without it, the node's
/// mode applies. `stitch mode` in the place says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_places_approval_wins_over_the_nodes_mode_both_ways() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let proj = d.join("proj");
  std::fs::create_dir_all(proj.join(".stitch")).unwrap();
  // Asks in every mode but yolo, sandboxed or not (it reaches beyond).
  let push = "git push nowhere-at-all main";
  let (base, _) = serve(vec![
    tool_use("t1", push),
    answer("Pushed, or tried."),
    tool_use("t2", push),
    answer("Declined."),
    tool_use("t3", push),
    answer("Tried again."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let in_proj = |text: &str| {
    let mut r = chat(None, text);
    if let Request::Chat { cwd, .. } = &mut r {
      *cwd = Some(proj.display().to_string());
    }
    r
  };
  let mode_in = |cwd: Option<&Path>| Request::Mode {
    mode: None,
    cwd: cwd.map(|p| p.display().to_string()),
  };
  let settings = proj.join(".stitch/settings.toml");
  // What a turn asked, each declined (a turn that asks never hangs).
  async fn asked_in(c: &mut Client, req: Request) -> Vec<String> {
    let mut asked = Vec::new();
    c.chat(&req, |r| match r {
      Reply::Approve { command, .. } => {
        asked.push(command.clone());
        let (answer, decided) = tokio::sync::oneshot::channel();
        answer
          .send(stitch_node::tools::Decision::decline(None))
          .unwrap();
        Some(decided)
      }
      _ => None,
    })
    .await
    .unwrap();
    asked
  }

  // The node asks; the place says yolo: nothing asks there.
  std::fs::write(&settings, "approval = \"yolo\"\n").unwrap();
  let asked = asked_in(&mut c, in_proj("Push it")).await;
  assert!(asked.is_empty(), "the place's yolo asks nothing");
  let Reply::Lines(lines) = c.request(&mode_in(Some(&proj))).await.unwrap() else {
    panic!()
  };
  assert!(lines[0].starts_with("yolo:"), "{lines:?}");
  assert!(
    lines.iter().any(|l| l.contains("approval = \"yolo\"")),
    "{lines:?}"
  );
  assert!(
    lines
      .iter()
      .any(|l| l.contains("the node's mode, elsewhere: ask:")),
    "{lines:?}"
  );
  let Reply::Lines(lines) = c.request(&mode_in(None)).await.unwrap() else {
    panic!()
  };
  assert!(
    lines[0].starts_with("ask:") && lines.len() == 1,
    "{lines:?}"
  );

  // The node is yolo; the place says ask: it asks there.
  c.request(&Request::Yolo { on: Some(true) }).await.unwrap();
  std::fs::write(&settings, "approval = \"ask\"\n").unwrap();
  let asked = asked_in(&mut c, in_proj("Push it again")).await;
  assert_eq!(asked, [push], "the place's ask asks");
  let Reply::Lines(lines) = c.request(&mode_in(Some(&proj))).await.unwrap() else {
    panic!()
  };
  assert!(lines[0].starts_with("ask:"), "{lines:?}");

  // Absent: the node's yolo there too.
  std::fs::write(&settings, "").unwrap();
  let asked = asked_in(&mut c, in_proj("Once more")).await;
  assert!(asked.is_empty(), "the node's yolo");
  let Reply::Lines(lines) = c.request(&mode_in(Some(&proj))).await.unwrap() else {
    panic!()
  };
  assert!(
    lines[0].starts_with("yolo:") && lines.len() == 1,
    "{lines:?}"
  );

  c.request(&Request::Yolo { on: Some(false) }).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A session started in the sandbox does not keep the thread in it once
/// yolo is on: it starts again outside it, and the next command writes
/// outside the place; once yolo is off, it starts again in the sandbox.
#[tokio::test(flavor = "multi_thread")]
async fn yolo_takes_a_sandboxed_session_out_of_the_sandbox_and_back() {
  if stitch_shell::Sandbox::available(None).is_err() {
    eprintln!("no sandbox for this test here");
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // Outside the place and the temporary directories: the checkout's
  // target directory, read-only to the thread's sandbox.
  let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
  let outside = std::fs::canonicalize(outside.path()).unwrap();
  let (one, two, three) = (
    outside.join("one"),
    outside.join("two"),
    outside.join("three"),
  );
  let (base, _) = serve(vec![
    tool_use("t1", &format!("touch '{}'", one.display())),
    answer("Refused."),
    tool_use("t2", &format!("touch '{}'", two.display())),
    answer("Made it."),
    tool_use("t3", &format!("touch '{}'", three.display())),
    answer("Refused again."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config.shell = None;
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let place = d.join("place");
  std::fs::create_dir_all(&place).unwrap();
  let mut results = Vec::new();
  let run = |thread: Option<String>, text: &str| {
    let mut req = chat(thread.as_deref(), text);
    if let Request::Chat { cwd, global, .. } = &mut req {
      *cwd = Some(place.display().to_string());
      *global = true;
    }
    req
  };
  let first = run(None, "Make one");
  let done = c
    .chat(&first, |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!()
  };
  if one.exists() {
    eprintln!(
      "the sandbox lets this test write {}: skipped",
      outside.display()
    );
    c.request(&Request::Stop).await.unwrap();
    node.wait().await.unwrap();
    return;
  }

  c.request(&Request::Yolo { on: Some(true) }).await.unwrap();
  let second = run(Some(thread.clone()), "Make two");
  c.chat(&second, |r| {
    assert!(!matches!(r, Reply::Approve { .. }), "yolo asks nothing");
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(two.exists(), "{results:?}");
  assert!(
    results[1].starts_with(
      "(Session \"main\" ran in the sandbox from before; restarted outside it: fresh directory \
       and environment.)"
    ),
    "{results:?}"
  );

  c.request(&Request::Yolo { on: Some(false) }).await.unwrap();
  let third = run(Some(thread), "Make three");
  c.chat(&third, |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(!three.exists(), "{results:?}");
  assert!(
    results[2]
      .starts_with("(Session \"main\" ran outside the sandbox from before (yolo); restarted in it"),
    "{results:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn web_images_reach_the_model_and_replay_from_blobs() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  // A file by its name: downloaded, no browser needed.
  let url = format!("http://{}/image.png", listener.local_addr().unwrap());
  let image_server = tokio::spawn(async move {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    let mut buf = [0; 4096];
    while !request.windows(4).any(|s| s == b"\r\n\r\n") {
      let n = socket.read(&mut buf).await.unwrap();
      assert!(n > 0);
      request.extend_from_slice(&buf[..n]);
    }
    // The provider is mocked; these bytes exercise lossless blob transport.
    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\nConnection: close\r\n\r\nimage").await.unwrap();
  });
  let (base, sent) = serve(vec![
    tool_call("web1", "browse_web", json!({"url": url})),
    answer("I can see the image."),
    answer("I still see it."),
  ])
  .await;
  setup(&data, &dir.path().join("home"), &base);
  let mut cfg = Config::load(&data).unwrap();
  cfg.shell = Some(stitch_node::config::ShellSettings {
    approval: Some("off".into()),
    ..Default::default()
  });
  cfg.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let done = client
    .chat(&chat(None, "Read this image"), |_| None)
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!()
  };
  image_server.await.unwrap();
  // Remove local downloads. Replay must use the persisted blob, not this path.
  std::fs::remove_dir_all(data.join("downloads")).unwrap();
  client
    .chat(&chat(Some(&thread), "Look again"), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    let names: Vec<_> = bodies[0]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| t["name"].as_str().unwrap())
      .collect();
    assert!(
      names.contains(&"browse_web") && names.contains(&"web_eval") && !names.contains(&"shell_run"),
      "the web tools are independent of the shell: {names:?}"
    );
    for body in &bodies[1..] {
      let blocks: Vec<_> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .collect();
      let image = blocks
        .iter()
        .find(|b| b["type"] == "image")
        .expect("image reaches model");
      // Its short ID comes first, for zoom_image.
      let at = blocks.iter().position(|b| b["type"] == "image").unwrap();
      assert!(
        blocks[at - 1]["text"]
          .as_str()
          .is_some_and(|t| t.starts_with("[image img:")),
        "{:?}",
        blocks[at - 1]
      );
      assert_eq!(image["source"]["media_type"], "image/png");
      assert_eq!(image["source"]["data"], "aW1hZ2U=");
      assert!(
        blocks
          .iter()
          .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "web1")
      );
    }
  }
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A provider mock that lists `models` on GET, answers requests for
/// `asked` with `asked_reply`, and the others with `responses` in order;
/// it records the requests: (the asked model's, the others').
async fn serve_routed(
  models: &'static str,
  asked: &'static str,
  asked_reply: String,
  responses: Vec<String>,
) -> (String, Arc<Mutex<(Vec<Value>, Vec<Value>)>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new((Vec::new(), Vec::new())));
  let log = seen.clone();
  tokio::spawn(async move {
    let responses = Arc::new(Mutex::new(responses.into_iter()));
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let (log, responses, asked_reply) = (log.clone(), responses.clone(), asked_reply.clone());
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let (head, body) = loop {
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
            break (head, buf[end + 4..end + 4 + len].to_vec());
          }
        };
        let response = if head.starts_with("get ") {
          format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{models}",
            models.len()
          )
        } else {
          let body: Value = serde_json::from_slice(&body).unwrap();
          let mine = body["model"] == asked;
          let mut seen = log.lock().unwrap();
          if mine {
            seen.0.push(body);
            asked_reply
          } else {
            seen.1.push(body);
            match responses.lock().unwrap().next() {
              Some(r) => r,
              None => return,
            }
          }
        };
        let _ = sock.write_all(response.as_bytes()).await;
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_to_another_model_is_a_thread_waited_for() {
  let dir = tempfile::tempdir().unwrap();
  let (base, sent) = serve_routed(
    r#"{"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}]}"#,
    "claude-haiku-5-5",
    answer("4"),
    vec![
      tool_call(
        "a1",
        "spawn",
        json!({"model": "anthropic/claude-haiku-5-5", "goal": "What is 2+2?", "tools": false}),
      ),
      tool_call("w1", "wait", json!({})),
      answer("It says 4."),
    ],
  )
  .await;
  let data = dir.path().join("data");
  setup(&data, &dir.path().join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let done = c
    .chat(&chat(None, "Ask Haiku"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { input_tokens, .. } = done else {
    panic!("{done:?}")
  };
  // This loop's three steps, and the asked model's answer it waited for.
  assert_eq!(input_tokens, 50 + 50 + 80 + 80, "{results:?}");
  assert!(results[0].starts_with("Spawned thread "), "{results:?}");
  assert!(results[1].starts_with("[Report "), "{results:?}");
  assert!(results[1].contains("4"), "{results:?}");
  {
    let seen = sent.lock().unwrap();
    let asked = &seen.0[0];
    assert!(
      asked.get("system").is_none(),
      "no agent prompt for a question"
    );
    assert!(
      asked
        .get("tools")
        .is_none_or(|t| t.as_array().unwrap().is_empty()),
      "an ask answers, without tools"
    );
    assert_eq!(asked["messages"].as_array().unwrap().len(), 1);
    assert_eq!(asked["messages"][0]["content"][0]["text"], "What is 2+2?");
  }
  // Had by waiting, its report does not wake the thread again.
  tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  assert_eq!(sent.lock().unwrap().1.len(), 3);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The user's text blocks in a request's messages, in order.
fn user_texts(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|m| m["role"] == "user")
    .flat_map(|m| m["content"].as_array().unwrap().clone())
    .filter(|c| c["type"] == "text")
    .map(|c| c["text"].as_str().unwrap().to_owned())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_sent_while_it_works_are_woven_in_after_the_tools() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("t1", "echo working"),
    answer("Done, and the logs are clean."),
    answer("Still here."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let mut woven = Vec::new();
  let mut order = Vec::new();
  let done = c
    .chat_with(&chat(None, "Do the thing"), Some(outgoing), |r| {
      match r {
        // While the model works: one message stays, one is taken back.
        Reply::Thread(_) => {
          for (id, text) in [(1, "also check the logs"), (2, "never mind")] {
            let text = text.into();
            to_chat
              .send(Request::Interject {
                id,
                text,
                attachments: vec![],
              })
              .unwrap();
          }
          to_chat.send(Request::Retract { id: 2 }).unwrap();
        }
        Reply::Woven(ids) => {
          woven.extend(ids.iter().copied());
          order.push("woven");
        }
        Reply::ToolResult { .. } => order.push("result"),
        Reply::Text(_) => order.push("text"),
        _ => {}
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "end_turn");
  assert_eq!(woven, vec![1]);
  // After the tool's result, before the next step.
  assert_eq!(order, vec!["text", "result", "woven", "text"]);
  {
    let bodies = sent.lock().unwrap();
    let msgs = bodies[1]["messages"].as_array().unwrap();
    let n = msgs.len();
    assert_eq!(msgs[n - 2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[n - 1]["content"][0]["text"], "also check the logs");
    assert!(!bodies[1].to_string().contains("never mind"));
  }

  // Written where it went: the thread continues with it in place.
  c.chat(&chat(Some(&thread), "Still there?"), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(
      user_texts(&bodies[2]),
      vec!["Do the thing", "also check the logs", "Still there?"]
    );
    let msgs = bodies[2]["messages"].as_array().unwrap();
    let at = msgs
      .iter()
      .position(|m| m["content"][0]["text"] == "also check the logs")
      .unwrap();
    assert_eq!(msgs[at - 1]["content"][0]["type"], "tool_result");
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupt_stops_the_command_and_the_model() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("t1", "sleep 60"),
    tool_use("t2", "echo free"),
    answer("Stopped, then."),
    HANG.into(),
    answer("Back again."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Stopped while its command runs: the command too, and every call has
  // its result.
  let started = std::time::Instant::now();
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let mut woven = false;
  let done = c
    .chat_with(&chat(None, "Wait a minute"), Some(outgoing), |r| match r {
      Reply::Approve { .. } => {
        let (answer, decided) = tokio::sync::oneshot::channel();
        answer.send(stitch_node::tools::Decision::allow()).unwrap();
        Some(decided)
      }
      Reply::ToolCall(_) => {
        let to_chat = to_chat.clone();
        tokio::spawn(async move {
          tokio::time::sleep(std::time::Duration::from_millis(500)).await;
          to_chat.send(Request::Interrupt).unwrap();
          // Too late to go in: it is the client's to send after.
          to_chat
            .send(Request::Interject {
              id: 1,
              text: "late".into(),
              attachments: vec![],
            })
            .unwrap();
        });
        None
      }
      Reply::Woven(_) => {
        woven = true;
        None
      }
      _ => None,
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "interrupted");
  assert!(!woven);
  assert!(started.elapsed() < std::time::Duration::from_secs(20));
  assert_eq!(sent.lock().unwrap().len(), 1, "no step after the stop");

  // The shell is free again: the command was stopped.
  let mut results = Vec::new();
  let done = c
    .chat(&chat(Some(&thread), "And now?"), |r| {
      if let Reply::ToolResult { ok, output } = r {
        results.push((*ok, output.clone()));
      }
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  assert!(
    results[0].0 && results[0].1.contains("exit 0"),
    "{results:?}"
  );
  {
    let bodies = sent.lock().unwrap();
    let msgs = bodies[1]["messages"].as_array().unwrap();
    let result = msgs
      .iter()
      .flat_map(|m| m["content"].as_array().unwrap())
      .find(|c| c["type"] == "tool_result")
      .unwrap();
    assert_eq!(result["is_error"], true);
    assert!(
      result["content"]
        .as_str()
        .unwrap()
        .contains("interrupted by the user")
    );
    assert_eq!(user_texts(&bodies[1]), vec!["Wait a minute", "And now?"]);
  }

  // Stopped while the model writes: the message is kept, nothing else.
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let done = c
    .chat_with(
      &chat(Some(&thread), "Take your time"),
      Some(outgoing),
      |r| {
        if let Reply::Thread(_) = r {
          let to_chat = to_chat.clone();
          tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            to_chat.send(Request::Interrupt).unwrap();
          });
        }
        None
      },
    )
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "interrupted");
  c.chat(&chat(Some(&thread), "Back?"), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    assert_eq!(
      user_texts(&bodies[4]),
      vec!["Wait a minute", "And now?", "Take your time", "Back?"]
    );
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A reply the network cut: the stream closes before its last event.
fn cut_off(text: &str) -> String {
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
  ])
}

/// Whether the thread's last turn never ended (`chat::unfinished`), read
/// from the store as the app reads it.
fn unfinished(data: &Path, home: &Path, thread: &str) -> Option<Unfinished> {
  unfinished_on("mbp", data, home, thread)
}

/// A turn of this node's that never ended, `why`.
fn ours(why: &'static str) -> Unfinished {
  Unfinished {
    why,
    elsewhere: None,
  }
}

/// `unfinished`, as the node `node` would say it of the same records.
fn unfinished_on(node: &str, data: &Path, home: &Path, thread: &str) -> Option<Unfinished> {
  let root = RecordRoot::open(home).unwrap();
  let index = Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let node: stitch_records::NodeName = node.parse().unwrap();
  let ctx = stitch_node::chat::ChatContext {
    node: &node,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  stitch_node::chat::unfinished(&ctx, &thread.parse().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_the_network_cut_is_sent_again_and_a_suspended_turn_resumes() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    cut_off("Half a rep"),
    answer("Whole this time."),
    HANG.into(),
    answer("Resumed."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // Cut off mid-stream: sent again, the client told to drop what it has.
  let mut texts = Vec::new();
  let done = c
    .chat(&chat(None, "Tell me"), |r| {
      match r {
        Reply::Text(t) => texts.push(t.clone()),
        Reply::Retry(why) => texts.push(format!("[retry: {why}]")),
        _ => {}
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "end_turn");
  assert_eq!(texts.len(), 3, "{texts:?}");
  assert!(
    texts[1].starts_with("[retry: ") && texts[1].contains("ended before"),
    "{texts:?}"
  );
  assert_eq!(texts[2], "Whole this time.");
  assert_eq!(sent.lock().unwrap().len(), 2);
  assert_eq!(unfinished(&data, &home, &thread), None, "it answered");

  // Suspended while the model writes: the message is kept, the stop
  // recorded as a suspension, and the turn resumes from it.
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let done = c
    .chat_with(
      &chat(Some(&thread), "Take your time"),
      Some(outgoing),
      |r| {
        if let Reply::Thread(_) = r {
          let to_chat = to_chat.clone();
          tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            to_chat.send(Request::Suspend).unwrap();
          });
        }
        None
      },
    )
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "suspended");
  assert_eq!(unfinished(&data, &home, &thread), Some(ours(SUSPENDED)));
  // Another node is not the one to resume it: it cannot tell a turn that
  // stopped from one running now.
  assert_eq!(
    unfinished_on("phone", &data, &home, &thread),
    Some(Unfinished {
      why: SUSPENDED,
      elsewhere: Some("mbp".into())
    })
  );

  let resume = Request::Resume {
    thread: thread.clone(),
    model: None,
  };
  let mut said = String::new();
  let done = c
    .chat(&resume, |r| {
      if let Reply::Text(t) = r {
        said.push_str(t);
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "end_turn");
  assert_eq!(said, "Resumed.");
  {
    let bodies = sent.lock().unwrap();
    // The resumed call is the suspended one sent again: the message once.
    assert_eq!(user_texts(&bodies[3]), vec!["Tell me", "Take your time"]);
  }
  assert_eq!(unfinished(&data, &home, &thread), None);
  // Nothing left to resume.
  let again = c.chat(&resume, |_| None).await;
  assert!(
    again
      .as_ref()
      .is_err_and(|e| e.to_string().contains("nothing to resume")),
    "{again:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_detached_loop_at_work_is_resumed_only_in_a_fork() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // A step written (a tool call and its result, asking nothing), then a
  // call that hangs; the fork's call answers.
  let (base, sent) = serve(vec![
    tool_call("t1", "shell_view", json!({"session": "none"})),
    HANG.into(),
    answer("Forked on."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&Request::Detach {
      goal: "Say hi".into(),
      model: None,
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
      tools: vec![],
      skills: vec![],
      keep: false,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  asked(&sent, 2).await;
  // Its records end in a tool result, as a turn cut off does; but its
  // loop has the turn, and resumes on its own node if it stops.
  assert_eq!(
    unfinished(&data, &home, &thread),
    Some(Unfinished {
      why: CUT_OFF,
      elsewhere: Some("mbp".into())
    })
  );
  let resume = |thread: &str| Request::Resume {
    thread: thread.to_owned(),
    model: None,
  };
  let refused = c.chat(&resume(&thread), |_| None).await;
  assert!(
    refused
      .as_ref()
      .is_err_and(|e| e.to_string().contains("fork the thread")),
    "{refused:?}"
  );
  assert_eq!(sent.lock().unwrap().len(), 2, "no call of its own");

  // Forked, it is this node's to go on from the same step.
  let Reply::Forked(fork) = c
    .request(&Request::Fork {
      thread: thread.clone(),
      upto: None,
      last: None,
      no_attachments: false,
      no_tools: false,
      summarize: false,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(unfinished(&data, &home, &fork), Some(ours(CUT_OFF)));
  let mut said = String::new();
  let done = c
    .chat(&resume(&fork), |r| {
      if let Reply::Text(t) = r {
        said.push_str(t);
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone {
    thread: ran, stop, ..
  } = done
  else {
    panic!("{done:?}")
  };
  assert_eq!((ran.as_str(), stop.as_str()), (fork.as_str(), "end_turn"));
  assert_eq!(said, "Forked on.");
  {
    // Sent from the step the loop was at: its tool result last.
    let bodies = sent.lock().unwrap();
    let last = bodies[2]["messages"]
      .as_array()
      .unwrap()
      .last()
      .unwrap()
      .clone();
    assert!(last.to_string().contains("tool_result"), "{last}");
  }
  assert_eq!(unfinished(&data, &home, &fork), None);
  // The original's loop is still at work, untouched.
  assert_eq!(
    unfinished(&data, &home, &thread).and_then(|u| u.elsewhere),
    Some("mbp".into())
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Whether a process is still there (a zombie not reaped yet counts as
/// gone: it runs nothing).
fn alive(pid: i32) -> bool {
  let o = std::process::Command::new("ps")
    .args(["-o", "stat=", "-p", &pid.to_string()])
    .output()
    .expect("ps runs");
  let stat = String::from_utf8_lossy(&o.stdout);
  let stat = stat.trim();
  !stat.is_empty() && !stat.starts_with('Z')
}

/// Whether `ps` runs here (an agent's sandbox refuses it), which `alive`
/// needs; said when not.
fn ps_runs() -> bool {
  let runs = std::process::Command::new("ps")
    .args(["-o", "stat=", "-p", &std::process::id().to_string()])
    .output()
    .is_ok_and(|o| o.status.success());
  if !runs {
    eprintln!("ps does not run here (a sandbox?): skipped");
  }
  runs
}

/// Waits for processes to be gone; the ones still there.
async fn gone(pids: &[i32]) -> Vec<i32> {
  for _ in 0..40 {
    if pids.iter().all(|p| !alive(*p)) {
      return Vec::new();
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  pids.iter().copied().filter(|p| alive(*p)).collect()
}

/// A script whose processes all ignore Ctrl+C, SIGTERM and SIGHUP: a
/// child, a grandchild, and itself, their pids written to `$1`.
const STUBBORN: &str = r#"trap '' INT TERM HUP
( trap '' INT TERM HUP; exec sleep 1000 ) &
echo $! >> "$1"
( trap '' INT TERM HUP; sleep 1001 & echo $! >> "$1"; wait ) &
echo $! >> "$1"
echo $$ >> "$1"
sleep 0.3
echo READY
for i in 1 2 3; do echo "line $i"; done
wait
"#;

/// `jobs`, `job_stop`, `job_output` and `max_secs`: a background command
/// whose processes ignore every polite signal is stopped with all of them,
/// listed as stopped with its output kept; one with a time limit is
/// stopped when it passes.
#[tokio::test(flavor = "multi_thread")]
async fn job_stop_ends_the_whole_process_group_and_the_job_is_kept() {
  if !ps_runs() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let script = d.join("stubborn.sh");
  std::fs::write(&script, STUBBORN).unwrap();
  let pids = d.join("pids");
  let limited = d.join("limited");
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": format!("bash '{}' '{}'", script.display(), pids.display()), "background": true, "session": "s"}),
    ),
    tool_call(
      "t2",
      "shell_wait",
      json!({"session": "s", "pattern": "READY", "timeout_secs": 10}),
    ),
    tool_call("t3", "jobs", json!({})),
    tool_call("t4", "job_stop", json!({"job": 1, "grace_secs": 1})),
    tool_call("t5", "jobs", json!({})),
    tool_call("t6", "job_output", json!({"job": 1, "lines": 2})),
    tool_call(
      "t7",
      "shell_run",
      json!({"command": format!("bash '{}' '{}'", script.display(), limited.display()), "background": true, "max_secs": 2, "stop_on_timeout": true}),
    ),
    tool_call("t8", "unwatch", json!({"job": 99})),
    answer("Stopped."),
    answer("It timed out."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  c.chat(&chat(None, "Run and stop it"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(results[0].contains("as job 1"), "{results:?}");
  assert!(results[0].contains("job_stop {\"job\": 1}"), "{results:?}");
  // Listed while it runs, with its last lines.
  assert!(
    results[2].contains("job 1 (shell): session \"s\"")
      && results[2].contains("running: tells when it ends")
      && results[2].contains("│ READY"),
    "{results:?}"
  );
  // Stopped: Ctrl+C and SIGTERM ignored, SIGKILL to the group.
  let stop = &results[3];
  assert!(stop.starts_with("Job 1 (session \"s\""), "{stop}");
  assert!(
    stop.contains("Ctrl+C")
      && stop.contains("SIGTERM to its process group")
      && stop.contains("SIGKILL"),
    "{stop}"
  );
  assert!(stop.contains("it ended with exit 137"), "{stop}");
  let listed: Vec<i32> = std::fs::read_to_string(&pids)
    .unwrap()
    .lines()
    .map(|l| l.trim().parse().unwrap())
    .collect();
  assert_eq!(listed.len(), 4, "{listed:?}");
  assert_eq!(
    gone(&listed).await,
    Vec::<i32>::new(),
    "every process of it ended"
  );
  assert!(
    results[4].contains("job 1 (shell): session \"s\""),
    "{results:?}"
  );
  assert!(results[4].contains("; stopped at "), "{results:?}");
  // Its output, kept after it ended.
  assert!(
    results[5].contains(": stopped; lines 4–5 of 5]\nline 3\n") && results[5].contains("Killed"),
    "{results:?}"
  );
  assert_eq!(results[7], "no job 99 of this thread");

  // The time limit: told it timed out, and stopped (the polite signals
  // take their grace, 5 seconds each).
  for _ in 0..600 {
    if sent.lock().unwrap().len() >= 10 {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  asked(&sent, 10).await;
  let note = {
    let bodies = sent.lock().unwrap();
    user_texts(&bodies[9]).pop().unwrap()
  };
  assert!(note.starts_with("[Job 2 (session \"bg2\""), "{note}");
  assert!(
    note.contains("timed out after 2s; stopped: Ctrl+C"),
    "{note}"
  );
  let listed: Vec<i32> = std::fs::read_to_string(&limited)
    .unwrap()
    .lines()
    .map(|l| l.trim().parse().unwrap())
    .collect();
  assert_eq!(
    gone(&listed).await,
    Vec::<i32>::new(),
    "every process of it ended"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Bash runs each command of a list as a job of its own, and goes on to
/// the next when one is killed by SIGTERM: `job_stop` signals each.
#[tokio::test(flavor = "multi_thread")]
async fn job_stop_follows_a_list_from_command_to_command() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 100; echo NEXT-$((1+1)); sleep 101; echo LAST-$((2+1))", "background": true}),
    ),
    tool_call("t2", "job_stop", json!({"job": 1, "signal": "term", "grace_secs": 2})),
    answer("Stopped."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  c.chat(&chat(None, "Run and stop it"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  let stop = &results[1];
  assert!(
    stop.contains("SIGTERM to its process groups ") && !stop.contains("Ctrl+C"),
    "{stop}"
  );
  assert!(stop.contains("NEXT-2") && stop.contains("LAST-3"), "{stop}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A test that spawns a child ignoring Ctrl+C and SIGTERM, writes its own
/// pid, cargo's and the child's to `$PIDS`, and hangs.
const HANGING_TEST: &str = r#"
#[test]
fn hang() {
  use std::io::Write;
  let child = std::process::Command::new("sh")
    .args(["-c", "trap '' INT TERM HUP; exec sleep 1005"])
    .spawn()
    .unwrap();
  let mut f = std::fs::OpenOptions::new()
    .append(true)
    .create(true)
    .open(std::env::var("PIDS").unwrap())
    .unwrap();
  writeln!(f, "{}\n{}\n{}", std::process::id(), std::os::unix::process::parent_id(), child.id()).unwrap();
  println!("HANGING");
  std::thread::sleep(std::time::Duration::from_secs(1000));
}
"#;

/// What `job_stop` was built for: a `just` recipe running `cargo test`,
/// whose test binary started a child that ignores Ctrl+C. Ctrl+C ends
/// `just`, cargo and the test, and leaves that child orphaned in the
/// process group; it is stopped too. Runs where `just` is installed.
#[tokio::test(flavor = "multi_thread")]
async fn job_stop_ends_a_just_recipe_running_cargo_with_its_children() {
  if !ps_runs() {
    return;
  }
  if std::process::Command::new("just")
    .arg("--version")
    .output()
    .is_err()
  {
    eprintln!("no just here: skipped");
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let krate = d.join("hang");
  std::fs::create_dir_all(krate.join("tests")).unwrap();
  std::fs::create_dir_all(krate.join("src")).unwrap();
  std::fs::write(
    krate.join("Cargo.toml"),
    "[package]\nname = \"hang\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
  )
  .unwrap();
  std::fs::write(krate.join("src/lib.rs"), "").unwrap();
  std::fs::write(krate.join("tests/hang.rs"), HANGING_TEST).unwrap();
  let pids = d.join("pids");
  std::fs::write(
    d.join("justfile"),
    format!(
      "test:\n    echo $$ $PPID >> '{p}'; PIDS='{p}' CARGO_TARGET_DIR='{t}' cargo test --manifest-path '{m}' -- --nocapture\n",
      p = pids.display(),
      t = d.join("target").display(),
      m = krate.join("Cargo.toml").display(),
    ),
  )
  .unwrap();
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": format!("cd '{}' && just test", d.display()), "background": true, "session": "s"}),
    ),
    tool_call(
      "t2",
      "shell_wait",
      json!({"session": "s", "pattern": "HANGING", "timeout_secs": 300}),
    ),
    tool_call("t3", "job_stop", json!({"job": 1, "grace_secs": 2})),
    answer("Stopped."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  c.chat(&chat(None, "Run and stop it"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(results[1].starts_with("matched: HANGING"), "{results:?}");
  let listed: Vec<i32> = std::fs::read_to_string(&pids)
    .unwrap()
    .split_whitespace()
    .map(|l| l.parse().unwrap())
    .collect();
  // The recipe's shell, just, the test binary, cargo, the child.
  assert_eq!(listed.len(), 5, "{listed:?}");
  eprintln!("stopped: {}", results[2]);
  assert!(results[2].contains("Ctrl+C"), "{results:?}");
  assert_eq!(gone(&listed).await, Vec::<i32>::new(), "{results:?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Approves every command.
fn approve_all(r: &Reply) -> Option<tokio::sync::oneshot::Receiver<stitch_node::tools::Decision>> {
  if let Reply::Approve { .. } = r {
    let (answer, decided) = tokio::sync::oneshot::channel();
    answer.send(stitch_node::tools::Decision::allow()).unwrap();
    return Some(decided);
  }
  None
}

/// Waits until the model was asked `n` times.
async fn asked(sent: &Arc<Mutex<Vec<Value>>>, n: usize) {
  for _ in 0..200 {
    if sent.lock().unwrap().len() >= n {
      return;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  panic!("asked {} times, not {n}", sent.lock().unwrap().len());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_command_wakes_its_thread_when_it_ends() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 2; echo built-it", "background": true}),
    ),
    answer("Started it."),
    answer("The build is done."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let done = c
    .chat(&chat(None, "Build it"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      approve_all(r)
    })
    .await
    .unwrap();
  let started = std::time::Instant::now();
  assert!(matches!(done, Reply::ChatDone { .. }));
  assert!(
    results[0].starts_with("Running in the background as job 1, in session \"bg1\""),
    "{results:?}"
  );
  // Listed while it runs.
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  assert_eq!(jobs.len(), 1, "{jobs:?}");
  assert_eq!(jobs[0].kind, "shell");
  assert!(jobs[0].what.contains("sleep 2"), "{jobs:?}");
  assert!(jobs[0].state.contains("job 1"), "{jobs:?}");
  // Its end wakes the thread, with what it printed.
  asked(&sent, 3).await;
  assert!(started.elapsed() < std::time::Duration::from_secs(10));
  {
    let bodies = sent.lock().unwrap();
    let note = user_texts(&bodies[2]).pop().unwrap();
    assert!(
      note.starts_with("[Job 1 (session \"bg1\", `sleep 2; echo built-it`) ended with exit 0"),
      "{note}"
    );
    assert!(note.contains("built-it"), "{note}");
  }
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  assert!(jobs.is_empty(), "{jobs:?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_ends_while_the_loop_runs_is_woven_in() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 2; echo woven-done", "background": true}),
    ),
    tool_use("t2", "sleep 4"),
    answer("Both done."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut notes = Vec::new();
  let done = c
    .chat(&chat(None, "Build and wait"), |r| {
      if let Reply::Note(n) = r {
        notes.push(n.clone());
      }
      approve_all(r)
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  assert_eq!(notes.len(), 1, "{notes:?}");
  assert!(notes[0].contains("woven-done"), "{notes:?}");
  {
    let bodies = sent.lock().unwrap();
    let msgs = bodies[2]["messages"].as_array().unwrap();
    let n = msgs.len();
    assert_eq!(msgs[n - 2]["content"][0]["type"], "tool_result");
    assert!(
      msgs[n - 1]["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("[Job 1"),
      "{:?}",
      msgs[n - 1]
    );
  }
  // Had in the loop: it wakes nothing after.
  tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  assert_eq!(sent.lock().unwrap().len(), 3);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn jobs_watch_for_lines_and_stop() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let server = "for i in 1 2 3; do echo tick $i; sleep 1; done; echo ready-now; sleep 60";
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": server, "background": true, "session": "srv"}),
    ),
    tool_call(
      "t2",
      "shell_watch",
      json!({"session": "srv", "pattern": "ready-now"}),
    ),
    tool_call(
      "t3",
      "shell_wait",
      json!({"session": "srv", "pattern": "tick \\d", "timeout_secs": 10}),
    ),
    answer("Waiting for it."),
    answer("It is ready."),
    answer("It stopped."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let done = c
    .chat(&chat(None, "Start the server"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      approve_all(r)
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!()
  };
  assert!(
    results[0].contains("job 1, in session \"srv\""),
    "{results:?}"
  );
  assert_eq!(
    results[1],
    "Watching as job 2: you are told when it prints a line matching \"ready-now\" or ends."
  );
  assert!(results[2].starts_with("matched: tick 1"), "{results:?}");
  // The line wakes it; the command runs on.
  asked(&sent, 5).await;
  {
    let bodies = sent.lock().unwrap();
    let note = user_texts(&bodies[4]).pop().unwrap();
    assert!(
      note.contains("printed a line matching \"ready-now\"") && note.contains("runs on"),
      "{note}"
    );
  }
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  assert_eq!(jobs.len(), 1, "{jobs:?}");
  assert_eq!(jobs[0].id, format!("{thread}/srv"));
  assert_eq!(jobs[0].state, "job 1: tells when it ends");
  // Stopped by the user: its job tells how it ended.
  let Reply::Lines(said) = c
    .request(&Request::StopJob {
      id: jobs[0].id.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(said, vec!["sent Ctrl+C to session srv"]);
  asked(&sent, 6).await;
  {
    let bodies = sent.lock().unwrap();
    let note = user_texts(&bodies[5]).pop().unwrap();
    assert!(note.starts_with("[Job 1 (session \"srv\""), "{note}");
    assert!(note.contains("ended with exit 130"), "{note}");
  }
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  assert!(jobs.is_empty(), "{jobs:?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Says hello as a client of `version`; the node's answer.
async fn hello_as(data: &Path, version: &str) -> Reply {
  let stream = tokio::net::UnixStream::connect(stitch_node::socket_path(data).unwrap())
    .await
    .unwrap();
  let (mut r, mut w) = stream.into_split();
  stitch_node::api::write(
    &mut w,
    &Request::Hello {
      version: version.into(),
    },
  )
  .await
  .unwrap();
  stitch_node::api::read::<Reply, _>(&mut r)
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_client_restarts_the_node_once_it_is_idle() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![HANG.into()]).await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();

  // At work: a chat waiting on its model.
  let mut c = Client::connect(&data).await.unwrap();
  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let chatting = tokio::spawn(async move {
    c.chat_with(&chat(None, "Take your time"), Some(outgoing), |_| None)
      .await
  });
  asked(&sent, 1).await;
  let Reply::Hello {
    busy, restarting, ..
  } = hello_as(&data, "99.0.0/local-999").await
  else {
    panic!()
  };
  assert!(!restarting);
  assert_eq!(busy.len(), 1, "{busy:?}");
  assert!(
    busy[0].starts_with("thread ") && busy[0].ends_with(" working"),
    "{busy:?}"
  );
  // An older client changes nothing; this version still talks to it.
  let Reply::Hello { restarting, .. } = hello_as(&data, "0.0.1/local-1").await else {
    panic!()
  };
  assert!(!restarting);
  assert!(Client::connect(&data).await.is_ok());

  // Idle again, it stops on its own, for the newer one to start.
  to_chat.send(Request::Interrupt).unwrap();
  let done = chatting.await.unwrap().unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }));
  tokio::time::timeout(std::time::Duration::from_secs(10), node.wait())
    .await
    .expect("it stopped")
    .unwrap();
  assert!(matches!(
    Client::connect(&data).await,
    Err(stitch_node::Error::NotRunning)
  ));

  // Idle from the start: it says so, and stops at once.
  let node = stitch_node::start(&data).await.unwrap();
  let Reply::Hello {
    busy, restarting, ..
  } = hello_as(&data, "99.0.0/local-999").await
  else {
    panic!()
  };
  assert!(restarting && busy.is_empty(), "{busy:?}");
  tokio::time::timeout(std::time::Duration::from_secs(10), node.wait())
    .await
    .expect("it stopped")
    .unwrap();
}

/// A new build's restart waits for idle, but a command left in a shell
/// (a server that never ends) holds it only so long, saying so meanwhile
/// (issue ubbm: a dev server kept a node on its old build for hours).
#[tokio::test(flavor = "multi_thread")]
async fn a_server_in_a_shell_holds_a_new_builds_restart_only_so_long() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 600", "background": true}),
    ),
    answer("Serving."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, "Serve it"), approve_all).await.unwrap();

  let restart = node.restart_when_idle_within(std::time::Duration::from_secs(6));
  restart("build test installed".into());
  tokio::time::sleep(std::time::Duration::from_secs(3)).await;
  let Reply::Status { pending, .. } = c.request(&Request::Status).await.unwrap() else {
    panic!()
  };
  let pending = pending.expect("a stop pending");
  assert!(
    pending.contains("once idle, waiting since ")
      && pending.contains(" (now: `sleep 600` running in a shell)")
      && pending.contains("commands in shells hold it 1 min more at most"),
    "{pending}"
  );
  let r = tokio::time::timeout(std::time::Duration::from_secs(15), node.wait_restart())
    .await
    .expect("it stopped, cutting the command")
    .unwrap();
  assert_eq!(r.expect("a restart").why, "build test installed");
}

/// A new build's restart held by work for long is never cut, but the user
/// is told once: which build waits, and what keeps the node busy (the
/// 2026-11-02 incident: a node waited for 9 hours and nobody knew).
#[tokio::test(flavor = "multi_thread")]
async fn a_restart_waiting_long_is_told_to_the_user_once() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 600", "background": true}),
    ),
    answer("Serving."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, "Serve it"), approve_all).await.unwrap();

  let restart = node.restart_when_idle_noticing(
    std::time::Duration::from_secs(600),
    std::time::Duration::from_secs(3),
  );
  restart("build abc installed".into());
  let start = std::time::Instant::now();
  let notices = loop {
    let n = node.take_notices();
    if !n.is_empty() {
      break n;
    }
    assert!(
      start.elapsed() < std::time::Duration::from_secs(20),
      "no notice"
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  };
  assert!(start.elapsed() >= std::time::Duration::from_secs(2));
  assert_eq!(notices.len(), 1, "{notices:?}");
  let body = &notices[0].body;
  assert!(
    body.contains("build abc installed")
      && body.contains("busy: `sleep 600` running in a shell")
      && body.contains("stitch node restart --now"),
    "{body}"
  );
  // Once: not again, and the node still runs, not cut.
  tokio::time::sleep(std::time::Duration::from_secs(5)).await;
  assert!(node.take_notices().is_empty());
  let Reply::Status { pending, .. } = c.request(&Request::Status).await.unwrap() else {
    panic!()
  };
  assert!(
    pending
      .unwrap()
      .contains("stopping for build abc installed once idle, waiting since ")
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paste_goes_whole_and_submit_presses_enter_after() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let got = d.join("got.bin");
  let text = format!("{}\nsecond line", "long ".repeat(800));
  // A program in raw mode that asked for bracketed paste, keeping what it
  // is sent: the paste, then the Enter.
  let want = format!("\u{1b}[200~{text}\u{1b}[201~\r");
  let script = format!(
    "printf '\\033[?2004h'; stty raw -echo; head -c {} > '{}'; stty sane; printf '\\033[?2004l'",
    want.len(),
    got.display()
  );
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": script, "timeout_secs": 1}),
    ),
    tool_call("t2", "shell_send", json!({"text": text, "submit": true})),
    answer("Sent."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.chat(&chat(None, "Paste it"), approve_all).await.unwrap();
  assert_eq!(std::fs::read_to_string(&got).unwrap(), want);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn screens_and_files_are_watched_at_no_model_cost() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let log = d.join("transcript.jsonl");
  let other = d.join("other.txt");
  std::fs::write(&log, "{\"type\":\"start\"}\n").unwrap();
  std::fs::write(&other, "x\n").unwrap();
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "sleep 3; echo DONE-$((40+2)); sleep 30", "background": true, "session": "s"}),
    ),
    tool_call("t2", "shell_watch", json!({"session": "s", "screen": "DONE-\\d+"})),
    tool_call(
      "t3",
      "watch_file",
      json!({"path": log.display().to_string(), "pattern": "end_turn", "quiet_secs": 1}),
    ),
    tool_call("t4", "watch_file", json!({"path": other.display().to_string()})),
    tool_call("t5", "unwatch", json!({"job": 4})),
    answer("Watching."),
    answer("The screen says done."),
    answer("The session ended its turn."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  c.chat(&chat(None, "Watch them"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(
    results[1].starts_with("Watching as job 2: you are told when the screen shows \"DONE-\\d+\""),
    "{results:?}"
  );
  assert!(
    results[2].contains("as job 3: you are told when a line matching \"end_turn\" was added"),
    "{results:?}"
  );
  assert_eq!(results[4], "job 4 cancelled");
  // The screen: it wakes the thread with what it shows.
  asked(&sent, 7).await;
  {
    let bodies = sent.lock().unwrap();
    let note = user_texts(&bodies[6]).pop().unwrap();
    assert!(note.starts_with("[Job 2 (session \"s\""), "{note}");
    assert!(note.contains("\"DONE-42\""), "{note}");
  }
  // The file: a line, then quiet.
  {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    writeln!(f, "{{\"type\":\"assistant\",\"stop_reason\":\"end_turn\"}}").unwrap();
  }
  std::fs::write(&other, "changed\n").unwrap();
  asked(&sent, 8).await;
  {
    let bodies = sent.lock().unwrap();
    let note = user_texts(&bodies[7]).pop().unwrap();
    assert!(note.starts_with("[Job 3 (file "), "{note}");
    assert!(note.contains("end_turn"), "{note}");
  }
  // The cancelled one tells nothing.
  tokio::time::sleep(std::time::Duration::from_secs(2)).await;
  assert_eq!(sent.lock().unwrap().len(), 8);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_person_types_into_a_session_is_minded() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "cat", "background": true, "session": "s"}),
    ),
    answer("It runs."),
    tool_call(
      "t2",
      "shell_send",
      json!({"session": "s", "text": "from the agent\n"}),
    ),
    tool_call("t3", "shell_view", json!({"session": "s"})),
    answer("I see what you typed."),
    tool_use("t4", "sleep 4"),
    answer("Noted what you typed meanwhile."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "Start cat"), approve_all).await.unwrap()
  else {
    panic!()
  };

  // Attached: the screen as it is, then what it shows as it goes.
  let mut a = Client::connect(&data).await.unwrap();
  let (keys, input) = tokio::sync::mpsc::unbounded_channel();
  let shown = Arc::new(Mutex::new(Vec::<u8>::new()));
  let seen = shown.clone();
  let id = format!("{thread}/s");
  let attached = tokio::spawn(async move {
    a.attach(&id, input, |b| seen.lock().unwrap().extend_from_slice(b))
      .await
  });
  keys.send(b"hello there\r".to_vec()).unwrap();
  for _ in 0..100 {
    if String::from_utf8_lossy(&shown.lock().unwrap()).contains("hello there") {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  assert!(String::from_utf8_lossy(&shown.lock().unwrap()).contains("\u{1b}[H\u{1b}[2J"));
  assert!(String::from_utf8_lossy(&shown.lock().unwrap()).contains("hello there"));
  drop(keys);
  assert!(!attached.await.unwrap().unwrap(), "detached, not ended");

  // The agent sends nothing on top of it, and hears what was typed.
  let mut results = Vec::new();
  c.chat(&chat(Some(&thread), "Go on"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(
    results[0].starts_with("[The user typed into session \"s\" (attached to it): «hello there⏎»"),
    "{results:?}"
  );
  assert!(results[0].contains("Nothing was sent"), "{results:?}");
  // Told once: the look after shows the screen, no note.
  assert!(!results[1].contains("The user typed"), "{results:?}");
  assert!(results[1].contains("hello there"), "{results:?}");
  assert!(!results[1].contains("from the agent"), "{results:?}");

  // Typed while the thread works: woven into its loop once the typing
  // pauses.
  let mut a = Client::connect(&data).await.unwrap();
  let (keys, input) = tokio::sync::mpsc::unbounded_channel();
  let id = format!("{thread}/s");
  let attached = tokio::spawn(async move { a.attach(&id, input, |_| {}).await });
  let mut notes = Vec::new();
  let typing = {
    let keys = keys.clone();
    tokio::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(500)).await;
      keys.send(b"stop that".to_vec()).unwrap();
    })
  };
  c.chat(&chat(Some(&thread), "Wait a bit"), |r| {
    if let Reply::Note(n) = r {
      notes.push(n.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  typing.await.unwrap();
  assert_eq!(notes.len(), 1, "{notes:?}");
  assert!(notes[0].contains("«stop that»"), "{notes:?}");
  drop(keys);
  attached.await.unwrap().unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_is_gone_is_never_a_fresh_shell_by_surprise() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    // Sent to a session that is not there (a restart ended it): refused,
    // not typed into a new shell.
    tool_call(
      "t1",
      "shell_send",
      json!({"session": "tui", "text": "hello tui\n"}),
    ),
    tool_call("t2", "shell_view", json!({"session": "tui"})),
    tool_call(
      "t3",
      "shell_run",
      json!({"session": "tui", "command": "echo one"}),
    ),
    tool_call(
      "t4",
      "shell_run",
      json!({"session": "tui", "command": "echo two"}),
    ),
    tool_call(
      "t5",
      "shell_run",
      json!({"session": "tui", "command": "exit"}),
    ),
    tool_call(
      "t6",
      "shell_send",
      json!({"session": "tui", "text": "more\n"}),
    ),
    answer("Done."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  c.chat(&chat(None, "Drive the tui"), |r| {
    if let Reply::ToolResult { ok, output } = r {
      results.push((*ok, output.clone()));
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(
    !results[0].0 && results[0].1.starts_with("no session tui here"),
    "{results:?}"
  );
  assert!(
    !results[1].0 && results[1].1.starts_with("no session tui here"),
    "{results:?}"
  );
  assert!(
    results[2]
      .1
      .starts_with("(New session \"tui\": a fresh shell"),
    "{results:?}"
  );
  assert!(!results[3].1.contains("New session"), "{results:?}");
  // Its shell exited: the name does not quietly become a new shell.
  assert!(
    !results[5].0 && results[5].1.starts_with("session tui has ended"),
    "{results:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_a_loop_did_not_take_wakes_nothing_and_waits() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_call(
      "t1",
      "shell_run",
      json!({"command": "cat", "background": true, "session": "s"}),
    ),
    answer("It runs."),
    // The loop's last step is slow: the typing pauses while the model
    // answers, so the loop ends without taking it.
    tool_use("t2", "sleep 1"),
    format!("{SLOW}{}", answer("All done.")),
    tool_call("t3", "shell_view", json!({"session": "s"})),
    answer("I see what you typed."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "Start cat"), approve_all).await.unwrap()
  else {
    panic!()
  };
  let mut a = Client::connect(&data).await.unwrap();
  let (keys, input) = tokio::sync::mpsc::unbounded_channel();
  let id = format!("{thread}/s");
  let attached = tokio::spawn(async move { a.attach(&id, input, |_| {}).await });
  let typing = {
    let keys = keys.clone();
    tokio::spawn(async move {
      // Typed at 1.3s: the pause ends at 2.8s, inside the slow answer.
      tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
      keys.send(b"mine".to_vec()).unwrap();
    })
  };
  let mut notes = Vec::new();
  c.chat(&chat(Some(&thread), "Wait"), |r| {
    if let Reply::Note(n) = r {
      notes.push(n.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  typing.await.unwrap();
  assert!(notes.is_empty(), "the loop took it after all: {notes:?}");
  // No wake for it: the model was not asked again.
  tokio::time::sleep(std::time::Duration::from_secs(3)).await;
  assert_eq!(sent.lock().unwrap().len(), 4, "woken for typing");
  // It waited for the agent's next call on the session.
  let mut results = Vec::new();
  c.chat(&chat(Some(&thread), "Look"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    approve_all(r)
  })
  .await
  .unwrap();
  assert!(results[0].contains("«mine»"), "{results:?}");
  drop(keys);
  attached.await.unwrap().unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn odds_and_ends_of_sessions() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    // A program left bracketed paste on and is gone: the shell at its
    // prompt gets plain text, not paste markers.
    tool_call("t1", "shell_run", json!({"session": "s", "command": "printf '\\033[?2004h'"})),
    tool_call("t2", "shell_send", json!({"session": "s", "text": "echo hi-$((1+1))", "submit": true})),
    // Submitted at the prompt, a command keeps its name.
    tool_call("t3", "shell_send", json!({"session": "s", "text": "sleep 30", "submit": true})),
    // A line job whose pattern says "screen" is still a line job.
    tool_call(
      "t4",
      "shell_run",
      json!({"session": "w", "command": "sleep 1; echo screen-$((3+4)); sleep 30", "background": true}),
    ),
    tool_call("t5", "shell_watch", json!({"session": "w", "pattern": "screen-\\d"})),
    answer("Done."),
    answer("Seen."),
  ])
  .await;
  let data = d.join("data");
  setup(&data, &d.join("home"), &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "Odds and ends"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      approve_all(r)
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(
    results[1].contains("hi-2") && results[1].contains("exit 0"),
    "{results:?}"
  );
  let Reply::Jobs(jobs) = c.request(&Request::Jobs).await.unwrap() else {
    panic!()
  };
  let s = jobs.iter().find(|j| j.id == format!("{thread}/s")).unwrap();
  assert_eq!(s.what, "sleep 30", "{jobs:?}");
  // The line job's note, woven or woken.
  let said = |sent: &Arc<Mutex<Vec<Value>>>| {
    sent
      .lock()
      .unwrap()
      .iter()
      .map(|b| b.to_string())
      .collect::<String>()
  };
  for _ in 0..100 {
    if said(&sent).contains("Job 2 (session") {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  let all = said(&sent);
  assert!(
    all.contains(r#"printed a line matching \"screen-\\d\""#),
    "{all}"
  );

  // Attached when its session closes: it says the session ended.
  let mut a = Client::connect(&data).await.unwrap();
  let (keys, input) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
  let id = format!("{thread}/s");
  let attached = tokio::spawn(async move { a.attach(&id, input, |_| {}).await });
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  for _ in 0..2 {
    c.request(&Request::StopJob {
      id: format!("{thread}/s"),
    })
    .await
    .unwrap();
  }
  let ended = tokio::time::timeout(std::time::Duration::from_secs(10), attached)
    .await
    .expect("the attach saw its session close")
    .unwrap();
  assert!(ended.unwrap(), "ended, not detached");
  drop(keys);

  // Attached when the node goes away: an error, not "the session ended".
  let mut a = Client::connect(&data).await.unwrap();
  let (keys, input) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
  let id = format!("{thread}/w");
  let attached = tokio::spawn(async move { a.attach(&id, input, |_| {}).await });
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let ended = tokio::time::timeout(std::time::Duration::from_secs(10), attached)
    .await
    .expect("the attach saw the node go")
    .unwrap();
  assert!(ended.is_err(), "{ended:?}");
  drop(keys);
}

/// The records of `kind` in the home workspace, read from its files.
fn home_records(home: &Path, kind: &str) -> Vec<stitch_store::IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  index.refresh(&root).unwrap();
  index.records(Some(kind)).unwrap()
}

/// The answers to `question`, read from the home workspace's files.
fn answers_to(home: &Path, question: &str) -> Vec<stitch_store::IndexedRecord> {
  let root = RecordRoot::open(home).unwrap();
  home_records(home, "answer")
    .into_iter()
    .filter(|a| {
      root
        .read_record(&a.id)
        .unwrap()
        .refs
        .first("answers")
        .is_some_and(|r| r.id.to_string() == question)
    })
    .collect()
}

async fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
  for _ in 0..200 {
    if let Some(v) = f() {
      return v;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_whose_client_leaves_waits_on_the_attention_list_and_goes_on() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, sent) = serve(vec![
    tool_use("t1", &format!("touch '{}'", made.display())),
    answer("Made it."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  // The client sees the approval, then goes (the TUI opening another
  // thread, a closed terminal).
  let (asked_tx, mut asked_rx) = tokio::sync::mpsc::unbounded_channel();
  let leaving = {
    let data = data.clone();
    tokio::spawn(async move {
      let mut c = Client::connect(&data).await.unwrap();
      let _ = c
        .chat_with(&chat(None, "Make a file"), None, |r| {
          if let Reply::Approve { question, .. } = r {
            let _ = asked_tx.send(question.clone());
            // Never answered here.
            let (keep, decided) = tokio::sync::oneshot::channel();
            std::mem::forget(keep);
            return Some(decided);
          }
          None
        })
        .await;
    })
  };
  let question = asked_rx.recv().await.unwrap();
  leaving.abort();
  let _ = leaving.await;
  // Not declined for want of an answer: open, on the attention list.
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert!(answers_to(&home, &question).is_empty());
  assert!(!made.exists());
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Answer {
    question: question.clone(),
    allow: true,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  until("the file", || made.exists().then_some(())).await;
  asked(&sent, 2).await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_answered_elsewhere_is_taken_back_from_the_client() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, _sent) = serve(vec![
    tool_use("t1", &format!("touch '{}'", made.display())),
    answer("Made it."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut held = Vec::new();
  let mut taken_back = Vec::new();
  let mut asked_id = None;
  let other = data.clone();
  let done = c
    .chat_with(&chat(None, "Make a file"), None, |r| match r {
      Reply::Approve { id, question, .. } => {
        asked_id = Some(*id);
        // Answered from another client (the phone, `stitch answer`).
        let (data, question) = (other.clone(), question.clone());
        tokio::spawn(async move {
          let mut c = Client::connect(&data).await.unwrap();
          c.request(&Request::Answer {
            question,
            allow: true,
            reason: None,
            always: false,
          })
          .await
          .unwrap();
        });
        let (keep, decided) = tokio::sync::oneshot::channel();
        held.push(keep);
        Some(decided)
      }
      Reply::Answered { id } => {
        taken_back.push(*id);
        None
      }
      _ => None,
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(made.exists());
  assert_eq!(taken_back, vec![asked_id.unwrap()]);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupt_withdraws_the_question_and_yolo_lets_a_waiting_one_go() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (first, second) = (d.join("first.txt"), d.join("second.txt"));
  let (base, _sent) = serve(vec![
    tool_use("t1", &format!("touch '{}'", first.display())),
    tool_use("t2", &format!("touch '{}'", second.display())),
    answer("Made it."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();

  // Interrupted while it asks: the question is withdrawn, not left open.
  let mut c = Client::connect(&data).await.unwrap();
  let (steer, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let mut held = Vec::new();
  let mut question = None;
  let done = c
    .chat_with(&chat(None, "Make a file"), Some(outgoing), |r| {
      if let Reply::Approve { question: q, .. } = r {
        question = Some(q.clone());
        let _ = steer.send(Request::Interrupt);
        let (keep, decided) = tokio::sync::oneshot::channel();
        held.push(keep);
        return Some(decided);
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  let question = question.unwrap();
  let answers = until("the withdrawal", || {
    let a = answers_to(&home, &question);
    (!a.is_empty()).then_some(a)
  })
  .await;
  assert_eq!(answers[0].fields["withdrawn"], true);
  assert_eq!(answers[0].fields["choice"], "decline");
  assert!(!first.exists());

  // Yolo turned on while one waits: it goes, and the client is told.
  let mut c = Client::connect(&data).await.unwrap();
  let mut held = Vec::new();
  let mut taken_back = 0;
  let done = c
    .chat_with(&chat(Some(&thread), "Again"), None, |r| match r {
      Reply::Approve { .. } => {
        let data = data.clone();
        tokio::spawn(async move {
          let mut c = Client::connect(&data).await.unwrap();
          c.request(&Request::Yolo { on: Some(true) }).await.unwrap();
        });
        let (keep, decided) = tokio::sync::oneshot::channel();
        held.push(keep);
        Some(decided)
      }
      Reply::Answered { .. } => {
        taken_back += 1;
        None
      }
      _ => None,
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(second.exists());
  assert_eq!(taken_back, 1);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_nothing_asks_any_more_is_withdrawn_at_start() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  // Left by an earlier run of this node: a chat's, which never resumes.
  let root = RecordRoot::open(&home).unwrap();
  let node_name: stitch_records::NodeName = "mbp".parse().unwrap();
  let thread = root
    .publish(
      &node_name,
      stitch_records::Draft::new("thread", stitch_records::Author::user(node_name.clone()))
        .unwrap()
        .body("# A chat\n"),
    )
    .unwrap();
  let q = root
    .publish(
      &node_name,
      stitch_records::Draft::new("question", stitch_records::Author::node(node_name.clone()))
        .unwrap()
        .field("approval", true)
        .unwrap()
        .field("command", "rm -rf /tmp/x")
        .unwrap()
        .field(
          "choices",
          toml::Value::Array(vec!["allow".into(), "decline".into()]),
        )
        .unwrap()
        .reference("thread", stitch_records::Ref::to(thread.id.clone()))
        .unwrap()
        .body("# Run `rm -rf /tmp/x`?\n"),
    )
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let answers = until("the withdrawal", || {
    let a = answers_to(&home, &q.id.to_string());
    (!a.is_empty()).then_some(a)
  })
  .await;
  assert_eq!(answers[0].fields["withdrawn"], true);
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wait_says_when_a_thread_it_waits_for_is_blocked_on_the_user() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, _sent) = serve_routed(
    r#"{"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}]}"#,
    "claude-haiku-5-5",
    tool_use("h1", &format!("touch '{}'", made.display())),
    vec![
      tool_call(
        "a1",
        "spawn",
        json!({"model": "anthropic/claude-haiku-5-5", "goal": "Make the file"}),
      ),
      tool_call("w1", "wait", json!({})),
      answer("It waits on your approval."),
    ],
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut results = Vec::new();
  let done = c
    .chat(&chat(None, "Have Haiku make it"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(
    results[1].contains("blocked: waiting for the user's approval to run `touch"),
    "{results:?}"
  );
  assert!(!made.exists());
  // On the attention list, with the node that asked.
  assert_eq!(
    home_records(&home, "question")
      .iter()
      .filter(|q| q.node == "mbp")
      .count(),
    1
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn always_allows_what_it_grants_in_that_place_from_then_on() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (a, b) = (d.join("a.txt"), d.join("b.txt"));
  let (base, _sent) = serve(vec![
    tool_use("t1", &format!("touch '{}'", a.display())),
    tool_use(
      "t2",
      &format!("touch -c '{}' && touch '{}'", a.display(), b.display()),
    ),
    answer("Both made."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let mut asked = Vec::new();
  let done = c
    .chat(&chat(None, "Make two files"), |r| {
      if let Reply::Approve { command, grant, .. } = r {
        asked.push((command.clone(), grant.clone()));
        let (answer, decided) = tokio::sync::oneshot::channel();
        answer.send(stitch_node::tools::Decision::always()).unwrap();
        return Some(decided);
      }
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(a.exists() && b.exists());
  // Asked once: the second `touch` was granted by the first answer.
  assert_eq!(asked.len(), 1, "{asked:?}");
  assert_eq!(asked[0].1, vec!["touch".to_owned()]);
  let answers = home_records(&home, "answer");
  assert_eq!(answers[0].fields["choice"], "always");
  // Saved for the place, on this node.
  let config = std::fs::read_to_string(data.join("config.toml")).unwrap();
  assert!(
    config.contains("[shell.places.") && config.contains("\"touch\""),
    "{config}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_thread_is_refused_never_asked_and_so_are_its_spawns() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let made = d.join("made.txt");
  let (base, sent) = serve_routed(
    r#"{"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}]}"#,
    "claude-haiku-5-5",
    // The spawned one tries to write, then gives up.
    tool_use("h1", &format!("touch '{}'", made.display())),
    vec![],
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::Detached(thread) = c
    .request(&Request::Detach {
      goal: "Make the file".into(),
      model: Some("anthropic/claude-haiku-5-5".into()),
      repo: None,
      cwd: None,
      parent: None,
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: Some(2),
      global: false,
      node: None,
      context: None,
      no_tools: false,
      tools: vec![],
      skills: vec![],
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
  let reported = |t: &str| {
    let root = RecordRoot::open(&home).unwrap();
    home_records(&home, "report").into_iter().any(|r| {
      root
        .read_record(&r.id)
        .unwrap()
        .refs
        .first("thread")
        .is_some_and(|x| x.id.to_string() == t)
    })
  };
  until("its report", || reported(&thread).then_some(())).await;
  assert!(!made.exists());
  assert!(home_records(&home, "question").is_empty(), "never asked");
  let threads = home_records(&home, "thread");
  let t = threads.iter().find(|t| t.id.to_string() == thread).unwrap();
  assert_eq!(t.fields["approval"], "read-only");
  // The model was told it was refused, and why.
  let refused = sent
    .lock()
    .unwrap()
    .0
    .iter()
    .any(|b| b.to_string().contains("this thread is read-only"));
  assert!(refused);
  // What it spawns is read-only too, whatever is asked.
  let Reply::Detached(child) = c
    .request(&Request::Detach {
      goal: "Make it anyway".into(),
      model: Some("anthropic/claude-haiku-5-5".into()),
      repo: None,
      cwd: None,
      parent: Some(thread.clone()),
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: Some(2),
      global: false,
      node: None,
      context: None,
      no_tools: false,
      tools: vec![],
      skills: vec![],
      keep: false,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the child's report", || reported(&child).then_some(())).await;
  let threads = home_records(&home, "thread");
  let t = threads.iter().find(|t| t.id.to_string() == child).unwrap();
  assert_eq!(t.fields["approval"], "read-only");
  assert!(!made.exists());
  assert!(home_records(&home, "question").is_empty(), "never asked");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The user switches a thread between read-only and writable: each turn
/// runs as the last switch says, the model is told, and what it spawns
/// after a switch to writable is not forced read-only.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_switches_a_thread_between_read_only_and_writable() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (refused, made) = (d.join("refused.txt"), d.join("made.txt"));
  let (base, sent) = serve(vec![
    answer("Hello."),
    tool_use("t1", &format!("touch '{}'", refused.display())),
    answer("It was refused."),
    tool_use("t2", &format!("touch '{}'", made.display())),
    answer("Made."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "Hi"), |_| None).await.unwrap() else {
    panic!()
  };
  let mode = |on: Option<bool>| Request::ReadOnly {
    thread: thread.clone(),
    on,
  };
  let lines = |r: Reply| match r {
    Reply::Lines(l) => l.join(" "),
    other => panic!("{other:?}"),
  };
  assert_eq!(lines(c.request(&mode(None)).await.unwrap()), "writable");

  // Read-only: refused, never asked, and told.
  assert_eq!(
    lines(c.request(&mode(Some(true))).await.unwrap()),
    "read-only"
  );
  let done = c
    .chat(&chat(Some(&thread), "Make it"), |r| {
      assert!(!matches!(r, Reply::Approve { .. }), "never asked");
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(!refused.exists());
  let said = |i: usize| sent.lock().unwrap()[i].to_string();
  assert!(said(1).contains("This thread is now read-only (switched by the user)"));
  assert!(said(2).contains("this thread is read-only"));
  let sets = home_records(&home, "set");
  let set = sets
    .iter()
    .find(|s| s.fields["field"] == "approval")
    .unwrap();
  assert!(set.by_user && set.fields["value"] == "read-only");

  // Writable again: the command asks, as any thread's does.
  assert_eq!(
    lines(c.request(&mode(Some(false))).await.unwrap()),
    "writable"
  );
  let mut asked = 0;
  let done = c
    .chat(&chat(Some(&thread), "Make the other"), |r| {
      if let Reply::Approve { .. } = r {
        asked += 1;
        let (answer, decided) = tokio::sync::oneshot::channel();
        answer.send(stitch_node::tools::Decision::allow()).unwrap();
        return Some(decided);
      }
      None
    })
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert_eq!(asked, 1);
  assert!(made.exists());
  assert!(said(3).contains("This thread is now writable (switched by the user)"));
  // The rules it is told are the mode's.
  let rules = |i: usize| {
    sent.lock().unwrap()[i]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .find(|t| t["name"] == "shell_run")
      .unwrap()["description"]
      .to_string()
  };
  assert!(rules(1).contains("This thread is read-only"));
  assert!(!rules(3).contains("This thread is read-only"));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A thread created read-only, switched to writable by the user: what it
/// spawns from then on is not forced read-only.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_switched_to_writable_spawns_writable_threads() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve_routed(
    r#"{"data": [{"id": "claude-haiku-5-5"}]}"#,
    "claude-haiku-5-5",
    answer("Nothing to do."),
    vec![],
  )
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let detach = |parent: Option<String>, read_only: bool| Request::Detach {
    goal: "Look around".into(),
    model: Some("anthropic/claude-haiku-5-5".into()),
    repo: None,
    cwd: None,
    parent,
    budget_usd: None,
    budget_tokens: None,
    budget_minutes: None,
    max_steps: Some(2),
    global: false,
    node: None,
    context: None,
    no_tools: false,
    tools: vec![],
    skills: vec![],
    keep: true,
    memory: false,
    read_only,
    scopes: Vec::new(),
  };
  let Reply::Detached(thread) = c.request(&detach(None, true)).await.unwrap() else {
    panic!()
  };
  let approval = |t: &str| {
    home_records(&home, "thread")
      .into_iter()
      .find(|r| r.id.to_string() == t)
      .map(|r| r.fields.get("approval").cloned())
  };
  until("the thread", || approval(&thread)).await;
  let Reply::Detached(before) = c
    .request(&detach(Some(thread.clone()), false))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the first child", || approval(&before)).await;
  assert_eq!(approval(&before).unwrap().unwrap(), "read-only");
  c.request(&Request::ReadOnly {
    thread: thread.clone(),
    on: Some(false),
  })
  .await
  .unwrap();
  let Reply::Detached(after) = c
    .request(&detach(Some(thread.clone()), false))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the second child", || approval(&after)).await;
  assert_eq!(approval(&after).unwrap(), None, "not forced read-only");
  // The first child keeps its own mode.
  let Reply::Lines(l) = c
    .request(&Request::ReadOnly {
      thread: before.clone(),
      on: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(l, vec!["read-only".to_owned()]);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The texts and block types of a request's messages, one line each.
fn shown(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|m| {
      let blocks: Vec<String> = match &m["content"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
          .iter()
          .map(|b| match b["type"].as_str() {
            Some("text") => b["text"].as_str().unwrap_or("").to_owned(),
            Some(t) => format!("<{t}>"),
            None => String::new(),
          })
          .collect(),
        _ => vec![],
      };
      format!(
        "{}: {}",
        m["role"].as_str().unwrap_or(""),
        blocks.join(" | ")
      )
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn forks_take_what_they_are_asked_and_summaries_stand_for_the_rest() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    answer("Hi."),
    tool_use("t1", "echo listed"),
    answer("Listed."),
    // The fork at m2.
    answer("Fork one."),
    // The fork without tools.
    answer("Fork two."),
    // The summary, then the compacted fork.
    answer("SUMMARY-TEXT"),
    answer("Fork three."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  // Quick is Sonnet, the built-in pair's: a stated window spares the mock
  // a lookup of it, which would take a scripted answer.
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap() else {
    panic!()
  };
  c.chat(&chat(Some(&thread), "list"), approve_all)
    .await
    .unwrap();
  let fork = |upto: Option<&str>, last, no_tools, summarize| Request::Fork {
    thread: thread.clone(),
    upto: upto.map(str::to_owned),
    last,
    no_attachments: false,
    no_tools,
    summarize,
  };

  // Up to a message: what came after it is not there.
  let Reply::Forked(f1) = c
    .request(&fork(Some("m2"), None, false, false))
    .await
    .unwrap()
  else {
    panic!()
  };
  c.chat(&chat(Some(&f1), "and then?"), |_| None)
    .await
    .unwrap();
  let body = sent.lock().unwrap()[3].clone();
  let lines = shown(&body);
  assert_eq!(lines[..2], ["user: hello", "assistant: Hi."], "{lines:?}");
  // A fork is a thread of its own: its first turn is told its own ID (the
  // prompt it took names the original's), with its instructions.
  assert_eq!(lines.len(), 3, "{lines:?}");
  assert!(
    lines[2].starts_with("user: [Stitch: the instructions")
      && lines[2].contains(&format!("This thread is {f1}: that ID is you"))
      && lines[2].ends_with(" | and then?"),
    "{lines:?}"
  );

  // Without tool calls: what was said only.
  let Reply::Forked(f2) = c.request(&fork(None, None, true, false)).await.unwrap() else {
    panic!()
  };
  c.chat(&chat(Some(&f2), "more"), |_| None).await.unwrap();
  let body = sent.lock().unwrap()[4].clone();
  let lines = shown(&body);
  assert!(
    lines
      .iter()
      .all(|l| !l.contains("<tool_use>") && !l.contains("<tool_result>")),
    "{lines:?}"
  );
  assert!(lines.iter().any(|l| l.contains("Listed.")), "{lines:?}");

  // Summarized, its last 3 messages kept as they were.
  let Reply::Forked(f3) = c.request(&fork(None, Some(3), false, true)).await.unwrap() else {
    panic!()
  };
  c.chat(&chat(Some(&f3), "go on"), |_| None).await.unwrap();
  let (summary_req, body) = {
    let s = sent.lock().unwrap();
    (s[5].clone(), s[6].clone())
  };
  // The summary saw what is not kept, not the kept messages.
  let asked = shown(&summary_req);
  assert!(asked.iter().any(|l| l.contains("hello")), "{asked:?}");
  assert!(!asked.iter().any(|l| l.contains("Listed.")), "{asked:?}");
  let lines = shown(&body);
  assert!(lines[0].contains("SUMMARY-TEXT"), "{lines:?}");
  assert!(!lines.iter().any(|l| l.contains("hello")), "{lines:?}");
  assert!(lines.iter().any(|l| l.contains("list")), "{lines:?}");
  assert!(lines.last().unwrap().contains("go on"));
  // Titled as edge talk did.
  let titles: Vec<String> = home_records(&home, "thread")
    .into_iter()
    .filter_map(|t| t.title)
    .collect();
  assert!(titles.iter().any(|t| t.starts_with("⑂ ")), "{titles:?}");
  assert!(titles.iter().any(|t| t.starts_with("✂ ")), "{titles:?}");
  // A message not there fails.
  assert!(
    c.request(&fork(Some("m99"), None, false, false))
      .await
      .is_err()
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// `Request::Transcript` shows what a fork inherits as the fork takes it
/// (its last messages; a summary first), marked, and names its source.
#[tokio::test(flavor = "multi_thread")]
async fn a_forks_transcript_shows_what_it_inherits() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("Hi."),
    answer("Second."),
    answer("SUMMARY-TEXT"),
    answer("After."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap() else {
    panic!()
  };
  c.chat(&chat(Some(&thread), "again"), |_| None)
    .await
    .unwrap();
  let fork = |last, summarize| Request::Fork {
    thread: thread.clone(),
    upto: None,
    last,
    no_attachments: false,
    no_tools: false,
    summarize,
  };
  async fn transcript(c: &mut Client, thread: &str) -> serde_json::Value {
    let Reply::Lines(l) = c
      .request(&Request::Transcript {
        thread: thread.to_owned(),
        after: None,
        before: None,
        tail: None,
      })
      .await
      .unwrap()
    else {
      panic!()
    };
    serde_json::from_str(&l[0]).unwrap()
  }
  let texts = |t: &serde_json::Value| -> Vec<(String, bool)> {
    t["entries"]
      .as_array()
      .unwrap()
      .iter()
      .filter(|e| e["kind"] == "user" || e["kind"] == "assistant")
      .map(|e| {
        (
          e["text"].as_str().unwrap_or_default().to_owned(),
          e["inherited"].as_bool().unwrap_or(false),
        )
      })
      .collect()
  };

  // Its last 2 messages: the source's whole history shown (design §2.15,
  // "A fork shows what it continues"), the last 2 marked as what its model
  // reads, then its divider; the source named.
  let Reply::Forked(f1) = c.request(&fork(Some(2), false)).await.unwrap() else {
    panic!()
  };
  let t = transcript(&mut c, &f1).await;
  assert_eq!(
    texts(&t),
    vec![
      ("hello".to_owned(), false),
      ("Hi.".to_owned(), false),
      ("again".to_owned(), true),
      ("Second.".to_owned(), true)
    ]
  );
  let kinds: Vec<&str> = t["entries"]
    .as_array()
    .unwrap()
    .iter()
    .map(|e| e["kind"].as_str().unwrap())
    .collect();
  assert_eq!(
    kinds,
    [
      "user",
      "assistant",
      "context",
      "user",
      "assistant",
      "forked"
    ]
  );
  let divider = t["entries"].as_array().unwrap().last().unwrap().clone();
  assert_eq!(
    (divider["how"].clone(), divider["last"].clone()),
    (json!("last"), json!(2))
  );
  assert_eq!(t["forked_from"]["thread"], thread.as_str());
  // The source's own transcript is whole and has none inherited.
  let t = transcript(&mut c, &thread).await;
  assert_eq!(texts(&t).len(), 4);
  assert!(texts(&t).iter().all(|(_, i)| !i));
  assert!(t["forked_from"].is_null());

  // Summarized, with its last 2 messages: the summary first, then it, then
  // what the fork says itself. (Without attachments, so a fork: the same
  // at the latest message, dropping nothing, is a compaction, design
  // §2.15.)
  let mut f2 = fork(Some(2), true);
  if let Request::Fork { no_attachments, .. } = &mut f2 {
    *no_attachments = true;
  }
  let Reply::Forked(f2) = c.request(&f2).await.unwrap() else {
    panic!()
  };
  c.chat(&chat(Some(&f2), "go on"), |_| None).await.unwrap();
  let t = transcript(&mut c, &f2).await;
  let got = texts(&t);
  // The summary is the divider's; what it keeps is marked; the fork's own
  // message is its own.
  let divider = t["entries"]
    .as_array()
    .unwrap()
    .iter()
    .find(|e| e["kind"] == "forked")
    .unwrap()
    .clone();
  assert_eq!(divider["how"], "summary+last");
  assert!(
    divider["summary"]
      .as_str()
      .unwrap()
      .contains("SUMMARY-TEXT")
  );
  assert!(got.contains(&("again".to_owned(), true)), "{got:?}");
  assert!(got.contains(&("Second.".to_owned(), true)), "{got:?}");
  assert!(got.iter().any(|(t, i)| t == "go on" && !i), "{got:?}");
  assert_eq!(
    got.iter().filter(|(t, _)| t == "again").count(),
    1,
    "{got:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Titles and summaries default to quick, but explicit auxiliary model
/// settings take priority (including role aliases).
#[tokio::test(flavor = "multi_thread")]
async fn auxiliary_models_prefer_quick_and_respect_overrides() {
  for explicit in [false, true] {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let (base, sent) = serve(vec![
      answer("Hi."),
      answer("A short title"),
      answer("SUMMARY"),
    ])
    .await;
    let (data, home) = (d.join("data"), d.join("home"));
    setup(&data, &home, &base);
    let mut config = Config::load(&data).unwrap();
    let settings = config.threads.as_mut().unwrap();
    settings.titles = Some(true);
    if explicit {
      settings.title_model = Some("default".into());
      settings.summary_model = Some("anthropic/claude-sonnet-5-5".into());
    }
    config
      .providers
      .get_mut("anthropic")
      .unwrap()
      .context_window = Some(1_000_000);
    config.save(&data).unwrap();
    let node = stitch_node::start(&data).await.unwrap();
    let mut c = Client::connect(&data).await.unwrap();
    for (role, model) in [
      ("default", "anthropic/claude-opus-5"),
      ("quick", "anthropic/claude-haiku-5-5"),
    ] {
      c.request(&Request::ModelSettings {
        input: json!({"action": role, "model": model}).to_string(),
      })
      .await
      .unwrap();
    }
    let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap()
    else {
      panic!()
    };
    asked(&sent, 2).await; // The background title gets its ordered response first.
    c.request(&Request::Fork {
      thread,
      upto: None,
      last: None,
      no_attachments: false,
      no_tools: false,
      summarize: true,
    })
    .await
    .unwrap();
    let bodies = sent.lock().unwrap().clone();
    assert_eq!(bodies[0]["model"], "claude-opus-5");
    assert_eq!(
      bodies[1]["model"],
      if explicit {
        "claude-opus-5"
      } else {
        "claude-haiku-5-5"
      }
    );
    assert_eq!(
      bodies[2]["model"],
      if explicit {
        "claude-sonnet-5-5"
      } else {
        "claude-haiku-5-5"
      }
    );
    // A tenth of the summary model's window, at most 64k: a long chain's
    // summary is not cut off (and thrown away) at 8192.
    assert_eq!(bodies[2]["max_tokens"], 64_000);
    assert!(
      shown(&bodies[2]).iter().any(|l| l.contains("hello")),
      "full source retained"
    );
    c.request(&Request::Stop).await.unwrap();
    node.wait().await.unwrap();
  }
}

/// Never publish a truncated summary as a continuity replacement. Only
/// the automatic quick choice can retry with the thread's stronger model.
#[tokio::test(flavor = "multi_thread")]
async fn incomplete_quick_summaries_fall_back_but_explicit_overrides_do_not() {
  for explicit in [false, true] {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let mut responses = vec![
      answer("Hi."),
      answer("HALF").replace("end_turn", "max_tokens"),
    ];
    if !explicit {
      responses.push(answer("COMPLETE"));
    }
    let (base, sent) = serve(responses).await;
    let (data, home) = (d.join("data"), d.join("home"));
    setup(&data, &home, &base);
    let mut config = Config::load(&data).unwrap();
    config
      .providers
      .get_mut("anthropic")
      .unwrap()
      .context_window = Some(1_000_000);
    if explicit {
      config.threads.as_mut().unwrap().summary_model = Some("quick".into());
    }
    config.save(&data).unwrap();
    let node = stitch_node::start(&data).await.unwrap();
    let mut c = Client::connect(&data).await.unwrap();
    c.request(&Request::ModelSettings {
      input: json!({"action": "quick", "model": "anthropic/claude-haiku-5-5"}).to_string(),
    })
    .await
    .unwrap();
    let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap()
    else {
      panic!()
    };
    let result = c
      .request(&Request::Fork {
        thread,
        upto: None,
        last: None,
        no_attachments: false,
        no_tools: false,
        summarize: true,
      })
      .await;
    if explicit {
      assert!(result.unwrap_err().to_string().contains("incomplete"));
      assert_eq!(
        home_records(&home, "thread").len(),
        1,
        "no lossy fork published"
      );
    } else {
      assert!(matches!(result.unwrap(), Reply::Forked(_)));
      assert_eq!(sent.lock().unwrap()[2]["model"], "claude-opus-5-5");
    }
    assert_eq!(sent.lock().unwrap()[1]["model"], "claude-haiku-5-5");
    c.request(&Request::Stop).await.unwrap();
    node.wait().await.unwrap();
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn summaries_keep_the_full_source_when_quick_has_too_little_context() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![answer("Hi."), answer("FULL-SUMMARY")]).await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = Config::load(&data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.providers.insert(
    "small".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(base),
      context_window: Some(8192),
      ..ProviderConfig::default()
    },
  );
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  c.request(&Request::ModelSettings {
    input: json!({"action": "quick", "model": "small/tiny"}).to_string(),
  })
  .await
  .unwrap();
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "SOURCE-MUST-SURVIVE"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  c.request(&Request::Fork {
    thread,
    upto: None,
    last: None,
    no_attachments: false,
    no_tools: false,
    summarize: true,
  })
  .await
  .unwrap();
  let summary = sent.lock().unwrap()[1].clone();
  assert_eq!(
    summary["model"], "claude-opus-5-5",
    "skip the undersized quick model"
  );
  assert!(
    shown(&summary)
      .iter()
      .any(|l| l.contains("SOURCE-MUST-SURVIVE"))
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A node whose Anthropic models take `window` tokens, compacting at 80%
/// and keeping 2 messages.
fn small_window(data: &Path, window: u64) {
  let mut config = stitch_node::config::Config::load(data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(window);
  config.threads = Some(stitch_node::config::ThreadSettings {
    compact_at: Some(80),
    compact_keep: Some(2),
    titles: Some(false),
    ..Default::default()
  });
  config.save(data).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_near_its_window_goes_on_in_a_new_one_from_a_summary() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    answer("Hi."),
    // Its last step sent 80 tokens of a 90-token window: compacted first.
    answer("SUMMARY-ONE"),
    answer("Second."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  small_window(&data, 90);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread: first, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap()
  else {
    panic!()
  };
  // Read before it was compacted: the cursor sits on the first thread.
  c.request(&Request::Read {
    thread: first.clone(),
    upto: None,
  })
  .await
  .unwrap();
  let mut notes = Vec::new();
  let mut told = Vec::new();
  let mut compactions = Vec::new();
  let Reply::ChatDone { thread: second, .. } = c
    .chat(&chat(Some(&first), "again"), |r| {
      match r {
        Reply::Note(n) => notes.push(n.clone()),
        Reply::Thread(t) => told.push(t.clone()),
        Reply::Compacted(c) => compactions.push(c.clone()),
        _ => {}
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(first, second, "it went on in a new thread");
  assert_eq!(told, vec![second.clone()], "the client is told where");
  // And of the compaction, for its divider (design §2.15).
  assert_eq!(compactions.len(), 1, "{compactions:?}");
  assert_eq!(
    (compactions[0].from.as_str(), compactions[0].to.as_str()),
    (first.as_str(), second.as_str())
  );
  assert_eq!(compactions[0].window, 90);
  assert!(compactions[0].summary.contains("SUMMARY-ONE"));
  // The list shows the conversation once, as its newest.
  let Reply::Lines(l) = c
    .request(&Request::Threads {
      forest: None,
      status: None,
      since: None,
      limit: None,
      thread: None,
      place: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let listed: serde_json::Value = serde_json::from_str(&l[0]).unwrap();
  let ids: Vec<&str> = listed["threads"]
    .as_array()
    .unwrap()
    .iter()
    .map(|t| t["id"].as_str().unwrap())
    .collect();
  assert_eq!(ids, [second.as_str()], "{listed}");
  // The conversation's cursor is its chain's: what the first part had read
  // is behind the new thread's steps, so it is unread; reading it by the
  // old ID writes on the newest, and the chain is read.
  assert_eq!(listed["threads"][0]["unread"], true, "{listed}");
  c.request(&Request::Read {
    thread: first.clone(),
    upto: None,
  })
  .await
  .unwrap();
  let Reply::Lines(l) = c
    .request(&Request::Threads {
      forest: None,
      status: None,
      since: None,
      limit: None,
      thread: None,
      place: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let again: serde_json::Value = serde_json::from_str(&l[0]).unwrap();
  assert_eq!(again["threads"][0]["unread"], false, "{again}");
  assert!(
    notes
      .iter()
      .any(|n| n.contains("continues in a new thread")),
    "{notes:?}"
  );
  // Shorter than what is kept: summarized whole.
  let lines = shown(&sent.lock().unwrap()[2]);
  assert!(lines[0].contains("SUMMARY-ONE"), "{lines:?}");
  assert_eq!(lines.len(), 2, "{lines:?}");
  let record = home_records(&home, "thread")
    .into_iter()
    .find(|t| t.id.to_string() == second)
    .unwrap();
  assert_eq!(record.fields["compacted"], true);
  assert!(record.title.unwrap().starts_with("↻ "));
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_too_long_for_the_model_is_compacted_and_tried_again() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let too_long = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 250000 tokens > 200000 maximum\"}}".to_owned();
  let (base, sent) = serve(vec![
    answer("Hi."),
    answer("Fine."),
    too_long,
    answer("SUMMARY-TWO"),
    answer("Third."),
    answer("Fourth."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  small_window(&data, 1_000_000);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread: first, .. } = c.chat(&chat(None, "hello"), |_| None).await.unwrap()
  else {
    panic!()
  };
  c.chat(&chat(Some(&first), "fine?"), |_| None)
    .await
    .unwrap();
  let Reply::ChatDone { thread: next, .. } = c
    .chat(&chat(Some(&first), "and again"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_ne!(next, first);
  let lines = shown(&sent.lock().unwrap()[4]);
  assert!(lines[0].contains("SUMMARY-TWO"), "{lines:?}");
  // The last two messages kept as they were, then the message.
  assert!(lines.iter().any(|l| l.contains("fine?")), "{lines:?}");
  assert!(!lines.iter().any(|l| l.contains("hello")), "{lines:?}");
  assert!(lines.last().unwrap().contains("and again"));
  // Sent to the old thread, it goes on in the new one.
  let Reply::ChatDone { thread: again, .. } = c
    .chat(&chat(Some(&first), "still there?"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(again, next);
  let compacted = home_records(&home, "thread")
    .into_iter()
    .filter(|t| t.fields.get("compacted").is_some())
    .count();
  assert_eq!(compacted, 1);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_finds_messages_across_threads_by_their_words() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("The terrarium misting runs every fifteen minutes."),
    answer("Sourdough wants a warm kitchen."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone {
    thread: misting, ..
  } = c
    .chat(&chat(None, "How often does the terrarium mist?"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  c.chat(&chat(None, "Bread tips?"), |_| None).await.unwrap();
  let search = |q: &str| Request::Search {
    query: q.into(),
    here: false,
    cwd: None,
    limit: 10,
  };
  let Reply::Hits(hits) = c.request(&search("terrarium misting")).await.unwrap() else {
    panic!()
  };
  assert!(!hits.is_empty());
  assert!(hits.iter().all(|h| h.thread == misting), "{hits:?}");
  assert!(
    hits
      .iter()
      .any(|h| h.msg == "m2" && h.snippet.contains("fifteen")),
    "{hits:?}"
  );
  assert_eq!(hits[0].found_by, "words");
  let Reply::Hits(none) = c.request(&search("quantum chromodynamics")).await.unwrap() else {
    panic!()
  };
  assert!(none.is_empty());
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn new_chats_get_titles_and_threads_are_renamed_pinned_and_quoted() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("Hi there."),
    answer("\"Greeting the node.\""),
    answer("Second."),
    answer("A second title"),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let mut config = stitch_node::config::Config::load(&data).unwrap();
  config.threads = None;
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread: first, .. } =
    c.chat(&chat(None, "hello node"), |_| None).await.unwrap()
  else {
    panic!()
  };
  let title_of = |t: &str| {
    let root = RecordRoot::open(&home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    index.title(&t.parse().unwrap()).unwrap()
  };
  until("its title", || {
    (title_of(&first).as_deref() == Some("Greeting the node")).then_some(())
  })
  .await;
  c.request(&Request::Rename {
    thread: first.clone(),
    title: "My own name".into(),
  })
  .await
  .unwrap();
  assert_eq!(title_of(&first).as_deref(), Some("My own name"));
  let Reply::ChatDone { thread: second, .. } =
    c.chat(&chat(None, "another"), |_| None).await.unwrap()
  else {
    panic!()
  };
  until("the second title", || {
    (title_of(&second).as_deref() == Some("A second title")).then_some(())
  })
  .await;
  // The latest first, unless one is pinned.
  let order = || {
    let root = RecordRoot::open(&home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    index
      .threads()
      .unwrap()
      .into_iter()
      .map(|(r, _)| r.id.to_string())
      .collect::<Vec<_>>()
  };
  assert_eq!(order(), vec![second.clone(), first.clone()]);
  c.request(&Request::Pin {
    thread: first.clone(),
    on: true,
  })
  .await
  .unwrap();
  assert_eq!(order(), vec![first.clone(), second.clone()]);
  c.request(&Request::Pin {
    thread: first.clone(),
    on: false,
  })
  .await
  .unwrap();
  assert_eq!(order(), vec![second.clone(), first.clone()]);
  // Quoted: a file framed as material from another conversation.
  let Reply::Lines(lines) = c
    .request(&Request::Quote {
      thread: first.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let quoted = std::fs::read_to_string(&lines[0]).unwrap();
  assert!(quoted.contains("quoted reference material"), "{quoted}");
  assert!(
    quoted.contains("hello node") && quoted.contains("Hi there."),
    "{quoted}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_is_one_segment_whatever_its_steps() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    tool_use("t1", "echo one"),
    tool_use("t2", "echo two"),
    answer("Two steps, one turn."),
    answer("And another turn."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let Reply::ChatDone { thread, .. } = c.chat(&chat(None, "go"), |_| None).await.unwrap() else {
    panic!()
  };
  let files = || {
    std::fs::read_dir(home.join("threads").join(&thread))
      .unwrap()
      .count()
  };
  assert_eq!(files(), 1, "three steps, one file");
  assert!(!home.join("journal").join(&thread).exists(), "sealed");
  c.chat(&chat(Some(&thread), "more"), |_| None)
    .await
    .unwrap();
  assert_eq!(files(), 2);
  // Read back whole: both turns' messages, in order.
  let Reply::Lines(_) = c
    .request(&Request::Quote {
      thread: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn your_instructions_are_told_before_the_checkouts() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![answer("Plainly.")]).await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let app = d.join("app");
  std::fs::create_dir_all(&app).unwrap();
  std::fs::write(app.join("AGENTS.md"), "THE REPO'S RULE\n").unwrap();
  for args in [
    &["init", "-q"][..],
    &["add", "."],
    &[
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@t",
      "commit",
      "-qm",
      "first",
    ],
  ] {
    assert!(
      std::process::Command::new("git")
        .arg("-C")
        .arg(&app)
        .args(args)
        .status()
        .unwrap()
        .success()
    );
  }
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let cwd = Some(app.display().to_string());
  let set = |global: bool, text: &str| Request::Instructions {
    place: None,
    cwd: cwd.clone(),
    global,
    set: Some(text.into()),
  };
  let Reply::Instructions { place, text } = c.request(&set(true, "SPEAK PLAINLY\n")).await.unwrap()
  else {
    panic!()
  };
  assert_eq!(
    (place.as_str(), text.as_deref()),
    ("global", Some("SPEAK PLAINLY\n"))
  );
  // A checkout seen for the first time: its project, named after it.
  c.request(&set(false, "first try\n")).await.unwrap();
  let Reply::Instructions { place, text } = c
    .request(&set(false, "ASK BEFORE PUSHING\n"))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(
    (place.as_str(), text.as_deref()),
    ("app", Some("ASK BEFORE PUSHING\n"))
  );

  let mut ask = chat(None, "Hi");
  if let Request::Chat { cwd: c, .. } = &mut ask {
    *c = cwd.clone();
  }
  c.chat(&ask, |_| None).await.unwrap();
  let system = sent.lock().unwrap()[0]["system"].to_string();
  let at = |s: &str| {
    system
      .find(s)
      .unwrap_or_else(|| panic!("no {s} in {system}"))
  };
  // The latest edit, the most general first, the repository's last.
  assert!(!system.contains("first try"));
  assert!(at("SPEAK PLAINLY") < at("ASK BEFORE PUSHING"));
  assert!(at("ASK BEFORE PUSHING") < at("THE REPO'S RULE"));

  // Emptied: none.
  let Reply::Instructions { text, .. } = c.request(&set(false, "")).await.unwrap() else {
    panic!()
  };
  assert_eq!(text, None);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A detached thread of this node, at work: a step written, then a call
/// that hangs (`serve` given a tool call, then `HANG`).
async fn detached_at_work(c: &mut Client, sent: &Arc<Mutex<Vec<Value>>>) -> String {
  let Reply::Detached(thread) = c
    .request(&Request::Detach {
      goal: "Say hi".into(),
      model: None,
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
      tools: vec![],
      skills: vec![],
      keep: false,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  asked(sent, 2).await;
  thread
}

/// A phone forks a thread another node started, whose system prompt (a
/// blob) it never fetched: the fork names it, as the original does,
/// rather than failing on it (2026-10-17: "Fork & Resume" on the phone
/// failed so, and wrote nothing).
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_names_a_system_prompt_this_node_lacks() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_call("t1", "shell_view", json!({"session": "none"})),
    HANG.into(),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let thread = detached_at_work(&mut c, &sent).await;
  let root = RecordRoot::open(&home).unwrap();
  let original = root.read_record(&thread.parse().unwrap()).unwrap();
  let hash = original
    .field_str("system_prompt")
    .expect("a system prompt")
    .to_owned();
  // As on a node that synced the thread's records, not its blobs.
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  std::fs::remove_file(blobs.path(&hash.parse().unwrap())).unwrap();

  let forked = c
    .request(&Request::Fork {
      thread: thread.clone(),
      upto: None,
      last: None,
      no_attachments: false,
      no_tools: false,
      summarize: false,
    })
    .await;
  let Ok(Reply::Forked(fork)) = forked else {
    panic!("{forked:?}")
  };
  let fork = root.read_record(&fork.parse().unwrap()).unwrap();
  assert_eq!(fork.field_str("system_prompt"), Some(hash.as_str()));
  assert!(
    fork.attachments.iter().any(|a| a.hash.to_string() == hash),
    "{:?}",
    fork.attachments
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A message typed into a thread whose turn another node's loop has is
/// refused, as a Resume is, saying which node and what to do instead:
/// two loops on one thread fork it (2026-10-17, `…isev`: the phone ran a
/// chat beside gamer's loop). Sent to that turn (`interject`), it goes
/// there.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_a_turn_another_node_has_is_refused() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![answer("Should not run.")]).await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  // gamer's detached thread, at work there: a step written, no report.
  let root = RecordRoot::open(&home).unwrap();
  let gamer: stitch_records::NodeName = "gamer".parse().unwrap();
  let thread = root
    .publish(
      &gamer,
      stitch_records::Draft::new("thread", stitch_records::Author::user(gamer.clone()))
        .unwrap()
        .body("# gamer's work\n")
        .field("detached", true)
        .unwrap()
        .field("goal", "Do the work")
        .unwrap(),
    )
    .unwrap()
    .id;
  let msg = |id: &str, role, text: &str| {
    stitch_records::Event::Msg(stitch_records::Msg {
      id: id.into(),
      at: jiff::Timestamp::now(),
      role,
      from: None,
      model: None,
      parts: vec![stitch_records::Part::Text { text: text.into() }],
      native: None,
    })
  };
  root
    .publish_segment(
      &gamer,
      &thread,
      None,
      None,
      vec![msg("m1", stitch_records::Role::User, "Do the work")],
    )
    .unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let refused = c
    .chat(&chat(Some(thread.as_str()), "What's going on?"), |_| None)
    .await;
  let said = format!("{refused:?}");
  assert!(refused.is_err(), "{said}");
  assert!(said.contains("gamer"), "names the node: {said}");
  assert!(said.contains("interject"), "offers to interject: {said}");
  assert!(said.contains("fork"), "offers to fork: {said}");
  assert!(sent.lock().unwrap().is_empty(), "no loop of its own");
  assert_eq!(
    stitch_node::chat::thread_messages(
      &stitch_node::chat::ChatContext {
        node: &"mbp".parse().unwrap(),
        root: &root,
        index: &Mutex::new(Index::open(&data.join("index.sqlite")).unwrap()),
        blobs: &stitch_store::BlobStore::open(&data.join("blobs")).unwrap(),
      },
      &thread
    )
    .unwrap()
    .len(),
    1,
    "nothing written to it"
  );

  // Sent to that turn instead: passed on to gamer.
  let Reply::Lines(lines) = c
    .request(&Request::Send {
      thread: thread.to_string(),
      text: "What's going on?".into(),
      from: None,
      keep: false,
      interject: true,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("on gamer"), "{lines:?}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// An error status the provider answers with, and headers.
fn refused(code: u16, body: Value, headers: &str) -> String {
  let b = body.to_string();
  format!(
    "HTTP/1.1 {code} Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{headers}connection: close\r\n\r\n{b}",
    b.len()
  )
}

/// A Claude subscription's usage limit, resetting in three hours.
fn usage_limit() -> String {
  let reset = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_secs()
    + 3 * 3600;
  refused(
    429,
    json!({"type": "error", "error": {"type": "rate_limit_error", "message": "This request would exceed your account's rate limit. Please try again later."}}),
    &format!(
      "anthropic-ratelimit-unified-status: rejected\r\nanthropic-ratelimit-unified-reset: {reset}\r\nanthropic-ratelimit-unified-representative-claim: five_hour\r\nretry-after: 10800\r\n"
    ),
  )
}

fn low_credit() -> String {
  refused(
    400,
    json!({"type": "error", "error": {"type": "invalid_request_error", "message": "Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}),
    "",
  )
}

/// The thread's transcript, as `chat::transcript` reads it.
fn transcript_of(data: &Path, home: &Path, thread: &str) -> Vec<(String, String)> {
  let root = RecordRoot::open(home).unwrap();
  let index = Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
  index.lock().unwrap().refresh(&root).unwrap();
  let blobs = stitch_store::BlobStore::open(&data.join("blobs")).unwrap();
  let node: stitch_records::NodeName = "mbp".parse().unwrap();
  let ctx = stitch_node::chat::ChatContext {
    node: &node,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  stitch_node::chat::transcript(&ctx, &thread.parse().unwrap()).unwrap()
}

/// A chat that fails: its thread, whether the node kept its message
/// (`Reply::Kept`), and the error.
async fn failing(c: &mut Client, req: &Request) -> (Option<String>, bool, String) {
  let (mut thread, mut kept) = (None, false);
  let err = c
    .chat(req, |r| {
      match r {
        Reply::Thread(t) => thread = Some(t.clone()),
        Reply::Kept => kept = true,
        _ => {}
      }
      None
    })
    .await
    .unwrap_err();
  (thread, kept, err.to_string())
}

/// The incident of 2026-10-22: a Claude subscription out of usage on the
/// phone, the chat frozen (its 429 retried), the message lost, no error.
#[tokio::test(flavor = "multi_thread")]
async fn an_account_out_of_usage_keeps_the_message_and_says_so() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    refused(
      400,
      json!({"type": "error", "error": {"type": "invalid_request_error", "message": "image exceeds 5 MB maximum"}}),
      "",
    ),
    usage_limit(),
    low_credit(),
    answer("Back again."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // A request refused as it is, on a new chat: nothing written, the
  // message goes back to the client to be changed.
  let (thread, kept, err) = failing(&mut c, &chat(None, "Look at this")).await;
  assert!(!kept, "{err}");
  assert!(err.contains("image exceeds"), "{err}");
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  assert!(
    index
      .record(&thread.unwrap().parse().unwrap())
      .unwrap()
      .is_none(),
    "no thread written"
  );

  // Out of usage on a new chat's first call: asked once, not retried; the
  // thread and its message written, with why it stopped.
  let (thread, kept, err) = failing(&mut c, &chat(None, "Plan my day")).await;
  let thread = thread.unwrap();
  assert!(kept, "{err}");
  assert_eq!(sent.lock().unwrap().len(), 2, "not retried");
  assert!(
    err.starts_with("Anthropic (API key): usage limit reached (5-hour window); it resets "),
    "{err}"
  );
  assert!(
    err.contains("(in 3 h)") || err.contains("(in 2 h 59 min)"),
    "{err}"
  );
  assert!(
    err.contains("/model") && err.contains("does not switch by itself"),
    "{err}"
  );
  let shown = transcript_of(&data, &home, &thread);
  assert_eq!(shown[0], ("you".into(), "Plan my day".into()));
  assert_eq!(shown[1].0, "failed");
  assert!(shown[1].1.contains("usage limit reached"), "{shown:?}");
  assert_eq!(unfinished(&data, &home, &thread), Some(ours(CUT_OFF)));
  // The web client's transcript says it too (it showed nothing until
  // 2026-11-08: an expired sign-in ended the turn in silence there).
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: thread.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let t: serde_json::Value = serde_json::from_str(&l[0]).unwrap();
  let entries = t["entries"].as_array().unwrap();
  let failed = entries.last().unwrap();
  assert_eq!(failed["kind"], "failed", "{entries:?}");
  assert!(
    failed["text"]
      .as_str()
      .unwrap()
      .contains("usage limit reached"),
    "{failed}"
  );

  // Out of credit on the thread: the message kept too.
  let (_, kept, err) = failing(&mut c, &chat(Some(&thread), "Are you there?")).await;
  assert!(kept, "{err}");
  assert!(
    err.starts_with("Anthropic (API key): out of credit or quota (Your credit balance is too low"),
    "{err}"
  );
  let shown = transcript_of(&data, &home, &thread);
  assert_eq!(shown[2], ("you".into(), "Are you there?".into()));
  assert_eq!(shown[3].0, "failed");

  // The next message goes on from both.
  let done = c
    .chat(&chat(Some(&thread), "Go on"), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert_eq!(
    user_texts(&sent.lock().unwrap()[3]),
    vec!["Plan my day", "Are you there?", "Go on"]
  );
  assert_eq!(unfinished(&data, &home, &thread), None);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A message woven in after a step's tools, its sender told it went to
/// the model, survives the next call failing (a known issue until
/// 2026-10-22: the step's events were dropped with the error).
#[tokio::test(flavor = "multi_thread")]
async fn a_woven_message_survives_the_next_call_failing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, sent) = serve(vec![
    tool_use("t1", "echo working"),
    usage_limit(),
    answer("Resumed."),
  ])
  .await;
  let data = d.join("data");
  let home = d.join("home");
  setup(&data, &home, &base);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let (to_chat, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let (mut thread, mut woven, mut kept) = (None, Vec::new(), false);
  let err = c
    .chat_with(&chat(None, "Do the thing"), Some(outgoing), |r| {
      match r {
        Reply::Thread(t) => {
          thread = Some(t.clone());
          to_chat
            .send(Request::Interject {
              id: 1,
              text: "also check the logs".into(),
              attachments: vec![],
            })
            .unwrap();
        }
        Reply::Woven(ids) => woven.extend(ids.iter().copied()),
        Reply::Kept => kept = true,
        _ => {}
      }
      None
    })
    .await
    .unwrap_err();
  assert_eq!(woven, vec![1]);
  assert!(kept, "{err}");
  let thread = thread.unwrap();
  let shown = transcript_of(&data, &home, &thread);
  let whos: Vec<&str> = shown.iter().map(|(w, _)| w.as_str()).collect();
  assert_eq!(
    whos,
    ["you", "claude-opus-5-5", "tool", "output", "you", "failed"],
    "{shown:?}"
  );
  assert_eq!(shown[4].1, "also check the logs");
  assert_eq!(unfinished(&data, &home, &thread), Some(ours(CUT_OFF)));

  // Resumed: the model gets it, after the tool's result.
  let done = c
    .chat(
      &Request::Resume {
        thread: thread.clone(),
        model: None,
      },
      |_| None,
    )
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  {
    let bodies = sent.lock().unwrap();
    let msgs = bodies[2]["messages"].as_array().unwrap();
    let n = msgs.len();
    assert_eq!(msgs[n - 2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[n - 1]["content"][0]["text"], "also check the logs");
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// `answer`, its prompt `input_tokens` long (what it costs).
fn answer_in(text: &str, input_tokens: u64) -> String {
  answer(text).replace(
    "\"usage\":{\"input_tokens\":80}",
    &format!("\"usage\":{{\"input_tokens\":{input_tokens}}}"),
  )
}

/// What a call of `answer_in` costs (claude-opus-5-5: $4 in, $20 out a
/// million tokens, 5 written).
fn priced_in(input_tokens: u64) -> f64 {
  (input_tokens as f64 * 4.0 + 5.0 * 20.0) / 1e6
}

/// A node whose Anthropic models' window is known (none looked up of the
/// mock), its summaries asked of `summary_model` when given.
fn known_window(data: &Path, summary_model: Option<&str>) {
  let mut config = Config::load(data).unwrap();
  config
    .providers
    .get_mut("anthropic")
    .unwrap()
    .context_window = Some(1_000_000);
  config.threads.as_mut().unwrap().summary_model = summary_model.map(str::to_owned);
  config.save(data).unwrap();
}

/// What a chat's thread cost so far (`thread_usd`: its compaction chain's,
/// `detached::spent_by_chain`), and the thread it ran on.
async fn chat_cost(c: &mut Client, thread: Option<&str>, text: &str) -> (String, f64) {
  match c.chat(&chat(thread, text), |_| None).await.unwrap() {
    Reply::ChatDone {
      thread, thread_usd, ..
    } => (thread, thread_usd.expect("priced")),
    other => panic!("{other:?}"),
  }
}

/// A compaction's summary as a chat starts, past its share of the window
/// (issue 2026-11-19T193029Z-beast-ijcq follow-up): written as spend on
/// the thread compacted, so the chain's cost has it exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn a_chats_compaction_as_it_starts_is_spent_on_the_chain_once() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("Hi."),
    answer_in("SUMMARY-ONE", 250_000),
    answer("Second."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  small_window(&data, 90);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let (first, _) = chat_cost(&mut c, None, "hello").await;
  let (second, usd) = chat_cost(&mut c, Some(&first), "again").await;
  assert_ne!(first, second, "compacted");
  let want = 2.0 * priced_in(80) + priced_in(250_000);
  assert!((usd - want).abs() < 1e-6, "{usd} != {want}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The user's compaction (`/compact`: summarize, keeping the last
/// messages): its summary is the chain's spend, exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn a_compaction_the_user_asked_for_is_spent_on_the_chain_once() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("Hi."),
    answer_in("SUMMARY-ONE", 250_000),
    answer("After."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data, None);
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let (first, _) = chat_cost(&mut c, None, "hello").await;
  let Reply::Forked(second) = c
    .request(&Request::Fork {
      thread: first.clone(),
      upto: None,
      last: Some(1),
      no_attachments: false,
      no_tools: false,
      summarize: true,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let (on, usd) = chat_cost(&mut c, Some(&second), "again").await;
  assert_eq!(on, second);
  let want = 2.0 * priced_in(80) + priced_in(250_000);
  assert!((usd - want).abs() < 1e-6, "{usd} != {want}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A summary paid for whose compaction then failed (incomplete, from a
/// summary model set explicitly: nothing to fall back to) is spent still,
/// on the thread that asked it.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_compactions_summary_is_spent_still() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _sent) = serve(vec![
    answer("Hi."),
    answer_in("HALF", 250_000).replace("end_turn", "max_tokens"),
    answer("After."),
  ])
  .await;
  let (data, home) = (d.join("data"), d.join("home"));
  setup(&data, &home, &base);
  known_window(&data, Some("anthropic/claude-opus-5-5"));
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let (first, _) = chat_cost(&mut c, None, "hello").await;
  let failed = c
    .request(&Request::Fork {
      thread: first.clone(),
      upto: None,
      last: Some(1),
      no_attachments: false,
      no_tools: false,
      summarize: true,
    })
    .await;
  assert!(
    failed.unwrap_err().to_string().contains("incomplete"),
    "not compacted"
  );
  let (on, usd) = chat_cost(&mut c, Some(&first), "again").await;
  assert_eq!(on, first);
  let want = 2.0 * priced_in(80) + priced_in(250_000);
  assert!((usd - want).abs() < 1e-6, "{usd} != {want}");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}
