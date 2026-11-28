//! agy's backend errors, which come as text: a turn that ends `end_turn`
//! with no usage after a message like `"\n⚠ RESOURCE_EXHAUSTED (code
//! 429): …\n"`. Made provider errors here, by design §2.8's rules: a
//! plan's limit that resets later is out of usage, a short wait a rate
//! limit.

use stitch_providers::{Error, Exhausted};

use crate::KIND;

/// A 429 that resets within this many seconds is a rate limit (agy has
/// retried it already); later, the plan's limit.
const LIMIT_AFTER_SECS: i64 = 600;

/// Whether streamed text may be the start of a backend error, to hold back
/// rather than show.
pub fn may_be_error(text: &str) -> bool {
  text.trim_start().starts_with('⚠')
}

/// The error a turn's text says, when it is one of agy's error lines.
pub fn backend_error(text: &str, now_secs: i64) -> Option<Error> {
  let line = text.trim();
  let rest = line.strip_prefix('⚠')?.trim();
  // `STATUS (code N): message`, else a message of its own.
  let (status, code, message) = match rest.split_once(" (code ") {
    Some((status, after)) if !status.contains(' ') => match after.split_once("):") {
      Some((n, message)) => (
        status.to_owned(),
        n.trim().parse::<u16>().unwrap_or(500),
        message.trim().to_owned(),
      ),
      None => (String::new(), 500, rest.to_owned()),
    },
    _ => (String::new(), 500, rest.to_owned()),
  };
  if code == 429 || status == "RESOURCE_EXHAUSTED" {
    let resets = resets_in(&message);
    let lower = message.to_lowercase();
    let limit = match resets {
      Some(s) => s >= LIMIT_AFTER_SECS,
      None => lower.contains("quota") && !lower.contains("per minute"),
    };
    if limit {
      return Some(Error::OutOfUsage {
        provider: KIND,
        status: 429,
        what: Exhausted::Limit,
        message,
        resets_at: resets.map(|s| now_secs + s),
        window: None,
      });
    }
    return Some(Error::Api {
      provider: KIND,
      status: 429,
      kind: "rate_limited".into(),
      message,
    });
  }
  let lower = message.to_lowercase();
  if lower.contains("not logged in") || lower.contains("sign in") || lower.contains("login") {
    return Some(Error::Api {
      provider: KIND,
      status: 401,
      kind: "not signed in".into(),
      message: format!("{message} (sign in by running `agy` once)"),
    });
  }
  Some(Error::Api {
    provider: KIND,
    status: code,
    kind: if status.is_empty() {
      "error".into()
    } else {
      status
    },
    message,
  })
}

/// Seconds from `Resets in 3h12m5s` (or `0s`, `45m`, `1.5s`).
fn resets_in(message: &str) -> Option<i64> {
  let lower = message.to_lowercase();
  let at = lower.find("resets in ")? + "resets in ".len();
  let s: String = lower[at..]
    .chars()
    .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
    .collect();
  let s = s.trim_end_matches('.');
  let mut total = 0f64;
  let mut num = String::new();
  let mut any = false;
  for c in s.chars() {
    match c {
      '0'..='9' | '.' => num.push(c),
      'h' | 'm' | 's' | 'd' => {
        let n: f64 = num.parse().ok()?;
        num.clear();
        any = true;
        total += n
          * match c {
            'd' => 86_400.0,
            'h' => 3_600.0,
            'm' => 60.0,
            _ => 1.0,
          };
      }
      _ => return None,
    }
  }
  (any && num.is_empty()).then_some(total.ceil() as i64)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn quota_lines_become_errors() {
    let e = backend_error(
      "\n⚠ RESOURCE_EXHAUSTED (code 429): You have exhausted your capacity on this model. Resets in 3h2m0s.\n",
      1000,
    )
    .unwrap();
    match e {
      Error::OutOfUsage {
        what, resets_at, ..
      } => {
        assert_eq!(what, Exhausted::Limit);
        assert_eq!(resets_at, Some(1000 + 3 * 3600 + 120));
      }
      other => panic!("{other:?}"),
    }
    // Back in a moment: a rate limit, not out of usage.
    let e = backend_error(
      "⚠ RESOURCE_EXHAUSTED (code 429): You have exhausted your capacity on this model. Resets in 0s.",
      0,
    )
    .unwrap();
    assert!(matches!(e, Error::Api { status: 429, .. }), "{e:?}");
    let e = backend_error("⚠ RESOURCE_EXHAUSTED (code 429): Daily quota exceeded", 0).unwrap();
    assert!(matches!(
      e,
      Error::OutOfUsage {
        resets_at: None,
        ..
      }
    ));
  }

  #[test]
  fn other_lines_are_api_errors_and_text_is_not() {
    let e = backend_error("\n⚠ INVALID_ARGUMENT (code 400): bad schema\n", 0).unwrap();
    match e {
      Error::Api {
        status,
        kind,
        message,
        ..
      } => {
        assert_eq!(
          (status, kind.as_str(), message.as_str()),
          (400, "INVALID_ARGUMENT", "bad schema")
        );
      }
      other => panic!("{other:?}"),
    }
    let e = backend_error(
      "⚠ failed to construct executor: plan model not specified",
      0,
    )
    .unwrap();
    assert!(matches!(e, Error::Api { status: 500, .. }));
    assert!(backend_error("All good ⚠ here", 0).is_none());
    assert!(may_be_error("\n⚠ X"));
    assert!(!may_be_error("hello"));
    assert_eq!(resets_in("Resets in 1h30m."), Some(5400));
    assert_eq!(resets_in("Resets in soon"), None);
  }
}
