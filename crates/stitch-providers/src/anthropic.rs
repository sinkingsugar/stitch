//! The Anthropic Messages API (`POST /v1/messages`, streaming).
//!
//! Defaults: adaptive thinking with summarized display (reasoning is shown
//! as it streams), an explicit effort, prompt caching (a breakpoint at the
//! end of the system prompt, shared by threads that have the same one, and
//! automatic caching of the conversation after it; five minutes, or an
//! hour), and server-side fallback (`fallbacks: "default"`) on the
//! models that take it, so a request a safety classifier declines is retried
//! on another model instead of stopping. Each can be turned off.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::http::post_sse;
use crate::sse::SseEvent;
use crate::{
  Content, Error, Event, Native, Request, Response, Result, Role, StopReason, Usage, native_for,
};

pub(crate) const KIND: &str = "anthropic";
const VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// `thinking.block_binding`: what the API does with a thinking block
/// whose prefix (the system prompt, the tools, an earlier message)
/// changed since the model wrote it. Unset, Fable 5.2 and Mythos (and
/// every 5.x model on accounts from 2026-08-31) answer 400 "bound to a
/// different conversation"; a thread's tools can change between turns
/// (its node, tool sets, MCP servers), so such blocks are dropped
/// instead. Models without the check accept it and ignore it.
const BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";
/// Models that accept `fallbacks: "default"` on the Claude API.
const FALLBACK_MODELS: [&str; 5] = [
  "claude-fable-5-2",
  "claude-fable-5-1",
  "claude-opus-5-5",
  "claude-opus-5",
  "claude-sonnet-5-5",
];

#[derive(Clone)]
pub struct Anthropic {
  pub base_url: String,
  pub api_key: String,
  /// `output_config.effort`; `None` leaves the model's default.
  pub effort: Option<String>,
  /// Adaptive thinking; off for older models that do not take it.
  pub thinking: bool,
  /// Server-side fallback on refusals, where the model supports it.
  pub fallbacks: bool,
  /// Prompt caching of the system prompt and the conversation.
  pub cache: bool,
  /// How long cached prompts last: `5m` (the default, writes at 1.25 times
  /// input) or `1h` (writes at twice input; it pays when turns are more
  /// than five minutes apart).
  pub cache_ttl: Option<String>,
  /// Managed subscription or a pasted `sk-ant-oat` access token.
  pub oauth: bool,
  /// A managed subscription's token source, asked for `api_key` before
  /// each request (`Provider::stream`).
  pub tokens: Option<crate::oauth::Tokens>,
  /// Claude's API (or a proxy of it). `false`: another vendor's
  /// Anthropic-compatible endpoint (Moonshot's Kimi, `moonshot`), which
  /// gets none of Claude's own fields (below, `body`).
  pub claude: bool,
}

/// Whether `base_url` is Moonshot's Anthropic-compatible endpoint (Kimi;
/// `api.moonshot.ai`, or `api.moonshot.cn` in China).
pub fn is_moonshot(base_url: &str) -> bool {
  let rest = base_url
    .strip_prefix("https://")
    .or_else(|| base_url.strip_prefix("http://"))
    .unwrap_or(base_url);
  let host = rest.split('/').next().unwrap_or("");
  host.starts_with("api.moonshot.") && host.matches('.').count() == 2
}

/// Moonshot's Anthropic-compatible endpoint (global), Kimi's built-in
/// provider (design §2.8, Kimi).
pub const MOONSHOT_URL: &str = "https://api.moonshot.ai/anthropic";

impl std::fmt::Debug for Anthropic {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Anthropic")
      .field("base_url", &self.base_url)
      .finish_non_exhaustive()
  }
}

impl Anthropic {
  pub fn new(api_key: impl Into<String>) -> Self {
    Anthropic {
      base_url: "https://api.anthropic.com".into(),
      api_key: api_key.into(),
      effort: Some("medium".into()),
      thinking: true,
      fallbacks: true,
      cache: true,
      cache_ttl: None,
      oauth: false,
      tokens: None,
      claude: true,
    }
  }

  /// Moonshot's Kimi at `base_url` (`MOONSHOT_URL`, or China's): always
  /// thinking, so no `thinking`; no fallbacks; effort `medium` (its
  /// `high`, below), not its default `max`.
  pub fn moonshot(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
    Anthropic {
      base_url: base_url.into(),
      thinking: false,
      fallbacks: false,
      claude: false,
      ..Anthropic::new(api_key)
    }
  }

