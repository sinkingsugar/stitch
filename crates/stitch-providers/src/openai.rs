//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`,
//! streaming): one adapter for llama.cpp's server, Ollama, OpenRouter and
//! the many services that speak this shape. Images go as data URLs; a
//! document the shape cannot carry (PDF) is replaced by a note saying so.
//!
//! A thread's requests say which thread they are where the server routes
//! by it to its cached prompt: `prompt_cache_key` on OpenAI's API,
//! `x-grok-conv-id` on xAI's. Other servers get neither: some refuse
//! fields they do not know.
//!
//! Reasoning a server streams (`reasoning_content`, or `reasoning`) is
//! kept as the turn's native content (`{"reasoning_content": …}`) and,
//! for the servers that need it back (DeepSeek: a 400 without it once
//! tools are in the request), sent back on the same model's assistant
//! messages, as the providers ask: the whole message, reasoning
//! included (design §2.8, DeepSeek).
//!
//! OpenRouter (design §2.8, OpenRouter) is this shape with its own
//! extensions: any of its model IDs (`anthropic/claude-sonnet-5.5`,
//! `~openai/gpt-sol-latest`, `…:nitro`), its unified `reasoning`, the
//! turn's `reasoning_details` kept and sent back on the same model's
//! turns (Claude's signed thinking, OpenAI's encrypted reasoning, so tool
//! loops keep their chain of thought), `cache_control` breakpoints for
//! the models that cache only when asked (Claude), `session_id` for its
//! sticky routing, PDFs as `file` parts, images a model makes, the cost
//! it bills in `usage.cost`, and its catalog filtered to the models that
//! take tools.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::http::post_sse;
use crate::sse::SseEvent;
use crate::{Content, Error, Event, Request, Response, Result, Role, StopReason, Usage};

pub(crate) const KIND: &str = "openai";

/// DeepSeek's OpenAI-compatible API, the built-in `deepseek` provider.
pub const DEEPSEEK_URL: &str = "https://api.deepseek.com";

/// OpenRouter's OpenAI-compatible API, the built-in `openrouter` provider.
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";

/// Who calls, for OpenRouter's app attribution: the referer is the app's
/// identity there. The app is public (listed in its rankings): no
/// `x-openrouter-app-visibility: hidden`, which only the request creating
/// the app would honour.
const OPENROUTER_HEADERS: &[(&str, &str)] = &[
  ("http-referer", "https://github.com/sinkingsugar/stitch"),
  ("x-openrouter-title", "Stitch"),
  ("x-openrouter-categories", "cli-agent,personal-agent"),
];

#[derive(Clone)]
pub struct OpenAiCompat {
  /// Up to and including the version path, e.g. `http://localhost:11434/v1`.
  pub base_url: String,
  /// Local servers usually take none.
  pub api_key: Option<String>,
  pub grok: bool,
  /// `prompt_cache_options.ttl` (`30m`, GPT-5.6 and later on OpenAI's
  /// API); `None` leaves the provider's.
  pub cache_ttl: Option<String>,
  /// `reasoning_effort`, sent only when set (servers that do not know it
  /// may refuse it).
  pub effort: Option<String>,
  /// Each assistant message's reasoning kept and sent back
  /// (`reasoning_content`): DeepSeek's thinking mode refuses a request
  /// with tools whose earlier assistant messages lack it. On for
  /// DeepSeek's API (`new`).
  pub reasoning_back: bool,
  /// OpenRouter's extensions (the module's doc). On for its API (`new`).
  pub openrouter: bool,
  /// OpenRouter: `reasoning.enabled` when no effort is set; `None`
  /// leaves each model its default (a model's own default effort, which
  /// `enabled: true` would set to medium).
  pub thinking: Option<bool>,
  /// OpenRouter: prompt-cache breakpoints on the models that cache only
  /// when asked (Claude); `cache_ttl` `1h` keeps them an hour.
  pub cache: bool,
  /// A managed subscription's token source, asked for `api_key` before
  /// each request (`Provider::stream`).
  pub tokens: Option<crate::oauth::Tokens>,
  /// OpenRouter: the provider's name, which every response's model is
  /// named with (`openrouter/anthropic/claude-sonnet-5.5`). Its IDs carry
  /// their maker, so bare they read as another provider's model in the
  /// records, the cost line and the spend. `None`: as the server says.
  pub router: Option<String>,
}

