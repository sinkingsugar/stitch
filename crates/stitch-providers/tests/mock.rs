//! The adapters against a local HTTP server that answers with canned
//! responses and records what it was sent.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_providers::{
  Anthropic, Content, Error, Event, Exhausted, Gemini, Message, Native, OpenAiCompat, Provider,
  Request, Role, StopReason, ToolDef,
  oauth::{TokenFuture, TokenSource, Tokens},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
struct Recorded {
  head: String,
  body: Value,
  /// The body's bytes, for one that is not JSON (an upload).
  raw: Vec<u8>,
}

impl Recorded {
  fn header(&self, name: &str) -> Option<String> {
    self.head.lines().find_map(|l| {
      let (k, v) = l.split_once(':')?;
      k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
    })
  }
}

/// Serves `responses` in order, one per connection; returns the base URL and
/// what was received.
async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let addr = listener.local_addr().unwrap();
  let base = format!("http://{addr}");
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    for response in responses {
      // A response may link back to the server (`{BASE}`).
      let response = response.replace("{BASE}", &format!("http://{addr}"));
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 8192];
      let (head, body) = loop {
        let n = sock.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
          let head = String::from_utf8_lossy(&buf[..end]).into_owned();
          let len: usize = head
            .lines()
            .find_map(|l| {
              l.to_ascii_lowercase()
                .strip_prefix("content-length:")
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
      log.lock().unwrap().push(Recorded {
        head,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        raw: body,
      });
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

fn sse(events: &[(&str, Value)]) -> String {
  let mut body = String::new();
  for (name, data) in events {
    body.push_str(&format!("event: {name}\ndata: {data}\n\n"));
  }
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

fn status(code: u16, body: Value, extra: &str) -> String {
  let b = body.to_string();
  format!(
    "HTTP/1.1 {code} Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n{b}",
    b.len()
  )
}

fn anthropic(base: &str) -> Provider {
  let mut a = Anthropic::new("test-key");
  a.base_url = base.to_owned();
  Provider::Anthropic(a)
}

/// A response with a thinking block, text, and a tool call whose input
/// arrives in pieces.
fn tool_turn() -> String {
  sse(&[
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 120, "cache_read_input_tokens": 100, "output_tokens": 1}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Checking the "}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "weather."}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig=="}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    ("ping", json!({"type": "ping"})),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Let me "}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "check."}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"city\": \"Pa"}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "ris\"}"}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 2}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 42}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
}

#[tokio::test]
async fn anthropic_streams_thinking_text_and_a_tool_call() {
  let (base, seen) = serve(vec![tool_turn()]).await;
  let mut request = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![
      Content::Text("Weather in Paris? See the photo.".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![1, 2, 3],
      },
    ])],
  );
  request.system = Some("Be brief.".into());
  request.tools.push(ToolDef {
    name: "weather".into(),
    description: "Current weather".into(),
    input_schema: json!({"type": "object", "properties": {"city": {"type": "string"}}}),
  });
  let mut events = Vec::new();
  let r = anthropic(&base)
    .stream(&request, &mut |e| events.push(e))
    .await
    .unwrap();

  assert_eq!(r.text(), "Let me check.");
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(
    r.content[1],
    Content::ToolCall {
      id: "toolu_1".into(),
      name: "weather".into(),
      input: json!({"city": "Paris"}),
    }
  );
  assert_eq!(
    (
      r.usage.input_tokens,
      r.usage.output_tokens,
      r.usage.cache_read_tokens
    ),
    (120, 42, 100)
  );
  assert!(events.contains(&Event::Thinking("Checking the ".into())));
  assert!(events.contains(&Event::ToolCallStart {
    id: "toolu_1".into(),
    name: "weather".into()
  }));
  assert_eq!(
    events
      .iter()
      .filter(|e| matches!(e, Event::ToolCallInput { .. }))
      .count(),
    2
  );
  // The thinking block is kept, signature and all, to be sent back.
  let native = r.native.as_ref().unwrap();
  assert_eq!(
    native.content[0],
    json!({"type": "thinking", "thinking": "Checking the weather.", "signature": "sig=="})
  );
  assert_eq!(native.content[2]["input"], json!({"city": "Paris"}));

  let sent = seen.lock().unwrap()[0].clone();
  assert!(sent.head.starts_with("POST /v1/messages "));
  assert_eq!(sent.header("x-api-key").as_deref(), Some("test-key"));
  assert_eq!(
    sent.header("anthropic-version").as_deref(),
    Some("2023-06-01")
  );
  assert_eq!(
    sent.header("anthropic-beta").as_deref(),
    Some("thinking-binding-controls-2026-08-01,server-side-fallback-2026-07-01")
  );
  let b = &sent.body;
  assert_eq!(b["stream"], true);
  // The system prompt ends the part threads share, so it is cached.
  assert_eq!(
    b["system"],
    json!([{"type": "text", "text": "Be brief.", "cache_control": {"type": "ephemeral"}}])
  );
  assert_eq!(
    b["thinking"],
    json!({
      "type": "adaptive",
      "display": "summarized",
      // A thinking block whose prefix changed (the thread's tools did) is
      // dropped, not a 400.
      "block_binding": {"prefix_mismatch_behavior": "drop_block"},
    })
  );
  assert_eq!(b["output_config"], json!({"effort": "medium"}));
  assert_eq!(b["cache_control"], json!({"type": "ephemeral"}));
  assert_eq!(b["fallbacks"], "default");
  assert_eq!(b["tools"][0]["eager_input_streaming"], true);
  assert_eq!(
    b["messages"][0]["content"][1]["source"],
    json!({"type": "base64", "media_type": "image/png", "data": "AQID"})
  );
}

#[tokio::test]
async fn anthropic_sends_its_own_turns_back_unchanged() {
  let (base, seen) = serve(vec![tool_turn(), tool_turn(), tool_turn()]).await;
  let p = anthropic(&base);
  let first = p
    .stream(
      &Request::new(
        "claude-opus-5-5",
        vec![Message::user(vec![Content::Text("hi".into())])],
      ),
      &mut |_| {},
    )
    .await
    .unwrap();
  let history = vec![
    Message::user(vec![Content::Text("hi".into())]),
    Message {
      role: Role::Assistant,
      content: first.content.clone(),
      native: first.native.clone(),
    },
    Message::user(vec![Content::ToolResult {
      id: "toolu_1".into(),
      output: "18°C".into(),
      is_error: false,
    }]),
  ];
  p.stream(
    &Request::new("claude-opus-5-5", history.clone()),
    &mut |_| {},
  )
  .await
  .unwrap();
  // Another model gets the neutral content, without the thinking block.
  p.stream(&Request::new("claude-sonnet-5-5", history), &mut |_| {})
    .await
    .unwrap();

  let seen = seen.lock().unwrap();
  let same = &seen[1].body["messages"][1]["content"];
  assert_eq!(same, &first.native.as_ref().unwrap().content);
  let other = &seen[2].body["messages"][1]["content"];
  assert_eq!(other[0], json!({"type": "text", "text": "Let me check."}));
  assert_eq!(other.as_array().unwrap().len(), 2);
  assert_eq!(
    seen[1].body["messages"][2]["content"][0]["type"],
    "tool_result"
  );
}

#[tokio::test]
async fn anthropic_retries_overload_and_reports_refusals_and_stream_errors() {
  let overloaded = status(
    529,
    json!({"type": "error", "error": {"type": "overloaded_error", "message": "busy"}}),
    "retry-after: 0\r\n",
  );
  let refusal = sse(&[
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 5}}}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber"}}, "usage": {"output_tokens": 0}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ]);
  let broken = sse(&[
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "m", "usage": {}}}),
    ),
    (
      "error",
      json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
    ),
  ]);
  let bad_request = status(
    400,
    json!({"type": "error", "error": {"type": "invalid_request_error", "message": "max_tokens: too large"}}),
    "",
  );
  let (base, seen) = serve(vec![overloaded, refusal, broken, bad_request]).await;
  let p = anthropic(&base);
  let req = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![Content::Text("x".into())])],
  );

  let r = p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(
    r.stop,
    StopReason::Refusal {
      category: Some("cyber".into())
    }
  );
  assert_eq!(seen.lock().unwrap().len(), 2, "one retry after the 529");

  let err = p.stream(&req, &mut |_| {}).await.unwrap_err();
  assert!(
    matches!(&err, Error::Api { status: 529, kind, .. } if kind == "overloaded_error"),
    "{err}"
  );

  let err = p.stream(&req, &mut |_| {}).await.unwrap_err();
  assert!(
    matches!(&err, Error::Api { status: 400, message, .. } if message.contains("max_tokens")),
    "{err}"
  );
  assert!(!err.retryable());
}

