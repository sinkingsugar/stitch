//! Google Gemini, natively: `models/{model}:streamGenerateContent` over
//! server-sent events, `models.list` for the models, and the Files API for
//! media too large to send inline (design §2.8, Gemini).
//!
//! Native rather than Gemini's OpenAI-compatible endpoint because that one
//! carries none of what makes Gemini worth having: audio and video input,
//! images in the reply, thought signatures (Gemini 3 refuses a function
//! call sent back without its own), and the model metadata (windows,
//! output limits, whether it thinks).
//!
//! Lessons from edge talk's `prompter-google.shs`:
//! - function results go in a `user` turn (Gemini 3.6 and later refuse the
//!   legacy `function` role);
//! - `thoughtSignature` sits beside the part it signs and goes back
//!   unchanged, so a model turn is replayed from its own parts (`Native`);
//! - temperature is left out (Gemini 3 recommends the default);
//! - image models answer with `inlineData` parts, asked for with
//!   `responseModalities`;
//! - video goes through the Files API, waiting until the file is `ACTIVE`.
//!   Edge talk kept the file's URI in the message, where it outlived the
//!   file (48 hours); here an upload is remembered per process, by the
//!   content's hash, and done again once it may have expired.
//!
//! The key goes in the `x-goog-api-key` header, never in a URL (edge talk
//! put it in the query, where it reaches logs).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::http::post_sse;
use crate::sse::SseEvent;
use crate::{
  Content, Error, Event, Native, Request, Response, Result, Role, StopReason, Usage, native_for,
};

pub(crate) const KIND: &str = "gemini";

/// The public API, up to its version path.
pub const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// The signature Gemini documents for a function call it did not make
/// (history from another model): it skips the check.
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";

/// Tool call IDs made here, for calls Gemini gave none; they are not sent
/// back as IDs.
const MADE_ID: &str = "gemini-call-";

/// Media up to this size goes inline, as long as the request's inline
/// media stays under `INLINE_TOTAL`; the rest through the Files API.
/// Gemini takes 20 MB of request inline, base64 included.
pub const INLINE_MAX: usize = 8 << 20;
const INLINE_TOTAL: usize = 14 << 20;

/// Uploaded files last 48 hours; one is used again for less than that.
const FILE_KEPT: Duration = Duration::from_secs(46 * 3600);

/// How long an uploaded video may take to be processed.
const PROCESSING: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct Gemini {
  /// Up to and including the version path, e.g. [`BASE_URL`].
  pub base_url: String,
  pub api_key: String,
  /// `thinkingLevel` (`minimal`, `low`, `medium`, `high`); `None` leaves
  /// the model's default.
  pub effort: Option<String>,
  /// How often an uploaded file's state is asked while it is processed.
  pub poll: Duration,
  /// Files larger than this go through the Files API ([`INLINE_MAX`]).
  pub inline_max: usize,
}

impl std::fmt::Debug for Gemini {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Gemini")
      .field("base_url", &self.base_url)
      .finish_non_exhaustive()
  }
}

/// What the API says of a model, kept per process.
#[derive(Clone, Copy, Debug, Default)]
struct ModelInfo {
  input: Option<u64>,
  output: Option<u64>,
  thinking: Option<bool>,
}

fn infos() -> &'static Mutex<HashMap<String, ModelInfo>> {
  static INFOS: OnceLock<Mutex<HashMap<String, ModelInfo>>> = OnceLock::new();
  INFOS.get_or_init(Default::default)
}

/// A file uploaded to the Files API.
#[derive(Clone, Debug)]
struct Uploaded {
  uri: String,
  mime: String,
  at: Instant,
}

fn uploads() -> &'static Mutex<HashMap<String, Uploaded>> {
  static UPLOADS: OnceLock<Mutex<HashMap<String, Uploaded>>> = OnceLock::new();
  UPLOADS.get_or_init(Default::default)
}

fn info_of(m: &Value) -> ModelInfo {
  ModelInfo {
    input: crate::positive(m.get("inputTokenLimit")),
    output: crate::positive(m.get("outputTokenLimit")),
    thinking: m.get("thinking").and_then(Value::as_bool),
  }
}

/// A model's name without the API's `models/` prefix.
fn bare(name: &str) -> &str {
  name.strip_prefix("models/").unwrap_or(name)
}

/// Whether `model` answers with images (Gemini's image models, "Nano
/// Banana"): asked for with `responseModalities`.
pub fn makes_images(model: &str) -> bool {
  let m = model.to_ascii_lowercase();
  m.contains("-image") || m.contains("nano-banana")
}