  /// The effort as the vendor takes it: Claude's as it is; Moonshot's
  /// `low`, `high` or `max` (`minimal` is `low`, `medium` `high`, `xhigh`
  /// `max`), as its docs map other values.
  fn effort(&self) -> Option<String> {
    let e = self.effort.as_deref()?;
    if self.claude {
      return Some(e.to_owned());
    }
    Some(
      match e {
        "minimal" | "low" => "low",
        "xhigh" | "max" | "ultra" => "max",
        _ => "high",
      }
      .to_owned(),
    )
  }

  /// The OpenAI-compatible root of a vendor's endpoint, where it lists its
  /// models (`https://api.moonshot.ai/anthropic` → `https://api.moonshot.ai/v1`).
  fn vendor_v1(&self) -> String {
    let base = self.base_url.trim_end_matches('/');
    let root = base.strip_suffix("/anthropic").unwrap_or(base);
    format!("{root}/v1")
  }

  pub fn is_subscription(&self) -> bool {
    self.claude && (self.oauth || self.api_key.starts_with("sk-ant-oat"))
  }

  fn content_json(c: &Content) -> Value {
    match c {
      Content::Text(t) => json!({"type": "text", "text": t}),
      // `portable` made it a note already.
      Content::File { .. } => json!({"type": "text", "text": crate::unreadable(c)}),
      Content::Image { media_type, data } => json!({
        "type": "image",
        "source": {"type": "base64", "media_type": media_type, "data": B64.encode(data)},
      }),
      Content::Document {
        media_type,
        name,
        data,
      } => {
        let mut d = json!({
          "type": "document",
          "source": {"type": "base64", "media_type": media_type, "data": B64.encode(data)},
        });
        if let Some(n) = name {
          d["title"] = json!(n);
        }
        d
      }
      Content::ToolCall { id, name, input } => {
        json!({"type": "tool_use", "id": id, "name": name, "input": crate::replay_tool_input(input)})
      }
      Content::ToolResult {
        id,
        output,
        is_error,
      } => {
        json!({"type": "tool_result", "tool_use_id": id, "content": output, "is_error": is_error})
      }
    }
  }

  fn body(&self, r: &Request) -> (Value, HeaderMap) {
    let messages: Vec<Value> = crate::lift_images(&r.messages)
      .iter()
      .map(|m| {
        let role = match m.role {
          Role::User => "user",
          Role::Assistant => "assistant",
        };
        let content = match native_for(m, KIND, &r.model) {
          Some(native) => replayable(native.clone()),
          None => Value::Array(
            crate::portable(&m.content)
              .iter()
              .map(|c| match c {
                // Moonshot's Messages take no documents.
                Content::Document { .. } if !self.claude => {
                  json!({"type": "text", "text": crate::unreadable(c)})
                }
                c => Self::content_json(c),
              })
              .collect(),
          ),
        };
        json!({"role": role, "content": content})
      })
      .collect();
    let mut body = json!({
      "model": r.model,
      "max_tokens": r.max_tokens,
      "stream": true,
      "messages": messages,
    });
    // Every breakpoint has the same lifetime: one of an hour may not
    // follow one of five minutes. Moonshot takes only the top-level one
    // (`top`), and ignores the others.
    let top = self.cache.then(|| match self.cache_ttl.as_deref() {
      Some(ttl) if ttl != "5m" => json!({"type": "ephemeral", "ttl": ttl}),
      _ => json!({"type": "ephemeral"}),
    });
    let cache = top.clone().filter(|_| self.claude);
    let mut system = Vec::new();
    if self.is_subscription() {
      // Required, and cached, whether caching is on or not.
      let mut block =
        json!({"type":"text", "text":"You are Claude Code, Anthropic's official CLI for Claude."});
      block["cache_control"] = cache.clone().unwrap_or(json!({"type": "ephemeral"}));
      system.push(block);
    }
    if let Some(s) = &r.system
      && !s.is_empty()
    {
      let mut block = json!({"type":"text", "text":s});
      if let Some(c) = &cache {
        block["cache_control"] = c.clone();
      }
      system.push(block);
    }
    if !system.is_empty() {
      body["system"] = json!(system);
    }
    if !r.tools.is_empty() {
      body["tools"] = Value::Array(
        r.tools
          .iter()
          .map(|t| {
            let mut tool = json!({
              "name": t.name,
              "description": t.description,
              "input_schema": t.input_schema,
            });
            if self.claude {
              tool["eager_input_streaming"] = json!(true);
            }
            tool
          })
          .collect(),
      );
      // Without a system prompt, the tools end the part threads share.
      if let Some(c) = &cache
        && r.system.as_deref().is_none_or(str::is_empty)
        && let Some(last) = body["tools"].as_array_mut().and_then(|t| t.last_mut())
      {
        last["cache_control"] = c.clone();
      }
    }
    let (mut headers, mut betas) = self.auth();
    // Kimi always thinks, and takes no `thinking`.
    if self.thinking && self.claude {
      body["thinking"] = json!({
        "type": "adaptive",
        "display": "summarized",
        "block_binding": {"prefix_mismatch_behavior": "drop_block"},
      });
      betas.push(BINDING_BETA);
    }
    if let Some(e) = self.effort() {
      body["output_config"] = json!({"effort": e});
    }
    if let Some(c) = top {
      body["cache_control"] = c;
    }
    if self.fallbacks && self.claude && FALLBACK_MODELS.contains(&r.model.as_str()) {
      body["fallbacks"] = json!("default");
      betas.push(FALLBACK_BETA);
    }
    if !betas.is_empty() {
      headers.insert(
        "anthropic-beta",
        HeaderValue::from_str(&betas.join(",")).expect("static betas"),
      );
    }
    (body, headers)
  }