#[tokio::test]
async fn openai_compatible_streams_text_tool_calls_and_usage() {
  let chunk = |v: Value| format!("data: {v}\n\n");
  let body = [
    chunk(json!({"model": "qwen3", "choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": "Hmm."}}]})),
    chunk(json!({"model": "qwen3", "choices": [{"index": 0, "delta": {"content": "Checking"}}]})),
    chunk(json!({"model": "qwen3", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "weather", "arguments": "{\"ci"}}]}}]})),
    chunk(json!({"model": "qwen3", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "ty\":\"Paris\"}"}}]}}]})),
    chunk(json!({"model": "qwen3", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]})),
    chunk(json!({"model": "qwen3", "choices": [], "usage": {"prompt_tokens": 30, "completion_tokens": 9, "prompt_tokens_details": {"cached_tokens": 12}}})),
    "data: [DONE]\n\n".to_owned(),
  ]
  .concat();
  let response = format!(
    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
  );
  let (base, seen) = serve(vec![response]).await;
  let p = Provider::OpenAi(OpenAiCompat::new(format!("{base}/v1"), None));
  let history = vec![
    Message::user(vec![
      Content::Text("Weather?".into()),
      Content::Image {
        media_type: "image/jpeg".into(),
        data: vec![9],
      },
      Content::Document {
        media_type: "application/pdf".into(),
        name: Some("lease.pdf".into()),
        data: vec![0; 10],
      },
    ]),
    Message {
      role: Role::Assistant,
      content: vec![
        Content::Text("Calling.".into()),
        Content::ToolCall {
          id: "call_0".into(),
          name: "weather".into(),
          input: json!({"city": "Rome"}),
        },
      ],
      native: Some(Native {
        provider: "anthropic".into(),
        model: "x".into(),
        content: json!([]),
      }),
    },
    Message::user(vec![Content::ToolResult {
      id: "call_0".into(),
      output: "20°C".into(),
      is_error: false,
    }]),
  ];
  let mut events = Vec::new();
  let r = p
    .stream(&Request::new("qwen3", history), &mut |e| events.push(e))
    .await
    .unwrap();
  assert_eq!(r.text(), "Checking");
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(
    r.content[1],
    Content::ToolCall {
      id: "call_1".into(),
      name: "weather".into(),
      input: json!({"city": "Paris"}),
    }
  );
  assert_eq!(
    (
      r.usage.input_tokens,
      r.usage.output_tokens,
      r.usage.cache_read_tokens
    ),
    // The prompt's 30 count the 12 cached.
    (18, 9, 12)
  );
  assert!(events.contains(&Event::Thinking("Hmm.".into())));

  let sent = seen.lock().unwrap()[0].clone();
  assert!(sent.head.starts_with("POST /v1/chat/completions "));
  assert_eq!(sent.header("authorization"), None);
  let m = &sent.body["messages"];
  assert_eq!(
    m[0]["content"][1]["image_url"]["url"],
    "data:image/jpeg;base64,CQ=="
  );
  assert!(
    m[0]["content"][2]["text"]
      .as_str()
      .unwrap()
      .contains("lease.pdf")
  );
  assert_eq!(m[1]["role"], "assistant");
  assert_eq!(m[1]["content"], "Calling.");
  assert_eq!(
    m[1]["tool_calls"][0]["function"]["arguments"],
    "{\"city\":\"Rome\"}"
  );
  assert_eq!(
    m[2],
    json!({"role": "tool", "tool_call_id": "call_0", "content": "20°C"})
  );
  assert_eq!(sent.body["stream_options"], json!({"include_usage": true}));
}

#[tokio::test]
async fn anthropic_subscription_headers_prompt_and_api_key_isolation() {
  let (base, seen) = serve(vec![tool_turn(), tool_turn()]).await;
  let mut p = Anthropic::new("sk-ant-oat-test");
  p.base_url = base.clone();
  let mut r = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![Content::Text("hi".into())])],
  );
  r.system = Some("Keep my instructions".into());
  p.stream(&r, &mut |_| {}).await.unwrap();
  p.api_key = "api-key".into();
  p.stream(&r, &mut |_| {}).await.unwrap();
  let seen = seen.lock().unwrap();
  let oauth = &seen[0];
  assert_eq!(
    oauth.header("authorization").as_deref(),
    Some("Bearer sk-ant-oat-test")
  );
  assert!(oauth.header("x-api-key").is_none());
  let beta = oauth.header("anthropic-beta").unwrap();
  for expected in [
    "claude-code-20250219",
    "oauth-2025-04-20",
    "thinking-binding-controls-2026-08-01",
    "server-side-fallback-2026-07-01",
  ] {
    assert!(beta.contains(expected));
  }
  assert_eq!(oauth.header("x-app").as_deref(), Some("cli"));
  assert_eq!(
    oauth.body["system"][0]["cache_control"]["type"],
    "ephemeral"
  );
  assert_eq!(oauth.body["system"][1]["text"], "Keep my instructions");
  assert_eq!(seen[1].header("x-api-key").as_deref(), Some("api-key"));
  assert!(seen[1].header("authorization").is_none());
  assert_eq!(seen[1].body["system"][0]["text"], "Keep my instructions");
  assert_eq!(seen[1].body["system"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn codex_responses_stream_and_replay_multimodal_and_tools() {
  use stitch_providers::Responses;
  let output = json!([
    {"type":"reasoning","id":"rs_1","encrypted_content":"opaque","summary":[]},
    {"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello","annotations":[]}]},
    {"type":"function_call","id":"fc_1","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Rome\"}"}
  ]);
  let terminal = json!({"type":"response.completed","response":{"status":"completed","model":"test-model","output":output,"usage":{"input_tokens":50,"output_tokens":10,"input_tokens_details":{"cached_tokens":20}}}});
  let response = sse(&[
    (
      "response.reasoning_summary_text.delta",
      json!({"type":"response.reasoning_summary_text.delta","delta":"Thinking"}),
    ),
    (
      "response.output_text.delta",
      json!({"type":"response.output_text.delta","delta":"Hello"}),
    ),
    (
      "response.output_item.added",
      json!({"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"weather"}}),
    ),
    (
      "response.function_call_arguments.delta",
      json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"city\":\"Rome\"}"}),
    ),
    ("response.completed", terminal),
  ]);
  let (base, seen) = serve(vec![response.clone(), response]).await;
  let mut p = Responses::codex("test-token".into(), "test-account".into());
  p.base_url = base;
  let mut r = Request::new(
    "test-model",
    vec![Message::user(vec![
      Content::Text("hi".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![1, 2],
      },
      Content::Document {
        media_type: "application/pdf".into(),
        name: Some("file.pdf".into()),
        data: vec![3, 4],
      },
    ])],
  );
  r.tools.push(ToolDef {
    name: "weather".into(),
    description: "Weather".into(),
    input_schema: json!({"type":"object"}),
  });
  let mut events = Vec::new();
  let reply = p.stream(&r, &mut |e| events.push(e)).await.unwrap();
  assert_eq!(reply.text(), "Hello");
  assert_eq!(reply.stop, StopReason::ToolUse);
  // The input's 50 count the 20 cached.
  assert_eq!(
    (reply.usage.input_tokens, reply.usage.cache_read_tokens),
    (30, 20)
  );
  assert!(events.contains(&Event::Thinking("Thinking".into())));
  assert!(events.contains(&Event::ToolCallInput {
    id: "call_1".into(),
    partial: "{\"city\":\"Rome\"}".into()
  }));
  r.messages.push(Message {
    role: Role::Assistant,
    content: reply.content,
    native: reply.native,
  });
  r.messages.push(Message::user(vec![Content::ToolResult {
    id: "call_1".into(),
    output: "sunny".into(),
    is_error: false,
  }]));
  p.stream(&r, &mut |_| {}).await.unwrap();
  let seen = seen.lock().unwrap();
  assert!(seen[0].head.starts_with("POST /responses "));
  assert_eq!(
    seen[0].header("chatgpt-account-id").as_deref(),
    Some("test-account")
  );
  assert_eq!(seen[0].body["store"], false);
  assert!(seen[0].body.get("max_output_tokens").is_none());
  assert_eq!(seen[0].body["tools"][0]["name"], "weather");
  assert_eq!(
    seen[0].body["input"][0]["content"][1]["image_url"],
    "data:image/png;base64,AQI="
  );
  assert_eq!(
    seen[0].body["input"][0]["content"][2]["filename"],
    "file.pdf"
  );
  assert_eq!(seen[1].body["input"][1], output[0]);
  assert_eq!(seen[1].body["input"][4]["type"], "function_call_output");
}

