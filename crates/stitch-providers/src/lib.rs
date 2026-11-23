//! Model providers, written by hand over HTTP and server-sent events
//! (design §2.2): no provider SDKs. One request shape and one stream of
//! events for every provider; each adapter maps them to its wire format.
//!
//! A response also carries the provider's own content (`Native`), kept so
//! that a conversation continued on the same provider and model sends the
//! assistant turns back byte for byte: thinking blocks and their signatures
//! must come back unchanged (preserved thinking).

mod anthropic;
pub mod decisions;
mod error;
pub mod gemini;
mod http;
pub mod images;
pub mod media;
pub mod oauth;
mod openai;
mod responses;
pub mod sse;

pub use anthropic::{Anthropic, MOONSHOT_URL, is_moonshot};
pub use error::{Error, Exhausted};
pub use gemini::Gemini;
pub use http::{attribute, attribution, with_attribution};
pub use openai::{DEEPSEEK_URL, OPENROUTER_URL, OpenAiCompat};
pub use responses::Responses;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
  User,
  Assistant,
}

/// Content in the provider-neutral shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
  Text(String),
  /// Image bytes; providers take PNG, JPEG, GIF and WebP.
  Image {
    media_type: String,
    data: Vec<u8>,
  },
  /// A file the provider may read natively: a PDF (every adapter), audio
  /// or video (Gemini). An adapter that cannot read one sends a note
  /// saying so instead (`unreadable`).
  Document {
    media_type: String,
    name: Option<String>,
    data: Vec<u8>,
  },
  /// A file left where it is (audio, video), read only by an adapter that
  /// takes it (Gemini); every other one sends a note naming it, so its
  /// bytes are never loaded for them. The path is content-addressed (a
  /// blob), so what is at it never changes.
  File {
    media_type: String,
    name: Option<String>,
    path: std::path::PathBuf,
    bytes: u64,
  },
  ToolCall {
    id: String,
    name: String,
    input: Value,
  },
  ToolResult {
    id: String,
    output: String,
    is_error: bool,
  },
}

/// A provider's own content for an assistant turn, to send back unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Native {
  /// The adapter kind (`anthropic`, `openai`).
  pub provider: String,
  pub model: String,
  pub content: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
  pub role: Role,
  pub content: Vec<Content>,
  /// Used instead of `content` when the request goes to the same provider
  /// and model.
  pub native: Option<Native>,
}

impl Message {
  pub fn user(content: Vec<Content>) -> Self {
    Message {
      role: Role::User,
      content,
      native: None,
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
  pub name: String,
  pub description: String,
  pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
  pub model: String,
  pub system: Option<String>,
  pub messages: Vec<Message>,
  pub tools: Vec<ToolDef>,
  pub max_tokens: u32,
  /// What the requests of one conversation share (its thread), for the
  /// providers that route by it to where its prompt is cached.
  pub cache_key: Option<String>,
}

impl Request {
  pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
    Request {
      model: model.into(),
      system: None,
      messages,
      tools: Vec::new(),
      max_tokens: 64_000,
      cache_key: None,
    }
  }
}

/// What arrives while a response streams.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
  Text(String),
  /// Reasoning summaries or progress notes, where the provider returns them.
  Thinking(String),
  ToolCallStart {
    id: String,
    name: String,
  },
  /// A piece of a tool call's JSON input, as it streams.
  ToolCallInput {
    id: String,
    partial: String,
  },
}

#[derive(Clone, Debug, PartialEq)]
pub enum StopReason {
  EndTurn,
  MaxTokens,
  ToolUse,
  /// Declined by the provider's safety classifiers, with its category.
  Refusal {
    category: Option<String>,
  },
  Other(String),
}

/// Tokens a response took. One meaning for every provider: `input_tokens`
/// is the prompt not read from or written to the cache (Anthropic's sense;
/// OpenAI's count includes cached tokens, and its adapters subtract them),
/// so the whole prompt is the sum of the three.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
  pub input_tokens: u64,
  pub output_tokens: u64,
  pub cache_read_tokens: u64,
  pub cache_write_tokens: u64,
  /// Of `cache_write_tokens`, those cached for an hour (priced higher).
  pub cache_write_1h_tokens: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
  /// The model that produced the response (a fallback may differ from the
  /// one asked for).
  pub model: String,
  pub content: Vec<Content>,
  pub native: Option<Native>,
  pub stop: StopReason,
  pub usage: Usage,
  /// What the call cost in USD, where the provider says (OpenRouter's
  /// `usage.cost`); `None` leaves it to the node's prices.
  pub cost: Option<f64>,
}