/// Whether `model` answers decisions (`gemini-decision-*`, the Decisions
/// API) rather than chats.
pub fn decides(model: &str) -> bool {
  model.to_ascii_lowercase().contains("-decision")
}

/// What a Gemini model reads natively besides images: PDFs, audio, video.
pub fn reads(media_type: &str) -> bool {
  media_type == "application/pdf"
    || media_type.starts_with("audio/")
    || media_type.starts_with("video/")
}

/// The model IDs of a `models.list` page worth chatting with: Gemini models
/// that `generateContent`, without embeddings, speech, live audio,
/// retrieval or other single-purpose models; image models stay (they
/// answer with images).
pub(crate) fn chat_models(list: &Value) -> Vec<String> {
  const NOT_CHAT: &[&str] = &[
    "embedding",
    "tts",
    "aqa",
    "imagen",
    "veo",
    "learnlm",
    "native-audio",
    "live",
    "computer-use",
    "robotics",
    "transcribe",
    // Omni models list `generateContent` but answer only the Interactions
    // API (seen 2026-10-18).
    "omni",
  ];
  list
    .get("models")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter(|m| {
      m.get("supportedGenerationMethods")
        .and_then(Value::as_array)
        .is_some_and(|ms| ms.iter().any(|x| x.as_str() == Some("generateContent")))
    })
    .filter_map(|m| m.get("name").and_then(Value::as_str).map(bare))
    .filter(|id| {
      let l = id.to_ascii_lowercase();
      l.starts_with("gemini-") && !NOT_CHAT.iter().any(|n| l.contains(n))
    })
    .map(str::to_owned)
    .collect()
}

impl Gemini {
  pub fn new(api_key: impl Into<String>) -> Self {
    Gemini {
      base_url: BASE_URL.into(),
      api_key: api_key.into(),
      effort: None,
      poll: Duration::from_secs(2),
      inline_max: INLINE_MAX,
    }
  }

  fn base(&self) -> &str {
    self.base_url.trim_end_matches('/')
  }

  fn auth(&self) -> Result<HeaderMap> {
    if self.api_key.is_empty() {
      return Err(Error::NoKey(KIND));
    }
    let mut headers = HeaderMap::new();
    headers.insert(
      "x-goog-api-key",
      HeaderValue::from_str(&self.api_key)
        .map_err(|_| Error::protocol(KIND, "the API key is not a valid header value"))?,
    );
    Ok(headers)
  }

  fn info_key(&self, model: &str) -> String {
    format!("{} {}", self.base(), bare(model))
  }