#[tokio::test]
async fn responses_truncation_and_failure_are_not_success() {
  use stitch_providers::Responses;
  for response in [
    sse(&[(
      "response.output_text.delta",
      json!({"type":"response.output_text.delta","delta":"partial"}),
    )]),
    sse(&[("response.failed", json!({"type":"response.failed"}))]),
  ] {
    let (base, _) = serve(vec![response]).await;
    let p = Responses::new(base, "key".into());
    assert!(
      p.stream(&Request::new("model", vec![]), &mut |_| {})
        .await
        .is_err()
    );
  }
}

fn ok(body: Value) -> String {
  status(200, body, "")
}

fn paths(seen: &Arc<Mutex<Vec<Recorded>>>) -> Vec<String> {
  seen
    .lock()
    .unwrap()
    .iter()
    .map(|r| {
      r.head
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
    })
    .collect()
}

#[tokio::test]
async fn context_windows_are_asked_where_each_server_tells() {
  // Anthropic: the model's `max_input_tokens`.
  let (base, seen) = serve(vec![ok(
    json!({"id": "claude-opus-5-5", "max_input_tokens": 1_000_000}),
  )])
  .await;
  let w = anthropic(&base)
    .context_window("claude-opus-5-5")
    .await
    .unwrap();
  assert_eq!(w, Some(1_000_000));
  assert_eq!(paths(&seen), ["GET /v1/models/claude-opus-5-5"]);

  let compat = |base: &str| Provider::OpenAi(OpenAiCompat::new(format!("{base}/v1"), None));
  // An older llama.cpp: its list tells what the model was trained for,
  // `/props` what it serves.
  let (base, seen) = serve(vec![
    ok(json!({"data": [{"id": "qwen3", "meta": {"n_ctx_train": 262144}}]})),
    ok(json!({"default_generation_settings": {"n_ctx": 8192}})),
  ])
  .await;
  assert_eq!(
    compat(&base).context_window("qwen3").await.unwrap(),
    Some(8192)
  );
  assert_eq!(paths(&seen), ["GET /v1/models", "GET /props"]);

  // Ollama, the model loaded: the window it runs with.
  let (base, seen) = serve(vec![
    ok(json!({"data": [{"id": "gemma4", "owned_by": "library"}]})),
    status(404, json!({"error": "not found"}), ""),
    ok(json!({"models": [{"name": "gemma4", "context_length": 4096}]})),
  ])
  .await;
  assert_eq!(
    compat(&base).context_window("gemma4").await.unwrap(),
    Some(4096)
  );
  assert_eq!(
    paths(&seen),
    ["GET /v1/models", "GET /props", "GET /api/ps"]
  );

  // Ollama, not loaded: the `num_ctx` it was made with.
  let (base, seen) = serve(vec![
    ok(json!({"data": [{"id": "gemma4"}]})),
    status(404, json!({"error": "not found"}), ""),
    ok(json!({"models": []})),
    ok(json!({
      "parameters": "temperature 1\nnum_ctx 16384",
      "model_info": {"general.architecture": "gemma4", "gemma4.context_length": 131072}
    })),
  ])
  .await;
  assert_eq!(
    compat(&base).context_window("gemma4").await.unwrap(),
    Some(16384)
  );
  assert_eq!(seen.lock().unwrap()[3].body, json!({"model": "gemma4"}));

  // OpenRouter: its list comes in pages, linked from the server's root.
  let (base, seen) = serve(vec![
    ok(json!({"data": [{"id": "a/b", "context_length": 8192}], "links": {"next": "/api/v1/models?offset=1"}})),
    ok(json!({"data": [{"id": "c/d", "context_length": 400000}], "links": {"next": null}})),
  ])
  .await;
  let p = Provider::OpenAi(OpenAiCompat::new(
    format!("{base}/api/v1"),
    Some("k".into()),
  ));
  assert_eq!(p.context_window("c/d").await.unwrap(), Some(400_000));
  assert_eq!(
    paths(&seen),
    ["GET /api/v1/models", "GET /api/v1/models?offset=1"]
  );
}

/// A token source that hands out `tokens` in turn (the last one again
/// when they run out) and counts the forced refreshes it is asked for.
struct Source {
  tokens: Mutex<Vec<&'static str>>,
  refreshes: Mutex<Vec<String>>,
}

impl Source {
  fn new(tokens: &[&'static str]) -> Arc<Self> {
    Arc::new(Source {
      tokens: Mutex::new(tokens.iter().rev().copied().collect()),
      refreshes: Mutex::new(Vec::new()),
    })
  }
}

impl TokenSource for Source {
  fn token(&self) -> TokenFuture<'_> {
    let mut t = self.tokens.lock().unwrap();
    let token = if t.len() > 1 {
      t.pop()
    } else {
      t.last().copied()
    };
    Box::pin(async move { Ok(token.unwrap().to_owned()) })
  }
  fn refreshed<'a>(&'a self, stale: &'a str) -> TokenFuture<'a> {
    self.refreshes.lock().unwrap().push(stale.to_owned());
    Box::pin(async move { Ok(format!("{stale}-refreshed")) })
  }
}

fn subscription(base: &str, source: &Arc<Source>) -> Provider {
  let mut a = Anthropic::new("start-token");
  a.base_url = base.to_owned();
  a.oauth = true;
  a.tokens = Some(Tokens(source.clone()));
  Provider::Anthropic(a)
}

fn unauthorized() -> String {
  status(
    401,
    json!({"type": "error", "error": {"type": "authentication_error", "message": "OAuth access token has expired"}}),
    "",
  )
}

fn bearers(seen: &Arc<Mutex<Vec<Recorded>>>) -> Vec<String> {
  seen
    .lock()
    .unwrap()
    .iter()
    .map(|r| r.header("authorization").unwrap())
    .collect()
}

#[tokio::test]
async fn subscription_tokens_are_asked_for_each_request() {
  // The token changes between two steps of a turn (its source refreshed
  // it): the second request sends the new one, not the turn's first.
  let (base, seen) = serve(vec![tool_turn(), tool_turn()]).await;
  let source = Source::new(&["first", "second"]);
  let p = subscription(&base, &source);
  let req = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![Content::Text("x".into())])],
  );
  p.stream(&req, &mut |_| {}).await.unwrap();
  p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(bearers(&seen), ["Bearer first", "Bearer second"]);
  assert!(source.refreshes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_401_refreshes_once_and_retries_once() {
  // Refused, refreshed, sent again: the step succeeds.
  let (base, seen) = serve(vec![unauthorized(), tool_turn()]).await;
  let source = Source::new(&["old"]);
  let p = subscription(&base, &source);
  let req = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![Content::Text("x".into())])],
  );
  let r = p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(bearers(&seen), ["Bearer old", "Bearer old-refreshed"]);
  assert_eq!(*source.refreshes.lock().unwrap(), ["old"]);

  // Refused again after the refresh: the error, with no third request.
  let (base, seen) = serve(vec![unauthorized(), unauthorized(), tool_turn()]).await;
  let source = Source::new(&["old"]);
  let p = subscription(&base, &source);
  let err = p.stream(&req, &mut |_| {}).await.unwrap_err();
  assert!(err.is_unauthorized(), "{err}");
  assert!(!err.retryable());
  assert_eq!(bearers(&seen), ["Bearer old", "Bearer old-refreshed"]);
  assert_eq!(source.refreshes.lock().unwrap().len(), 1);

  // A plain API key refused is not retried.
  let (base, seen) = serve(vec![unauthorized(), tool_turn()]).await;
  let err = anthropic(&base)
    .stream(&req, &mut |_| {})
    .await
    .unwrap_err();
  assert!(err.is_unauthorized(), "{err}");
  assert_eq!(seen.lock().unwrap().len(), 1);
}

// Gemini, native (`generateContent` over SSE, `models.list`, the Files API).

fn gemini(base: &str) -> Gemini {
  let mut g = Gemini::new("test-key");
  g.base_url = format!("{base}/v1beta");
  g.poll = std::time::Duration::from_millis(1);
  g
}