impl Response {
  pub fn text(&self) -> String {
    self
      .content
      .iter()
      .filter_map(|c| match c {
        Content::Text(t) => Some(t.as_str()),
        _ => None,
      })
      .collect()
  }
}

/// A configured provider.
#[derive(Clone, Debug)]
pub enum Provider {
  Anthropic(Anthropic),
  OpenAi(OpenAiCompat),
  Responses(Responses),
  Gemini(Gemini),
}

impl Provider {
  pub fn is_subscription(&self) -> bool {
    match self {
      Self::Anthropic(p) => p.is_subscription(),
      Self::Responses(p) => p.account_id.is_some() || p.grok,
      Self::OpenAi(p) => p.grok,
      Self::Gemini(_) => false,
    }
  }
  pub fn kind(&self) -> &'static str {
    match self {
      Provider::Anthropic(_) => anthropic::KIND,
      Provider::OpenAi(_) => openai::KIND,
      Provider::Responses(_) => responses::KIND,
      Provider::Gemini(_) => gemini::KIND,
    }
  }

  /// Who answers and how it is paid for, as the user knows it:
  /// `Anthropic (Claude subscription)`, `OpenAI (API key)`, … For saying
  /// what ran out (`Error::OutOfUsage`).
  pub fn label(&self) -> String {
    let (who, plan) = match self {
      Provider::Anthropic(p) if !p.claude => ("Moonshot", ""),
      Provider::Anthropic(_) => ("Anthropic", "Claude subscription"),
      Provider::Responses(p) if p.grok => ("xAI", "Grok subscription"),
      Provider::Responses(p) if p.account_id.is_some() => ("OpenAI", "ChatGPT subscription"),
      Provider::Responses(p) if p.base_url.contains("x.ai") => ("xAI", ""),
      Provider::Responses(_) => ("OpenAI", ""),
      Provider::OpenAi(p) if p.grok => ("xAI", "Grok subscription"),
      Provider::OpenAi(p) if p.base_url.contains("x.ai") => ("xAI", ""),
      Provider::OpenAi(p) if p.base_url.contains("openai.com") => ("OpenAI", ""),
      Provider::OpenAi(p) if p.base_url.contains("deepseek.com") => ("DeepSeek", ""),
      Provider::OpenAi(p) if p.openrouter => ("OpenRouter", ""),
      Provider::OpenAi(_) => ("The provider", ""),
      Provider::Gemini(_) => ("Gemini", ""),
    };
    if self.is_subscription() {
      format!("{who} ({plan})")
    } else if self.has_key() {
      format!("{who} (API key)")
    } else {
      who.to_owned()
    }
  }

  /// Whether it has a key or a token to ask with (a local server may
  /// need none).
  pub fn has_key(&self) -> bool {
    match self {
      Provider::Anthropic(p) => !p.api_key.is_empty(),
      Provider::Responses(p) => !p.api_key.is_empty(),
      Provider::OpenAi(p) => p.api_key.as_ref().is_some_and(|k| !k.is_empty()),
      Provider::Gemini(p) => !p.api_key.is_empty(),
    }
  }

  /// A managed subscription's token source, if it has one.
  pub fn tokens(&self) -> Option<&oauth::Tokens> {
    match self {
      Provider::Anthropic(p) => p.tokens.as_ref(),
      Provider::OpenAi(p) => p.tokens.as_ref(),
      Provider::Responses(p) => p.tokens.as_ref(),
      // An API key only: no subscription.
      Provider::Gemini(_) => None,
    }
  }

  /// The same provider, sending `token`.
  fn with_token(&self, token: String) -> Provider {
    let mut p = self.clone();
    match &mut p {
      Provider::Anthropic(p) => p.api_key = token,
      Provider::OpenAi(p) => p.api_key = Some(token),
      Provider::Responses(p) => p.api_key = token,
      Provider::Gemini(p) => p.api_key = token,
    }
    p
  }

  /// Itself, with its token source's current token if it has one.
  async fn current(&self) -> Result<std::borrow::Cow<'_, Provider>> {
    Ok(match self.tokens() {
      Some(t) => std::borrow::Cow::Owned(self.with_token(t.0.token().await?)),
      None => std::borrow::Cow::Borrowed(self),
    })
  }

  /// The chat models it offers, by ID.
  pub async fn models(&self) -> Result<Vec<String>> {
    match &*self.current().await? {
      Provider::Anthropic(p) => p.models().await,
      Provider::OpenAi(p) => p.models().await,
      Provider::Responses(p) => p.models().await,
      Provider::Gemini(p) => p.models().await,
    }
  }

  /// The tokens `model` takes in at most, where the provider says.
  pub async fn context_window(&self, model: &str) -> Result<Option<u64>> {
    match &*self.current().await? {
      Provider::Anthropic(p) => p.context_window(model).await,
      Provider::OpenAi(p) => p.context_window(model).await,
      Provider::Responses(p) => p.context_window(model).await,
      Provider::Gemini(p) => p.context_window(model).await,
    }
  }

  /// Sends `request` and streams the response into `on_event`.
  ///
  /// A managed subscription's token is asked of its source for each
  /// request, so a long turn goes on past the expiry of the token it
  /// started with. A request the provider refuses as unauthorized (401)
  /// before anything streamed gets a refreshed token and is sent once
  /// more; a second refusal is the error.
  pub async fn stream(
    &self,
    request: &Request,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    let Some(tokens) = self.tokens() else {
      return self.stream_as(request, on_event).await;
    };
    let token = tokens.0.token().await?;
    let mut streamed = false;
    let first = {
      let mut seen = |e: Event| {
        streamed = true;
        on_event(e)
      };
      self
        .with_token(token.clone())
        .stream_as(request, &mut seen)
        .await
    };
    match first {
      Err(e) if e.is_unauthorized() && !streamed => {
        let fresh = tokens.0.refreshed(&token).await?;
        self.with_token(fresh).stream_as(request, on_event).await
      }
      r => r,
    }
  }

  /// Sends `request` with the credentials it holds.
  async fn stream_as(
    &self,
    request: &Request,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    match self {
      Provider::Anthropic(p) => p.stream(request, on_event).await,
      Provider::OpenAi(p) => p.stream(request, on_event).await,
      Provider::Responses(p) => p.stream(request, on_event).await,
      Provider::Gemini(p) => p.stream(request, on_event).await,
    }
  }
}