  /// The chat models this key can use (`models.list`, every page).
  pub async fn models(&self) -> Result<Vec<String>> {
    let headers = self.auth()?;
    let mut ids = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..50 {
      let mut url = format!("{}/models?pageSize=1000", self.base());
      if let Some(t) = &token {
        url.push_str("&pageToken=");
        url.push_str(&percent(t));
      }
      let page = crate::http::get_json(KIND, &url, headers.clone()).await?;
      {
        let mut infos = infos().lock().expect("model infos");
        for m in page
          .get("models")
          .and_then(Value::as_array)
          .into_iter()
          .flatten()
        {
          if let Some(name) = m.get("name").and_then(Value::as_str) {
            infos.insert(self.info_key(name), info_of(m));
          }
        }
      }
      ids.extend(chat_models(&page));
      match page.get("nextPageToken").and_then(Value::as_str) {
        Some(t) if !t.is_empty() => token = Some(t.to_owned()),
        _ => break,
      }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
  }

  /// What the API says of `model` (`models.get`), asked once per process.
  async fn info(&self, model: &str) -> Result<ModelInfo> {
    let key = self.info_key(model);
    if let Some(i) = infos().lock().expect("model infos").get(&key) {
      return Ok(*i);
    }
    let url = format!("{}/models/{}", self.base(), bare(model));
    // A model the API does not describe is asked about once, not on
    // every request.
    let info = match crate::http::get_json(KIND, &url, self.auth()?).await {
      Ok(m) => info_of(&m),
      Err(e @ Error::Api { status: 404, .. }) => {
        infos()
          .lock()
          .expect("model infos")
          .insert(key, ModelInfo::default());
        return Err(e);
      }
      Err(e) => return Err(e),
    };
    infos().lock().expect("model infos").insert(key, info);
    Ok(info)
  }

  /// The tokens `model` takes in at most (`inputTokenLimit`).
  pub async fn context_window(&self, model: &str) -> Result<Option<u64>> {
    Ok(self.info(model).await?.input)
  }

  /// The Files API's upload endpoint: the version path under `/upload`
  /// (`…/v1beta` → `…/upload/v1beta/files`).
  fn upload_url(&self) -> String {
    let base = self.base();
    let host_end = base
      .find("://")
      .and_then(|s| base[s + 3..].find('/').map(|p| s + 3 + p))
      .unwrap_or(base.len());
    format!("{}/upload{}/files", &base[..host_end], &base[host_end..])
  }

  /// A file in the Files API for `data`: one uploaded before by this
  /// process (same key, same content, not expired), else uploaded now
  /// (resumable protocol, start then upload and finalize), waiting until
  /// it is processed.
  async fn file(&self, media_type: &str, name: Option<&str>, src: Source<'_>) -> Result<Uploaded> {
    let headers = self.auth()?;
    // A blob's path names its content: the file is not read to know it
    // was uploaded.
    let key = {
      let mut h = Sha256::new();
      h.update(self.base().as_bytes());
      h.update([0]);
      h.update(self.api_key.as_bytes());
      h.update([0]);
      match src {
        Source::Bytes(data) => h.update(data),
        Source::Path(path, bytes) => {
          h.update(b"path\0");
          h.update(path.to_string_lossy().as_bytes());
          h.update(bytes.to_le_bytes());
        }
      }
      B64.encode(h.finalize())
    };
    if let Some(u) = uploads().lock().expect("uploads").get(&key)
      && u.at.elapsed() < FILE_KEPT
    {
      return Ok(u.clone());
    }
    let read;
    let data: &[u8] = match src {
      Source::Bytes(data) => data,
      Source::Path(path, _) => {
        read = std::fs::read(path)
          .map_err(|e| Error::protocol(KIND, format!("{}: {e}", path.display())))?;
        &read
      }
    };
    let client = crate::http::client();
    let mut start = headers.clone();
    let hv = |s: &str| {
      HeaderValue::from_str(s).map_err(|_| Error::protocol(KIND, format!("a bad header: {s}")))
    };
    start.insert("x-goog-upload-protocol", hv("resumable")?);
    start.insert("x-goog-upload-command", hv("start")?);
    start.insert(
      "x-goog-upload-header-content-length",
      hv(&data.len().to_string())?,
    );
    start.insert("x-goog-upload-header-content-type", hv(media_type)?);
    let r = client
      .post(self.upload_url())
      .headers(start)
      .json(&json!({"file": {"display_name": name.unwrap_or("attachment")}}))
      .timeout(Duration::from_secs(60))
      .send()
      .await?;
    let status = r.status().as_u16();
    let session = r
      .headers()
      .get("x-goog-upload-url")
      .and_then(|v| v.to_str().ok())
      .map(str::to_owned);
    let text = r.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
      return Err(crate::http::api_error(KIND, status, &text));
    }
    let session =
      session.ok_or_else(|| Error::protocol(KIND, "the upload started without a session URL"))?;
    let mut send = headers.clone();
    send.insert(CONTENT_LENGTH, hv(&data.len().to_string())?);
    send.insert("x-goog-upload-offset", hv("0")?);
    send.insert("x-goog-upload-command", hv("upload, finalize")?);
    let r = client
      .post(&session)
      .headers(send)
      .body(data.to_vec())
      .send()
      .await?;
    let status = r.status().as_u16();
    let text = r.text().await?;
    if !(200..300).contains(&status) {
      return Err(crate::http::api_error(KIND, status, &text));
    }
    let reply: Value = serde_json::from_str(&text)
      .map_err(|_| Error::protocol(KIND, "an upload reply that is not JSON"))?;
    let mut file = reply.get("file").cloned().unwrap_or(reply);
    let deadline = Instant::now() + PROCESSING;
    loop {
      match file.get("state").and_then(Value::as_str) {
        None | Some("ACTIVE") => break,
        Some("FAILED") => {
          return Err(Error::protocol(
            KIND,
            format!(
              "the file `{}` failed processing: {}",
              name.unwrap_or("attachment"),
              file
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
            ),
          ));
        }
        Some(_) if Instant::now() > deadline => {
          return Err(Error::protocol(
            KIND,
            format!(
              "the file `{}` was still processing after {} seconds",
              name.unwrap_or("attachment"),
              PROCESSING.as_secs()
            ),
          ));
        }
        Some(_) => {
          tokio::time::sleep(self.poll).await;
          let id = file.get("name").and_then(Value::as_str).unwrap_or_default();
          let url = format!("{}/{id}", self.base());
          file = crate::http::get_json(KIND, &url, headers.clone()).await?;
        }
      }
    }
    let uri = file
      .get("uri")
      .and_then(Value::as_str)
      .ok_or_else(|| Error::protocol(KIND, "an uploaded file without a URI"))?
      .to_owned();
    let mime = file
      .get("mimeType")
      .and_then(Value::as_str)
      .unwrap_or(media_type)
      .to_owned();
    let u = Uploaded {
      uri,
      mime,
      at: Instant::now(),
    };
    uploads().lock().expect("uploads").insert(key, u.clone());
    Ok(u)
  }