/// Gemini's stream: `data:` lines only.
fn data_sse(chunks: &[Value]) -> String {
  let body: String = chunks
    .iter()
    .map(|c| format!("data: {c}\r\n\r\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// `models.get` for a thinking model with a 1M window.
fn gemini_info(name: &str, output: u64) -> String {
  ok(json!({
    "name": format!("models/{name}"), "inputTokenLimit": 1_048_576, "outputTokenLimit": output,
    "thinking": true, "supportedGenerationMethods": ["generateContent"]
  }))
}

#[tokio::test]
async fn gemini_lists_chat_models_over_every_page() {
  let (base, seen) = serve(vec![
    ok(json!({"models": [
      {"name": "models/gemini-3.8-flash", "supportedGenerationMethods": ["generateContent"], "inputTokenLimit": 1_048_576},
      {"name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent"]},
      {"name": "models/gemini-3.8-live", "supportedGenerationMethods": ["bidiGenerateContent"]}
    ], "nextPageToken": "page/2=="})),
    ok(json!({"models": [
      {"name": "models/gemini-3.1-flash-image", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-3.8-flash-tts", "supportedGenerationMethods": ["generateContent"]}
    ]})),
  ])
  .await;
  let p = Provider::Gemini(gemini(&base));
  assert_eq!(
    p.models().await.unwrap(),
    ["gemini-3.1-flash-image", "gemini-3.8-flash"]
  );
  assert_eq!(
    paths(&seen),
    [
      "GET /v1beta/models?pageSize=1000",
      "GET /v1beta/models?pageSize=1000&pageToken=page%2F2%3D%3D"
    ]
  );
  let r = seen.lock().unwrap()[0].clone();
  assert_eq!(r.header("x-goog-api-key").as_deref(), Some("test-key"));
  // What the list said of a model is kept: no lookup for its window.
  assert_eq!(
    p.context_window("gemini-3.8-flash").await.unwrap(),
    Some(1_048_576)
  );
  assert_eq!(seen.lock().unwrap().len(), 2);

  // Without a key nothing is asked: the provider is "not listed".
  let p = Provider::Gemini(Gemini::new(""));
  assert!(matches!(p.models().await, Err(Error::NoKey("gemini"))));
  assert!(!p.has_key());
}

#[tokio::test]
async fn gemini_streams_thinking_text_images_and_a_tool_call() {
  let (base, seen) = serve(vec![
    gemini_info("gemini-3.8-flash", 32_768),
    data_sse(&[
      json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Thinking it over", "thought": true}]}}]}),
      json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Here is "}]}}]}),
      json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "a picture."}]}}]}),
      json!({"candidates": [{"content": {"role": "model", "parts": [
        {"inlineData": {"mimeType": "image/png", "data": "AQID"}, "thoughtSignature": "aW1n"},
        {"functionCall": {"name": "get_weather", "args": {"city": "Paris"}, "id": "c1"}, "thoughtSignature": "Y2FsbA=="}
      ]}, "finishReason": "STOP"}],
       "usageMetadata": {"promptTokenCount": 120, "cachedContentTokenCount": 100, "candidatesTokenCount": 9, "thoughtsTokenCount": 30}}),
    ]),
  ])
  .await;
  let p = Provider::Gemini(gemini(&base));
  let mut req = Request::new(
    "gemini-3.8-flash",
    vec![
      Message::user(vec![
        Content::Text("Weather and a picture?".into()),
        Content::Image {
          media_type: "image/jpeg".into(),
          data: vec![9, 9],
        },
        Content::Document {
          media_type: "audio/mpeg".into(),
          name: Some("note.mp3".into()),
          data: vec![1, 2, 3, 4],
        },
        Content::Document {
          media_type: "application/zip".into(),
          name: Some("a.zip".into()),
          data: vec![0],
        },
      ]),
      Message {
        role: Role::Assistant,
        content: vec![Content::ToolCall {
          id: "toolu_1".into(),
          name: "shell_run".into(),
          input: json!({"command": "date"}),
        }],
        native: None,
      },
      Message::user(vec![Content::ToolResult {
        id: "toolu_1".into(),
        output: "Sunday".into(),
        is_error: false,
      }]),
      Message::user(vec![Content::Text("Go on.".into())]),
    ],
  );
  req.system = Some("Be brief.".into());
  req.tools = vec![ToolDef {
    name: "get_weather".into(),
    description: "Weather.".into(),
    input_schema: json!({"type": "object", "additionalProperties": false, "properties": {"city": {"type": "string"}}}),
  }];
  let mut events = Vec::new();
  let r = p.stream(&req, &mut |e| events.push(e)).await.unwrap();

  let seen = seen.lock().unwrap().clone();
  assert_eq!(
    seen[0].head.lines().next().unwrap(),
    "GET /v1beta/models/gemini-3.8-flash HTTP/1.1"
  );
  let sent = &seen[1];
  assert_eq!(
    sent.head.lines().next().unwrap(),
    "POST /v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse HTTP/1.1"
  );
  assert_eq!(sent.header("x-goog-api-key").as_deref(), Some("test-key"));
  let b = &sent.body;
  assert_eq!(
    b["systemInstruction"],
    json!({"parts": [{"text": "Be brief."}]})
  );
  assert_eq!(b["generationConfig"]["maxOutputTokens"], 32_768);
  assert_eq!(
    b["generationConfig"]["thinkingConfig"],
    json!({"includeThoughts": true})
  );
  assert!(b["generationConfig"].get("responseModalities").is_none());
  assert!(b["generationConfig"].get("temperature").is_none());
  assert_eq!(
    b["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"],
    req.tools[0].input_schema
  );
  let contents = b["contents"].as_array().unwrap();
  assert_eq!(
    contents
      .iter()
      .map(|c| c["role"].as_str().unwrap())
      .collect::<Vec<_>>(),
    ["user", "model", "user"]
  );
  assert_eq!(
    contents[0]["parts"],
    json!([
      {"text": "Weather and a picture?"},
      {"inlineData": {"mimeType": "image/jpeg", "data": "CQk="}},
      {"inlineData": {"mimeType": "audio/mpeg", "data": "AQIDBA=="}},
      {"text": "[Attached `a.zip` (application/zip, 1 bytes): this model cannot read it.]"}
    ])
  );
  // Another model's call carries the signature that skips the check.
  assert_eq!(
    contents[1]["parts"],
    json!([{"functionCall": {"name": "shell_run", "args": {"command": "date"}, "id": "toolu_1"},
            "thoughtSignature": "skip_thought_signature_validator"}])
  );
  // Results go in a user turn, named; the next message joins it.
  assert_eq!(
    contents[2]["parts"],
    json!([
      {"functionResponse": {"name": "shell_run", "id": "toolu_1", "response": {"output": "Sunday"}}},
      {"text": "Go on."}
    ])
  );

  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(r.text(), "Here is a picture.");
  assert_eq!(
    r.content,
    vec![
      Content::Text("Here is a picture.".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![1, 2, 3]
      },
      Content::ToolCall {
        id: "c1".into(),
        name: "get_weather".into(),
        input: json!({"city": "Paris"})
      },
    ]
  );
  assert_eq!(r.usage.input_tokens, 20);
  assert_eq!(r.usage.cache_read_tokens, 100);
  assert_eq!(r.usage.output_tokens, 39);
  assert_eq!(events[0], Event::Thinking("Thinking it over".into()));
  assert!(events.contains(&Event::ToolCallStart {
    id: "c1".into(),
    name: "get_weather".into()
  }));
  // The image is kept without its bytes; signatures as they came.
  let native = r.native.clone().unwrap();
  assert_eq!(native.provider, "gemini");
  assert_eq!(
    native.content["parts"],
    json!([
      {"text": "Thinking it over", "thought": true},
      {"text": "Here is a picture."},
      {"inlineData": {"mimeType": "image/png"}, "thoughtSignature": "aW1n"},
      {"functionCall": {"name": "get_weather", "args": {"city": "Paris"}, "id": "c1"}, "thoughtSignature": "Y2FsbA=="}
    ])
  );
}