/// What an adapter sends for a file it cannot read (audio or video
/// anywhere but Gemini): a note naming it, so the model knows it is there.
pub(crate) fn unreadable(c: &Content) -> String {
  match c {
    Content::Document {
      media_type,
      name,
      data,
    } => format!(
      "[Attached `{}` ({media_type}, {} bytes): this model cannot read it.]",
      name.as_deref().unwrap_or("attachment"),
      data.len()
    ),
    Content::File {
      media_type,
      name,
      bytes,
      ..
    } => format!(
      "[Attached `{}` ({media_type}, {bytes} bytes): this model cannot read it.]",
      name.as_deref().unwrap_or("attachment"),
    ),
    _ => "[Attached content this model cannot read.]".into(),
  }
}

/// The content an adapter that reads images and PDFs only (every one but
/// Gemini) sends for a message: other files as notes (a `File` never
/// read). Images in the model's own turns are taken out first
/// (`lift_images`). Each image goes as the type its bytes say
/// (`media::image_type`), whatever it was labelled; one of a type these
/// adapters do not take (HEIC) as a note saying so.
pub(crate) fn portable(content: &[Content]) -> Vec<Content> {
  content
    .iter()
    .map(|c| match c {
      Content::Image { media_type, data } => {
        let t = media::image_type(media_type, data);
        match media::sniff_image(data) {
          Some(t) if !media::PROVIDER_IMAGES.contains(&t) => {
            Content::Text(media::unsendable(t, data.len()))
          }
          _ => Content::Image {
            media_type: t.to_owned(),
            data: data.clone(),
          },
        }
      }
      Content::Document { media_type, .. } if media_type != "application/pdf" => {
        Content::Text(unreadable(c))
      }
      Content::File { .. } => Content::Text(unreadable(c)),
      other => other.clone(),
    })
    .collect()
}

