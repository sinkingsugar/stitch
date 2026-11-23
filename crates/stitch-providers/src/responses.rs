//! Responses API, with the Codex subscription request profile.
//!
//! Prompts are cached by the provider unasked; a thread's requests carry
//! its ID as `prompt_cache_key` (and Codex's `session-id`, which the
//! ChatGPT backend routes by), so they reach the machine that cached it.
use crate::{
  Content, Error, Event, Native, Request, Response, Result, Role, StopReason, Usage, native_for,
};
use crate::{http::post_sse, sse::SseEvent};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

pub(crate) const KIND: &str = "responses";
#[derive(Clone)]
pub struct Responses {
  pub base_url: String,
  pub api_key: String,
  pub account_id: Option<String>,
  pub effort: Option<String>,
  pub grok: bool,
  /// `prompt_cache_options.ttl` (`30m`, GPT-5.6 and later on the API);
  /// `None` leaves the provider's.
  pub cache_ttl: Option<String>,
  /// A managed subscription's token source, asked for `api_key` before
  /// each request (`Provider::stream`).
  pub tokens: Option<crate::oauth::Tokens>,
}
impl std::fmt::Debug for Responses {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Responses")
      .field("base_url", &self.base_url)
      .finish_non_exhaustive()
  }
}

/// The client version a Codex model list is asked as.
const CODEX_LIST_VERSION: &str = "0.999.0";

impl Responses {
  pub fn new(base_url: impl Into<String>, api_key: String) -> Self {
    Self {
      base_url: base_url.into(),
      api_key,
      account_id: None,
      effort: None,
      grok: false,
      cache_ttl: None,
      tokens: None,
    }
  }
  pub fn codex(token: String, account_id: String) -> Self {
    Self {
      account_id: Some(account_id),
      ..Self::new("https://chatgpt.com/backend-api/codex", token)
    }
  }
  fn body(&self, r: &Request) -> Value {
    let mut input = Vec::new();
    for m in &crate::lift_images(&r.messages) {
      if m.role == Role::Assistant
        && let Some(items) = native_for(m, KIND, &r.model).and_then(Value::as_array)
      {
        input.extend(items.iter().cloned());
        continue;
      }
      let mut parts = Vec::new();
      let flush = |parts: &mut Vec<Value>, input: &mut Vec<Value>| {
        if !parts.is_empty() {
          input.push(json!({"role": if m.role == Role::User {"user"} else {"assistant"}, "content": std::mem::take(parts)}));
        }
      };
      for c in &crate::portable(&m.content) {
        match c {
          Content::Text(text) => parts.push(json!({"type": if m.role == Role::User {"input_text"} else {"output_text"}, "text":text})),
          Content::File {..} => parts.push(json!({"type":"input_text", "text":crate::unreadable(c)})),
          Content::Image {media_type, data} => parts.push(json!({"type":"input_image", "image_url":format!("data:{media_type};base64,{}",B64.encode(data))})),
          Content::Document {media_type, name, data} => parts.push(json!({"type":"input_file", "filename":name.as_deref().unwrap_or("document.pdf"), "file_data":format!("data:{media_type};base64,{}",B64.encode(data))})),
          Content::ToolCall {id, name, input: args} => {
            flush(&mut parts, &mut input);
            input.push(json!({"type":"function_call", "call_id":id, "name":name, "arguments":crate::replay_tool_input(args).to_string()}));
          }
          Content::ToolResult {id, output, ..} => {
            flush(&mut parts, &mut input);
            input.push(json!({"type":"function_call_output", "call_id":id, "output":output}));
          }
        }
      }
      flush(&mut parts, &mut input);
    }
    let mut body = json!({"model":r.model,"input":input,"stream":true,"store":false,
      "instructions":r.system.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or("You are a helpful assistant."),
      "include":["reasoning.encrypted_content"]});
    if self.grok {
      body
        .as_object_mut()
        .expect("request object")
        .remove("include");
    }
    if self.account_id.is_none() {
      body["max_output_tokens"] = json!(r.max_tokens);
    }
    if let Some(e) = &self.effort {
      body["reasoning"] = json!({"effort":e,"summary":"auto"});
    }
    if let Some(k) = &r.cache_key {
      body["prompt_cache_key"] = json!(k);
    }
    if let Some(ttl) = &self.cache_ttl
      && self.account_id.is_none()
      && self.base_url.starts_with("https://api.openai.com/")
    {
      body["prompt_cache_options"] = json!({"ttl": ttl});
    }
    if !r.tools.is_empty() {
      body["tools"] = json!(r.tools.iter().map(|t| json!({"type":"function", "name":t.name,"description":t.description,"parameters":t.input_schema,"strict":false})).collect::<Vec<_>>());
    }
    body
  }
  /// The models the endpoint offers (`GET {base}/models`).
  pub async fn models(&self) -> Result<Vec<String>> {
    Ok(crate::model_ids(&self.model_list().await?))
  }