#[tokio::test]
async fn gemini_replays_its_own_turns_and_makes_images_on_image_models() {
  let reply = data_sse(&[
    json!({"candidates": [{"content": {"parts": [{"text": "OK"}]}, "finishReason": "STOP"}]}),
  ]);
  let missing = status(
    404,
    json!({"error": {"code": 404, "message": "not found", "status": "NOT_FOUND"}}),
    "",
  );
  let (base, seen) = serve(vec![
    missing.clone(),
    reply.clone(),
    missing,
    reply.clone(),
    reply,
  ])
  .await;
  let p = Provider::Gemini(gemini(&base));
  let native = Native {
    provider: "gemini".into(),
    model: "gemini-3.1-flash-image".into(),
    content: json!({"parts": [
      {"text": "A circle."},
      {"inlineData": {"mimeType": "image/png"}, "thoughtSignature": "aW1n"}
    ]}),
  };
  let turn = Message {
    role: Role::Assistant,
    content: vec![
      Content::Text("A circle.".into()),
      Content::Text("[image img:1234]".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![7, 7, 7],
      },
    ],
    native: Some(native),
  };
  let mut req = Request::new(
    "gemini-3.1-flash-image",
    vec![
      Message::user(vec![Content::Text("Draw a circle.".into())]),
      turn,
      Message::user(vec![Content::Text("Make it blue.".into())]),
    ],
  );
  req.tools = vec![ToolDef {
    name: "t".into(),
    description: "d".into(),
    input_schema: json!({"type": "object"}),
  }];
  let r = p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(r.text(), "OK");
  // No signature, no image: nothing of its own to keep.
  assert!(r.native.is_none());
  let b = seen.lock().unwrap()[1].body.clone();
  assert_eq!(
    b["contents"][1]["parts"],
    json!([
      {"text": "A circle."},
      {"inlineData": {"mimeType": "image/png", "data": "BwcH"}, "thoughtSignature": "aW1n"}
    ])
  );
  assert_eq!(
    b["generationConfig"]["responseModalities"],
    json!(["TEXT", "IMAGE"])
  );
  // An image model with no word on thinking is not asked to think, nor
  // given tools; with no output limit known, none is sent.
  assert!(b["generationConfig"].get("thinkingConfig").is_none());
  assert!(b["generationConfig"].get("maxOutputTokens").is_none());
  assert!(b.get("tools").is_none());

  // On another model the turn is sent from its content: its image with
  // the line naming it, no signature.
  req.model = "gemini-3.1-flash-image-preview".into();
  p.stream(&req, &mut |_| {}).await.unwrap();
  let b = seen.lock().unwrap()[3].body.clone();
  assert_eq!(
    b["contents"][1]["parts"],
    json!([
      {"text": "A circle."},
      {"text": "[image img:1234]"},
      {"inlineData": {"mimeType": "image/png", "data": "BwcH"}}
    ])
  );
  assert_eq!(
    paths(&seen),
    [
      "GET /v1beta/models/gemini-3.1-flash-image",
      "POST /v1beta/models/gemini-3.1-flash-image:streamGenerateContent?alt=sse",
      "GET /v1beta/models/gemini-3.1-flash-image-preview",
      "POST /v1beta/models/gemini-3.1-flash-image-preview:streamGenerateContent?alt=sse",
    ]
  );

  // Image models take no function calls, even in the history: the calls
  // another model made are told as text.
  let req = Request::new(
    "gemini-3.1-flash-image",
    vec![
      Message {
        role: Role::Assistant,
        content: vec![Content::ToolCall {
          id: "toolu_1".into(),
          name: "shell_run".into(),
          input: json!({"command": "date"}),
        }],
        native: None,
      },
      Message::user(vec![
        Content::ToolResult {
          id: "toolu_1".into(),
          output: "Sunday".into(),
          is_error: false,
        },
        Content::Text("Draw it.".into()),
      ]),
    ],
  );
  p.stream(&req, &mut |_| {}).await.unwrap();
  let b = seen.lock().unwrap()[4].body.clone();
  assert_eq!(
    b["contents"],
    json!([
      {"role": "model", "parts": [{"text": "[Called the tool `shell_run` with {\"command\":\"date\"}]"}]},
      {"role": "user", "parts": [{"text": "[The result of `shell_run`: Sunday]"}, {"text": "Draw it."}]}
    ])
  );
}

#[tokio::test]
async fn gemini_refusals_and_errors() {
  let (base, _) = serve(vec![
    gemini_info("gemini-3.8-flash", 65_536),
    data_sse(&[json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}})]),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "No"}]}, "finishReason": "IMAGE_SAFETY"}]})]),
    status(
      400,
      json!({"error": {"code": 400, "message": "Bad schema", "status": "INVALID_ARGUMENT"}}),
      "",
    ),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "Half"}]}}]})]),
  ])
  .await;
  let p = Provider::Gemini(gemini(&base));
  let req = Request::new(
    "gemini-3.8-flash",
    vec![Message::user(vec![Content::Text("Hi".into())])],
  );
  let r = p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(
    r.stop,
    StopReason::Refusal {
      category: Some("PROHIBITED_CONTENT".into())
    }
  );
  let r = p.stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(
    r.stop,
    StopReason::Refusal {
      category: Some("IMAGE_SAFETY".into())
    }
  );
  match p.stream(&req, &mut |_| {}).await {
    Err(Error::Api {
      status: 400, kind, ..
    }) => assert_eq!(kind, "INVALID_ARGUMENT"),
    other => panic!("{other:?}"),
  }
  // A stream that ends with no finish reason was cut.
  assert!(p.stream(&req, &mut |_| {}).await.unwrap_err().is_network());
}

#[tokio::test]
async fn gemini_sends_large_media_through_the_files_api_once() {
  let video: Vec<u8> = (0..64u8).collect();
  let (base, seen) = serve(vec![
    gemini_info("gemini-3.8-flash", 65_536),
    // Upload: start, then the bytes, then the file processed.
    "HTTP/1.1 200 OK\r\nx-goog-upload-url: {BASE}/upload-session/1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
      .into(),
    ok(json!({"file": {"name": "files/abc", "uri": "https://files/abc", "mimeType": "video/mp4", "state": "PROCESSING"}})),
    ok(json!({"name": "files/abc", "uri": "https://files/abc", "mimeType": "video/mp4", "state": "ACTIVE"})),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "A video."}]}, "finishReason": "STOP"}]})]),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "Again."}]}, "finishReason": "STOP"}]})]),
  ])
  .await;
  let mut g = gemini(&base);
  g.inline_max = 16;
  let p = Provider::Gemini(g);
  let req = Request::new(
    "gemini-3.8-flash",
    vec![Message::user(vec![
      Content::Document {
        media_type: "video/mp4".into(),
        name: Some("clip.mp4".into()),
        data: video.clone(),
      },
      Content::Text("What happens?".into()),
    ])],
  );
  assert_eq!(
    p.stream(&req, &mut |_| {}).await.unwrap().text(),
    "A video."
  );
  // The same file again is not uploaded again.
  assert_eq!(p.stream(&req, &mut |_| {}).await.unwrap().text(), "Again.");
  assert_eq!(
    paths(&seen),
    [
      "GET /v1beta/models/gemini-3.8-flash",
      "POST /upload/v1beta/files",
      "POST /upload-session/1",
      "GET /v1beta/files/abc",
      "POST /v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse",
      "POST /v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse",
    ]
  );
  let seen = seen.lock().unwrap().clone();
  let start = &seen[1];
  assert_eq!(
    start.header("x-goog-upload-protocol").as_deref(),
    Some("resumable")
  );
  assert_eq!(
    start
      .header("x-goog-upload-header-content-length")
      .as_deref(),
    Some("64")
  );
  assert_eq!(
    start.header("x-goog-upload-header-content-type").as_deref(),
    Some("video/mp4")
  );
  assert_eq!(start.body, json!({"file": {"display_name": "clip.mp4"}}));
  let upload = &seen[2];
  assert_eq!(
    upload.header("x-goog-upload-command").as_deref(),
    Some("upload, finalize")
  );
  assert_eq!(upload.header("x-goog-api-key").as_deref(), Some("test-key"));
  assert_eq!(upload.raw, video);
  for sent in &seen[4..] {
    assert_eq!(
      sent.body["contents"][0]["parts"][0],
      json!({"fileData": {"mimeType": "video/mp4", "fileUri": "https://files/abc"}})
    );
  }
}