impl std::fmt::Debug for OpenAiCompat {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("OpenAiCompat")
      .field("base_url", &self.base_url)
      .finish_non_exhaustive()
  }
}

impl OpenAiCompat {
  pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
    let base_url: String = base_url.into();
    OpenAiCompat {
      reasoning_back: base_url.starts_with(DEEPSEEK_URL),
      openrouter: base_url.starts_with("https://openrouter.ai/"),
      base_url,
      api_key,
      grok: false,
      cache_ttl: None,
      effort: None,
      thinking: None,
      cache: true,
      tokens: None,
      router: None,
    }
  }

  /// OpenRouter's `cache_control` for `model`: on Claude only, the models
  /// that cache nothing unasked (the others cache by themselves, and
  /// Gemini's explicit caches bill storage).
  fn breakpoint(&self, model: &str) -> Option<Value> {
    let claude = model.trim_start_matches('~').starts_with("anthropic/");
    (self.openrouter && self.cache && claude).then(|| match self.cache_ttl.as_deref() {
      Some("1h") => json!({"type": "ephemeral", "ttl": "1h"}),
      _ => json!({"type": "ephemeral"}),
    })
  }

  fn is_deepseek(&self) -> bool {
    self.base_url.starts_with("https://api.deepseek.com")
  }

  fn is_openai(&self) -> bool {
    self.base_url.starts_with("https://api.openai.com/")
  }

  fn is_xai(&self) -> bool {
    self.grok || self.base_url.starts_with("https://api.x.ai/")
  }

  /// The server's root, without the OpenAI-compatible version path
  /// (`http://localhost:11434/v1` → `http://localhost:11434`).
  fn root(&self) -> &str {
    let base = self.base_url.trim_end_matches('/');
    base.strip_suffix("/v1").unwrap_or(base)
  }

  fn messages(&self, r: &Request) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(s) = &r.system {
      match self.breakpoint(&r.model) {
        // The system prompt (and the tools before it) cached apart, so
        // threads with the same one share it (as on Anthropic's API).
        Some(c) if !s.is_empty() => out.push(json!({"role": "system", "content": [
          {"type": "text", "text": s, "cache_control": c},
        ]})),
        _ => out.push(json!({"role": "system", "content": s})),
      }
    }
    for m in &crate::lift_images(&r.messages) {
      let mut parts = Vec::new();
      let mut calls = Vec::new();
      for c in &crate::portable(&m.content) {
        match c {
          Content::Text(t) => parts.push(json!({"type": "text", "text": t})),
          Content::Image { media_type, data } => parts.push(json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{media_type};base64,{}", B64.encode(data))},
          })),
          // OpenRouter reads PDFs (natively where the model does, else
          // parsed by its own engine).
          Content::Document {
            media_type,
            name,
            data,
          } if self.openrouter && media_type == "application/pdf" => parts.push(json!({
            "type": "file",
            "file": {
              "filename": name.as_deref().unwrap_or("document.pdf"),
              "file_data": format!("data:application/pdf;base64,{}", B64.encode(data)),
            },
          })),
          Content::Document { .. } | Content::File { .. } => parts.push(json!({
            "type": "text",
            "text": crate::unreadable(c),
          })),
          Content::ToolCall { id, name, input } => calls.push(json!({
            "id": id,
            "type": "function",
            "function": {"name": name, "arguments": crate::replay_tool_input(input).to_string()},
          })),
          // Tool results are messages of their own in this shape.
          Content::ToolResult { id, output, .. } => {
            out.push(json!({"role": "tool", "tool_call_id": id, "content": output}));
          }
        }
      }
      match m.role {
        Role::User if !parts.is_empty() => out.push(json!({"role": "user", "content": parts})),
        Role::User => {}
        Role::Assistant => {
          // Assistant text goes as a plain string, which every server takes.
          let text: String = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect();
          let mut msg = json!({"role": "assistant", "content": text});
          if !calls.is_empty() {
            msg["tool_calls"] = Value::Array(calls);
          }
          if self.reasoning_back {
            // The same model's own reasoning; another model's turn has
            // none to give, and says so with an empty one.
            let reasoning = crate::native_for(m, KIND, &r.model)
              .and_then(|n| n.get("reasoning_content"))
              .and_then(Value::as_str)
              .unwrap_or("");
            msg["reasoning_content"] = json!(reasoning);
          }
          // OpenRouter: the same model's reasoning back as it came, its
          // details (signed, encrypted) where it gave them.
          if self.openrouter
            && let Some(n) = crate::native_for(m, KIND, &r.model)
          {
            if let Some(d) = n
              .get("reasoning_details")
              .filter(|d| d.as_array().is_some_and(|a| !a.is_empty()))
            {
              msg["reasoning_details"] = d.clone();
            } else if let Some(t) = n
              .get("reasoning")
              .and_then(Value::as_str)
              .filter(|t| !t.is_empty())
            {
              msg["reasoning"] = json!(t);
            }
          }
          out.push(msg);
        }
      }
    }
    out
  }

  fn body(&self, r: &Request) -> (Value, HeaderMap) {
    let mut body = json!({
      "model": r.model,
      "messages": self.messages(r),
      "stream": true,
      "stream_options": {"include_usage": true},
      "max_tokens": r.max_tokens,
    });
    if !r.tools.is_empty() {
      body["tools"] = Value::Array(
        r.tools
          .iter()
          .map(|t| {
            json!({"type": "function", "function": {
              "name": t.name, "description": t.description, "parameters": t.input_schema,
            }})
          })
          .collect(),
      );
    }
    let mut headers = self.auth();
    if self.openrouter {
      self.openrouter_fields(r, &mut body);
      return (body, headers);
    }
    if let Some(e) = &self.effort {
      body["reasoning_effort"] = json!(e);
    }
    if let Some(k) = &r.cache_key {
      if self.is_openai() {
        body["prompt_cache_key"] = json!(k);
      } else if self.is_xai()
        && let Ok(v) = HeaderValue::from_str(k)
      {
        headers.insert("x-grok-conv-id", v);
      }
    }
    if let Some(ttl) = &self.cache_ttl
      && self.is_openai()
    {
      body["prompt_cache_options"] = json!({"ttl": ttl});
    }
    (body, headers)
  }

  /// OpenRouter's own request fields.
  fn openrouter_fields(&self, r: &Request, body: &mut Value) {
    // Its unified reasoning: an effort, else on or off as set, else the
    // model's own default.
    if let Some(e) = &self.effort {
      body["reasoning"] = json!({"effort": e});
    } else if let Some(on) = self.thinking {
      body["reasoning"] = json!({"enabled": on});
    }
    if let Some(k) = &r.cache_key {
      // Its sticky routing keeps a thread on the provider that holds its
      // cached prompt; OpenAI's models route by their own key too.
      body["session_id"] = json!(k.chars().take(256).collect::<String>());
      if r.model.trim_start_matches('~').starts_with("openai/") {
        body["prompt_cache_key"] = json!(k);
      }
    }
    // Claude: the conversation cached up to its last block, the
    // breakpoint moving with it (with the system prompt's, two of four).
    if let Some(c) = self.breakpoint(&r.model) {
      body["cache_control"] = c;
    }
    // A model that makes images says so in its name; it answers with
    // them only when asked.
    if r.model.contains("-image") {
      body["modalities"] = json!(["image", "text"]);
    }
  }

  fn auth(&self) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if self.openrouter {
      for (k, v) in OPENROUTER_HEADERS {
        headers.insert(*k, HeaderValue::from_static(v));
      }
    }
    if self.grok {
      headers.insert(
        "x-grok-client-surface",
        HeaderValue::from_static(crate::oauth::GROK_SURFACE),
      );
      headers.insert(
        "x-grok-client-version",
        HeaderValue::from_static(crate::oauth::GROK_VERSION),
      );
    }
    if let Some(k) = &self.api_key
      && let Ok(v) = HeaderValue::from_str(&format!("Bearer {k}"))
    {
      headers.insert(AUTHORIZATION, v);
    }
    headers
  }

  /// The models the server offers (`GET {base}/models`).
  pub async fn models(&self) -> Result<Vec<String>> {
    // A hosted API without a key: not asked (a local server takes none).
    // OpenRouter's catalog is public, but its models are not this node's
    // to run; the others would only answer 401.
    let hosted = self.openrouter || self.is_deepseek() || self.is_openai() || self.is_xai();
    if hosted && self.api_key.as_deref().is_none_or(str::is_empty) && self.tokens.is_none() {
      return Err(Error::NoKey(KIND));
    }
    if !self.openrouter {
      return Ok(crate::model_ids(&self.model_list(None).await?));
    }
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..20 {
      let list = self.model_list(after.as_deref()).await?;
      ids.extend(openrouter_chat_models(&list));
      match list.pointer("/links/next").and_then(Value::as_str) {
        Some(next) if !next.is_empty() => after = Some(next.to_owned()),
        _ => break,
      }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
  }

  /// `GET {base}/models`, or the page `next` of it (a link from the
  /// server's root, as OpenRouter gives it).
  async fn model_list(&self, next: Option<&str>) -> Result<Value> {
    let base = self.base_url.trim_end_matches('/');
    let url = match next {
      Some(next) => reqwest::Url::parse(base)
        .and_then(|b| b.join(next))
        .map_err(|e| Error::protocol(KIND, format!("a model list page: {e}")))?
        .to_string(),
      None => format!("{base}/models"),
    };
    crate::http::get_json(KIND, &url, self.auth()).await
  }

  /// The tokens `model` takes in at most, asked the way its server tells:
  /// Gemini's native model, the model list (OpenRouter, xAI, llama.cpp),
  /// llama.cpp's `/props`, then Ollama's running and installed models.
  /// What the server serves wins over what the model was trained for.
  pub async fn context_window(&self, model: &str) -> Result<Option<u64>> {
    if self.base_url.contains("generativelanguage.googleapis.com") {
      return self.gemini_window(model).await;
    }
    let mut after: Option<String> = None;
    let mut list = Value::Null;
    // OpenRouter's variants (`…:nitro`, `…:floor`) have their model's.
    let base = model
      .rsplit_once(':')
      .map(|(b, _)| b)
      .filter(|_| self.openrouter);
    // OpenRouter's list comes in pages.
    for _ in 0..20 {
      list = self.model_list(after.as_deref()).await?;
      if let Some(w) = crate::window_in_list(&list, model)
        .or_else(|| base.and_then(|b| crate::window_in_list(&list, b)))
      {
        return Ok(Some(w));
      }
      match list.pointer("/links/next").and_then(Value::as_str) {
        Some(next) if !next.is_empty() => after = Some(next.to_owned()),
        _ => break,
      }
    }
    // OpenAI's, xAI's, DeepSeek's and OpenRouter's lists said all they
    // will.
    if self.is_openai() || self.is_xai() || self.is_deepseek() || self.openrouter {
      return Ok(None);
    }
    let root = self.root();
    if let Ok(props) = crate::http::get_json(KIND, &format!("{root}/props"), self.auth()).await
      && let Some(w) = crate::positive(props.pointer("/default_generation_settings/n_ctx"))
    {
      return Ok(Some(w));
    }
    if let Some(w) = self.ollama_window(model).await {
      return Ok(Some(w));
    }
    // An older llama.cpp tells only what the model was trained for.
    Ok(
      list
        .get("data")
        .and_then(Value::as_array)
        .and_then(|d| {
          d.iter()
            .find(|m| m.get("id").and_then(Value::as_str) == Some(model))
        })
        .and_then(|m| crate::positive(m.pointer("/meta/n_ctx_train"))),
    )
  }

  /// Ollama: the window of the model as loaded (`/api/ps`), else the
  /// `num_ctx` it was made with, else what it was trained for
  /// (`/api/show`).
  async fn ollama_window(&self, model: &str) -> Option<u64> {
    let root = self.root();
    if let Ok(ps) = crate::http::get_json(KIND, &format!("{root}/api/ps"), self.auth()).await
      && let Some(w) = ps
        .get("models")
        .and_then(Value::as_array)
        .and_then(|ms| {
          ms.iter().find(|m| {
            ["name", "model"]
              .iter()
              .any(|k| m.get(*k).and_then(Value::as_str) == Some(model))
          })
        })
        .and_then(|m| crate::positive(m.get("context_length")))
    {
      return Some(w);
    }
    let show = crate::http::post_json(
      KIND,
      &format!("{root}/api/show"),
      self.auth(),
      &json!({"model": model}),
    )
    .await
    .ok()?;
    let num_ctx = show
      .get("parameters")
      .and_then(Value::as_str)
      .and_then(|p| {
        p.lines().find_map(|l| {
          let mut words = l.split_whitespace();
          (words.next() == Some("num_ctx"))
            .then(|| words.next()?.parse::<u64>().ok())
            .flatten()
        })
      });
    num_ctx.or_else(|| {
      let info = show.get("model_info")?;
      let arch = info.get("general.architecture")?.as_str()?;
      crate::positive(info.get(format!("{arch}.context_length").as_str()))
    })
  }

  /// Gemini's OpenAI-compatible list says nothing of windows; its native
  /// model does (`inputTokenLimit`).
  async fn gemini_window(&self, model: &str) -> Result<Option<u64>> {
    let base = self.base_url.trim_end_matches('/');
    let native = base.strip_suffix("/openai").unwrap_or(base);
    let name = model.strip_prefix("models/").unwrap_or(model);
    let mut headers = HeaderMap::new();
    if let Some(k) = &self.api_key
      && let Ok(v) = HeaderValue::from_str(k)
    {
      headers.insert("x-goog-api-key", v);
    }
    let m = crate::http::get_json(KIND, &format!("{native}/models/{name}"), headers).await?;
    Ok(crate::positive(m.get("inputTokenLimit")))
  }

  pub async fn stream(
    &self,
    request: &Request,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    let (body, headers) = self.body(request);
    let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
    let mut state = StreamState {
      model: request.model.clone(),
      asked: request.model.clone(),
      keep_reasoning: self.reasoning_back,
      openrouter: self.openrouter,
      router: self.router.clone(),
      ..StreamState::default()
    };
    post_sse(KIND, &url, headers, &body, |e| state.on_sse(e, on_event)).await?;
    state.finish()
  }
}