  /// The version and authentication headers, and the betas they need.
  fn auth(&self) -> (HeaderMap, Vec<&'static str>) {
    let mut headers = HeaderMap::new();
    headers.insert("anthropic-version", HeaderValue::from_static(VERSION));
    let mut betas = Vec::new();
    if self.is_subscription() {
      if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
        headers.insert("authorization", v);
      }
      headers.insert(
        "user-agent",
        HeaderValue::from_static(crate::oauth::CLAUDE_USER_AGENT),
      );
      headers.insert("x-app", HeaderValue::from_static("cli"));
      betas.extend(["claude-code-20250219", "oauth-2025-04-20"]);
    } else if !self.claude {
      // Moonshot documents a bearer key on every endpoint.
      if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", self.api_key)) {
        headers.insert("authorization", v);
      }
    } else if let Ok(v) = HeaderValue::from_str(&self.api_key) {
      headers.insert("x-api-key", v);
    }
    (headers, betas)
  }

  /// GETs `path` of the API, authenticated.
  async fn get(&self, path: &str) -> Result<Value> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let (mut headers, betas) = self.auth();
    if !betas.is_empty() {
      headers.insert(
        "anthropic-beta",
        HeaderValue::from_str(&betas.join(",")).expect("static betas"),
      );
    }
    let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
    crate::http::get_json(KIND, &url, headers).await
  }

  /// A vendor's OpenAI-compatible model list (`GET {root}/v1/models`),
  /// which says each model's `context_length`.
  async fn vendor_models(&self) -> Result<Value> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let (headers, _) = self.auth();
    let url = format!("{}/models", self.vendor_v1());
    crate::http::get_json(KIND, &url, headers).await
  }

  /// The models this account can use (`GET /v1/models`).
  pub async fn models(&self) -> Result<Vec<String>> {
    if !self.claude {
      return Ok(crate::model_ids(&self.vendor_models().await?));
    }
    Ok(crate::model_ids(&self.get("/v1/models?limit=1000").await?))
  }

  /// The tokens `model` takes in at most (`max_input_tokens` of
  /// `GET /v1/models/{id}`; a vendor's list's `context_length`).
  pub async fn context_window(&self, model: &str) -> Result<Option<u64>> {
    if !self.claude {
      return Ok(crate::window_in_list(&self.vendor_models().await?, model));
    }
    let m = self.get(&format!("/v1/models/{model}")).await?;
    Ok(crate::positive(m.get("max_input_tokens")))
  }

  pub async fn stream(
    &self,
    request: &Request,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let (body, headers) = self.body(request);
    let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
    let mut state = StreamState {
      model: request.model.clone(),
      ..StreamState::default()
    };
    post_sse(KIND, &url, headers, &body, |e| state.on_sse(e, on_event)).await?;
    state.finish()
  }
}

