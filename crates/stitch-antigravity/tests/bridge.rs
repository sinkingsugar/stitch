//! The bridge against a fake `agy`: this test binary itself, run with
//! `FAKE_AGY` set, speaking the ACP subset the bridge uses and calling
//! the bridge's MCP endpoint as agy does (from its home's
//! `mcp_config.json`).
//!
//! The fake's prompt is read by its last line: `call <tool> <json>` calls
//! a tool and says its result; `wait` waits to be cancelled; `quota` ends
//! with agy's 429 line; anything else is echoed.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use stitch_antigravity::{Antigravity, Bridge, Options};
use stitch_providers::{
  Content, Error, Event, Message, Provider, Request, Response, Role, StopReason, ToolDef,
};

type Test = std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>;
type Case = (&'static str, fn() -> Test);

fn main() {
  if std::env::var_os("FAKE_AGY").is_some() {
    fake::main();
    return;
  }
  let rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap();
  let tests: &[Case] = &[
    ("a_tool_step_goes_through_the_held_call", || {
      Box::pin(a_tool_step_goes_through_the_held_call())
    }),
    ("a_stopped_turn_is_cancelled_and_starts_fresh", || {
      Box::pin(a_stopped_turn_is_cancelled_and_starts_fresh())
    }),
    ("a_rewind_starts_a_fresh_session", || {
      Box::pin(a_rewind_starts_a_fresh_session())
    }),
    ("the_model_list_is_refreshed_when_stale", || {
      Box::pin(the_model_list_is_refreshed_when_stale())
    }),
    ("errors_are_errors", || Box::pin(errors_are_errors())),
    ("stitch_owns_compaction", || {
      Box::pin(stitch_owns_compaction())
    }),
    ("a_tools_image_goes_back_in_its_result", || {
      Box::pin(a_tools_image_goes_back_in_its_result())
    }),
    ("processes_are_bounded_and_reaped", || {
      Box::pin(processes_are_bounded_and_reaped())
    }),
    ("tests_never_see_the_users_sign_in", || {
      Box::pin(tests_never_see_the_users_sign_in())
    }),
    ("a_held_call_stopped_then_a_message_reaches_agy", || {
      Box::pin(a_held_call_stopped_then_a_message_reaches_agy())
    }),
    (
      "a_restart_during_a_held_call_then_a_message_reaches_agy",
      || Box::pin(a_restart_during_a_held_call_then_a_message_reaches_agy()),
    ),
    ("calls_agy_makes_at_once_are_answered_each_its_own", || {
      Box::pin(calls_agy_makes_at_once_are_answered_each_its_own())
    }),
    (
      "sessions_outlive_the_node_and_new_tools_restart_agy",
      || Box::pin(sessions_outlive_the_node_and_new_tools_restart_agy()),
    ),
  ];
  let filter: Vec<String> = std::env::args()
    .skip(1)
    .filter(|a| !a.starts_with('-'))
    .collect();
  let mut ran = 0;
  for (name, test) in tests {
    if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
      continue;
    }
    eprint!("test {name} ... ");
    rt.block_on(async {
      tokio::time::timeout(Duration::from_secs(60), test())
        .await
        .unwrap_or_else(|_| panic!("{name} timed out"))
    });
    eprintln!("ok");
    ran += 1;
  }
  eprintln!("test result: ok. {ran} passed");
}

struct Env {
  _dir: tempfile::TempDir,
  data: PathBuf,
  opts: Options,
}