/// The messages as an adapter that takes no images in the model's turns
/// sends them (every one but Gemini): the images of a model's turn (an
/// image model made them, or `generate_image` did for it) moved to the
/// end of the user message after it, past its tool results, under a line
/// saying whose they are; a new user message when none follows. So every
/// model sees them, where its API allows images.
pub fn lift_images(messages: &[Message]) -> Vec<Message> {
  let mut out: Vec<Message> = Vec::with_capacity(messages.len());
  let mut carried: Vec<Content> = Vec::new();
  for m in messages {
    let mut m = m.clone();
    match m.role {
      Role::Assistant => {
        if !carried.is_empty() {
          out.push(Message::user(std::mem::take(&mut carried)));
        }
        // Each image with the line naming it just before it (the node's
        // `[image img:…]`), which a native copy of the turn lacks.
        let mut kept = Vec::with_capacity(m.content.len());
        let mut images = Vec::new();
        for c in m.content.drain(..) {
          if matches!(c, Content::Image { .. }) {
            if let Some(Content::Text(t)) = kept.last()
              && t.starts_with("[image ")
            {
              images.push(kept.pop().expect("the line just seen"));
            }
            images.push(c);
          } else {
            kept.push(c);
          }
        }
        m.content = kept;
        if !images.is_empty() {
          carried.push(Content::Text(
            "[The images of the assistant's turn above, made for it:]".into(),
          ));
          carried.extend(images);
        }
      }
      Role::User => m.content.append(&mut carried),
    }
    out.push(m);
  }
  if !carried.is_empty() {
    out.push(Message::user(carried));
  }
  out
}

/// Model IDs from a list (`{"data": [{"id"}]}`, or the Codex backend's
/// `{"models": [{"slug"}]}`), without what is not for chat (embeddings,
/// speech, images, moderation), sorted.
pub(crate) fn model_ids(list: &Value) -> Vec<String> {
  const NOT_CHAT: &[&str] = &[
    "embedding",
    "tts",
    "whisper",
    "dall-e",
    "moderation",
    "transcribe",
    "audio",
    "realtime",
    "image",
    "davinci",
    "babbage",
  ];
  let items = list
    .get("data")
    .or_else(|| list.get("models"))
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  let mut ids: Vec<String> = items
    .iter()
    // The Codex backend marks models not to offer.
    .filter(|m| {
      m.get("visibility")
        .and_then(Value::as_str)
        .is_none_or(|v| v == "list")
    })
    .filter_map(|m| {
      m.get("id")
        .or_else(|| m.get("slug"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    })
    .filter(|id| {
      let l = id.to_lowercase();
      !NOT_CHAT.iter().any(|n| l.contains(n))
    })
    .collect();
  ids.sort();
  ids.dedup();
  ids
}

/// The window `model` has in a model list, under whichever name its server
/// gives it: Codex's `context_window` (else `max_context_window`), xAI's
/// and OpenRouter's `context_length`, llama.cpp's `meta.n_ctx` (what it
/// serves, not `n_ctx_train`). Found by `id`, `slug` or an alias.
pub(crate) fn window_in_list(list: &Value, model: &str) -> Option<u64> {
  let items = list
    .get("data")
    .or_else(|| list.get("models"))
    .and_then(Value::as_array)?;
  let entry = items.iter().find(|m| {
    ["id", "slug"]
      .iter()
      .any(|k| m.get(*k).and_then(Value::as_str) == Some(model))
      || m
        .get("aliases")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(model)))
  })?;
  [
    "/context_window",
    "/max_context_window",
    "/context_length",
    "/top_provider/context_length",
    "/meta/n_ctx",
  ]
  .iter()
  .find_map(|p| positive(entry.pointer(p)))
}

/// A token count that says something: a positive number.
pub(crate) fn positive(v: Option<&Value>) -> Option<u64> {
  v.and_then(Value::as_u64).filter(|n| *n > 0)
}