#[derive(Default)]
struct StreamState {
  model: String,
  /// Content blocks by index, as the API sends them, deltas applied.
  blocks: BTreeMap<u64, Value>,
  /// Tool input JSON as it streams, by block index.
  partial: BTreeMap<u64, String>,
  stop: Option<StopReason>,
  usage: Usage,
  done: bool,
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
  v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn append(block: &mut Value, field: &str, s: &str) {
  let cur = block
    .get(field)
    .and_then(Value::as_str)
    .unwrap_or("")
    .to_owned();
  block[field] = json!(cur + s);
}

impl StreamState {
  fn read_usage(&mut self, u: &Value) {
    let n = |k: &str| u.get(k).and_then(Value::as_u64);
    if let Some(v) = n("input_tokens") {
      self.usage.input_tokens = v;
    }
    if let Some(v) = n("output_tokens") {
      self.usage.output_tokens = v;
    }
    if let Some(v) = n("cache_read_input_tokens") {
      self.usage.cache_read_tokens = v;
    }
    if let Some(v) = n("cache_creation_input_tokens") {
      self.usage.cache_write_tokens = v;
    }
    if let Some(v) = u
      .pointer("/cache_creation/ephemeral_1h_input_tokens")
      .and_then(Value::as_u64)
    {
      self.usage.cache_write_1h_tokens = v;
    }
  }

