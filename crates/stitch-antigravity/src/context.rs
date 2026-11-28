//! Context windows and usage, so that Stitch compacts a thread before agy
//! would (design §2.8, Antigravity: Stitch owns compaction).
//!
//! agy's windows (its `usage_update` `size`, 1.5.19): Gemini's 1,048,576
//! (4 and its orchestrators too), Claude 4.x's 250,000 and Claude 5.5's
//! 1,000,000, GPT-OSS's 131,072. It compacts a conversation on its
//! own at a limit below that, set by the server (its embedded defaults
//! name limits of 80k to 840k), which `agy acp` does not say and its
//! settings cannot turn off. So the window told to Stitch is a guess at
//! that limit, and once agy is seen compacting (the context it reports
//! falls by more than 30%), the size it had just before, learned for the
//! model.
//!
//! agy tells usage once per prompt, summed over the prompt's model calls,
//! where Stitch reads a step's prompt size from that step's usage (its
//! compaction measures the latest). So each step is told the context agy
//! reports for its latest request (`used`), the part the step before had
//! counted as cache reads; the last step of a turn gets the turn's output
//! and the cache reads agy counted that the earlier steps did not.

use stitch_providers::Usage;

/// The window agy has for `model`, by family, when no session said it.
pub fn known_size(model: &str) -> Option<u64> {
  if model.starts_with("gemini") {
    Some(1_048_576)
  } else if model.starts_with("claude-opus-5") || model.starts_with("claude-sonnet-5") {
    Some(1_000_000)
  } else if model.starts_with("claude") {
    Some(250_000)
  } else if model.starts_with("gpt-oss") {
    Some(131_072)
  } else {
    None
  }
}

/// Where agy may compact a window of `size` on its own: the limits its
/// embedded defaults name for windows of that class.
pub fn compaction_guess(size: u64) -> u64 {
  let guess = if size >= 1_000_000 {
    512_000
  } else if size >= 200_000 {
    160_000
  } else {
    80_000
  };
  guess.min(size)
}

/// Below this context, a fall is not taken for a compaction.
const COMPACTION_FLOOR: u64 = 20_000;

/// A session's usage as it is told, step by step.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
  /// The context of the session's latest request (`used`).
  last_used: u64,
  /// The context before a compaction seen in this step.
  compacted_from: Option<u64>,
  /// The context the step before was told.
  prev: u64,
  /// What this turn's earlier steps were told, and how many.
  told_cache: u64,
  steps: u32,
}

impl Meter {
  /// A `usage_update`'s `used`; the context before, when agy compacted.
  pub fn used(&mut self, used: u64) -> Option<u64> {
    let before = self.last_used;
    self.last_used = used;
    if before >= COMPACTION_FLOOR && used * 10 < before * 7 {
      self.compacted_from = Some(self.compacted_from.unwrap_or(0).max(before));
      return Some(before);
    }
    None
  }

  /// A new prompt: its steps are counted afresh.
  pub fn new_turn(&mut self) {
    self.steps = 0;
    self.told_cache = 0;
    self.compacted_from = None;
  }

  fn context(&mut self) -> u64 {
    self.compacted_from.take().unwrap_or(self.last_used)
  }

  /// A step that ends with a tool call: its prompt as agy last reported
  /// it, the previous step's part as cache reads; no output yet.
  pub fn step(&mut self) -> Usage {
    let ctx = self.context();
    let cache = self.prev.min(ctx);
    self.prev = ctx;
    self.told_cache += cache;
    self.steps += 1;
    Usage {
      input_tokens: ctx - cache,
      cache_read_tokens: cache,
      ..Usage::default()
    }
  }

  /// The turn's last step, given the prompt's `total`: exact when it was
  /// the turn's only step; else its context, with the cache reads the
  /// earlier steps were not told, and the turn's output.
  pub fn end(&mut self, total: Usage) -> Usage {
    let compacted = self.compacted_from.is_some();
    let ctx = self.context();
    let steps = self.steps;
    self.steps = 0;
    let told = std::mem::take(&mut self.told_cache);
    self.prev = ctx;
    if steps == 0 && !compacted {
      return total;
    }
    let cache = total.cache_read_tokens.saturating_sub(told).min(ctx);
    Usage {
      input_tokens: ctx - cache,
      cache_read_tokens: cache,
      output_tokens: total.output_tokens,
      ..Usage::default()
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn windows_and_guesses() {
    assert_eq!(known_size("gemini-4-hydrogen"), Some(1_048_576));
    assert_eq!(known_size("claude-opus-4-6-thinking"), Some(250_000));
    assert_eq!(known_size("gpt-oss-120b-medium"), Some(131_072));
    assert_eq!(known_size("gemini-4-neon"), Some(1_048_576));
    assert_eq!(known_size("gemini-pro-orchestrator"), Some(1_048_576));
    assert_eq!(known_size("claude-opus-5-5"), Some(1_000_000));
    assert_eq!(known_size("claude-sonnet-5-5"), Some(1_000_000));
    assert_eq!(compaction_guess(1_000_000), 512_000);
    assert_eq!(known_size("other"), None);
    assert_eq!(compaction_guess(1_048_576), 512_000);
    assert_eq!(compaction_guess(250_000), 160_000);
    assert_eq!(compaction_guess(131_072), 80_000);
    assert_eq!(compaction_guess(50_000), 50_000);
  }

  #[test]
  fn each_step_is_told_its_own_prompt_and_the_turn_its_output() {
    let mut m = Meter::default();
    m.new_turn();
    // One model call: agy's numbers as they are.
    m.used(10_000);
    let total = Usage {
      input_tokens: 9_000,
      cache_read_tokens: 900,
      output_tokens: 100,
      ..Usage::default()
    };
    assert_eq!(m.end(total), total);
    // Three calls, two tools: each step its context, the last the output.
    m.new_turn();
    m.used(12_000);
    let a = m.step();
    assert_eq!((a.input_tokens, a.cache_read_tokens), (2_000, 10_000));
    m.used(15_000);
    let b = m.step();
    assert_eq!((b.input_tokens, b.cache_read_tokens), (3_000, 12_000));
    m.used(15_400);
    let c = m.end(Usage {
      input_tokens: 6_000,
      cache_read_tokens: 36_000,
      output_tokens: 400,
      ..Usage::default()
    });
    // The prompt size Stitch measures is the latest request's.
    assert_eq!(c.input_tokens + c.cache_read_tokens, 15_400);
    assert_eq!(c.cache_read_tokens, 14_000);
    assert_eq!(c.output_tokens, 400);
  }

  #[test]
  fn a_compaction_is_seen_and_told_at_its_size_before() {
    let mut m = Meter::default();
    m.new_turn();
    assert_eq!(m.used(150_000), None);
    assert_eq!(m.used(30_000), Some(150_000));
    let u = m.end(Usage::default());
    assert_eq!(u.input_tokens + u.cache_read_tokens, 150_000);
    // Small contexts falling are not compactions.
    let mut m = Meter::default();
    m.used(10_000);
    assert_eq!(m.used(1_000), None);
  }
}