/// The native content to send for a message, if it was produced by this
/// provider and model.
pub(crate) fn native_for<'a>(m: &'a Message, kind: &str, model: &str) -> Option<&'a Value> {
  m.native
    .as_ref()
    .filter(|n| n.provider == kind && n.model == model)
    .map(|n| &n.content)
}

/// A tool call's input as a model gave it, decoded when it is a string
/// holding a JSON object (a model double-encoding its arguments, once or
/// more): the object then, so the tool runs on it and the history keeps
/// it. Anything else stays as it is (an input cut off, which the tool
/// layer reports as invalid).
pub fn decode_tool_input(input: Value) -> Value {
  let Value::String(mut s) = input else {
    return input;
  };
  let original = Value::String(s.clone());
  // A few levels at most.
  for _ in 0..4 {
    match serde_json::from_str::<Value>(&s) {
      Ok(v @ Value::Object(_)) => return v,
      Ok(Value::String(inner)) => s = inner,
      _ => break,
    }
  }
  original
}

/// A tool call's input as a request sends it back: always an object, as
/// the APIs require of a call in the history (Anthropic refuses the whole
/// request otherwise). A string holding an object is decoded; anything
/// else is wrapped as `{"_raw": input}`, so a history stored with a
/// broken input still replays.
pub fn replay_tool_input(input: &Value) -> Value {
  match decode_tool_input(input.clone()) {
    v @ Value::Object(_) => v,
    v => serde_json::json!({ "_raw": v }),
  }
}

#[cfg(test)]
mod tool_input_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_string_holding_an_object_is_decoded() {
    let obj = json!({"thread": "t1", "interject": true});
    assert_eq!(decode_tool_input(obj.clone()), obj);
    assert_eq!(decode_tool_input(json!(obj.to_string())), obj);
    // Encoded twice.
    assert_eq!(
      decode_tool_input(json!(json!(obj.to_string()).to_string())),
      obj
    );
    // Not an object: left alone.
    assert_eq!(decode_tool_input(json!("{\"cut")), json!("{\"cut"));
    assert_eq!(decode_tool_input(json!("[1, 2]")), json!("[1, 2]"));
    assert_eq!(decode_tool_input(json!("\"x\"")), json!("\"x\""));
    assert_eq!(decode_tool_input(json!(3)), json!(3));
  }

  #[test]
  fn a_replayed_input_is_always_an_object() {
    let obj = json!({"command": "ls"});
    assert_eq!(replay_tool_input(&obj), obj);
    assert_eq!(replay_tool_input(&json!(obj.to_string())), obj);
    assert_eq!(
      replay_tool_input(&json!("{\"cut")),
      json!({"_raw": "{\"cut"})
    );
    assert_eq!(replay_tool_input(&json!([1])), json!({"_raw": [1]}));
    assert_eq!(replay_tool_input(&Value::Null), json!({"_raw": null}));
  }
}

#[cfg(test)]
mod model_list_tests {
  use super::*;

  #[test]
  fn windows_are_read_from_each_kind_of_model_list() {
    let codex = serde_json::json!({"models": [
      {"slug": "gpt-6-sol", "context_window": 272000, "max_context_window": 872000},
      {"slug": "gpt-5.5", "max_context_window": 272000}
    ]});
    assert_eq!(window_in_list(&codex, "gpt-6-sol"), Some(272_000));
    assert_eq!(window_in_list(&codex, "gpt-5.5"), Some(272_000));
    let xai = serde_json::json!({"data": [
      {"id": "grok-4.7", "aliases": ["grok-latest"], "context_length": 500000}
    ]});
    assert_eq!(window_in_list(&xai, "grok-latest"), Some(500_000));
    let openrouter = serde_json::json!({"data": [
      {"id": "openai/gpt-4", "context_length": null, "top_provider": {"context_length": 8192}}
    ]});
    assert_eq!(window_in_list(&openrouter, "openai/gpt-4"), Some(8192));
    let llama = serde_json::json!({"data": [
      {"id": "qwen3", "meta": {"n_ctx_train": 262144, "n_ctx": 32768}}
    ]});
    assert_eq!(window_in_list(&llama, "qwen3"), Some(32_768));
    let openai = serde_json::json!({"data": [{"id": "gpt-6-sol", "owned_by": "openai"}]});
    assert_eq!(window_in_list(&openai, "gpt-6-sol"), None);
    assert_eq!(window_in_list(&xai, "grok-9"), None);
  }