  fn on_sse(&mut self, e: SseEvent, on_event: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
    let v: Value = serde_json::from_str(&e.data)
      .map_err(|err| Error::protocol(KIND, format!("bad event data: {err}")))?;
    let index = v.get("index").and_then(Value::as_u64).unwrap_or(0);
    match str_of(&v, "type") {
      "message_start" => {
        let m = &v["message"];
        if let Some(model) = m.get("model").and_then(Value::as_str) {
          self.model = model.to_owned();
        }
        self.read_usage(&m["usage"]);
      }
      "content_block_start" => {
        let block = v["content_block"].clone();
        if str_of(&block, "type") == "tool_use" {
          on_event(Event::ToolCallStart {
            id: str_of(&block, "id").to_owned(),
            name: str_of(&block, "name").to_owned(),
          });
        }
        self.blocks.insert(index, block);
      }
      "content_block_delta" => {
        let d = &v["delta"];
        let block = self
          .blocks
          .get_mut(&index)
          .ok_or_else(|| Error::protocol(KIND, format!("delta for unknown block {index}")))?;
        match str_of(d, "type") {
          "text_delta" => {
            let t = str_of(d, "text");
            append(block, "text", t);
            on_event(Event::Text(t.to_owned()));
          }
          "thinking_delta" => {
            let t = str_of(d, "thinking");
            append(block, "thinking", t);
            if !t.is_empty() {
              on_event(Event::Thinking(t.to_owned()));
            }
          }
          "signature_delta" => append(block, "signature", str_of(d, "signature")),
          "input_json_delta" => {
            let p = str_of(d, "partial_json");
            self.partial.entry(index).or_default().push_str(p);
            on_event(Event::ToolCallInput {
              id: str_of(block, "id").to_owned(),
              partial: p.to_owned(),
            });
          }
          "citations_delta" => {
            let citation = d["citation"].clone();
            match block.get_mut("citations").and_then(Value::as_array_mut) {
              Some(list) => list.push(citation),
              None => block["citations"] = json!([citation]),
            }
          }
          _ => {}
        }
      }
      "content_block_stop" => {
        if let Some(p) = self.partial.get(&index)
          && let Some(block) = self.blocks.get_mut(&index)
        {
          // An input cut off (max_tokens) does not parse; it stays a
          // string and the tool layer reports it as invalid.
          // A string holding an object (arguments encoded twice) is
          // the object.
          block["input"] = if p.trim().is_empty() {
            json!({})
          } else {
            crate::decode_tool_input(
              serde_json::from_str(p).unwrap_or_else(|_| Value::String(p.clone())),
            )
          };
        }
      }
      "message_delta" => {
        let d = &v["delta"];
        if let Some(reason) = d.get("stop_reason").and_then(Value::as_str) {
          self.stop = Some(match reason {
            "end_turn" | "stop_sequence" => StopReason::EndTurn,
            "max_tokens" | "model_context_window_exceeded" => StopReason::MaxTokens,
            "tool_use" => StopReason::ToolUse,
            "refusal" => StopReason::Refusal {
              category: d
                .get("stop_details")
                .or_else(|| v.get("stop_details"))
                .and_then(|s| s.get("category"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            },
            other => StopReason::Other(other.to_owned()),
          });
        }
        self.read_usage(&v["usage"]);
      }
      "message_stop" => self.done = true,
      "error" => {
        let err = &v["error"];
        let kind = str_of(err, "type").to_owned();
        return Err(Error::Api {
          provider: KIND,
          status: if kind == "overloaded_error" { 529 } else { 500 },
          kind,
          message: str_of(err, "message").to_owned(),
        });
      }
      _ => {}
    }
    Ok(())
  }

  fn finish(self) -> Result<Response> {
    if !self.done {
      return Err(Error::cut(KIND, "the stream ended before message_stop"));
    }
    let blocks: Vec<Value> = self.blocks.into_values().collect();
    let content = blocks
      .iter()
      .filter_map(|b| match str_of(b, "type") {
        "text" => Some(Content::Text(str_of(b, "text").to_owned())),
        "tool_use" => Some(Content::ToolCall {
          id: str_of(b, "id").to_owned(),
          name: str_of(b, "name").to_owned(),
          input: b.get("input").cloned().unwrap_or(json!({})),
        }),
        _ => None,
      })
      .collect();
    Ok(Response {
      native: Some(Native {
        provider: KIND.into(),
        model: self.model.clone(),
        content: Value::Array(echo_blocks(blocks)),
      }),
      model: self.model,
      content,
      stop: self.stop.unwrap_or(StopReason::Other("none".into())),
      usage: self.usage,
      cost: None,
    })
  }
}

/// Native content as it is sent back: a `tool_use` block's input always
/// an object, which the API requires (a history stored with a string
/// input would otherwise fail every request after it).
fn replayable(mut native: Value) -> Value {
  if let Some(blocks) = native.as_array_mut() {
    for b in blocks {
      if str_of(b, "type") == "tool_use"
        && let Some(input) = b.get_mut("input")
        && !input.is_object()
      {
        *input = crate::replay_tool_input(input);
      }
    }
  }
  native
}

/// The blocks to send back on the next turn. After a mid-output fallback,
/// model-internal blocks before the last `fallback` block are not echoed;
/// text, paired server-tool blocks and everything after the boundary are.
fn echo_blocks(blocks: Vec<Value>) -> Vec<Value> {
  let Some(boundary) = blocks.iter().rposition(|b| str_of(b, "type") == "fallback") else {
    return blocks;
  };
  let results: Vec<String> = blocks
    .iter()
    .filter(|b| str_of(b, "type").ends_with("_tool_result"))
    .map(|b| str_of(b, "tool_use_id").to_owned())
    .collect();
  blocks
    .into_iter()
    .enumerate()
    .filter(|(i, b)| {
      if *i >= boundary {
        return true;
      }
      match str_of(b, "type") {
        "text" | "fallback" => true,
        "server_tool_use" => results.iter().any(|r| r == str_of(b, "id")),
        t if t.ends_with("_tool_result") => true,
        _ => false,
      }
    })
    .map(|(_, b)| b)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{Message, ToolDef};

  #[test]
  fn an_image_goes_as_its_bytes_say() {
    let r = Request::new(
      "claude-opus-5-5",
      vec![Message::user(vec![crate::media::png_labelled_jpeg()])],
    );
    let (b, _) = Anthropic::new("sk-ant-api-x").body(&r);
    let source = &b["messages"][0]["content"][0]["source"];
    assert_eq!(source["media_type"], "image/png");
  }

  #[test]
  fn cache_breakpoints_share_one_lifetime() {
    let mut r = Request::new("claude-opus-5-5", vec![Message::user(vec![])]);
    r.tools.push(ToolDef {
      name: "t".into(),
      description: "d".into(),
      input_schema: json!({"type": "object"}),
    });
    let mut p = Anthropic::new("sk-ant-oat-x");
    p.cache_ttl = Some("1h".into());
    let hour = json!({"type": "ephemeral", "ttl": "1h"});
    // Without a system prompt, the tools end the shared part.
    let (b, _) = p.body(&r);
    assert_eq!(b["system"][0]["cache_control"], hour);
    assert_eq!(b["tools"][0]["cache_control"], hour);
    assert_eq!(b["cache_control"], hour);
    r.system = Some("Be brief.".into());
    let (b, _) = p.body(&r);
    assert_eq!(b["system"][1]["cache_control"], hour);
    assert!(b["tools"][0].get("cache_control").is_none());
    // Off: no breakpoints but the one a subscription requires.
    p.cache = false;
    let (b, _) = p.body(&r);
    assert!(b.get("cache_control").is_none());
    assert!(b["system"][1].get("cache_control").is_none());
    assert_eq!(
      b["system"][0]["cache_control"],
      json!({"type": "ephemeral"})
    );
    let mut key = Anthropic::new("sk-ant-api-x");
    key.cache = false;
    let (b, _) = key.body(&r);
    assert_eq!(b["system"], json!([{"type": "text", "text": "Be brief."}]));
  }

  #[test]
  fn thinking_before_a_fallback_is_not_echoed() {
    let blocks = vec![
      json!({"type": "thinking", "thinking": "", "signature": "s1"}),
      json!({"type": "text", "text": "partial"}),
      json!({"type": "server_tool_use", "id": "a"}),
      json!({"type": "web_search_tool_result", "tool_use_id": "a"}),
      json!({"type": "server_tool_use", "id": "b"}),
      json!({"type": "fallback", "from": {"model": "x"}, "to": {"model": "y"}}),
      json!({"type": "thinking", "thinking": "", "signature": "s2"}),
      json!({"type": "text", "text": "rest"}),
    ];
    let kept: Vec<String> = echo_blocks(blocks)
      .iter()
      .map(|b| str_of(b, "type").to_owned())
      .collect();
    assert_eq!(
      kept,
      [
        "text",
        "server_tool_use",
        "web_search_tool_result",
        "fallback",
        "thinking",
        "text"
      ]
    );
    let none = vec![json!({"type": "thinking"}), json!({"type": "text"})];
    assert_eq!(echo_blocks(none.clone()), none);
  }

  fn sse(data: Value) -> SseEvent {
    SseEvent {
      event: None,
      data: data.to_string(),
    }
  }

  #[test]
  fn a_tool_input_encoded_twice_is_decoded() {
    let obj = json!({"thread": "t1", "interject": true});
    // The model's arguments: a JSON string holding the object.
    let partial = json!(obj.to_string()).to_string();
    let mut s = StreamState::default();
    let mut ignore = |_: Event| {};
    for e in [
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {}}}),
      json!({"type": "content_block_start", "index": 0,
        "content_block": {"type": "tool_use", "id": "c1", "name": "send", "input": {}}}),
      json!({"type": "content_block_delta", "index": 0,
        "delta": {"type": "input_json_delta", "partial_json": partial}}),
      json!({"type": "content_block_stop", "index": 0}),
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {}}),
      json!({"type": "message_stop"}),
    ] {
      s.on_sse(sse(e), &mut ignore).unwrap();
    }
    let r = s.finish().unwrap();
    assert_eq!(
      r.content,
      vec![Content::ToolCall {
        id: "c1".into(),
        name: "send".into(),
        input: obj.clone(),
      }]
    );
    assert_eq!(r.native.unwrap().content[0]["input"], obj, "stored decoded");
  }

  #[test]
  fn a_history_with_a_string_tool_input_replays_as_objects() {
    // As a thread stuck before the fix stored it: the call's input a
    // string, in its content and in its native blocks.
    let obj = json!({"thread": "t1"});
    let call = |input: Value| Content::ToolCall {
      id: "c1".into(),
      name: "send".into(),
      input,
    };
    let result = Message::user(vec![Content::ToolResult {
      id: "c1".into(),
      output: "invalid type: string".into(),
      is_error: true,
    }]);
    let native = |input: Value| Native {
      provider: KIND.into(),
      model: "claude-opus-5-5".into(),
      content: json!([
        {"type": "thinking", "thinking": "", "signature": "s"},
        {"type": "tool_use", "id": "c1", "name": "send", "input": input},
      ]),
    };
    let assistant = |input: Value, native: Option<Native>| Message {
      role: Role::Assistant,
      content: vec![call(input)],
      native,
    };
    let encoded = json!(obj.to_string());
    let cut = json!("{\"thread\": \"t");
    let r = Request::new(
      "claude-opus-5-5",
      vec![
        Message::user(vec![Content::Text("go".into())]),
        assistant(encoded.clone(), Some(native(encoded.clone()))),
        result.clone(),
        assistant(cut.clone(), Some(native(cut.clone()))),
        result.clone(),
        // Another model's turn: sent from its content.
        assistant(encoded.clone(), None),
        result.clone(),
        assistant(cut.clone(), None),
        result,
      ],
    );
    let (b, _) = Anthropic::new("sk-ant-x").body(&r);
    let inputs: Vec<&Value> = b["messages"]
      .as_array()
      .unwrap()
      .iter()
      .flat_map(|m| m["content"].as_array().unwrap())
      .filter(|c| c["type"] == "tool_use")
      .map(|c| &c["input"])
      .collect();
    let raw = json!({"_raw": cut});
    assert_eq!(inputs, [&obj, &raw, &obj, &raw]);
  }
}