  /// A part for media: inline while it and the request's inline media
  /// stay small, else a file of the Files API (uploaded once).
  async fn media(
    &self,
    media_type: &str,
    name: Option<&str>,
    src: Source<'_>,
    inline_total: &mut usize,
  ) -> Result<Value> {
    let n = src.len();
    if n <= self.inline_max && *inline_total + n <= INLINE_TOTAL {
      *inline_total += n;
      let data = match src {
        Source::Bytes(b) => B64.encode(b),
        Source::Path(path, _) => B64.encode(
          std::fs::read(path)
            .map_err(|e| Error::protocol(KIND, format!("{}: {e}", path.display())))?,
        ),
      };
      return Ok(json!({"inlineData": {"mimeType": media_type, "data": data}}));
    }
    let f = self.file(media_type, name, src).await?;
    Ok(json!({"fileData": {"mimeType": f.mime, "fileUri": f.uri}}))
  }

  /// The request's `contents`: consecutive turns of one role joined, tool
  /// results in `user` turns, a model turn of this model from its own
  /// parts (signatures and images as they were).
  async fn contents(&self, r: &Request) -> Result<Vec<Value>> {
    let calls = !makes_images(&r.model);
    // Function responses name their function; results carry only an ID.
    let names: HashMap<&str, &str> = r
      .messages
      .iter()
      .flat_map(|m| &m.content)
      .filter_map(|c| match c {
        Content::ToolCall { id, name, .. } => Some((id.as_str(), name.as_str())),
        _ => None,
      })
      .collect();
    let mut inline_total = 0usize;
    let mut out: Vec<Value> = Vec::new();
    // Images of a model turn replayed natively that its own parts do not
    // hold (`generate_image` made them for it): given to the user turn
    // after it, past its function responses.
    let mut carried: Vec<Value> = Vec::new();
    for m in &r.messages {
      let role = match m.role {
        Role::User => "user",
        Role::Assistant => "model",
      };
      // Two model turns in a row: the first one's images before the next.
      if m.role == Role::Assistant && !carried.is_empty() {
        out.push(json!({"role": "user", "parts": std::mem::take(&mut carried)}));
      }
      let replayed = (m.role == Role::Assistant)
        .then(|| native_for(m, KIND, &r.model))
        .flatten()
        .and_then(|n| restore(n, &m.content));
      let mut parts = match replayed {
        Some((parts, used)) => {
          // Each with the line naming it (`[image img:…]`), as
          // `lift_images` carries them for the other adapters.
          let mut left: Vec<Value> = Vec::new();
          let mut seen = 0;
          for (i, c) in m.content.iter().enumerate() {
            let Content::Image { media_type, data } = c else {
              continue;
            };
            seen += 1;
            if seen <= used {
              continue;
            }
            if let Some(Content::Text(t)) = i.checked_sub(1).map(|j| &m.content[j])
              && t.starts_with("[image ")
            {
              left.push(json!({"text": t}));
            }
            left.push(json!({"inlineData": {
              "mimeType": crate::media::image_type(media_type, data),
              "data": B64.encode(data),
            }}));
          }
          if !left.is_empty() {
            carried.push(json!({"text": "[The images made for the model's turn above:]"}));
            carried.extend(left);
          }
          parts
        }
        None => {
          let mut parts = Vec::new();
          let mut signed = false;
          for c in &m.content {
            match c {
              Content::Text(t) if t.is_empty() => {}
              Content::Text(t) => parts.push(json!({"text": t})),
              // Gemini takes HEIC and HEIF as well: the type the bytes say.
              Content::Image { media_type, data } => parts.push(json!({
                "inlineData": {
                  "mimeType": crate::media::image_type(media_type, data),
                  "data": B64.encode(data),
                },
              })),
              Content::Document {
                media_type,
                name,
                data,
              } if reads(media_type) => {
                parts.push(
                  self
                    .media(
                      media_type,
                      name.as_deref(),
                      Source::Bytes(data),
                      &mut inline_total,
                    )
                    .await?,
                );
              }
              Content::File {
                media_type,
                name,
                path,
                bytes,
              } if reads(media_type) => {
                parts.push(
                  self
                    .media(
                      media_type,
                      name.as_deref(),
                      Source::Path(path, *bytes),
                      &mut inline_total,
                    )
                    .await?,
                );
              }
              Content::Document { .. } | Content::File { .. } => {
                parts.push(json!({"text": crate::unreadable(c)}))
              }
              // A model without function calling (the image models)
              // refuses calls in the history: they are told as text.
              Content::ToolCall { name, input, .. } if !calls => parts.push(json!({
                "text": format!("[Called the tool `{name}` with {input}]"),
              })),
              Content::ToolResult {
                id,
                output,
                is_error,
              } if !calls => parts.push(json!({"text": format!(
                "[{} of `{}`: {output}]",
                if *is_error { "The error" } else { "The result" },
                names.get(id.as_str()).copied().unwrap_or("tool"),
              )})),
              Content::ToolCall { id, name, input } => {
                let mut call = json!({
                  "name": name,
                  "args": crate::replay_tool_input(input),
                });
                if !id.starts_with(MADE_ID) {
                  call["id"] = json!(id);
                }
                let mut part = json!({"functionCall": call});
                // Gemini 3 checks the first call of a step for its
                // signature; a call it did not make has none.
                if !signed {
                  part["thoughtSignature"] = json!(SKIP_SIGNATURE);
                  signed = true;
                }
                parts.push(part);
              }
              Content::ToolResult {
                id,
                output,
                is_error,
              } => {
                let mut resp = json!({
                  "name": names.get(id.as_str()).copied().unwrap_or("tool"),
                  "response": if *is_error { json!({"error": output}) } else { json!({"output": output}) },
                });
                if !id.starts_with(MADE_ID) {
                  resp["id"] = json!(id);
                }
                parts.push(json!({"functionResponse": resp}));
              }
            }
          }
          parts
        }
      };
      if m.role == Role::User {
        parts.append(&mut carried);
      }
      if parts.is_empty() {
        continue;
      }
      match out.last_mut() {
        Some(last) if last["role"] == role => {
          if let Some(a) = last["parts"].as_array_mut() {
            a.extend(parts);
          }
        }
        _ => out.push(json!({"role": role, "parts": parts})),
      }
    }
    if !carried.is_empty() {
      out.push(json!({"role": "user", "parts": carried}));
    }
    Ok(out)
  }