impl Env {
  fn new() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let token = data.join("token");
    std::fs::write(&token, "{}").unwrap();
    let opts = Options {
      dir: data.join("antigravity"),
      program: std::env::current_exe().unwrap(),
      args: vec!["acp".into()],
      env: vec![("FAKE_AGY".into(), "1".into())],
      token,
      sign_in_home: data.clone(),
      hook: "true".into(),
    };
    Env {
      _dir: dir,
      data,
      opts,
    }
  }

  fn provider(&self) -> Provider {
    self.provider_of(Arc::new(Bridge::new(self.opts.clone())))
  }

  fn provider_of(&self, bridge: Arc<Bridge>) -> Provider {
    Provider::External(Arc::new(Antigravity {
      bridge,
      effort: Some("high".into()),
    }))
  }

  /// What the fake was asked, in order: `(method, params)`.
  fn log(&self) -> Vec<(String, Value)> {
    let path = self
      .data
      .join("antigravity/home/.gemini/antigravity-cli/fake.jsonl");
    std::fs::read_to_string(path)
      .unwrap_or_default()
      .lines()
      .map(|l| {
        let v: Value = serde_json::from_str(l).unwrap();
        (
          v["method"].as_str().unwrap().to_owned(),
          v["params"].clone(),
        )
      })
      .collect()
  }

  /// The tool lists each process got, in order.
  fn listed(&self) -> Vec<Value> {
    self
      .log()
      .into_iter()
      .filter(|(m, _)| m == "mcp/tools")
      .map(|(_, p)| p)
      .collect()
  }

  /// The configuration directory of the process whose tools include `tool`.
  fn config_dir_with(&self, tool: &str) -> PathBuf {
    std::fs::read_dir(self.data.join("antigravity/procs"))
      .unwrap()
      .map(|e| e.unwrap().path().join(".gemini/config"))
      .find(|d| {
        std::fs::read_to_string(d.join("mcp_config.json"))
          .is_ok_and(|t| t.contains(&format!("\"{tool}\"")))
      })
      .unwrap()
  }

  fn config_with(&self, tool: &str) -> Value {
    serde_json::from_slice(
      &std::fs::read(self.config_dir_with(tool).join("mcp_config.json")).unwrap(),
    )
    .unwrap()
  }

  fn count(&self, method: &str) -> usize {
    self.log().iter().filter(|(m, _)| m == method).count()
  }

  fn prompts(&self) -> Vec<String> {
    self
      .log()
      .into_iter()
      .filter(|(m, _)| m == "session/prompt")
      .map(|(_, p)| fake::prompt_text(&p))
      .collect()
  }
}

fn tool(name: &str) -> ToolDef {
  ToolDef {
    name: name.into(),
    description: format!("The {name} tool."),
    input_schema: json!({"type": "object", "properties": {"x": {"type": "number"}}}),
  }
}

fn user(t: &str) -> Message {
  Message::user(vec![Content::Text(t.into())])
}

fn reply(r: &Response) -> Message {
  Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  }
}

fn request(messages: &[Message], tools: &[&str]) -> Request {
  let mut r = Request::new("fake-pro", messages.to_vec());
  r.system = Some("You are a test.".into());
  r.cache_key = Some("thread-1".into());
  r.tools = tools.iter().map(|t| tool(t)).collect();
  r
}

async fn ask(p: &Provider, r: &Request) -> (Result<Response, Error>, String) {
  let mut streamed = String::new();
  let out = p
    .stream(r, &mut |e| {
      if let Event::Text(t) = e {
        streamed.push_str(&t)
      }
    })
    .await;
  (out, streamed)
}

fn call_of(r: &Response) -> (String, String, Value) {
  r.content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, name, input } => Some((id.clone(), name.clone(), input.clone())),
      _ => None,
    })
    .expect("a tool call")
}