#[tokio::test]
async fn files_are_read_only_by_gemini_and_uploaded_once_by_their_path() {
  let dir = std::env::temp_dir().join(format!("stitch-gemini-file-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let path = dir.join("blob-of-a-video");
  let video: Vec<u8> = (0..64u8).rev().collect();
  std::fs::write(&path, &video).unwrap();
  let file = Content::File {
    media_type: "video/mp4".into(),
    name: Some("clip.mp4".into()),
    path: path.clone(),
    bytes: 64,
  };
  let (base, seen) = serve(vec![
    gemini_info("gemini-3.7-flash", 65_536),
    "HTTP/1.1 200 OK\r\nx-goog-upload-url: {BASE}/upload-session/2\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
      .into(),
    ok(json!({"file": {"name": "files/f2", "uri": "https://files/f2", "mimeType": "video/mp4", "state": "ACTIVE"}})),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "One."}]}, "finishReason": "STOP"}]})]),
    data_sse(&[json!({"candidates": [{"content": {"parts": [{"text": "Two."}]}, "finishReason": "STOP"}]})]),
  ])
  .await;
  let mut g = gemini(&base);
  g.inline_max = 16;
  let p = Provider::Gemini(g);
  let req = Request::new(
    "gemini-3.7-flash",
    vec![Message::user(vec![
      file.clone(),
      Content::Text("And?".into()),
    ])],
  );
  assert_eq!(p.stream(&req, &mut |_| {}).await.unwrap().text(), "One.");
  // Uploaded by its path: the file is not read again (it is gone).
  std::fs::remove_file(&path).unwrap();
  assert_eq!(p.stream(&req, &mut |_| {}).await.unwrap().text(), "Two.");
  let seen = seen.lock().unwrap().clone();
  assert_eq!(seen[2].raw, video);
  assert_eq!(
    seen[4].body["contents"][0]["parts"][0],
    json!({"fileData": {"mimeType": "video/mp4", "fileUri": "https://files/f2"}})
  );

  // Every other adapter names it, never reading it.
  let (base, seen) = serve(vec![sse(&[
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 3}}}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])])
  .await;
  let req = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![file, Content::Text("And?".into())])],
  );
  anthropic(&base).stream(&req, &mut |_| {}).await.unwrap();
  assert_eq!(
    seen.lock().unwrap()[0].body["messages"][0]["content"][0]["text"],
    "[Attached `clip.mp4` (video/mp4, 64 bytes): this model cannot read it.]"
  );
  let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn images_made_for_a_gemini_turn_follow_its_function_responses() {
  let (base, seen) = serve(vec![
    gemini_info("gemini-3.6-flash", 65_536),
    data_sse(&[
      json!({"candidates": [{"content": {"parts": [{"text": "Nice."}]}, "finishReason": "STOP"}]}),
    ]),
  ])
  .await;
  let p = Provider::Gemini(gemini(&base));
  // A turn that called generate_image, replayed from its own parts; the
  // image the tool made was added to it, with its line.
  let turn = Message {
    role: Role::Assistant,
    content: vec![
      Content::ToolCall {
        id: "c9".into(),
        name: "generate_image".into(),
        input: json!({"prompt": "a cat"}),
      },
      Content::Text("[image img:abc `generated-1.png`]".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: vec![4, 4],
      },
    ],
    native: Some(Native {
      provider: "gemini".into(),
      model: "gemini-3.6-flash".into(),
      content: json!({"parts": [{"functionCall": {"name": "generate_image", "args": {"prompt": "a cat"}, "id": "c9"}, "thoughtSignature": "c2ln"}]}),
    }),
  };
  let req = Request::new(
    "gemini-3.6-flash",
    vec![
      Message::user(vec![Content::Text("Draw a cat.".into())]),
      turn,
      Message::user(vec![Content::ToolResult {
        id: "c9".into(),
        output: "Made.".into(),
        is_error: false,
      }]),
    ],
  );
  p.stream(&req, &mut |_| {}).await.unwrap();
  let b = seen.lock().unwrap()[1].body.clone();
  assert_eq!(
    b["contents"][1]["parts"],
    json!([{"functionCall": {"name": "generate_image", "args": {"prompt": "a cat"}, "id": "c9"}, "thoughtSignature": "c2ln"}])
  );
  assert_eq!(
    b["contents"][2]["parts"],
    json!([
      {"functionResponse": {"name": "generate_image", "id": "c9", "response": {"output": "Made."}}},
      {"text": "[The images made for the model's turn above:]"},
      {"text": "[image img:abc `generated-1.png`]"},
      {"inlineData": {"mimeType": "image/png", "data": "BAQ="}}
    ])
  );
}

#[tokio::test]
async fn an_account_out_of_usage_is_not_retried() {
  // A Claude subscription's limit: one request, not four with backoff
  // (which looked like a frozen chat, 2026-10-22).
  let limit = status(
    429,
    json!({"type": "error", "error": {"type": "rate_limit_error", "message": "This request would exceed your account's rate limit. Please try again later."}}),
    "anthropic-ratelimit-unified-status: rejected\r\nanthropic-ratelimit-unified-reset: 1800003600\r\nanthropic-ratelimit-unified-representative-claim: five_hour\r\nretry-after: 0\r\n",
  );
  let credit = status(
    400,
    json!({"type": "error", "error": {"type": "invalid_request_error", "message": "Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}),
    "",
  );
  let (base, seen) = serve(vec![limit, credit]).await;
  let p = anthropic(&base);
  let req = Request::new(
    "claude-opus-5-5",
    vec![Message::user(vec![Content::Text("x".into())])],
  );
  let err = p.stream(&req, &mut |_| {}).await.unwrap_err();
  assert!(
    matches!(
      &err,
      Error::OutOfUsage { status: 429, what: Exhausted::Limit, resets_at: Some(1_800_003_600), window: Some(w), .. } if w == "five_hour"
    ),
    "{err:?}"
  );
  assert_eq!(seen.lock().unwrap().len(), 1, "not retried");
  let err = p.stream(&req, &mut |_| {}).await.unwrap_err();
  assert!(
    matches!(
      &err,
      Error::OutOfUsage {
        status: 400,
        what: Exhausted::Credit,
        resets_at: None,
        ..
      }
    ),
    "{err:?}"
  );
  assert!(!err.retryable() && !err.is_network());
}

/// Kimi's turn on Moonshot's Messages endpoint: thinking (signed), text,
/// a tool call; Moonshot's usage, a write cached for an hour.
fn kimi_turn() -> String {
  sse(&[
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "kimi-k3.1-pro", "usage": {"input_tokens": 50, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 400, "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 400}, "output_tokens": 1}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Look first."}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "k-sig"}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "tool_1", "name": "view_file", "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"path\":\"a.png\"}"}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 30}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
}

#[tokio::test]
async fn kimi_gets_none_of_claudes_fields_and_its_thinking_back() {
  let (base, seen) = serve(vec![kimi_turn(), kimi_turn()]).await;
  let mut k = Anthropic::moonshot("sk-moon", format!("{base}/anthropic"));
  k.cache_ttl = Some("1h".into());
  let p = Provider::Anthropic(k);
  assert_eq!(p.label(), "Moonshot (API key)");
  assert!(!p.is_subscription());
  let tool = ToolDef {
    name: "view_file".into(),
    description: "d".into(),
    input_schema: json!({"type": "object"}),
  };
  let mut first = Request::new(
    "kimi-k3.1-pro",
    vec![Message::user(vec![
      Content::Text("Look.".into()),
      Content::Image {
        media_type: "image/png".into(),
        data: b"\x89PNG\r\n\x1a\n0000".to_vec(),
      },
      Content::Document {
        media_type: "application/pdf".into(),
        name: Some("lease.pdf".into()),
        data: vec![0; 10],
      },
    ])],
  );
  first.system = Some("Be brief.".into());
  first.tools.push(tool);
  let r = p.stream(&first, &mut |_| {}).await.unwrap();
  assert_eq!(r.usage.cache_write_tokens, 400);
  assert_eq!(r.usage.cache_write_1h_tokens, 400);
  let mut next = first.clone();
  next.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: r.native.clone(),
  });
  next.messages.push(Message::user(vec![Content::ToolResult {
    id: "tool_1".into(),
    output: "seen".into(),
    is_error: false,
  }]));
  p.stream(&next, &mut |_| {}).await.unwrap();

  let seen = seen.lock().unwrap();
  let (req, b) = (&seen[0], &seen[0].body);
  assert!(req.head.starts_with("POST /anthropic/v1/messages"));
  assert_eq!(
    req.header("authorization").as_deref(),
    Some("Bearer sk-moon")
  );
  assert!(req.header("x-api-key").is_none());
  assert!(req.header("anthropic-beta").is_none());
  // None of Claude's own fields.
  assert!(b.get("thinking").is_none());
  assert!(b.get("fallbacks").is_none());
  assert!(b["tools"][0].get("eager_input_streaming").is_none());
  assert!(b["tools"][0].get("cache_control").is_none());
  assert!(b["system"][0].get("cache_control").is_none());
  assert_eq!(b["system"].as_array().unwrap().len(), 1);
  // Its effort (`medium` is its `high`), its one cache breakpoint.
  assert_eq!(b["output_config"], json!({"effort": "high"}));
  assert_eq!(
    b["cache_control"],
    json!({"type": "ephemeral", "ttl": "1h"})
  );
  // An image as it is, a PDF as a note.
  let content = &b["messages"][0]["content"];
  assert_eq!(content[1]["type"], "image");
  assert_eq!(content[2]["type"], "text");
  assert!(content[2]["text"].as_str().unwrap().contains("lease.pdf"));
  // Its signed thinking goes back with its turn.
  let back = &seen[1].body["messages"][1]["content"];
  assert_eq!(
    back[0],
    json!({"type": "thinking", "thinking": "Look first.", "signature": "k-sig"})
  );
  assert_eq!(back[1]["type"], "tool_use");
}