  async fn body(&self, r: &Request) -> Result<Value> {
    let info = self.info(&r.model).await.unwrap_or_default();
    let images = makes_images(&r.model);
    let mut config = json!({});
    // Within the model's limit where it is known; else only a small cap
    // (titles, summaries), the model's own maximum being the default.
    match info.output {
      Some(o) => config["maxOutputTokens"] = json!(o.min(u64::from(r.max_tokens))),
      None if r.max_tokens < 32_768 => config["maxOutputTokens"] = json!(r.max_tokens),
      None => {}
    }
    if info.thinking.unwrap_or(!images) {
      let mut thinking = json!({"includeThoughts": true});
      if let Some(e) = &self.effort {
        thinking["thinkingLevel"] = json!(e);
      }
      config["thinkingConfig"] = thinking;
    }
    if images {
      config["responseModalities"] = json!(["TEXT", "IMAGE"]);
    }
    let mut body = json!({
      "contents": self.contents(r).await?,
      "generationConfig": config,
    });
    if let Some(s) = r.system.as_deref().filter(|s| !s.trim().is_empty()) {
      body["systemInstruction"] = json!({"parts": [{"text": s}]});
    }
    // Image models take no function declarations.
    if !r.tools.is_empty() && !images {
      body["tools"] = json!([{"functionDeclarations": r.tools.iter().map(|t| json!({
        "name": t.name,
        "description": t.description,
        // JSON Schema as it is: `parameters` takes only an OpenAPI subset,
        // which MCP servers' schemas often leave.
        "parametersJsonSchema": t.input_schema,
      })).collect::<Vec<_>>()}]);
      body["toolConfig"] = json!({"functionCallingConfig": {"mode": "AUTO"}});
    }
    Ok(body)
  }

  pub async fn stream(
    &self,
    request: &Request,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    let body = self.body(request).await?;
    self.send(request, &body, on_event).await
  }