async fn a_tool_step_goes_through_the_held_call() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("call echo_tool {\"x\": 1}")];
  let (r, streamed) = ask(&p, &request(&m, &["echo_tool"])).await;
  let r = r.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(r.usage.input_tokens, 1000);
  assert_eq!(r.model, "antigravity/fake-pro");
  assert_eq!(r.text(), "Calling.");
  assert_eq!(streamed, "Calling.");
  let (id, name, input) = call_of(&r);
  assert_eq!((name.as_str(), input), ("echo_tool", json!({"x": 1})));
  // The model and effort were set, the system prompt appended.
  let log = env.log();
  let new = log.iter().find(|(m, _)| m == "session/new").unwrap();
  assert_eq!(
    new.1["_meta"]["antigravity"]["systemPrompt"]["append"],
    "You are a test."
  );
  assert!(log.iter().any(|(m, p)| m == "session/set_config_option"
    && p["configId"] == "thought_level"
    && p["value"] == "high"));

  // The result, and a message that came meanwhile, answer the held call.
  m.push(reply(&r));
  m.push(Message::user(vec![Content::ToolResult {
    id,
    output: "OUT".into(),
    is_error: false,
  }]));
  m.push(user("and hurry"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(
    r.text()
      .starts_with("Result: OUT\n\n[Not part of the tool's output")
  );
  assert!(r.text().ends_with("and hurry"), "{}", r.text());
  // Each step is told its own prompt (agy's `used`), the last the turn's
  // output and the cache reads agy counted.
  assert_eq!(r.usage.input_tokens + r.usage.cache_read_tokens, 1200);
  assert_eq!(r.usage.cache_read_tokens, 5);
  assert_eq!(r.usage.output_tokens, 4);

  // The next user message is a prompt in the same session.
  m.push(reply(&r));
  m.push(user("hello"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.text(), "echo: hello");
  assert_eq!(env.count("session/new"), 1);
  assert_eq!(env.prompts().last().unwrap(), "hello");

  // A tool not in this thread's set is refused at once; the turn goes on.
  m.push(reply(&r));
  m.push(user("call secret_tool {}"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(
    r.text().contains("not one of this thread's tools"),
    "{}",
    r.text()
  );
  // The window agy said (500,000), less a guess at its own compaction.
  assert_eq!(p.context_window("fake-pro").await.unwrap(), Some(160_000));
  assert_eq!(p.models().await.unwrap(), ["fake-pro", "fake-flash"]);
}

async fn a_tools_image_goes_back_in_its_result() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("call view_file {\"path\": \"a.png\"}")];
  let r = ask(&p, &request(&m, &["view_file"])).await.0.unwrap();
  let (id, ..) = call_of(&r);
  m.push(reply(&r));
  m.push(Message::user(vec![Content::ToolResult {
    id: id.clone(),
    output: "a.png, 2 by 2".into(),
    is_error: false,
  }]));
  // The node's message of the tool's media, after the results.
  let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
  m.push(Message::user(vec![
    Content::Text(format!("From view_file (call {id}), a.png:")),
    Content::Text("[image img:0000aaaa]".into()),
    Content::Image {
      media_type: "image/png".into(),
      data: png.clone(),
    },
  ]));
  let r = ask(&p, &request(&m, &["view_file"])).await.0.unwrap();
  assert_eq!(r.text(), "Result: a.png, 2 by 2");
  let result = env
    .log()
    .into_iter()
    .find(|(m, _)| m == "mcp/result")
    .unwrap()
    .1;
  let content = result["content"].as_array().unwrap();
  assert_eq!(content[0], json!({"type": "text", "text": "a.png, 2 by 2"}));
  assert_eq!(
    content[1]["text"],
    format!("From view_file (call {id}), a.png:")
  );
  assert_eq!(content[2]["text"], "[image img:0000aaaa]");
  assert_eq!(content[3]["type"], "image");
  assert_eq!(content[3]["mimeType"], "image/png");
  use base64::Engine as _;
  assert_eq!(
    base64::engine::general_purpose::STANDARD
      .decode(content[3]["data"].as_str().unwrap())
      .unwrap(),
    png
  );
  assert_eq!(result["isError"], false);
}

async fn stitch_owns_compaction() {
  let env = Env::new();
  let bridge = Arc::new(Bridge::new(env.opts.clone()));
  let p = env.provider_of(bridge.clone());
  // Before any session: agy's window by family, less a guess at its own
  // compaction.
  assert_eq!(bridge.window("gemini-3.8-flash"), Some(512_000));
  assert_eq!(bridge.window("claude-sonnet-4-6"), Some(160_000));
  assert_eq!(bridge.window("gpt-oss-120b-medium"), Some(80_000));
  // The fake's window (500,000), told by the session, guessed under.
  let mut m = vec![user("hi")];
  let r = ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(bridge.window("fake-pro"), Some(160_000));
  // agy compacting on its own: its size before is learned (and kept), and
  // the session starts fresh next, from Stitch's record.
  m.push(reply(&r));
  m.push(user("compact"));
  let r = ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(r.usage.input_tokens + r.usage.cache_read_tokens, 60_000);
  assert_eq!(bridge.window("fake-pro"), Some(60_000));
  assert_eq!(
    Bridge::new(env.opts.clone()).window("fake-pro"),
    Some(60_000)
  );
  m.push(reply(&r));
  m.push(user("after"));
  let r = ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(env.count("session/new"), 2);
  assert!(r.text().contains("User:\ncompact"), "{}", r.text());
  // Stitch's own compaction: a new thread, its first message the summary;
  // its session starts from that and the messages kept, with the system
  // prompt.
  let summary = "[Summary of the conversation so far: the user asked for HERON-5.]";
  let mut compacted = request(
    &[
      user(summary),
      Message {
        role: Role::Assistant,
        content: vec![Content::Text("Noted.".into())],
        native: None,
      },
      user("go on"),
    ],
    &[],
  );
  compacted.cache_key = Some("thread-1-compacted".into());
  let r = ask(&p, &compacted).await.0.unwrap();
  assert_eq!(env.count("session/new"), 3);
  let prompt = env.prompts().last().unwrap().clone();
  assert!(prompt.contains(&format!("User:\n{summary}")), "{prompt}");
  assert!(prompt.contains("Assistant:\nNoted."), "{prompt}");
  assert!(prompt.ends_with("go on"), "{prompt}");
  assert!(
    r.text()
      .starts_with("echo: [Stitch: this conversation began")
  );
  let news: Vec<Value> = env
    .log()
    .into_iter()
    .filter(|(m, _)| m == "session/new")
    .map(|(_, p)| p)
    .collect();
  assert_eq!(
    news.last().unwrap()["_meta"]["antigravity"]["systemPrompt"]["append"],
    "You are a test."
  );
}

async fn a_stopped_turn_is_cancelled_and_starts_fresh() {
  let env = Env::new();
  let p = env.provider();
  // A turn answered, then one stopped while it streams.
  let first = vec![user("hello")];
  let r = ask(&p, &request(&first, &[])).await.0.unwrap();
  let mut m2 = first.clone();
  m2.push(reply(&r));
  m2.push(user("wait"));
  let stopped = tokio::time::timeout(Duration::from_millis(500), ask(&p, &request(&m2, &[]))).await;
  assert!(stopped.is_err(), "the fake's turn waits");
  // agy was told.
  for _ in 0..50 {
    if env.count("session/cancel") == 1 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  assert_eq!(env.count("session/cancel"), 1);
  // The next message: a fresh session, the conversation as a transcript.
  m2.push(user("again"));
  let r = ask(&p, &request(&m2, &[])).await.0.unwrap();
  assert_eq!(env.count("session/new"), 2);
  let text = r.text();
  assert!(text.contains("User:\nhello"), "{text}");
  assert!(text.contains("Assistant:\necho: hello"), "{text}");
  assert!(text.ends_with("wait\n\nagain"), "{text}");
}

/// The user stopped the turn while Stitch ran a tool agy called: the
/// node's next request carries that call's result ("interrupted by the
/// user") and the user's new message, which answer the held call, so the
/// message reaches agy; and the message after that, as a prompt.
async fn a_held_call_stopped_then_a_message_reaches_agy() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("call echo_tool {\"x\": 1}")];
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  let (id, _, _) = call_of(&r);
  m.push(reply(&r));
  m.push(Message::user(vec![Content::ToolResult {
    id,
    output: "interrupted by the user".into(),
    is_error: true,
  }]));
  m.push(user("pls stop"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  let text = r.text();
  assert!(text.contains("interrupted by the user"), "{text}");
  assert!(text.contains("arrived while it ran.]\npls stop"), "{text}");
  assert_eq!(r.stop, StopReason::EndTurn);
  m.push(reply(&r));
  m.push(user("continue pls"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.text(), "echo: continue pls");
  assert_eq!(env.count("session/new"), 1);
  assert_eq!(env.prompts().last().unwrap(), "continue pls");
}

/// The node restarted while Stitch ran a tool agy called (the call agy
/// waited on is gone): the next message starts a fresh session with the
/// conversation as a transcript, the message last, and that session is the
/// thread's from then on.
async fn a_restart_during_a_held_call_then_a_message_reaches_agy() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("call echo_tool {\"x\": 1}")];
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  let (id, _, _) = call_of(&r);
  drop(p);
  let p = env.provider();
  // The node's record of the cut call, as it reads it back.
  m.push(reply(&r));
  m.push(Message::user(vec![Content::ToolResult {
    id,
    output: "interrupted: no result was recorded".into(),
    is_error: true,
  }]));
  m.push(user("continue pls"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  let text = r.text();
  assert!(text.starts_with("echo: "), "{text}");
  assert!(text.contains("call echo_tool"), "{text}");
  assert!(text.contains("no result was recorded"), "{text}");
  assert!(text.ends_with("continue pls"), "{text}");
  assert_eq!(env.count("session/new"), 2);
  m.push(reply(&r));
  m.push(user("and then"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.text(), "echo: and then");
  assert_eq!(env.count("session/new"), 2);
}

/// agy makes several calls of one response at once: Stitch is given them
/// one step at a time, and each answer goes back to its own call.
async fn calls_agy_makes_at_once_are_answered_each_its_own() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("calls echo_tool 3")];
  let mut seen = Vec::new();
  let r = loop {
    let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
    if r.stop != StopReason::ToolUse {
      break r;
    }
    let (id, name, input) = call_of(&r);
    assert_eq!(name, "echo_tool");
    let x = input["x"].as_u64().unwrap();
    seen.push(x);
    m.push(reply(&r));
    m.push(Message::user(vec![Content::ToolResult {
      id,
      output: format!("OUT{x}"),
      is_error: false,
    }]));
  };
  seen.sort();
  assert_eq!(seen, [1, 2, 3]);
  assert!(
    r.text().ends_with("Results: OUT1|OUT2|OUT3"),
    "{}",
    r.text()
  );
  assert_eq!(env.count("session/new"), 1);
}

async fn a_rewind_starts_a_fresh_session() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("one")];
  let r = ask(&p, &request(&m, &[])).await.0.unwrap();
  m.push(reply(&r));
  m.push(user("two"));
  ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(env.count("session/new"), 1);
  // Rewound to before "two", then another message: the session holds
  // "two" and its reply, so a fresh one, with what is left as a
  // transcript.
  m.truncate(2);
  m.push(user("three"));
  let r3 = ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(env.count("session/new"), 2);
  let text = r3.text();
  assert!(text.contains("User:\none"), "{text}");
  assert!(text.contains("Assistant:\necho: one"), "{text}");
  assert!(!text.contains("two"), "{text}");
  assert!(text.ends_with("three"), "{text}");
  // And that session goes on.
  m.push(reply(&r3));
  m.push(user("four"));
  let r4 = ask(&p, &request(&m, &[])).await.0.unwrap();
  assert_eq!(r4.text(), "echo: four");
  assert_eq!(env.count("session/new"), 2);
}

async fn the_model_list_is_refreshed_when_stale() {
  let env = Env::new();
  let file = env
    .data
    .join("antigravity/home/.gemini/antigravity-cli/fake-models");
  let first =
    || Arc::new(Bridge::new(env.opts.clone()).with_models_ttl(Duration::from_millis(300)));
  let bridge = first();
  assert_eq!(bridge.models().await.unwrap(), ["fake-pro", "fake-flash"]);
  // The plan changes: fresh, the old list is served without asking.
  std::fs::create_dir_all(file.parent().unwrap()).unwrap();
  std::fs::write(&file, "gemini-4-neon,claude-opus-5-5").unwrap();
  assert_eq!(bridge.models().await.unwrap(), ["fake-pro", "fake-flash"]);
  // Stale: asked again.
  tokio::time::sleep(Duration::from_millis(400)).await;
  assert_eq!(
    bridge.models().await.unwrap(),
    ["gemini-4-neon", "claude-opus-5-5"]
  );
  // Stale and agy answers without models: the old list stays.
  std::fs::write(&file, "").unwrap();
  tokio::time::sleep(Duration::from_millis(400)).await;
  let kept = bridge.models().await.unwrap();
  assert_eq!(kept, ["gemini-4-neon", "claude-opus-5-5"]);
  // A new list again; and with no old list, a failure is an error.
  std::fs::write(&file, "gemini-4-argon").unwrap();
  tokio::time::sleep(Duration::from_millis(400)).await;
  assert_eq!(bridge.models().await.unwrap(), ["gemini-4-argon"]);
  let mut broken = env.opts.clone();
  broken.program = env.data.join("no-such-agy");
  let b2 = Bridge::new(broken).with_models_ttl(Duration::from_millis(1));
  assert!(b2.models().await.is_err(), "no old list, no answer");
}

async fn errors_are_errors() {
  let env = Env::new();
  let p = env.provider();
  let (r, streamed) = ask(&p, &request(&[user("quota")], &[])).await;
  match r {
    Err(Error::OutOfUsage { resets_at, .. }) => assert!(resets_at.is_some()),
    other => panic!("{other:?}"),
  }
  assert_eq!(streamed, "", "the error line is not shown as text");
  // An unknown model.
  let mut bad = request(&[user("hi")], &[]);
  bad.model = "nope".into();
  bad.cache_key = Some("thread-9".into());
  match ask(&p, &bad).await.0 {
    Err(Error::Api {
      status: 404,
      message,
      ..
    }) => assert!(message.contains("fake-flash")),
    other => panic!("{other:?}"),
  }
  // Not signed in.
  let mut opts = env.opts.clone();
  opts.token = env.data.join("missing");
  let signed_out = env.provider_of(Arc::new(Bridge::new(opts)));
  assert!(!signed_out.has_key());
  assert!(matches!(
    ask(&signed_out, &request(&[user("hi")], &[])).await.0,
    Err(Error::Api { status: 401, .. })
  ));
  assert!(matches!(signed_out.models().await, Err(Error::NoKey(_))));
}

async fn tests_never_see_the_users_sign_in() {
  // `.cargo/config.toml` points `STITCH_AGY_HOME` at a home with none.
  let home = stitch_antigravity::sign_in_home();
  assert!(
    home.ends_with("target/no-agy-sign-in"),
    "{}",
    home.display()
  );
  let dir = tempfile::tempdir().unwrap();
  let opts = Options::new(dir.path());
  assert!(!opts.token.is_file());
  let bridge = Arc::new(Bridge::new(opts.clone()));
  assert!(bridge.ready().is_err());
  assert!(opts.status().starts_with("not"), "{}", opts.status());
}

async fn sessions_outlive_the_node_and_new_tools_restart_agy() {
  let env = Env::new();
  let p = env.provider();
  let mut m = vec![user("hi")];
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  m.push(reply(&r));
  drop(p);
  // Another node run: the session is resumed, not made again.
  let p = env.provider();
  m.push(user("again"));
  let r = ask(&p, &request(&m, &["echo_tool"])).await.0.unwrap();
  assert_eq!(r.text(), "echo: again");
  assert_eq!(env.count("session/new"), 1);
  assert_eq!(env.count("session/resume"), 1);
  assert_eq!(env.count("initialize"), 2);
  // Another tool set: a process of its own, listing exactly that set; the
  // session made in the first resumed there.
  m.push(reply(&r));
  m.push(user("call new_tool {}"));
  let r = ask(&p, &request(&m, &["echo_tool", "new_tool"]))
    .await
    .0
    .unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(env.count("initialize"), 3);
  assert_eq!(env.count("session/resume"), 2);
  assert_eq!(env.count("session/new"), 1);
  assert_eq!(
    env.listed().last().unwrap(),
    &json!(["echo_tool", "new_tool"])
  );
  let config = env.config_with("new_tool");
  let server = &config["mcpServers"]["stitch"];
  assert_eq!(server["tools"]["echo_tool"]["eager"], true);
  assert_eq!(server["timeoutSeconds"], 86_400);
  assert!(server["url"].as_str().unwrap().contains("/mcp/"));
  // Another thread with another set sees only its own tools.
  let mut other = request(&[user("hi")], &["other_tool"]);
  other.cache_key = Some("thread-2".into());
  ask(&p, &other).await.0.unwrap();
  assert_eq!(env.listed().last().unwrap(), &json!(["other_tool"]));
  // agy's home: the token linked, the hook gating every tool.
  let home = env.data.join("antigravity/home/.gemini");
  assert_eq!(
    std::fs::read_link(home.join("antigravity-cli/antigravity-oauth-token")).unwrap(),
    env.opts.token
  );
  let hooks: Value = serde_json::from_slice(
    &std::fs::read(env.config_dir_with("new_tool").join("hooks.json")).unwrap(),
  )
  .unwrap();
  assert_eq!(hooks["stitch-gate"]["PreToolUse"][0]["matcher"], "*");
}

async fn processes_are_bounded_and_reaped() {
  let env = Env::new();
  let bridge = Arc::new(Bridge::new(env.opts.clone()));
  let p = env.provider_of(bridge.clone());
  // Six threads, six tool sets: never more than four processes idle.
  let mut firsts = Vec::new();
  for i in 0..6 {
    let mut r = request(&[user("hi")], &[&format!("tool_{i}")]);
    r.cache_key = Some(format!("set-{i}"));
    firsts.push(ask(&p, &r).await.0.unwrap());
    assert!(bridge.processes().await <= 4);
  }
  assert_eq!(bridge.processes().await, 4);
  // One with a held call is never stopped.
  let mut held = request(&[user("call tool_9 {}")], &["tool_9"]);
  held.cache_key = Some("held".into());
  let r = ask(&p, &held).await.0.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  bridge.reap_now(Duration::ZERO).await;
  assert_eq!(bridge.processes().await, 1, "the held one stays");
  // The thread whose process was stopped resumes in a new one.
  let mut again = request(&[user("hi")], &["tool_0"]);
  again.cache_key = Some("set-0".into());
  again.messages.push(reply(&firsts[0]));
  again.messages.push(user("more"));
  let (before, resumed) = (env.count("session/new"), env.count("session/resume"));
  let r = ask(&p, &again).await.0.unwrap();
  assert_eq!(r.text(), "echo: more");
  assert_eq!(env.count("session/new"), before);
  assert_eq!(env.count("session/resume"), resumed + 1);
}

/// The fake `agy acp`.
mod fake {
  use std::collections::HashMap;
  use std::io::Write as _;
  use std::path::PathBuf;
  use std::sync::{Arc, Mutex};

  use serde_json::{Value, json};
  use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
  use tokio::sync::Notify;

  struct Fake {
    home: PathBuf,
    out: Mutex<std::io::Stdout>,
    cancels: Mutex<HashMap<String, Arc<Notify>>>,
    mcp: Mutex<Option<String>>,
  }

  pub fn prompt_text(params: &Value) -> String {
    params["prompt"]
      .as_array()
      .map(|a| {
        a.iter()
          .filter_map(|b| b["text"].as_str())
          .collect::<Vec<_>>()
          .join("\n\n")
      })
      .unwrap_or_default()
  }

  impl Fake {
    fn send(&self, v: Value) {
      let mut out = self.out.lock().unwrap();
      writeln!(out, "{v}").unwrap();
      out.flush().unwrap();
    }
    fn log(&self, method: &str, params: &Value) {
      let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(self.home.join(".gemini/antigravity-cli/fake.jsonl"))
        .unwrap();
      // One write, so the lines of processes writing at once never mix.
      let line = format!("{}\n", json!({"method": method, "params": params}));
      f.write_all(line.as_bytes()).unwrap();
    }
    fn update(&self, session: &str, update: Value) {
      self.send(json!({"jsonrpc": "2.0", "method": "session/update",
        "params": {"sessionId": session, "update": update}}));
    }
    fn chunk(&self, session: &str, text: &str) {
      self.update(
        session,
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}),
      );
    }
    fn sessions(&self) -> Vec<String> {
      std::fs::read_to_string(self.home.join(".gemini/antigravity-cli/fake-sessions"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
    }

    async fn mcp(&self, body: Value) -> Value {
      let config: Value = serde_json::from_slice(
        &std::fs::read(self.home.join(".gemini/config/mcp_config.json")).unwrap(),
      )
      .unwrap();
      let server = &config["mcpServers"]["stitch"];
      let url = server["url"].as_str().unwrap();
      let (addr, path) = url.trim_start_matches("http://").split_once('/').unwrap();
      let (addr, path) = (addr.to_owned(), format!("/{path}"));
      let mut headers = String::new();
      for (k, v) in server["headers"].as_object().unwrap() {
        headers.push_str(&format!("{k}: {}\r\n", v.as_str().unwrap()));
      }
      if let Some(s) = &*self.mcp.lock().unwrap() {
        headers.push_str(&format!("mcp-session-id: {s}\r\n"));
      }
      let body = body.to_string();
      let mut stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
      let req = format!(
        "POST {path} HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
      );
      stream.write_all(req.as_bytes()).await.unwrap();
      let mut raw = Vec::new();
      stream.read_to_end(&mut raw).await.unwrap();
      let raw = String::from_utf8_lossy(&raw);
      let (head, body) = raw.split_once("\r\n\r\n").unwrap();
      for line in head.lines() {
        if let Some(s) = line.to_lowercase().strip_prefix("mcp-session-id: ") {
          *self.mcp.lock().unwrap() = Some(s.trim().to_owned());
        }
      }
      serde_json::from_str(body).unwrap_or(Value::Null)
    }

    async fn prompt(self: Arc<Self>, id: Value, params: Value) {
      let session = params["sessionId"].as_str().unwrap().to_owned();
      let text = prompt_text(&params);
      let last = text.lines().last().unwrap_or("").to_owned();
      let usage = json!({"inputTokens": 10, "cachedReadTokens": 5, "outputTokens": 3, "thoughtTokens": 1, "totalTokens": 19});
      let done = |stop: &str, usage: Option<Value>| {
        let mut r = json!({"stopReason": stop});
        if let Some(u) = usage {
          r["usage"] = u;
        }
        json!({"jsonrpc": "2.0", "id": id, "result": r})
      };
      if let Some(rest) = last.strip_prefix("call ") {
        let (tool, args) = rest.split_once(' ').unwrap_or((rest, "{}"));
        let args: Value = serde_json::from_str(args).unwrap();
        self.chunk(&session, "Calling.");
        self.update(
          &session,
          json!({"sessionUpdate": "usage_update", "used": 1000, "size": 500000}),
        );
        self.update(
          &session,
          json!({"sessionUpdate": "tool_call", "toolCallId": "call_1", "name": format!("mcp_stitch_{tool}"), "status": "in_progress"}),
        );
        let r = self
          .mcp(
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
            "name": tool, "arguments": args,
            "_meta": {"antigravity.google/conversation_id": session}}}),
          )
          .await;
        self.log("mcp/result", &r["result"]);
        let out = r["result"]["content"][0]["text"]
          .as_str()
          .unwrap_or("")
          .to_owned();
        let failed = if r["result"]["isError"] == true {
          " (error)"
        } else {
          ""
        };
        self.chunk(&session, &format!("Result: {out}{failed}"));
        self.update(
          &session,
          json!({"sessionUpdate": "usage_update", "used": 1200, "size": 500000}),
        );
        self.send(done("end_turn", Some(usage)));
      } else if let Some(rest) = last.strip_prefix("calls ") {
        // As agy does with several calls in one response: all at once.
        let (tool, n) = rest.split_once(' ').unwrap();
        let n: u64 = n.parse().unwrap();
        self.chunk(&session, "Calling them.");
        let calls: Vec<_> = (1..=n)
          .map(|x| {
            let f = self.clone();
            let session = session.clone();
            let tool = tool.to_owned();
            tokio::spawn(async move {
              let r = f
                .mcp(
                  json!({"jsonrpc": "2.0", "id": 10 + x, "method": "tools/call", "params": {
                "name": tool, "arguments": {"x": x},
                "_meta": {"antigravity.google/conversation_id": session}}}),
                )
                .await;
              r["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned()
            })
          })
          .collect();
        let mut outs = Vec::new();
        for c in calls {
          outs.push(c.await.unwrap());
        }
        self.chunk(&session, &format!("Results: {}", outs.join("|")));
        self.send(done("end_turn", Some(usage)));
      } else if last == "wait" {
        let n = self
          .cancels
          .lock()
          .unwrap()
          .entry(session.clone())
          .or_default()
          .clone();
        self.chunk(&session, "Waiting");
        n.notified().await;
        self.send(done("cancelled", Some(usage)));
      } else if last == "compact" {
        // As agy compacting on its own: the context falls.
        for used in [60_000, 9_000] {
          self.update(
            &session,
            json!({"sessionUpdate": "usage_update", "used": used, "size": 500000}),
          );
        }
        self.chunk(&session, "Compacted.");
        self.send(done("end_turn", Some(usage)));
      } else if last == "quota" {
        self.chunk(
          &session,
          "\n⚠ RESOURCE_EXHAUSTED (code 429): You have exhausted your quota on this model. Resets in 5h0m0s.\n",
        );
        self.send(done("end_turn", None));
      } else {
        self.update(
          &session,
          json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}}),
        );
        self.chunk(&session, &format!("echo: {text}"));
        self.update(
          &session,
          json!({"sessionUpdate": "usage_update", "used": 100, "size": 500000}),
        );
        self.send(done("end_turn", Some(usage)));
      }
    }
  }

  /// The models the fake offers: `fake-models` in its home (comma
  /// separated) when there is one.
  fn offered() -> Vec<String> {
    let path = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
      .join(".gemini/antigravity-cli/fake-models");
    match std::fs::read_to_string(path) {
      Ok(t) => t
        .split(',')
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .collect(),
      Err(_) => vec!["fake-pro".into(), "fake-flash".into()],
    }
  }

  fn options() -> Value {
    let models: Vec<Value> = offered()
      .iter()
      .map(|m| json!({"value": m, "name": m}))
      .collect();
    json!([
      {"id": "model", "type": "select", "currentValue": "fake-pro", "options": models},
      {"id": "thought_level", "type": "select", "currentValue": "high",
       "options": [{"value": "low"}, {"value": "high"}]}
    ])
  }

  pub fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()
      .unwrap();
    rt.block_on(run());
  }

  async fn run() {
    let fake = Arc::new(Fake {
      home: std::env::var_os("HOME").unwrap().into(),
      out: Mutex::new(std::io::stdout()),
      cancels: Mutex::new(HashMap::new()),
      mcp: Mutex::new(None),
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
      let msg: Value = serde_json::from_str(&line).unwrap();
      let method = msg["method"].as_str().unwrap_or("").to_owned();
      let params = msg["params"].clone();
      if !method.is_empty() {
        fake.log(&method, &params);
      }
      let id = msg["id"].clone();
      let ok = |r: Value| json!({"jsonrpc": "2.0", "id": id, "result": r});
      let err = |code: i64, m: &str| json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}});
      match method.as_str() {
        "initialize" => {
          // As agy: the MCP server of its configuration, and its tools.
          let f = fake.clone();
          let init = f
            .mcp(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}}))
            .await;
          assert!(init["result"].is_object());
          let list = f
            .mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
            .await;
          let names: Vec<Value> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect();
          f.log("mcp/tools", &Value::Array(names));
          fake.send(ok(
            json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
          ));
        }
        "session/new" => {
          let sid = format!(
            "s-{}",
            std::process::id() as u64 * 1000 + fake.sessions().len() as u64
          );
          let mut all = fake.sessions();
          all.push(sid.clone());
          std::fs::write(
            fake.home.join(".gemini/antigravity-cli/fake-sessions"),
            all.join("\n"),
          )
          .unwrap();
          fake.send(ok(json!({"sessionId": sid, "configOptions": options()})));
        }
        "session/resume" => {
          if fake.sessions().iter().any(|s| params["sessionId"] == **s) {
            fake.send(ok(json!({})));
          } else {
            fake.send(err(-32002, "Session not found"));
          }
        }
        "session/set_config_option" => {
          let known = match params["configId"].as_str() {
            Some("model") => {
              ["fake-pro", "fake-flash"].contains(&params["value"].as_str().unwrap_or(""))
            }
            Some("thought_level") => true,
            _ => false,
          };
          if known {
            fake.send(ok(json!({"configOptions": options()})));
          } else {
            fake.send(err(-32602, "Unknown model ID"));
          }
        }
        "session/prompt" => {
          tokio::spawn(fake.clone().prompt(id.clone(), params));
        }
        "session/cancel" => {
          let n = fake
            .cancels
            .lock()
            .unwrap()
            .entry(params["sessionId"].as_str().unwrap().to_owned())
            .or_default()
            .clone();
          n.notify_one();
        }
        "session/close" => fake.send(ok(json!({}))),
        _ if !id.is_null() => fake.send(err(-32601, "not supported")),
        _ => {}
      }
    }
  }
}
