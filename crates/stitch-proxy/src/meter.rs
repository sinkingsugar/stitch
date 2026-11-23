//! Usage counted from what the provider answers: a JSON body's `usage`, or
//! the usage a server-sent event stream carries (Anthropic's
//! `message_start` and `message_delta`, OpenAI's last Chat Completions
//! chunk, Responses' `response.completed`). Fed the bytes as they pass,
//! without holding the stream back.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_providers::sse::SseParser;

/// A JSON body is kept up to this size to read its usage at the end.
const MAX_JSON: usize = 8 << 20;

/// Tokens of a request, or summed over many. `input` is the prompt not
/// read from or written to the cache (Anthropic's sense; OpenAI's prompt
/// count includes cached tokens, which are taken out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
  #[serde(default)]
  pub requests: u64,
  #[serde(default)]
  pub input: u64,
  #[serde(default)]
  pub output: u64,
  #[serde(default)]
  pub cache_read: u64,
  #[serde(default)]
  pub cache_write: u64,
}

impl Counts {
  /// What a budget counts: every token, cache reads at a tenth (what
  /// they cost), so a long cached conversation does not spend a day's
  /// budget on rereading itself.
  pub fn budget(&self) -> u64 {
    self.input + self.output + self.cache_write + self.cache_read.div_ceil(10)
  }

  pub fn add(&mut self, o: &Counts) {
    self.requests += o.requests;
    self.input += o.input;
    self.output += o.output;
    self.cache_read += o.cache_read;
    self.cache_write += o.cache_write;
  }

  /// The larger of each field: streams repeat usage cumulatively
  /// (Anthropic's `message_delta` says the output so far).
  fn max(&mut self, o: &Counts) {
    self.input = self.input.max(o.input);
    self.output = self.output.max(o.output);
    self.cache_read = self.cache_read.max(o.cache_read);
    self.cache_write = self.cache_write.max(o.cache_write);
  }
}

/// The usage object of a provider's JSON (`usage`, `message.usage`,
/// `response.usage`), as counts.
pub fn usage_of(v: &Value) -> Option<Counts> {
  let u = v
    .get("usage")
    .or_else(|| v.get("message").and_then(|m| m.get("usage")))
    .or_else(|| v.get("response").and_then(|m| m.get("usage")))
    .filter(|u| u.is_object())?;
  let n = |k: &str| u.get(k).and_then(Value::as_u64);
  let cached = u
    .get("prompt_tokens_details")
    .or_else(|| u.get("input_tokens_details"))
    .and_then(|d| d.get("cached_tokens"))
    .and_then(Value::as_u64);
  let prompt = n("input_tokens")
    .or_else(|| n("prompt_tokens"))
    .unwrap_or(0);
  let (input, cache_read) = match cached {
    // OpenAI's prompt count includes what was read from the cache.
    Some(c) => (prompt.saturating_sub(c), c),
    None => (prompt, n("cache_read_input_tokens").unwrap_or(0)),
  };
  Some(Counts {
    requests: 0,
    input,
    output: n("output_tokens")
      .or_else(|| n("completion_tokens"))
      .unwrap_or(0),
    cache_read,
    cache_write: n("cache_creation_input_tokens").unwrap_or(0),
  })
}

pub enum Meter {
  Sse {
    parser: SseParser,
    counts: Counts,
  },
  Json {
    buf: Vec<u8>,
  },
  /// Not a body usage is read from (a model list, an error page).
  None,
}

impl Meter {
  /// The meter for a response of this content type.
  pub fn for_content_type(content_type: &str) -> Meter {
    if content_type.starts_with("text/event-stream") {
      Meter::Sse {
        parser: SseParser::default(),
        counts: Counts::default(),
      }
    } else if content_type.contains("json") {
      Meter::Json { buf: Vec::new() }
    } else {
      Meter::None
    }
  }

  pub fn feed(&mut self, bytes: &[u8]) {
    match self {
      Meter::Sse { parser, counts } => {
        for e in parser.feed(bytes) {
          if let Ok(v) = serde_json::from_str::<Value>(&e.data)
            && let Some(c) = usage_of(&v)
          {
            counts.max(&c);
          }
        }
      }
      Meter::Json { buf } => {
        if buf.len() + bytes.len() <= MAX_JSON {
          buf.extend_from_slice(bytes);
        }
      }
      Meter::None => {}
    }
  }

  /// What the response used, one request.
  pub fn finish(self) -> Counts {
    let mut c = match self {
      Meter::Sse { counts, .. } => counts,
      Meter::Json { buf } => serde_json::from_slice::<Value>(&buf)
        .ok()
        .and_then(|v| usage_of(&v))
        .unwrap_or_default(),
      Meter::None => Counts::default(),
    };
    c.requests = 1;
    c
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fed(content_type: &str, chunks: &[&str]) -> Counts {
    let mut m = Meter::for_content_type(content_type);
    for c in chunks {
      m.feed(c.as_bytes());
    }
    m.finish()
  }

  #[test]
  fn anthropic_stream_split_anywhere() {
    let stream = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":2000,\"output_tokens\":1}}}\n\n\
      event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n\
      event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":42}}\n\n";
    // Fed three bytes at a time: events split across chunks.
    let bytes: Vec<String> = stream
      .as_bytes()
      .chunks(3)
      .map(|c| String::from_utf8_lossy(c).into_owned())
      .collect();
    let refs: Vec<&str> = bytes.iter().map(String::as_str).collect();
    let c = fed("text/event-stream; charset=utf-8", &refs);
    assert_eq!(
      c,
      Counts {
        requests: 1,
        input: 12,
        output: 42,
        cache_read: 2000,
        cache_write: 100
      }
    );
    assert_eq!(c.budget(), 12 + 42 + 100 + 200);
  }

  #[test]
  fn openai_chat_stream_usage_chunk() {
    let c = fed(
      "text/event-stream",
      &[
        "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":30}}}\n\n",
        "data: [DONE]\n\n",
      ],
    );
    assert_eq!((c.input, c.cache_read, c.output), (20, 30, 7));
  }

  #[test]
  fn responses_completed_and_json() {
    let c = fed(
      "text/event-stream",
      &[
        "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":9,\"output_tokens\":3}}}\n\n",
      ],
    );
    assert_eq!((c.input, c.output), (9, 3));
    let c = fed(
      "application/json",
      &[
        "{\"id\":\"m\",\"usage\":{\"input_tokens\":",
        "5,\"output_tokens\":6}}",
      ],
    );
    assert_eq!((c.requests, c.input, c.output), (1, 5, 6));
    assert_eq!(fed("text/html", &["<html>"]).budget(), 0);
  }
}