  /// Makes images with an image model (`generate_image`, design §2.8,
  /// Images): the prompt after the images it works from, one turn, the
  /// aspect ratio asked for where one is.
  pub async fn generate_image(&self, r: &crate::images::ImageRequest) -> Result<Response> {
    if !makes_images(&r.model) {
      return Err(Error::protocol(
        KIND,
        format!("{} makes no images: name an image model", r.model),
      ));
    }
    let mut content: Vec<Content> = r
      .images
      .iter()
      .map(|(media_type, data)| Content::Image {
        media_type: media_type.clone(),
        data: data.clone(),
      })
      .collect();
    content.push(Content::Text(r.prompt.clone()));
    let request = Request::new(r.model.clone(), vec![crate::Message::user(content)]);
    let mut body = self.body(&request).await?;
    if let Some(ratio) = &r.aspect_ratio {
      body["generationConfig"]["imageConfig"] = json!({"aspectRatio": ratio});
    }
    self.send(&request, &body, &mut |_| {}).await
  }

  /// Answers a decision's questions (the Decisions API, `POST
  /// /decisions`): one short request, never retried (its caller has a
  /// fallback and no time to wait).
  pub async fn decide(
    &self,
    r: &crate::decisions::DecisionRequest,
  ) -> Result<crate::decisions::Decided> {
    if !decides(&r.model) {
      return Err(Error::protocol(
        KIND,
        format!("{} answers no decisions: name a decision model", r.model),
      ));
    }
    let mut headers = self.auth()?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let url = format!("{}/decisions", self.base());
    let body = crate::decisions::body(r);
    let reply = crate::http::post_json_within(KIND, &url, headers, &body, r.timeout).await?;
    crate::decisions::parse(r, &reply, KIND)
  }

  async fn send(
    &self,
    request: &Request,
    body: &Value,
    on_event: &mut (dyn FnMut(Event) + Send),
  ) -> Result<Response> {
    let mut headers = self.auth()?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let url = format!(
      "{}/models/{}:streamGenerateContent?alt=sse",
      self.base(),
      bare(&request.model)
    );
    let mut state = StreamState {
      model: request.model.clone(),
      ..StreamState::default()
    };
    post_sse(KIND, &url, headers, body, |e| state.on_sse(e, on_event)).await?;
    state.finish()
  }
}

/// A model turn's parts from its native copy: the images, kept without
/// their bytes (they are the message's attachments), filled from the
/// message's images in order, and how many it took. `None` when they do
/// not match, so the turn is sent from its content instead.
fn restore(native: &Value, content: &[Content]) -> Option<(Vec<Value>, usize)> {
  let mut images = content.iter().filter_map(|c| match c {
    Content::Image { media_type, data } => Some((media_type, data)),
    _ => None,
  });
  let mut parts = native.get("parts")?.as_array()?.clone();
  let mut used = 0;
  for p in &mut parts {
    if let Some(d) = p.get_mut("inlineData")
      && d.get("data").is_none()
    {
      let (media_type, data) = images.next()?;
      d["mimeType"] = json!(crate::media::image_type(media_type, data));
      d["data"] = json!(B64.encode(data));
      used += 1;
    }
  }
  Some((parts, used))
}

/// Media to send: bytes in memory, or a file read only when it is.
#[derive(Clone, Copy)]
enum Source<'a> {
  Bytes(&'a [u8]),
  Path(&'a std::path::Path, u64),
}

impl Source<'_> {
  fn len(&self) -> usize {
    match self {
      Source::Bytes(b) => b.len(),
      Source::Path(_, n) => usize::try_from(*n).unwrap_or(usize::MAX),
    }
  }
}

/// Query values are percent-encoded whole (a page token may hold `=`).
fn percent(s: &str) -> String {
  s.bytes()
    .map(|b| match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
        (b as char).to_string()
      }
      _ => format!("%{b:02X}"),
    })
    .collect()
}

#[derive(Default)]
struct StreamState {
  model: String,
  /// The model turn's parts as streamed, text pieces joined.
  parts: Vec<Value>,
  text: String,
  images: Vec<Content>,
  calls: Vec<Content>,
  finish: Option<String>,
  blocked: Option<String>,
  usage: Usage,
}

