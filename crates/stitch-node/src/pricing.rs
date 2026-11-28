//! Cost estimates for usage events, in USD, for models with known prices
//! (list prices, October 2026: Anthropic's, Moonshot's Kimi, DeepSeek's).
//! Each model's cache writes are priced as its provider bills them:
//! Anthropic's at 1.25 times input for five minutes and twice for an
//! hour, Kimi's at once and twice input, DeepSeek's not at all. Unknown
//! models have no estimate.

use stitch_providers::Usage;

/// A model's prices in USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Prices {
  input: f64,
  output: f64,
  cache_read: f64,
  /// Writes cached for five minutes (or unsaid), and for an hour.
  write_5m: f64,
  write_1h: f64,
}

/// Anthropic's: writes at 1.25 and 2 times input.
const fn claude(input: f64, output: f64, cache_read: f64) -> Prices {
  Prices {
    input,
    output,
    cache_read,
    write_5m: input * 1.25,
    write_1h: input * 2.0,
  }
}

/// Moonshot's K3 series: writes at once (5m) and twice (1h) input.
const fn kimi(input: f64, output: f64, cache_read: f64) -> Prices {
  Prices {
    input,
    output,
    cache_read,
    write_5m: input,
    write_1h: input * 2.0,
  }
}

/// DeepSeek's, at its peak rates (off-peak hours cost half): it bills no
/// cache writes.
const fn deepseek(input: f64, output: f64, cache_read: f64) -> Prices {
  Prices {
    input,
    output,
    cache_read,
    write_5m: input,
    write_1h: input,
  }
}

fn prices(model: &str) -> Option<Prices> {
  Some(match model {
    "claude-fable-5-2" | "claude-mythos-5-2" => claude(8.0, 40.0, 0.20),
    "claude-fable-5-1" | "claude-mythos-5-1" => claude(10.0, 50.0, 0.25),
    "claude-fable-5" => claude(10.0, 50.0, 1.0),
    "claude-opus-5-5" => claude(4.0, 20.0, 0.20),
    "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => {
      claude(5.0, 25.0, 0.50)
    }
    "claude-sonnet-5-5" | "claude-sonnet-5" => claude(2.0, 10.0, 0.20),
    "claude-sonnet-4-6" => claude(3.0, 15.0, 0.30),
    "claude-haiku-5-5" => claude(0.10, 0.50, 0.01),
    "claude-haiku-4-5" => claude(1.0, 5.0, 0.10),
    // platform.kimi.ai/docs/pricing/chat, 2026-10-29.
    "kimi-k3.1-pro" => kimi(2.0, 10.0, 0.20),
    "kimi-k3.1-pro-highspeed" => kimi(4.0, 20.0, 0.20),
    "kimi-k3" => kimi(3.0, 15.0, 0.30),
    // K2 writes nothing to the cache.
    "kimi-k2.7-code" | "kimi-k2.6" => kimi(0.95, 4.0, 0.19),
    "kimi-k2.7-code-highspeed" => kimi(1.90, 8.0, 0.38),
    // api-docs.deepseek.com/quick_start/pricing, 2026-10-29, peak.
    "deepseek-flash" | "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" => {
      deepseek(0.30, 1.20, 0.006)
    }
    "deepseek-v4-pro" => deepseek(1.32, 3.96, 0.044),
    _ => return None,
  })
}

/// (input, output) list prices in USD per million tokens, shown by
/// `models` (`modelset`): the ones above, and other providers' known ones
/// (Google's, for prompts up to 200k tokens). Providers' model lists carry
/// no prices; `stitch model set <pattern> --price` gives the rest.
pub fn list_price(model: &str) -> Option<(f64, f64)> {
  if let Some(p) = prices(model) {
    return Some((p.input, p.output));
  }
  let model = model.strip_suffix("-preview").unwrap_or(model);
  Some(match model {
    "claude-opus-4-5-20251101" => (5.0, 25.0),
    "claude-sonnet-4-5-20250929" => (3.0, 15.0),
    "claude-haiku-4-5-20251001" => (1.0, 5.0),
    "gemini-2.5-pro" => (1.25, 10.0),
    "gemini-2.5-flash" => (0.30, 2.50),
    "gemini-2.5-flash-lite" => (0.10, 0.40),
    "gemini-3-pro" | "gemini-3.1-pro" => (2.0, 12.0),
    "gemini-3-flash" | "gemini-3.5-flash" => (0.50, 3.0),
    // Decisions (design §2.28): input only, no output is generated.
    "gemini-decision-1" => (0.10, 0.0),
    _ => return None,
  })
}