  /// The tokens `model` takes in at most, where the model list says
  /// (Codex, xAI; OpenAI's API does not).
  pub async fn context_window(&self, model: &str) -> Result<Option<u64>> {
    Ok(crate::window_in_list(&self.model_list().await?, model))
  }

  /// `GET {base}/models`. The Codex backend leaves out the models whose
  /// `minimal_client_version` is above the version asking, so it is asked
  /// as a recent client.
  async fn model_list(&self) -> Result<Value> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let base = self.base_url.trim_end_matches('/');
    let url = if self.account_id.is_some() {
      format!("{base}/models?client_version={CODEX_LIST_VERSION}")
    } else {
      format!("{base}/models")
    };
    crate::http::get_json(KIND, &url, self.headers(None)?).await
  }

  fn headers(&self, cache_key: Option<&str>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let mut insert = |k: &'static str, v: &str| -> Result<()> {
      headers.insert(
        k,
        HeaderValue::from_str(v)
          .map_err(|_| Error::protocol(KIND, "invalid authentication header"))?,
      );
      Ok(())
    };
    insert("authorization", &format!("Bearer {}", self.api_key))?;
    if let Some(id) = &self.account_id {
      insert("chatgpt-account-id", id)?;
      insert("originator", "codex_cli")?;
      insert("openai-beta", "responses=experimental")?;
      insert("user-agent", "codex_cli_rs/0.50.0")?;
      if let Some(k) = cache_key {
        insert("session-id", k)?;
      }
    }
    if self.grok {
      insert("x-grok-client-surface", crate::oauth::GROK_SURFACE)?;
      insert("x-grok-client-version", crate::oauth::GROK_VERSION)?;
    }
    Ok(headers)
  }
  pub async fn stream(
    &self,
    r: &Request,
    emit: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let mut state = State::default();
    post_sse(
      KIND,
      &format!("{}/responses", self.base_url.trim_end_matches('/')),
      self.headers(r.cache_key.as_deref())?,
      &self.body(r),
      |e| state.event(e, emit),
    )
    .await?;
    state.finish(&r.model)
  }
}
#[derive(Default)]
struct State {
  response: Option<Value>,
  calls: std::collections::BTreeMap<String, String>,
  completed: std::collections::BTreeMap<u64, Value>,
  streamed_output: bool,
}
impl State {
  fn event(&mut self, e: SseEvent, emit: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
    if e.data.trim() == "[DONE]" {
      return Ok(());
    }
    let v: Value =
      serde_json::from_str(&e.data).map_err(|_| Error::protocol(KIND, "invalid stream JSON"))?;
    let text = |key: &str| v[key].as_str().unwrap_or_default().to_owned();
    match v["type"]
      .as_str()
      .unwrap_or(e.event.as_deref().unwrap_or_default())
    {
      "response.output_text.delta" => {
        self.streamed_output = true;
        emit(Event::Text(text("delta")));
      }
      "response.output_item.done" => {
        let index = v["output_index"]
          .as_u64()
          .ok_or_else(|| Error::protocol(KIND, "completed item without output index"))?;
        if !v["item"].is_object() {
          return Err(Error::protocol(KIND, "invalid completed output item"));
        }
        self.completed.insert(index, v["item"].clone());
      }
      "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
        emit(Event::Thinking(text("delta")))
      }
      "response.output_item.added" if v["item"]["type"] == "function_call" => {
        self.streamed_output = true;
        let item = &v["item"];
        let id = item["call_id"].as_str().unwrap_or_default().to_owned();
        self.calls.insert(
          item["id"].as_str().unwrap_or_default().to_owned(),
          id.clone(),
        );
        emit(Event::ToolCallStart {
          id,
          name: item["name"].as_str().unwrap_or_default().to_owned(),
        });
      }
      "response.function_call_arguments.delta" => {
        let id = self
          .calls
          .get(&text("item_id"))
          .ok_or_else(|| Error::protocol(KIND, "tool delta without call"))?;
        emit(Event::ToolCallInput {
          id: id.clone(),
          partial: text("delta"),
        });
      }
      "response.completed" | "response.incomplete" => {
        self.response = Some(v["response"].clone());
      }
      "response.failed" | "error" => {
        // Its error, when it says one: `response.error` or the event's
        // own `code` and `message`.
        let e = v
          .get("response")
          .and_then(|r| r.get("error"))
          .filter(|e| e.is_object())
          .unwrap_or(&v);
        let code = e.get("code").and_then(Value::as_str).unwrap_or("");
        let message = e.get("message").and_then(Value::as_str).unwrap_or("");
        if code.is_empty() && message.is_empty() {
          return Err(Error::protocol(KIND, "response failed"));
        }
        let body = json!({"error": {"type": code, "code": code, "message": message}}).to_string();
        let usage = matches!(
          code,
          "insufficient_quota" | "usage_limit_reached" | "usage_not_included"
        );
        return Err(match usage {
          true => {
            crate::http::out_of_usage(KIND, 429, &HeaderMap::new(), &body, crate::http::now_secs())
              .unwrap_or_else(|| crate::http::api_error(KIND, 429, &body))
          }
          false => crate::http::api_error(KIND, 500, &body),
        });
      }
      _ => {}
    }
    Ok(())
  }
  fn finish(self, requested: &str) -> Result<Response> {
    let v = self
      .response
      .ok_or_else(|| Error::cut(KIND, "stream ended before response completed"))?;
    let output = v["output"]
      .as_array()
      .ok_or_else(|| Error::protocol(KIND, "missing response output"))?;
    // Some streams deliver completed items individually but leave the
    // terminal output empty. Keep their exact native form (reasoning and
    // tool calls included), ordered by output_index, never concatenated
    // with a terminal snapshot that already contains them.
    let completed: Vec<_> = self.completed.into_values().collect();
    let output = if output.is_empty() {
      &completed
    } else {
      output
    };
    if output.is_empty() && self.streamed_output {
      return Err(Error::protocol(
        KIND,
        "streamed output missing from completed response",
      ));
    }
    let model = v["model"].as_str().unwrap_or(requested).to_owned();
    let mut content = Vec::new();
    let mut stop = match v["status"].as_str() {
      Some("completed") => StopReason::EndTurn,
      Some("incomplete") => match v
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
      {
        Some("max_output_tokens") => StopReason::MaxTokens,
        Some("content_filter") => StopReason::Refusal { category: None },
        reason => StopReason::Other(reason.unwrap_or("incomplete").into()),
      },
      _ => return Err(Error::protocol(KIND, "invalid terminal response status")),
    };
    for item in output {
      match item["type"].as_str() {
        Some("message") => {
          for part in item["content"].as_array().into_iter().flatten() {
            if part["type"] == "output_text" {
              content.push(Content::Text(
                part["text"].as_str().unwrap_or_default().into(),
              ));
            }
            if part["type"] == "refusal" {
              stop = StopReason::Refusal { category: None };
            }
          }
        }
        Some("function_call") => {
          let args = item["arguments"].as_str().unwrap_or("{}");
          content.push(Content::ToolCall {
            id: item["call_id"].as_str().unwrap_or_default().into(),
            name: item["name"].as_str().unwrap_or_default().into(),
            input: crate::decode_tool_input(
              serde_json::from_str(args)
                .map_err(|_| Error::protocol(KIND, "invalid tool arguments"))?,
            ),
          });
          if stop == StopReason::EndTurn {
            stop = StopReason::ToolUse;
          }
        }
        _ => {}
      }
    }
    let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    // `input_tokens` counts those read from and written to the cache too.
    let cached = n("/usage/input_tokens_details/cached_tokens");
    let written = n("/usage/input_tokens_details/cache_write_tokens");
    Ok(Response {
      model: model.clone(),
      content,
      stop,
      native: Some(Native {
        provider: KIND.into(),
        model,
        content: json!(output),
      }),
      usage: Usage {
        input_tokens: n("/usage/input_tokens").saturating_sub(cached + written),
        output_tokens: n("/usage/output_tokens"),
        cache_read_tokens: cached,
        cache_write_tokens: written,
        cache_write_1h_tokens: 0,
      },
      cost: None,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Message;
  #[test]
  fn an_image_goes_as_its_bytes_say() {
    let p = Responses::codex("token".into(), "account".into());
    let r = Request::new(
      "model",
      vec![Message::user(vec![crate::media::png_labelled_jpeg()])],
    );
    let b = p.body(&r);
    let url = b["input"][0]["content"][0]["image_url"].as_str().unwrap();
    assert!(url.starts_with("data:image/png;base64,"), "{url}");
  }

  #[test]
  fn codex_profile_and_native_replay() {
    let p = Responses::codex("token".into(), "account".into());
    let native = json!([{"type":"reasoning","encrypted_content":"opaque","summary":[]}]);
    let r = Request::new(
      "model",
      vec![Message {
        role: Role::Assistant,
        content: vec![],
        native: Some(Native {
          provider: KIND.into(),
          model: "model".into(),
          content: native.clone(),
        }),
      }],
    );
    let b = p.body(&r);
    assert_eq!(b["input"], native);
    assert_eq!(b["store"], false);
    assert!(b["instructions"].as_str().is_some_and(|s| !s.is_empty()));
    for k in ["max_output_tokens", "temperature", "prompt_cache_retention"] {
      assert!(b.get(k).is_none());
    }
    assert_eq!(p.headers(None).unwrap()["chatgpt-account-id"], "account");
    let mut api = Responses::new("https://api.openai.com/v1", "key".into());
    assert_eq!(api.body(&r)["max_output_tokens"], r.max_tokens);
    assert!(
      api
        .headers(None)
        .unwrap()
        .get("chatgpt-account-id")
        .is_none()
    );
    // A thread's requests share a cache key; Codex routes by its session.
    let mut r = r;
    r.cache_key = Some("thread-1".into());
    assert_eq!(p.body(&r)["prompt_cache_key"], "thread-1");
    assert_eq!(
      p.headers(Some("thread-1")).unwrap()["session-id"],
      "thread-1"
    );
    assert!(
      api
        .headers(Some("thread-1"))
        .unwrap()
        .get("session-id")
        .is_none()
    );
    assert!(api.body(&r).get("prompt_cache_options").is_none());
    api.cache_ttl = Some("30m".into());
    assert_eq!(api.body(&r)["prompt_cache_options"], json!({"ttl": "30m"}));
    let mut codex = p.clone();
    codex.cache_ttl = Some("30m".into());
    assert!(codex.body(&r).get("prompt_cache_options").is_none());
  }
  #[test]
  fn completed_items_survive_empty_terminal_output_and_replay() {
    let items = json!([
      {"id":"r","type":"reasoning","encrypted_content":"opaque","summary":[]},
      {"id":"m","type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]},
      {"id":"f","type":"function_call","call_id":"call","name":"tool","arguments":"{\"x\":1}"}
    ]);
    for terminal in [json!([]), items.clone()] {
      let mut s = State::default();
      let mut send = |v: Value| {
        s.event(
          SseEvent {
            event: None,
            data: v.to_string(),
          },
          &mut |_| {},
        )
      };
      send(json!({"type":"response.output_text.delta","delta":"hello"})).unwrap();
      // Completion order need not be output order; repeated completion
      // events must not duplicate either text or tool calls.
      for i in [2, 0, 1, 1] {
        send(json!({"type":"response.output_item.done","output_index":i,"item":items[i]})).unwrap();
      }
      send(
        json!({"type":"response.completed","response":{"status":"completed","output":terminal}}),
      )
      .unwrap();
      let r = s.finish("model").unwrap();
      assert_eq!(r.text(), "hello");
      assert_eq!(r.stop, StopReason::ToolUse);
      assert_eq!(r.content.len(), 2);
      assert_eq!(r.native.as_ref().unwrap().content, items);
      let p = Responses::codex("token".into(), "account".into());
      let request = Request::new(
        "model",
        vec![Message {
          role: Role::Assistant,
          content: r.content,
          native: r.native,
        }],
      );
      assert_eq!(p.body(&request)["input"], items);
    }
  }

  #[test]
  fn streamed_output_without_completed_items_is_not_silently_lost() {
    for event in [
      json!({"type":"response.output_text.delta","delta":"hello"}),
      json!({"type":"response.output_item.added","item":{"id":"f","type":"function_call","call_id":"call","name":"tool"}}),
    ] {
      let mut s = State::default();
      for v in [
        event,
        json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
      ] {
        s.event(
          SseEvent {
            event: None,
            data: v.to_string(),
          },
          &mut |_| {},
        )
        .unwrap();
      }
      assert!(s.finish("model").is_err());
    }
  }

  #[test]
  fn stream_terminal_usage_tools_and_errors() {
    let mut s = State::default();
    let mut events = Vec::new();
    let mut send = |v: Value| {
      s.event(
        SseEvent {
          event: None,
          data: v.to_string(),
        },
        &mut |e| events.push(e),
      )
    };
    send(json!({"type":"response.output_text.delta","delta":"hello"})).unwrap();
    send(json!({"type":"response.completed","response":{"status":"completed","model":"actual","output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]},{"type":"function_call","call_id":"call","name":"tool","arguments":"{}"}],"usage":{"input_tokens":10,"output_tokens":3,"input_tokens_details":{"cached_tokens":4,"cache_write_tokens":1}}}})).unwrap();
    let r = s.finish("requested").unwrap();
    assert_eq!(r.model, "actual");
    assert_eq!(r.text(), "hello");
    assert_eq!(r.stop, StopReason::ToolUse);
    assert_eq!(
      (
        r.usage.input_tokens,
        r.usage.cache_read_tokens,
        r.usage.cache_write_tokens
      ),
      (5, 4, 1)
    );
    assert_eq!(events.len(), 1);
    assert!(State::default().finish("m").is_err());
    assert!(
      State::default()
        .event(
          SseEvent {
            event: None,
            data: json!({"type":"response.failed"}).to_string()
          },
          &mut |_| {}
        )
        .is_err()
    );
  }
}