#[tokio::test]
async fn kimi_lists_its_models_and_windows_from_its_v1_list() {
  let list = ok(json!({"object": "list", "data": [
    {"id": "kimi-k3.1-pro", "object": "model", "context_length": 1048576, "supports_image_in": true},
    {"id": "kimi-k2.6", "object": "model", "context_length": 262144},
  ]}));
  let (base, seen) = serve(vec![list.clone(), list]).await;
  let mut k = Anthropic::moonshot("sk-moon", format!("{base}/anthropic"));
  k.effort = Some("max".into());
  let p = Provider::Anthropic(k);
  assert_eq!(
    p.models().await.unwrap(),
    vec!["kimi-k2.6".to_owned(), "kimi-k3.1-pro".to_owned()]
  );
  assert_eq!(p.context_window("kimi-k2.6").await.unwrap(), Some(262_144));
  assert_eq!(paths(&seen), vec!["GET /v1/models", "GET /v1/models"]);
}

fn deepseek_turn(reasoning: &str) -> String {
  let chunk = |v: Value| format!("data: {v}\n\n");
  let body = [
    chunk(json!({"model": "deepseek-flash", "choices": [{"index": 0, "delta": {"role": "assistant", "reasoning_content": reasoning}}]})),
    chunk(json!({"model": "deepseek-flash", "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {"name": "get_date", "arguments": "{}"}}]}}]})),
    chunk(json!({"model": "deepseek-flash", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]})),
    chunk(json!({"model": "deepseek-flash", "choices": [], "usage": {"prompt_tokens": 100, "completion_tokens": 9, "prompt_cache_hit_tokens": 64, "prompt_cache_miss_tokens": 36}})),
    "data: [DONE]\n\n".to_owned(),
  ]
  .concat();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

#[tokio::test]
async fn deepseek_gets_its_reasoning_back_and_other_models_turns_an_empty_one() {
  let (base, seen) = serve(vec![
    deepseek_turn("Need the date."),
    deepseek_turn("Again."),
  ])
  .await;
  let mut d = OpenAiCompat::new(base.clone(), Some("sk-ds".into()));
  d.reasoning_back = true;
  d.effort = Some("max".into());
  let p = Provider::OpenAi(d);
  let tool = ToolDef {
    name: "get_date".into(),
    description: "d".into(),
    input_schema: json!({"type": "object"}),
  };
  // A thread begun on another model (Claude), then DeepSeek's turn.
  let claude_turn = Message {
    role: Role::Assistant,
    content: vec![Content::ToolCall {
      id: "toolu_0".into(),
      name: "get_date".into(),
      input: json!({}),
    }],
    native: Some(Native {
      provider: "anthropic".into(),
      model: "claude-opus-5-5".into(),
      content: json!([{"type": "thinking", "thinking": "Claude's.", "signature": "s"}]),
    }),
  };
  let mut first = Request::new(
    "deepseek-flash",
    vec![
      Message::user(vec![Content::Text("Date?".into())]),
      claude_turn,
      Message::user(vec![Content::ToolResult {
        id: "toolu_0".into(),
        output: "2026-10-29".into(),
        is_error: false,
      }]),
    ],
  );
  first.tools.push(tool);
  let r = p.stream(&first, &mut |_| {}).await.unwrap();
  // The prompt not read from the cache, as everywhere.
  assert_eq!((r.usage.input_tokens, r.usage.cache_read_tokens), (36, 64));
  let native = r.native.clone().unwrap();
  assert_eq!(
    (native.provider.as_str(), native.model.as_str()),
    ("openai", "deepseek-flash")
  );
  let mut next = first.clone();
  next.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: r.native.clone(),
  });
  next.messages.push(Message::user(vec![Content::ToolResult {
    id: "call_1".into(),
    output: "2026-10-29".into(),
    is_error: false,
  }]));
  p.stream(&next, &mut |_| {}).await.unwrap();

  let seen = seen.lock().unwrap();
  assert_eq!(seen[0].body["reasoning_effort"], "max");
  let msgs = seen[1].body["messages"].as_array().unwrap();
  let assistants: Vec<&Value> = msgs.iter().filter(|m| m["role"] == "assistant").collect();
  // Another model's turn: an empty reasoning, never Claude's own.
  assert_eq!(assistants[0]["reasoning_content"], "");
  // Its own turn: its reasoning, as it came.
  assert_eq!(assistants[1]["reasoning_content"], "Need the date.");
  assert_eq!(assistants[1]["tool_calls"][0]["id"], "call_1");
}

#[tokio::test]
async fn other_compatible_servers_get_no_reasoning_back() {
  let (base, seen) = serve(vec![deepseek_turn("Hmm."), deepseek_turn("Hmm.")]).await;
  let p = Provider::OpenAi(OpenAiCompat::new(format!("{base}/v1"), None));
  let first = Request::new(
    "qwen3",
    vec![Message::user(vec![Content::Text("Date?".into())])],
  );
  let r = p.stream(&first, &mut |_| {}).await.unwrap();
  // Not kept where it does not go back.
  assert!(r.native.is_none());
  let mut next = first.clone();
  next.messages.push(Message {
    role: Role::Assistant,
    content: r.content,
    native: None,
  });
  p.stream(&next, &mut |_| {}).await.unwrap();
  let seen = seen.lock().unwrap();
  assert!(seen[0].body.get("reasoning_effort").is_none());
  assert!(
    seen[1].body["messages"][1]
      .get("reasoning_content")
      .is_none()
  );
}

/// An OpenRouter turn: Claude's reasoning streamed as text and as details
/// (the signature last), an image, a tool call, then the usage with what
/// it cost.
fn openrouter_turn(model: &str) -> String {
  let chunk = |v: Value| format!("data: {v}\n\n");
  let png = "iVBORw0KGgo=";
  let body = [
    ": OPENROUTER PROCESSING\n\n".to_owned(),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {"role": "assistant", "reasoning": "Need ",
      "reasoning_details": [{"type": "reasoning.text", "text": "Need ", "signature": null, "format": "anthropic-claude-v1", "index": 0}]}}]})),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {"reasoning": "the date.",
      "reasoning_details": [{"type": "reasoning.text", "text": "the date.", "format": "anthropic-claude-v1", "index": 0}]}}]})),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {
      "reasoning_details": [{"type": "reasoning.text", "text": "", "signature": "sig-1", "format": "anthropic-claude-v1", "index": 0}]}}]})),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {"content": "Here.",
      "images": [{"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{png}")}}]}}]})),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "toolu_1", "type": "function", "function": {"name": "get_date", "arguments": "{}"}}]}}]})),
    chunk(json!({"model": model, "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]})),
    chunk(json!({"model": model, "choices": [], "usage": {"prompt_tokens": 1000, "completion_tokens": 20,
      "prompt_tokens_details": {"cached_tokens": 800, "cache_write_tokens": 150}, "cost": 0.00123}})),
    "data: [DONE]\n\n".to_owned(),
  ]
  .concat();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

fn openrouter(base: &str) -> OpenAiCompat {
  let mut p = OpenAiCompat::new(format!("{base}/api/v1"), Some("sk-or-v1-k".into()));
  p.openrouter = true;
  p.router = Some("openrouter".into());
  p
}

#[tokio::test]
async fn openrouter_maps_reasoning_caching_files_images_and_cost() {
  let claude = "anthropic/claude-sonnet-5.5";
  let (base, seen) = serve(vec![
    openrouter_turn(claude),
    openrouter_turn(claude),
    openrouter_turn("openai/gpt-5.6-sol"),
  ])
  .await;
  let mut o = openrouter(&base);
  o.effort = Some("high".into());
  o.cache_ttl = Some("1h".into());
  let p = Provider::OpenAi(o);
  let mut first = Request::new(
    claude,
    vec![Message::user(vec![
      Content::Text("Date? See the PDF.".into()),
      Content::Document {
        media_type: "application/pdf".into(),
        name: Some("cal.pdf".into()),
        data: b"%PDF-1.7".to_vec(),
      },
    ])],
  );
  first.system = Some("Be brief.".into());
  first.cache_key = Some("thread-1".into());
  first.tools.push(ToolDef {
    name: "get_date".into(),
    description: "d".into(),
    input_schema: json!({"type": "object"}),
  });
  let mut thinking = String::new();
  let r = p
    .stream(&first, &mut |e| {
      if let Event::Thinking(t) = e {
        thinking.push_str(&t);
      }
    })
    .await
    .unwrap();
  assert_eq!(thinking, "Need the date.");
  // The prompt not read from or written to the cache; what it cost.
  assert_eq!(
    (
      r.usage.input_tokens,
      r.usage.cache_read_tokens,
      r.usage.cache_write_tokens,
      r.usage.output_tokens
    ),
    (50, 800, 150, 20)
  );
  assert_eq!(r.cost, Some(0.00123));
  assert_eq!(r.stop, StopReason::ToolUse);
  assert_eq!(p.label(), "OpenRouter (API key)");
  // Named with the provider, never as Anthropic's own.
  assert_eq!(r.model, "openrouter/anthropic/claude-sonnet-5.5");
  // Text, the image it made, the call.
  assert_eq!(r.content[0], Content::Text("Here.".into()));
  assert!(
    matches!(&r.content[1], Content::Image { media_type, data } if media_type == "image/png" && data.starts_with(b"\x89PNG"))
  );
  assert!(matches!(&r.content[2], Content::ToolCall { id, .. } if id == "toolu_1"));
  // Its details gathered by index: the text joined, the signature kept.
  let native = r.native.clone().unwrap();
  assert_eq!(
    (native.provider.as_str(), native.model.as_str()),
    ("openai", claude)
  );
  assert_eq!(
    native.content,
    json!({"reasoning_details": [{"type": "reasoning.text", "text": "Need the date.", "signature": "sig-1", "format": "anthropic-claude-v1", "index": 0}]})
  );

  // The same model's next turn gets them back as they came.
  let mut next = first.clone();
  next.messages.push(Message {
    role: Role::Assistant,
    content: r.content.clone(),
    native: r.native.clone(),
  });
  next.messages.push(Message::user(vec![Content::ToolResult {
    id: "toolu_1".into(),
    output: "2026-11-17".into(),
    is_error: false,
  }]));
  p.stream(&next, &mut |_| {}).await.unwrap();
  // Another model gets none of Claude's, nor Claude's breakpoints.
  let mut other = next.clone();
  other.model = "openai/gpt-5.6-sol".into();
  p.stream(&other, &mut |_| {}).await.unwrap();

  let seen = seen.lock().unwrap();
  let r0 = &seen[0];
  assert!(r0.head.starts_with("POST /api/v1/chat/completions"));
  assert_eq!(
    r0.header("authorization").as_deref(),
    Some("Bearer sk-or-v1-k")
  );
  assert_eq!(
    r0.header("http-referer").as_deref(),
    Some("https://github.com/sinkingsugar/stitch")
  );
  assert_eq!(r0.header("x-openrouter-title").as_deref(), Some("Stitch"));
  assert_eq!(
    r0.header("x-openrouter-categories").as_deref(),
    Some("cli-agent,personal-agent")
  );
  // Public in OpenRouter's rankings: no `hidden` visibility (the default).
  assert_eq!(r0.header("x-openrouter-app-visibility"), None);
  let b = &r0.body;
  assert_eq!(b["model"], claude);
  assert_eq!(b["reasoning"], json!({"effort": "high"}));
  assert!(b.get("reasoning_effort").is_none());
  assert_eq!(b["session_id"], "thread-1");
  assert!(b.get("prompt_cache_key").is_none());
  assert!(b.get("modalities").is_none());
  // Claude: the system prompt cached apart, the conversation after it.
  let hour = json!({"type": "ephemeral", "ttl": "1h"});
  assert_eq!(b["cache_control"], hour);
  assert_eq!(
    b["messages"][0],
    json!({"role": "system", "content": [{"type": "text", "text": "Be brief.", "cache_control": hour}]})
  );
  assert_eq!(
    b["messages"][1]["content"][1],
    json!({"type": "file", "file": {"filename": "cal.pdf", "file_data": "data:application/pdf;base64,JVBERi0xLjc="}})
  );
  assert_eq!(b["tools"][0]["function"]["name"], "get_date");

  let msgs = seen[1].body["messages"].as_array().unwrap();
  let assistant = msgs.iter().find(|m| m["role"] == "assistant").unwrap();
  assert_eq!(
    assistant["reasoning_details"][0]["signature"], "sig-1",
    "{assistant}"
  );
  assert_eq!(assistant["tool_calls"][0]["id"], "toolu_1");
  // The image it made goes back as the user's, after its turn.
  assert!(msgs.iter().any(|m| {
    m["role"] == "user"
      && m["content"]
        .as_array()
        .is_some_and(|c| c.iter().any(|p| p["type"] == "image_url"))
  }));

  let b = &seen[2].body;
  let assistant = b["messages"]
    .as_array()
    .unwrap()
    .iter()
    .find(|m| m["role"] == "assistant")
    .unwrap();
  assert!(assistant.get("reasoning_details").is_none() && assistant.get("reasoning").is_none());
  assert!(b.get("cache_control").is_none());
  assert_eq!(
    b["messages"][0],
    json!({"role": "system", "content": "Be brief."})
  );
  // OpenAI's models route by their own cache key too.
  assert_eq!(b["prompt_cache_key"], "thread-1");
}

#[tokio::test]
async fn openrouter_reasoning_text_alone_goes_back_and_settings_turn_parts_off() {
  let chunk = |v: Value| format!("data: {v}\n\n");
  let turn = [
    chunk(json!({"choices": [{"index": 0, "delta": {"reasoning": "Hmm."}}]})),
    chunk(json!({"choices": [{"index": 0, "delta": {"content": "Hi."}, "finish_reason": "stop"}]})),
    "data: [DONE]\n\n".to_owned(),
  ]
  .concat();
  let turn = format!(
    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{turn}"
  );
  let (base, seen) = serve(vec![turn.clone(), turn]).await;
  let mut o = openrouter(&base);
  o.thinking = Some(false);
  o.cache = false;
  let p = Provider::OpenAi(o);
  let model = "anthropic/claude-haiku-5.5:nitro";
  let mut first = Request::new(model, vec![Message::user(vec![Content::Text("Hi".into())])]);
  first.system = Some("S".into());
  let r = p.stream(&first, &mut |_| {}).await.unwrap();
  assert_eq!(r.cost, None);
  assert_eq!(
    r.native.as_ref().unwrap().content,
    json!({"reasoning": "Hmm."})
  );
  let mut next = first.clone();
  next.messages.push(Message {
    role: Role::Assistant,
    content: r.content,
    native: r.native,
  });
  next
    .messages
    .push(Message::user(vec![Content::Text("Again".into())]));
  p.stream(&next, &mut |_| {}).await.unwrap();
  let seen = seen.lock().unwrap();
  let b = &seen[0].body;
  assert_eq!(b["reasoning"], json!({"enabled": false}));
  // Caching off: no breakpoints, even on Claude.
  assert!(b.get("cache_control").is_none());
  assert_eq!(b["messages"][0]["content"], "S");
  assert_eq!(seen[1].body["messages"][2]["reasoning"], "Hmm.");
}

#[tokio::test]
async fn openrouter_lists_the_models_a_thread_can_run_and_their_windows() {
  let list = json!({"data": [
    {"id": "anthropic/claude-opus-5.5", "context_length": 1000000,
     "supported_parameters": ["tools", "reasoning"], "architecture": {"output_modalities": ["text"]},
     "endpoints": ["/api/v1/chat/completions", "/api/v1/messages"]},
    {"id": "anthropic/claude-opus-5.5:batch", "context_length": 1000000,
     "supported_parameters": ["tools"], "architecture": {"output_modalities": ["text"]}},
    {"id": "~openai/gpt-sol-latest", "context_length": 400000, "supported_parameters": ["tools"]},
    {"id": "some/no-tools", "context_length": 8192, "supported_parameters": ["temperature"]},
    {"id": "some/images-only", "supported_parameters": ["tools"], "architecture": {"output_modalities": ["image"]}},
    {"id": "some/completions-only", "supported_parameters": ["tools"], "endpoints": ["/api/v1/completions"]},
  ], "links": {"next": null}});
  let (base, seen) = serve(vec![ok(list.clone()), ok(list)]).await;
  let p = Provider::OpenAi(openrouter(&base));
  assert_eq!(
    p.models().await.unwrap(),
    ["anthropic/claude-opus-5.5", "~openai/gpt-sol-latest"]
  );
  // A variant has its model's window.
  assert_eq!(
    p.context_window("anthropic/claude-opus-5.5:nitro")
      .await
      .unwrap(),
    Some(1_000_000)
  );
  assert_eq!(paths(&seen), ["GET /api/v1/models", "GET /api/v1/models"]);
  // Its catalog is public, but without a key none of it is this node's.
  let mut keyless = openrouter(&base);
  keyless.api_key = None;
  assert!(matches!(
    Provider::OpenAi(keyless).models().await,
    Err(Error::NoKey(_))
  ));
}

#[tokio::test]
async fn hosted_apis_without_a_key_are_not_asked_for_models() {
  // DeepSeek, OpenAI and xAI would answer 401: no key is "no key".
  for base in [
    "https://api.deepseek.com",
    "https://api.openai.com/v1",
    "https://api.x.ai/v1",
  ] {
    let p = Provider::OpenAi(OpenAiCompat::new(base, None));
    assert!(matches!(p.models().await, Err(Error::NoKey(_))), "{base}");
  }
}