  #[test]
  fn portable_images_go_as_their_bytes_say_and_heic_as_a_note() {
    let mut heic = 24u32.to_be_bytes().to_vec();
    heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
    let out = portable(&[
      crate::media::png_labelled_jpeg(),
      Content::Image {
        media_type: "image/jpeg".into(),
        data: heic,
      },
      Content::Image {
        media_type: "image/webp".into(),
        data: vec![1, 2, 3],
      },
    ]);
    assert!(matches!(&out[0], Content::Image { media_type, .. } if media_type == "image/png"));
    assert!(matches!(&out[1], Content::Text(t) if t.contains("cannot read image/heic")));
    assert!(matches!(&out[2], Content::Image { media_type, .. } if media_type == "image/webp"));
  }

  #[test]
  fn other_adapters_get_notes_for_media_and_images_after_the_models_turn() {
    let image = Content::Image {
      media_type: "image/png".into(),
      data: vec![1],
    };
    let pdf = Content::Document {
      media_type: "application/pdf".into(),
      name: None,
      data: vec![1],
    };
    let video = Content::Document {
      media_type: "video/mp4".into(),
      name: Some("clip.mp4".into()),
      data: vec![1, 2],
    };
    // Never read: the path does not exist.
    let audio = Content::File {
      media_type: "audio/mpeg".into(),
      name: Some("talk.mp3".into()),
      path: "/nonexistent/blob".into(),
      bytes: 9,
    };
    let all = vec![image.clone(), pdf.clone(), video, audio];
    let note = |n: &str| Content::Text(n.into());
    assert_eq!(
      portable(&all),
      vec![
        image.clone(),
        pdf,
        note("[Attached `clip.mp4` (video/mp4, 2 bytes): this model cannot read it.]"),
        note("[Attached `talk.mp3` (audio/mpeg, 9 bytes): this model cannot read it.]"),
      ]
    );

    // The images of a model's turn go after the user's next message's
    // results; with none after it, in a message of their own.
    let call = Content::ToolCall {
      id: "c".into(),
      name: "generate_image".into(),
      input: serde_json::json!({}),
    };
    let result = Content::ToolResult {
      id: "c".into(),
      output: "made".into(),
      is_error: false,
    };
    let turn = |content: Vec<Content>| Message {
      role: Role::Assistant,
      content,
      native: None,
    };
    let lifted = lift_images(&[
      Message::user(vec![note("draw")]),
      turn(vec![note("Here."), call.clone(), image.clone()]),
      Message::user(vec![result.clone()]),
      turn(vec![note("Done."), image.clone()]),
    ]);
    let label = note("[The images of the assistant's turn above, made for it:]");
    assert_eq!(lifted.len(), 5);
    assert_eq!(lifted[1].content, vec![note("Here."), call]);
    assert_eq!(
      lifted[2].content,
      vec![result, label.clone(), image.clone()]
    );
    assert_eq!(lifted[3].content, vec![note("Done.")]);
    assert_eq!(lifted[4].role, Role::User);
    assert_eq!(lifted[4].content, vec![label, image]);
  }

  #[test]
  fn model_lists_keep_chat_models() {
    let openai = serde_json::json!({"data": [
      {"id": "gpt-6-astra"}, {"id": "text-embedding-3-large"}, {"id": "whisper-1"},
      {"id": "gpt-6-luna"}, {"id": "tts-1-hd"}, {"id": "dall-e-3"}
    ]});
    assert_eq!(model_ids(&openai), ["gpt-6-astra", "gpt-6-luna"]);
    let codex = serde_json::json!({"models": [
      {"slug": "gpt-6-codex", "visibility": "list"},
      {"slug": "internal-thing", "visibility": "hide"}
    ]});
    assert_eq!(model_ids(&codex), ["gpt-6-codex"]);
    assert!(model_ids(&serde_json::json!({})).is_empty());
  }
}
