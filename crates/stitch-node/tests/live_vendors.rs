//! Kimi (Moonshot) and DeepSeek against their real APIs, by hand only
//! (`--ignored`): the keys as this node keeps them (`stitch key set kimi`,
//! `stitch key set deepseek`), from the data directory in
//! `STITCH_LIVE_DATA` (default `~/.stitch`). Nothing of a key is printed.
//! Small requests: `kimi-k3.1-pro` at effort `low`, `deepseek-flash`.
//!
//! `STITCH_LIVE_IMAGE=<png> cargo test -p stitch-node --test live_vendors -- --ignored --nocapture --test-threads 1`

use std::path::PathBuf;

use serde_json::json;
use stitch_node::config::Config;
use stitch_providers::{Anthropic, Content, Message, Native, Provider, Request, Role, ToolDef};

fn data() -> PathBuf {
  std::env::var_os("STITCH_LIVE_DATA")
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".stitch"))
}

fn provider(model: &str) -> Provider {
  let (mut p, _, _) = Config::default().resolve(&data(), Some(model)).unwrap();
  assert!(p.has_key(), "{model}: no key on this node");
  match &mut p {
    Provider::Anthropic(a) => a.effort = Some("low".into()),
    Provider::OpenAi(o) => o.effort = Some("low".into()),
    _ => {}
  }
  p
}

fn add_tool() -> ToolDef {
  ToolDef {
    name: "add".into(),
    description: "Adds two integers and returns their sum.".into(),
    input_schema: json!({
      "type": "object",
      "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}},
      "required": ["a", "b"],
    }),
  }
}

fn request(model: &str, messages: Vec<Message>) -> Request {
  let mut r = Request::new(model, messages);
  r.max_tokens = 4000;
  r
}

async fn turn(p: &Provider, r: &Request, what: &str) -> stitch_providers::Response {
  let mut thought = 0usize;
  let res = p
    .stream(r, &mut |e| {
      if let stitch_providers::Event::Thinking(t) = e {
        thought += t.len();
      }
    })
    .await;
  match res {
    Ok(res) => {
      println!(
        "{what}: ok, stop {:?}, thinking {thought} chars, native {}, usage {:?}, text {:?}",
        res.stop,
        res.native.is_some(),
        res.usage,
        res.text().chars().take(160).collect::<String>()
      );
      res
    }
    Err(e) => panic!("{what}: FAILED: {e}"),
  }
}

/// Runs the tool loop until the model answers without a call (at most 5
/// steps): every call to `add` answered.
async fn tool_loop(p: &Provider, model: &str, mut history: Vec<Message>, what: &str) {
  for step in 1..=5 {
    let mut r = request(model, history.clone());
    r.tools.push(add_tool());
    let res = turn(p, &r, &format!("{what} step {step}")).await;
    let calls: Vec<(String, serde_json::Value)> = res
      .content
      .iter()
      .filter_map(|c| match c {
        Content::ToolCall { id, input, .. } => Some((id.clone(), input.clone())),
        _ => None,
      })
      .collect();
    history.push(Message {
      role: Role::Assistant,
      content: res.content.clone(),
      native: res.native.clone(),
    });
    if calls.is_empty() {
      println!("{what}: answered after {step} steps");
      return;
    }
    let results = calls
      .into_iter()
      .map(|(id, input)| {
        let sum = input["a"].as_i64().unwrap_or(0) + input["b"].as_i64().unwrap_or(0);
        Content::ToolResult {
          id,
          output: sum.to_string(),
          is_error: false,
        }
      })
      .collect();
    history.push(Message::user(results));
  }
  panic!("{what}: no answer in 5 steps");
}

fn image() -> Option<Content> {
  let path = std::env::var("STITCH_LIVE_IMAGE").ok()?;
  Some(Content::Image {
    media_type: "image/png".into(),
    data: std::fs::read(path).unwrap(),
  })
}

/// A Claude turn that called `add` (no thinking of Kimi's or DeepSeek's),
/// and its result.
fn claude_turn() -> Vec<Message> {
  vec![
    Message::user(vec![Content::Text(
      "Use the add tool to add 2 and 3, then add 10 to that. Then say the result.".into(),
    )]),
    Message {
      role: Role::Assistant,
      content: vec![
        Content::Text("Adding 2 and 3.".into()),
        Content::ToolCall {
          id: "toolu_01A".into(),
          name: "add".into(),
          input: json!({"a": 2, "b": 3}),
        },
      ],
      native: Some(Native {
        provider: "anthropic".into(),
        model: "claude-opus-5-5".into(),
        content: json!([{"type": "thinking", "thinking": "Claude's own.", "signature": "s"}]),
      }),
    },
    Message::user(vec![Content::ToolResult {
      id: "toolu_01A".into(),
      output: "5".into(),
      is_error: false,
    }]),
  ]
}

fn ask(text: &str) -> Vec<Message> {
  vec![Message::user(vec![Content::Text(text.into())])]
}