/// The models of an OpenRouter list a thread can run: those that take
/// tools (every turn offers them) and answer in text over Chat
/// Completions; not the `:batch` ones, which do not stream. A field the
/// list leaves out does not exclude a model.
fn openrouter_chat_models(list: &Value) -> Vec<String> {
  let has = |m: &Value, path: &str, what: &str| {
    m.pointer(path)
      .and_then(Value::as_array)
      .is_none_or(|a| a.iter().any(|v| v.as_str() == Some(what)))
  };
  list
    .get("data")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter(|m| has(m, "/supported_parameters", "tools"))
    .filter(|m| has(m, "/architecture/output_modalities", "text"))
    .filter(|m| has(m, "/endpoints", "/api/v1/chat/completions"))
    .filter_map(|m| m.get("id").and_then(Value::as_str))
    .filter(|id| !id.ends_with(":batch"))
    .map(str::to_owned)
    .collect()
}

/// Adds a streamed piece of OpenRouter's `reasoning_details` to those
/// gathered, by its `index`: text and summaries joined, other fields (a
/// signature, encrypted data, the format) as they last came.
fn merge_detail(details: &mut BTreeMap<u64, Value>, piece: &Value) {
  let Some(fields) = piece.as_object() else {
    return;
  };
  let index = piece
    .get("index")
    .and_then(Value::as_u64)
    .or_else(|| {
      // Without an index, a piece of the same kind goes on the last one.
      let (last, d) = details.last_key_value()?;
      (d.get("type") == piece.get("type")).then_some(*last)
    })
    .unwrap_or_else(|| details.last_key_value().map_or(0, |(k, _)| k + 1));
  let into = details.entry(index).or_insert_with(|| json!({}));
  for (k, v) in fields {
    match (k.as_str(), v, into.get(k)) {
      ("text" | "summary", Value::String(s), Some(Value::String(had))) => {
        into[k] = json!(format!("{had}{s}"));
      }
      // A null never hides what came before it.
      (_, Value::Null, Some(_)) => {}
      _ => into[k] = v.clone(),
    }
  }
}

