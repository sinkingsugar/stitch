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
    ("errors_are_errors", || Box::pin(errors_are_errors())),
    ("tests_never_see_the_users_sign_in", || {
      Box::pin(tests_never_see_the_users_sign_in())
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
    let path = self.data.join("antigravity/home/fake.jsonl");
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
  assert_eq!(r.usage.input_tokens, 10);
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
  // The context window agy said.
  assert_eq!(p.context_window("fake-pro").await.unwrap(), Some(500_000));
  assert_eq!(p.models().await.unwrap(), ["fake-pro", "fake-flash"]);
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
  // A new tool: agy started again with it eager; the session resumed there.
  m.push(reply(&r));
  m.push(user("call new_tool {}"));
  let r = ask(&p, &request(&m, &["echo_tool", "new_tool"]))
    .await
    .0
    .unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(env.count("initialize"), 3);
  assert_eq!(env.count("session/resume"), 2);
  let config: Value = serde_json::from_slice(
    &std::fs::read(
      env
        .data
        .join("antigravity/home/.gemini/config/mcp_config.json"),
    )
    .unwrap(),
  )
  .unwrap();
  let server = &config["mcpServers"]["stitch"];
  assert_eq!(server["tools"]["new_tool"]["eager"], true);
  assert_eq!(server["timeoutSeconds"], 86_400);
  let listed = env
    .log()
    .into_iter()
    .rev()
    .find(|(m, _)| m == "mcp/tools")
    .unwrap()
    .1;
  assert_eq!(listed, json!(["echo_tool", "new_tool"]));
  // agy's home: the token linked, the hook gating every tool.
  let home = env.data.join("antigravity/home/.gemini");
  assert_eq!(
    std::fs::read_link(home.join("antigravity-cli/antigravity-oauth-token")).unwrap(),
    env.opts.token
  );
  let hooks: Value =
    serde_json::from_slice(&std::fs::read(home.join("config/hooks.json")).unwrap()).unwrap();
  assert_eq!(hooks["stitch-gate"]["PreToolUse"][0]["matcher"], "*");
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
        .open(self.home.join("fake.jsonl"))
        .unwrap();
      writeln!(f, "{}", json!({"method": method, "params": params})).unwrap();
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
      std::fs::read_to_string(self.home.join("fake-sessions"))
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
      let addr = url
        .trim_start_matches("http://")
        .trim_end_matches("/mcp")
        .to_owned();
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
        "POST /mcp HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
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
          json!({"sessionUpdate": "tool_call", "toolCallId": "call_1", "name": format!("mcp_stitch_{tool}"), "status": "in_progress"}),
        );
        let r = self
          .mcp(
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
            "name": tool, "arguments": args,
            "_meta": {"antigravity.google/conversation_id": session}}}),
          )
          .await;
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

  fn options() -> Value {
    json!([
      {"id": "model", "type": "select", "currentValue": "fake-pro",
       "options": [{"value": "fake-pro", "name": "Pro"}, {"value": "fake-flash", "name": "Flash"}]},
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
          std::fs::write(fake.home.join("fake-sessions"), all.join("\n")).unwrap();
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
