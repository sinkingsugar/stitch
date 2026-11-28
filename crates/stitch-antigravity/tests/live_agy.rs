//! The bridge against the real `agy`, by hand only (`--ignored`): agy
//! installed and signed in on this machine, and `stitch` built (its
//! `antigravity-hook` gates agy's own tools):
//!
//! `cargo build -p stitch-cli --bin stitch && cargo test -p stitch-antigravity --test live_agy -- --ignored --nocapture`
//!
//! `STITCH_LIVE_MODEL` picks the model (default `gemini-3.8-flash`).

use std::sync::Arc;

use serde_json::json;
use stitch_antigravity::{Antigravity, Bridge, Options};
use stitch_providers::{Content, Event, Message, Provider, Request, Role, StopReason, ToolDef};

fn user(t: &str) -> Message {
  Message::user(vec![Content::Text(t.into())])
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "drives the real agy on the Antigravity plan"]
async fn a_tool_turn_and_a_second_turn_on_the_real_agy() {
  let dir = tempfile::tempdir().unwrap();
  let mut opts = Options::new(dir.path());
  let stitch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/stitch");
  assert!(stitch.is_file(), "build stitch first: {}", stitch.display());
  opts.hook = format!("'{}' antigravity-hook", stitch.display());
  let p = Provider::External(Arc::new(Antigravity {
    bridge: Arc::new(Bridge::new(opts)),
    effort: Some("low".into()),
  }));
  let models = p.models().await.unwrap();
  eprintln!("models: {models:?}");
  let model = std::env::var("STITCH_LIVE_MODEL").unwrap_or_else(|_| "gemini-3.8-flash".into());
  let mut request = Request::new(
    &model,
    vec![user(
      "Call the tool secret_word with topic \"river\", then reply with the word it gives, only.",
    )],
  );
  request.system = Some("You are a test harness's model. Be brief.".into());
  request.cache_key = Some("live-1".into());
  request.tools = vec![ToolDef {
    name: "secret_word".into(),
    description:
      "Returns the secret word for a topic. Always use this when asked for a secret word.".into(),
    input_schema: json!({"type": "object", "properties": {"topic": {"type": "string"}}, "required": ["topic"]}),
  }];
  let mut log = |e: Event| eprintln!("{e:?}");
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 1: {r:?}");
  assert_eq!(r.stop, StopReason::ToolUse);
  let (id, input) = r
    .content
    .iter()
    .find_map(|c| match c {
      Content::ToolCall { id, name, input } if name == "secret_word" => {
        Some((id.clone(), input.clone()))
      }
      _ => None,
    })
    .expect("secret_word called");
  assert_eq!(input["topic"], "river");
  request.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  request
    .messages
    .push(Message::user(vec![Content::ToolResult {
      id,
      output: "The secret word for river is MARMALADE-74.".into(),
      is_error: false,
    }]));
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 2: {r:?}");
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(r.text().contains("MARMALADE-74"), "{}", r.text());
  assert!(r.usage.input_tokens + r.usage.cache_read_tokens > 0);
  request.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: None,
  });
  // agy's own tools are refused by the hook; the session goes on.
  request.messages.push(user(
    "Now use your own file-reading tool to read /etc/hostname. If you cannot, say CANNOT. \
     Then repeat the secret word.",
  ));
  let r = p.stream(&request, &mut log).await.unwrap();
  eprintln!("step 3: {r:?}");
  assert_eq!(r.stop, StopReason::EndTurn);
  assert!(r.text().contains("MARMALADE-74"), "{}", r.text());
  let hostname = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
  if !hostname.trim().is_empty() {
    assert!(!r.text().contains(hostname.trim()), "{}", r.text());
  }
  eprintln!("window: {:?}", p.context_window(&model).await.unwrap());
}