/// An image a model made (`data:image/png;base64,…`), as content.
fn data_url_image(url: &str) -> Option<Content> {
  let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
  let media_type = head.strip_suffix(";base64")?;
  Some(Content::Image {
    media_type: media_type.to_owned(),
    data: B64.decode(data.trim()).ok()?,
  })
}

#[derive(Default)]
struct ToolCallBuf {
  id: String,
  name: String,
  arguments: String,
  started: bool,
}

#[derive(Default)]
struct StreamState {
  model: String,
  /// The model as asked for, which the native copy names (a server may
  /// answer with a dated name).
  asked: String,
  text: String,
  /// Kept only where it goes back (`reasoning_back`).
  keep_reasoning: bool,
  reasoning: String,
  /// OpenRouter: its reasoning (always kept, to go back), its details by
  /// index, the images the model made and what the call cost.
  openrouter: bool,
  /// The name the model is given (`OpenAiCompat::router`).
  router: Option<String>,
  details: BTreeMap<u64, Value>,
  images: Vec<Content>,
  cost: Option<f64>,
  calls: BTreeMap<u64, ToolCallBuf>,
  stop: Option<StopReason>,
  usage: Usage,
  done: bool,
}

impl StreamState {
  fn on_sse(&mut self, e: SseEvent, on_event: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
    if e.data.trim() == "[DONE]" {
      self.done = true;
      return Ok(());
    }
    let v: Value = serde_json::from_str(&e.data)
      .map_err(|err| Error::protocol(KIND, format!("bad chunk: {err}")))?;
    if let Some(err) = v.get("error") {
      let field = |k: &str| err.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
      return Err(Error::Api {
        provider: KIND,
        status: err.get("code").and_then(Value::as_u64).unwrap_or(500) as u16,
        kind: field("type"),
        message: field("message"),
      });
    }
    if let Some(m) = v.get("model").and_then(Value::as_str) {
      self.model = m.to_owned();
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
      let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
      let detail = |k: &str| {
        u.get("prompt_tokens_details")
          .and_then(|d| d.get(k))
          .and_then(Value::as_u64)
          .unwrap_or(0)
      };
      // DeepSeek says its cache hits as `prompt_cache_hit_tokens`.
      let cached = detail("cached_tokens").max(n("prompt_cache_hit_tokens"));
      let written = detail("cache_write_tokens");
      // `prompt_tokens` counts those read from and written to the cache too.
      self.usage.input_tokens = n("prompt_tokens").saturating_sub(cached + written);
      self.usage.output_tokens = n("completion_tokens");
      self.usage.cache_read_tokens = cached;
      self.usage.cache_write_tokens = written;
      // OpenRouter bills in credits, which are dollars.
      if let Some(c) = u.get("cost").and_then(Value::as_f64) {
        self.cost = Some(c);
      }
    }
    let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
      return Ok(());
    };
    let delta = &choice["delta"];
    if let Some(t) = delta.get("content").and_then(Value::as_str)
      && !t.is_empty()
    {
      self.text.push_str(t);
      on_event(Event::Text(t.to_owned()));
    }
    // llama.cpp and several servers stream reasoning under one of these.
    for k in ["reasoning_content", "reasoning"] {
      if let Some(t) = delta.get(k).and_then(Value::as_str)
        && !t.is_empty()
      {
        self.reasoning.push_str(t);
        on_event(Event::Thinking(t.to_owned()));
      }
    }
    if self.openrouter {
      for d in delta
        .get("reasoning_details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
      {
        merge_detail(&mut self.details, d);
      }
      for i in delta
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
      {
        if let Some(img) = i
          .pointer("/image_url/url")
          .and_then(Value::as_str)
          .and_then(data_url_image)
        {
          self.images.push(img);
        }
      }
    }
    for tc in delta
      .get("tool_calls")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
    {
      let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
      let buf = self.calls.entry(index).or_default();
      if let Some(id) = tc.get("id").and_then(Value::as_str) {
        buf.id = id.to_owned();
      }
      if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
        buf.name.push_str(name);
      }
      if !buf.started && !buf.id.is_empty() && !buf.name.is_empty() {
        buf.started = true;
        on_event(Event::ToolCallStart {
          id: buf.id.clone(),
          name: buf.name.clone(),
        });
      }
      if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str)
        && !args.is_empty()
      {
        buf.arguments.push_str(args);
        on_event(Event::ToolCallInput {
          id: buf.id.clone(),
          partial: args.to_owned(),
        });
      }
    }
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
      self.stop = Some(match reason {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "content_filter" => StopReason::Refusal { category: None },
        other => StopReason::Other(other.to_owned()),
      });
    }
    Ok(())
  }

  fn finish(self) -> Result<Response> {
    if !self.done && self.stop.is_none() {
      return Err(Error::cut(KIND, "the stream ended before a finish reason"));
    }
    let mut content = Vec::new();
    if !self.text.is_empty() {
      content.push(Content::Text(self.text));
    }
    content.extend(self.images);
    for (_, c) in self.calls {
      let input = if c.arguments.trim().is_empty() {
        json!({})
      } else {
        crate::decode_tool_input(
          serde_json::from_str(&c.arguments).unwrap_or(Value::String(c.arguments)),
        )
      };
      content.push(Content::ToolCall {
        id: c.id,
        name: c.name,
        input,
      });
    }
    // The reasoning, to send back on this model's later turns.
    let native = if self.openrouter {
      (!self.reasoning.is_empty() || !self.details.is_empty()).then(|| crate::Native {
        provider: KIND.into(),
        model: self.asked.clone(),
        // The details hold the text too, with what proves it.
        content: if self.details.is_empty() {
          json!({"reasoning": self.reasoning})
        } else {
          json!({"reasoning_details": self.details.into_values().collect::<Vec<_>>()})
        },
      })
    } else {
      (self.keep_reasoning && !self.reasoning.is_empty()).then(|| crate::Native {
        provider: KIND.into(),
        model: self.asked.clone(),
        content: json!({"reasoning_content": self.reasoning}),
      })
    };
    let model = match self.router {
      Some(r) => format!("{r}/{}", self.model),
      None => self.model,
    };
    Ok(Response {
      model,
      content,
      native,
      stop: self.stop.unwrap_or(StopReason::EndTurn),
      usage: self.usage,
      cost: self.cost,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Message;

  #[test]
  fn an_image_goes_as_its_bytes_say() {
    let r = Request::new(
      "m",
      vec![crate::Message::user(
        vec![crate::media::png_labelled_jpeg()],
      )],
    );
    let (b, _) = OpenAiCompat::new("https://api.openai.com/v1", None).body(&r);
    let url = b["messages"][0]["content"][0]["image_url"]["url"]
      .as_str()
      .unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");
  }

  #[test]
  fn a_thread_is_routed_to_its_cache_where_the_server_routes_by_it() {
    let mut r = Request::new("m", vec![]);
    r.cache_key = Some("thread-1".into());
    let mut openai = OpenAiCompat::new("https://api.openai.com/v1", Some("k".into()));
    let (b, h) = openai.body(&r);
    assert_eq!(b["prompt_cache_key"], "thread-1");
    assert!(h.get("x-grok-conv-id").is_none());
    assert!(b.get("prompt_cache_options").is_none());
    openai.cache_ttl = Some("30m".into());
    assert_eq!(
      openai.body(&r).0["prompt_cache_options"],
      json!({"ttl": "30m"})
    );
    let (b, h) = OpenAiCompat::new("https://api.x.ai/v1", Some("k".into())).body(&r);
    assert!(b.get("prompt_cache_key").is_none());
    assert_eq!(h["x-grok-conv-id"], "thread-1");
    let mut local = OpenAiCompat::new("http://localhost:11434/v1", None);
    local.cache_ttl = Some("30m".into());
    let (b, h) = local.body(&r);
    assert!(b.get("prompt_cache_key").is_none() && b.get("prompt_cache_options").is_none());
    assert!(h.get("x-grok-conv-id").is_none());
  }

  #[test]
  fn a_string_tool_input_replays_as_an_object() {
    let call = |input: Value| Message {
      role: Role::Assistant,
      content: vec![Content::ToolCall {
        id: "c1".into(),
        name: "send".into(),
        input,
      }],
      native: None,
    };
    let r = Request::new(
      "m",
      vec![
        call(json!("{\"thread\": \"t1\"}")),
        call(json!("{\"thread\": \"t")),
      ],
    );
    let (b, _) = OpenAiCompat::new("https://api.openai.com/v1", Some("k".into())).body(&r);
    let args: Vec<&str> = b["messages"]
      .as_array()
      .unwrap()
      .iter()
      .filter_map(|m| m["tool_calls"][0]["function"]["arguments"].as_str())
      .collect();
    assert_eq!(
      args,
      [r#"{"thread":"t1"}"#, r#"{"_raw":"{\"thread\": \"t"}"#]
    );
  }
}