/// (input, output) list prices in USD per million tokens of image models
/// (Google's, October 2026; output at the image rate, the dearer one).
pub fn image_price(model: &str) -> Option<(f64, f64)> {
  let model = model.strip_suffix("-preview").unwrap_or(model);
  Some(match model {
    "gemini-3.1-flash-image" => (0.50, 60.0),
    "gemini-3.1-flash-lite-image" => (0.50, 30.0),
    "gemini-2.5-flash-image" => (0.30, 30.0),
    "gemini-3-pro-image" => (2.0, 120.0),
    _ => return None,
  })
}

/// What a model call cost: as the provider said, else estimated from its
/// usage at list prices (cache reads at a tenth of input, writes at 1.25
/// times); `None` when its price is not known. `model` may be
/// `provider/model`.
pub fn priced(model: &str, u: &Usage, usd: Option<f64>) -> Option<f64> {
  if usd.is_some() {
    return usd;
  }
  // A plan that bills no tokens, whatever its models' names elsewhere.
  if model.starts_with("antigravity/") {
    return None;
  }
  let name = model.rsplit('/').next().unwrap_or(model);
  if let Some(e) = estimate(name, u) {
    return Some(e);
  }
  let (input, output) = list_price(name).or_else(|| image_price(name))?;
  let m = 1_000_000.0;
  Some(
    u.input_tokens as f64 * input / m
      + u.output_tokens as f64 * output / m
      + u.cache_read_tokens as f64 * input / 10.0 / m
      + u.cache_write_tokens as f64 * input * 1.25 / m,
  )
}

pub fn estimate(model: &str, u: &Usage) -> Option<f64> {
  let p = prices(model)?;
  let m = 1_000_000.0;
  Some(
    u.input_tokens as f64 * p.input / m
      + u.output_tokens as f64 * p.output / m
      + u.cache_read_tokens as f64 * p.cache_read / m
      + u.cache_write_tokens.saturating_sub(u.cache_write_1h_tokens) as f64 * p.write_5m / m
      + u.cache_write_1h_tokens as f64 * p.write_1h / m,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn usage(input: u64, output: u64, read: u64, write: u64, write_1h: u64) -> Usage {
    Usage {
      input_tokens: input,
      output_tokens: output,
      cache_read_tokens: read,
      cache_write_tokens: write,
      cache_write_1h_tokens: write_1h,
    }
  }

  fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
  }

  #[test]
  fn cache_writes_are_priced_as_each_provider_bills_them() {
    // A million written for five minutes and a million for an hour.
    let u = usage(0, 0, 0, 2_000_000, 1_000_000);
    // Claude: 1.25 and 2 times input.
    assert!(close(
      estimate("claude-opus-5-5", &u).unwrap(),
      4.0 * 1.25 + 8.0
    ));
    // Kimi: once and twice input.
    assert!(close(estimate("kimi-k3.1-pro", &u).unwrap(), 2.0 + 4.0));
    assert!(close(estimate("kimi-k3", &u).unwrap(), 3.0 + 6.0));
  }

  #[test]
  fn kimi_and_deepseek_are_priced() {
    let u = usage(1_000_000, 1_000_000, 1_000_000, 0, 0);
    assert!(close(
      estimate("kimi-k3.1-pro", &u).unwrap(),
      2.0 + 10.0 + 0.20
    ));
    assert!(close(
      estimate("kimi-k3.1-pro-highspeed", &u).unwrap(),
      4.0 + 20.0 + 0.20
    ));
    // DeepSeek at its peak rates, its cache hits at a fiftieth of input.
    assert!(close(
      estimate("deepseek-flash", &u).unwrap(),
      0.30 + 1.20 + 0.006
    ));
    assert!(close(
      priced("deepseek/deepseek-v4-pro", &u, None).unwrap(),
      1.32 + 3.96 + 0.044
    ));
    assert_eq!(list_price("kimi-k3.1-pro"), Some((2.0, 10.0)));
  }
}