#[tokio::test]
#[ignore]
async fn kimi_live() {
  let model = "kimi-k3.1-pro";
  let p = provider("kimi/kimi-k3.1-pro");
  // Basic, with a system prompt and the one-hour cache asked for.
  let mut basic = request(model, ask("Say hello in three words."));
  basic.system = Some("You are terse.".into());
  if let Provider::Anthropic(a) = &p {
    let mut a = a.clone();
    a.cache_ttl = Some("1h".into());
    turn(&Provider::Anthropic(a), &basic, "kimi basic (1h cache)").await;
  }
  turn(&p, &basic, "kimi basic").await;
  tool_loop(
    &p,
    model,
    ask("Use the add tool to add 2 and 3, then add 10 to that (two calls, one after the other). Then say the result."),
    "kimi tools",
  )
  .await;
  if let Some(img) = image() {
    let r = request(
      model,
      vec![Message::user(vec![
        img,
        Content::Text("What is the large title at the top of this screenshot? One word.".into()),
      ])],
    );
    turn(&p, &r, "kimi image").await;
  }
  // A history whose tool-calling turn is Claude's (no thinking block).
  let mut r = request(model, claude_turn());
  r.tools.push(add_tool());
  let res = p.stream(&r, &mut |_| {}).await;
  match res {
    Ok(res) => println!("kimi cross-model: ok, {:?}", res.text()),
    Err(e) => println!("kimi cross-model: FAILED: {e}"),
  }
  // Model lists: its documented `/v1/models`, and `/anthropic/v1/models`.
  println!("kimi /v1/models: {:?}", p.models().await.map(|m| m.len()));
  println!("kimi window: {:?}", p.context_window(model).await);
  if let Provider::Anthropic(a) = &p {
    let claude_style = Provider::Anthropic(Anthropic {
      claude: true,
      ..a.clone()
    });
    println!(
      "kimi /anthropic/v1/models: {:?}",
      claude_style.models().await.map(|m| m.len())
    );
  }
}

/// Claude Fable 5.2 (the node's Anthropic key or subscription): a tool
/// loop with its thinking replayed, then the same history sent with the
/// tools changed (as a thread's do when it moves to another node), which
/// fails the prefix check: its thinking blocks are dropped, not a 400.
#[tokio::test]
#[ignore]
async fn fable_5_2_live() {
  let model = "claude-fable-5-2";
  // A subscription signed in to, else the key.
  let (mut p, _, _) = Config::default()
    .resolve_authenticated(&data(), Some("anthropic/claude-fable-5-2"))
    .await
    .unwrap();
  assert!(p.has_key(), "{model}: no key on this node");
  // High: at low, it may call the tool without thinking, and with no
  // thinking block there is nothing for the prefix check to test.
  if let Provider::Anthropic(a) = &mut p {
    a.effort = Some("high".into());
  }
  let mut history = ask(
    "Use the add tool to add the 7th and 9th Fibonacci numbers (F1 = F2 = 1). Then say the result.",
  );
  let mut r = request(model, history.clone());
  r.tools.push(add_tool());
  let res = turn(&p, &r, "fable tools step 1").await;
  let id = res
    .content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, .. } => Some(id.clone()),
      _ => None,
    })
    .expect("fable called add");
  let native = res.native.as_ref().expect("native blocks");
  assert!(
    native
      .content
      .as_array()
      .unwrap()
      .iter()
      .any(|b| b["type"] == "thinking"),
    "fable thought before its call: {}",
    native.content
  );
  history.push(Message {
    role: Role::Assistant,
    content: res.content.clone(),
    native: res.native.clone(),
  });
  history.push(Message::user(vec![Content::ToolResult {
    id,
    output: "47".into(),
    is_error: false,
  }]));
  let mut same = request(model, history.clone());
  same.tools.push(add_tool());
  turn(&p, &same, "fable tools step 2 (same prefix)").await;
  let mut changed = same.clone();
  changed.tools[0].description = "Adds two integers; returns the sum.".into();
  turn(&p, &changed, "fable tools step 2 (tools changed)").await;
}

#[tokio::test]
#[ignore]
async fn deepseek_live() {
  let model = "deepseek-flash";
  let p = provider("deepseek/deepseek-flash");
  turn(
    &p,
    &request(model, ask("Say hello in three words.")),
    "deepseek basic",
  )
  .await;
  if let Some(img) = image() {
    let r = request(
      model,
      vec![Message::user(vec![
        img,
        Content::Text("What is the large title at the top of this screenshot? One word.".into()),
      ])],
    );
    turn(&p, &r, "deepseek image").await;
  }
  tool_loop(
    &p,
    model,
    ask("Use the add tool to add 2 and 3, then add 10 to that (two calls, one after the other). Then say the result."),
    "deepseek tools",
  )
  .await;
  tool_loop(&p, model, claude_turn(), "deepseek cross-model").await;
  // Cache hits: the same long prefix twice.
  let long = "Stitch is an agent orchestrator. ".repeat(400);
  let mut r = request(model, ask("Reply with OK."));
  r.system = Some(long);
  turn(&p, &r, "deepseek cache 1").await;
  tokio::time::sleep(std::time::Duration::from_secs(5)).await;
  let res = turn(&p, &r, "deepseek cache 2").await;
  println!("deepseek cache read: {}", res.usage.cache_read_tokens);
}