impl StreamState {
  fn on_sse(&mut self, e: SseEvent, on_event: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
    if e.data.trim().is_empty() {
      return Ok(());
    }
    let v: Value = serde_json::from_str(&e.data)
      .map_err(|err| Error::protocol(KIND, format!("bad chunk: {err}")))?;
    if let Some(err) = v.get("error") {
      let field = |k: &str| err.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
      return Err(Error::Api {
        provider: KIND,
        status: err.get("code").and_then(Value::as_u64).unwrap_or(500) as u16,
        kind: field("status"),
        message: field("message"),
      });
    }
    if let Some(u) = v.get("usageMetadata") {
      let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
      let cached = n("cachedContentTokenCount");
      self.usage.input_tokens =
        (n("promptTokenCount") + n("toolUsePromptTokenCount")).saturating_sub(cached);
      self.usage.cache_read_tokens = cached;
      // Thinking is output, billed as such.
      self.usage.output_tokens = n("candidatesTokenCount") + n("thoughtsTokenCount");
    }
    if let Some(reason) = v
      .pointer("/promptFeedback/blockReason")
      .and_then(Value::as_str)
    {
      self.blocked = Some(reason.to_owned());
    }
    let Some(candidate) = v.get("candidates").and_then(|c| c.get(0)) else {
      return Ok(());
    };
    for part in candidate
      .pointer("/content/parts")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
    {
      self.on_part(part, on_event)?;
    }
    if let Some(r) = candidate.get("finishReason").and_then(Value::as_str) {
      self.finish = Some(r.to_owned());
    }
    Ok(())
  }

  fn on_part(&mut self, part: &Value, on_event: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
    let thought = part.get("thought").and_then(Value::as_bool) == Some(true);
    if let Some(call) = part.get("functionCall") {
      let name = call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
      let id = call
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{MADE_ID}{}-{name}", self.calls.len()));
      let input = crate::decode_tool_input(call.get("args").cloned().unwrap_or_else(|| json!({})));
      on_event(Event::ToolCallStart {
        id: id.clone(),
        name: name.clone(),
      });
      on_event(Event::ToolCallInput {
        id: id.clone(),
        partial: input.to_string(),
      });
      self.calls.push(Content::ToolCall { id, name, input });
      self.parts.push(part.clone());
      return Ok(());
    }
    if let Some(d) = part.get("inlineData") {
      // Interim images of the model's thinking are not its answer.
      if thought {
        return Ok(());
      }
      let media_type = d
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_owned();
      let data = B64
        .decode(d.get("data").and_then(Value::as_str).unwrap_or_default())
        .map_err(|e| Error::protocol(KIND, format!("an inline image that is not base64: {e}")))?;
      if media_type.starts_with("image/") {
        self.images.push(Content::Image { media_type, data });
        // Kept without its bytes: the message's attachment holds them.
        let mut kept = part.clone();
        if let Some(d) = kept.get_mut("inlineData").and_then(Value::as_object_mut) {
          d.remove("data");
        }
        self.parts.push(kept);
      }
      return Ok(());
    }
    if let Some(t) = part.get("text").and_then(Value::as_str) {
      if !t.is_empty() {
        if thought {
          on_event(Event::Thinking(t.to_owned()));
        } else {
          self.text.push_str(t);
          on_event(Event::Text(t.to_owned()));
        }
      }
      let signature = part.get("thoughtSignature").cloned();
      // Pieces of one part are joined, up to its signature, which ends it.
      if let Some(last) = self.parts.last_mut()
        && last.get("text").is_some()
        && (last.get("thought").and_then(Value::as_bool) == Some(true)) == thought
        && last.get("thoughtSignature").is_none()
      {
        let joined = format!("{}{t}", last["text"].as_str().unwrap_or_default());
        last["text"] = json!(joined);
        if let Some(s) = signature {
          last["thoughtSignature"] = s;
        }
      } else if !t.is_empty() || signature.is_some() {
        self.parts.push(part.clone());
      }
      return Ok(());
    }
    // Anything else (code execution, grounding) is kept for the replay.
    if part.get("thoughtSignature").is_some() {
      self.parts.push(part.clone());
    }
    Ok(())
  }

