//! Context windows: the tokens a model takes in at most, which compaction
//! measures a thread against (`fork::needs_compacting`). The provider's
//! `context_window` in `config.toml` when set; else what the provider says
//! of the model, asked once and kept while the node runs; else a guess by
//! model name.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::node::Shared;

/// How long a lookup that failed or told nothing stands before the
/// provider is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(600);
/// How long a lookup may take; the guess stands in meanwhile.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// What the providers said, by `provider/model`.
#[derive(Default)]
pub(crate) struct Windows(Mutex<HashMap<String, (Instant, Option<u64>)>>);

impl Windows {
  fn known(&self, model: &str) -> Option<Option<u64>> {
    let map = self.0.lock().expect("windows lock");
    match map.get(model) {
      // Antigravity's moves when agy is seen compacting (design §2.8):
      // asked again (no network: the bridge's own numbers).
      Some((at, Some(_))) if model.starts_with("antigravity/") && at.elapsed() >= RETRY_AFTER => {
        None
      }
      Some((_, Some(w))) => Some(Some(*w)),
      Some((at, None)) if at.elapsed() < RETRY_AFTER => Some(None),
      _ => None,
    }
  }

  fn learn(&self, model: &str, window: Option<u64>) {
    self
      .0
      .lock()
      .expect("windows lock")
      .insert(model.to_owned(), (Instant::now(), window));
  }
}

/// The window set in `config.toml` for `model`'s provider.
pub(crate) fn configured(s: &Shared, model: &str) -> Option<u64> {
  let (provider, _) = model.split_once('/')?;
  s.config.providers.get(provider)?.context_window
}

/// The tokens `model` (`provider/model`) takes in at most.
pub(crate) async fn of(s: &Shared, model: &str) -> u64 {
  if let Some(w) = configured(s, model) {
    return w;
  }
  let name = model.split_once('/').map_or(model, |(_, m)| m);
  let told = match s.windows.known(model) {
    Some(w) => w,
    None => {
      let asked = async {
        let (provider, _, m) = s.config.resolve_authenticated(&s.data, Some(model)).await?;
        Ok::<_, crate::Error>(provider.context_window(&m).await?)
      };
      let told = match tokio::time::timeout(LOOKUP_TIMEOUT, asked).await {
        Ok(Ok(w)) => w,
        Ok(Err(e)) => {
          eprintln!("{model}: context window not looked up: {e}");
          None
        }
        Err(_) => {
          eprintln!("{model}: context window not looked up: timed out");
          None
        }
      };
      s.windows.learn(model, told);
      told
    }
  };
  told.unwrap_or_else(|| guess(name))
}

/// A window by model name, for providers that do not say (OpenAI's API
/// lists no windows): the documented windows of each family, October
/// 2026, and a conservative 128k for the rest.
pub(crate) fn guess(name: &str) -> u64 {
  let name = name.to_ascii_lowercase();
  let is = |prefixes: &[&str]| prefixes.iter().any(|p| name.starts_with(p));
  if let Some(claude) = name.strip_prefix("claude-") {
    claude_window(claude)
  } else if is(&["gpt-5.4-mini", "gpt-5.4-nano"]) {
    400_000
  } else if is(&[
    "gpt-6", "gpt-5.4", "gpt-5.5", "gpt-5.6", "gpt-5.7", "gpt-5.8", "gpt-5.9",
  ]) {
    1_050_000
  } else if is(&["gpt-5"]) {
    400_000
  } else if is(&["gpt-4.1"]) {
    1_047_576
  } else if is(&["o3", "o4"]) {
    200_000
  } else if is(&["grok-4.3", "grok-4.20"]) {
    1_000_000
  } else if is(&["grok-4.5", "grok-4.6", "grok-4.7"]) {
    500_000
  } else if is(&["grok"]) {
    256_000
  } else if is(&["gemini"]) {
    1_000_000
  } else if is(&["kimi-k3"]) {
    1_048_576
  } else if is(&["kimi-k2", "kimi-for-coding"]) {
    262_144
  } else if is(&["deepseek-flash", "deepseek-v4"]) {
    1_000_000
  } else {
    128_000
  }
}

/// Claude models before 4.6 (and Haiku 4.5) take 200k; 4.6 and every
/// family since, 1M.
fn claude_window(name: &str) -> u64 {
  let mut parts = name.split('-');
  let (family, major, minor) = (parts.next(), parts.next(), parts.next());
  let old = match (family, major) {
    (Some("haiku"), Some("4")) => true,
    (Some("opus" | "sonnet"), Some("4")) => !matches!(minor, Some("6" | "7" | "8" | "9")),
    // `claude-3-5-sonnet`, `claude-2.1`, `claude-instant-1.2`.
    (Some(f), _) => f.starts_with('2') || f.starts_with('3') || f == "instant",
    _ => false,
  };
  if old { 200_000 } else { 1_000_000 }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn current_claude_models_take_a_million_tokens() {
    for m in [
      "claude-opus-5-5",
      "claude-fable-5-2",
      "claude-fable-5-1",
      "claude-sonnet-5-5",
      "claude-haiku-5-5",
      "claude-opus-4-6",
      "claude-sonnet-4-6",
    ] {
      assert_eq!(guess(m), 1_000_000, "{m}");
    }
    for m in [
      "claude-haiku-4-5",
      "claude-sonnet-4-5",
      "claude-sonnet-4-20250514",
      "claude-opus-4-1",
      "claude-3-5-sonnet-latest",
    ] {
      assert_eq!(guess(m), 200_000, "{m}");
    }
  }

  #[test]
  fn other_families_by_their_documented_windows() {
    for (m, w) in [
      ("gpt-6-sol", 1_050_000),
      ("gpt-5.6-terra", 1_050_000),
      ("gpt-5.4", 1_050_000),
      ("gpt-5.4-mini", 400_000),
      ("gpt-5.2", 400_000),
      ("gpt-5-mini", 400_000),
      ("o3-pro", 200_000),
      ("grok-4.7", 500_000),
      ("grok-4.20-0309-reasoning", 1_000_000),
      ("grok-build-0.1", 256_000),
      ("kimi-k3.1-pro", 1_048_576),
      ("kimi-k3.1-pro-highspeed", 1_048_576),
      ("kimi-k3", 1_048_576),
      ("kimi-k2.6", 262_144),
      ("deepseek-flash", 1_000_000),
      ("deepseek-v4-pro", 1_000_000),
      ("qwen3:32b", 128_000),
    ] {
      assert_eq!(guess(m), w, "{m}");
    }
  }
}