  fn finish(self) -> Result<Response> {
    let stop = match (self.finish.as_deref(), &self.blocked) {
      (_, Some(reason)) if self.parts.is_empty() => StopReason::Refusal {
        category: Some(reason.clone()),
      },
      (None, _) => {
        return Err(Error::cut(KIND, "the stream ended before a finish reason"));
      }
      (Some("STOP"), _) if !self.calls.is_empty() => StopReason::ToolUse,
      (Some("STOP" | "FINISH_REASON_UNSPECIFIED"), _) => StopReason::EndTurn,
      (Some("MAX_TOKENS"), _) => StopReason::MaxTokens,
      (
        Some(
          r @ ("SAFETY"
          | "RECITATION"
          | "BLOCKLIST"
          | "PROHIBITED_CONTENT"
          | "SPII"
          | "IMAGE_SAFETY"
          | "IMAGE_PROHIBITED_CONTENT"
          | "IMAGE_RECITATION"),
        ),
        _,
      ) => StopReason::Refusal {
        category: Some(r.to_owned()),
      },
      (Some(r), _) => StopReason::Other(r.to_owned()),
    };
    let mut content = Vec::new();
    if !self.text.is_empty() {
      content.push(Content::Text(self.text));
    }
    content.extend(self.images);
    content.extend(self.calls);
    // A turn with nothing to replay (no signature, no image) needs no
    // copy of its own: its content says it all.
    let native = self
      .parts
      .iter()
      .any(|p| p.get("thoughtSignature").is_some() || p.get("inlineData").is_some())
      .then(|| Native {
        provider: KIND.into(),
        model: self.model.clone(),
        content: json!({"parts": self.parts}),
      });
    Ok(Response {
      model: self.model,
      content,
      native,
      stop,
      usage: self.usage,
      cost: None,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_upload_endpoint_is_the_version_path_under_upload() {
    assert_eq!(
      Gemini::new("k").upload_url(),
      "https://generativelanguage.googleapis.com/upload/v1beta/files"
    );
    let mut g = Gemini::new("k");
    g.base_url = "http://127.0.0.1:9/v1beta/".into();
    assert_eq!(g.upload_url(), "http://127.0.0.1:9/upload/v1beta/files");
  }

  #[test]
  fn model_lists_keep_chat_and_image_models() {
    let list = json!({"models": [
      {"name": "models/gemini-3.8-flash", "supportedGenerationMethods": ["generateContent", "countTokens"]},
      {"name": "models/gemini-3.1-flash-image", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent"]},
      {"name": "models/gemini-3.5-flash-preview-tts", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-3.5-flash-native-audio", "supportedGenerationMethods": ["bidiGenerateContent"]},
      {"name": "models/imagen-5.0-generate", "supportedGenerationMethods": ["predict"]},
      {"name": "models/gemma-4-27b-it", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-omni-1.1-flash", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-3.5-transcribe", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/gemini-nano-banana-2.1", "supportedGenerationMethods": ["generateContent"]},
      {"name": "models/aqa", "supportedGenerationMethods": ["generateAnswer"]}
    ]});
    assert_eq!(
      chat_models(&list),
      [
        "gemini-3.8-flash",
        "gemini-3.1-flash-image",
        "gemini-nano-banana-2.1"
      ]
    );
    assert!(makes_images("gemini-3.1-flash-image"));
    assert!(makes_images("gemini-nano-banana-2.1"));
    assert!(!makes_images("gemini-3.8-flash"));
  }

  #[tokio::test]
  async fn an_image_goes_as_its_bytes_say() {
    let r = Request::new(
      "gemini-x",
      vec![crate::Message::user(
        vec![crate::media::png_labelled_jpeg()],
      )],
    );
    let contents = Gemini::new("k").contents(&r).await.unwrap();
    assert_eq!(
      contents[0]["parts"][0]["inlineData"]["mimeType"],
      "image/png"
    );
  }

  #[test]
  fn streamed_text_pieces_join_up_to_their_signature() {
    let mut s = StreamState {
      model: "m".into(),
      ..StreamState::default()
    };
    let mut events = Vec::new();
    let mut on = |e: Event| events.push(e);
    for chunk in [
      json!({"candidates": [{"content": {"parts": [{"text": "Plan", "thought": true}]}}]}),
      json!({"candidates": [{"content": {"parts": [{"text": "Hel"}]}}]}),
      json!({"candidates": [{"content": {"parts": [{"text": "lo", "thoughtSignature": "c2ln"}]}}]}),
      json!({"candidates": [{"content": {"parts": [{"text": "", "thoughtSignature": "c2lnMg=="}]}, "finishReason": "STOP"}],
             "usageMetadata": {"promptTokenCount": 100, "cachedContentTokenCount": 60, "candidatesTokenCount": 5, "thoughtsTokenCount": 7}}),
    ] {
      s.on_sse(
        SseEvent {
          event: None,
          data: chunk.to_string(),
        },
        &mut on,
      )
      .unwrap();
    }
    let r = s.finish().unwrap();
    assert_eq!(r.text(), "Hello");
    assert_eq!(
      r.native.unwrap().content,
      json!({"parts": [
        {"text": "Plan", "thought": true},
        {"text": "Hello", "thoughtSignature": "c2ln"},
        {"text": "", "thoughtSignature": "c2lnMg=="}
      ]})
    );
    assert_eq!(
      r.usage,
      Usage {
        input_tokens: 40,
        cache_read_tokens: 60,
        output_tokens: 12,
        ..Usage::default()
      }
    );
    assert_eq!(r.stop, StopReason::EndTurn);
    assert_eq!(events[0], Event::Thinking("Plan".into()));
  }
}
