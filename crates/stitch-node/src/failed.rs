//! A turn's model call that failed: what is written in the thread about it
//! (a `failed` event) and what the user is told, above all when the
//! account is out of usage (design §2.8, 2026-10-22): which provider and
//! plan, when it resets, and what to do. Stitch never switches to another
//! provider by itself.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Value, json};
use stitch_providers::{Error as ProviderError, Exhausted};
use stitch_records::Event;

/// The event written with a failed step: `t = "failed"`, the provider's
/// error as said, and, for an account out of usage, what ran out.
pub(crate) fn event(e: &ProviderError, label: &str) -> Event {
  let mut v = json!({
    "t": "failed",
    "at": Timestamp::now().to_string(),
    "provider": label,
    "error": e.to_string(),
  });
  if let ProviderError::OutOfUsage {
    what,
    resets_at,
    window,
    message,
    ..
  } = e
  {
    let mut usage = json!({
      "what": match what {
        Exhausted::Limit => "limit",
        Exhausted::Credit => "credit",
      },
      "message": message,
    });
    if let Some(at) = resets_at.and_then(|s| Timestamp::from_second(s).ok()) {
      usage["resets_at"] = at.to_string().into();
    }
    if let Some(w) = window {
      usage["window"] = w.as_str().into();
    }
    v["usage"] = usage;
  }
  Event::Other(v)
}

/// The `failed` event of a turn that failed before its model call (its
/// provider not resolved: no key, an OAuth refresh refused): a provider's
/// error as `event` writes it, any other as said, with no provider named.
pub(crate) fn before_call(e: &crate::Error) -> Event {
  match e {
    crate::Error::Provider(p) => event(p, ""),
    e => Event::Other(json!({
      "t": "failed",
      "at": Timestamp::now().to_string(),
      "error": e.to_string(),
      // Not the model's: what the turn needed first (a project's checkout,
      // a sign-in) was not there.
      "before": true,
    })),
  }
}

/// What the user is told of a failed model call, from its error.
pub fn said(e: &ProviderError, label: &str) -> String {
  shown(&event(e, label))
}

/// A `failed` event as the transcript and clients show it, its reset
/// time in this node's time zone.
pub fn shown(e: &Event) -> String {
  let Event::Other(v) = e else {
    return String::new();
  };
  let tz = TimeZone::try_system().unwrap_or(TimeZone::UTC);
  shown_at(v, Timestamp::now(), &tz)
}

pub(crate) fn shown_at(v: &Value, now: Timestamp, tz: &TimeZone) -> String {
  let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
  let label = match text("provider") {
    "" => "The provider",
    l => l,
  };
  let Some(usage) = v.get("usage") else {
    if v.get("before").and_then(Value::as_bool) == Some(true) {
      return format!("The turn did not start: {}", text("error"));
    }
    return format!("The model call failed: {}", text("error"));
  };
  let u = |k: &str| usage.get(k).and_then(Value::as_str).unwrap_or("");
  let switch = "switch to another model with /model and go on (send a message, or Resume). \
                Stitch does not switch by itself.";
  if u("what") == "credit" {
    return format!(
      "{label}: out of credit or quota ({}). Add credit or raise the limit with the provider, \
       or {switch}",
      u("message").trim_end_matches('.')
    );
  }
  let window = match u("window") {
    "" => String::new(),
    "five_hour" => " (5-hour window)".into(),
    "seven_day" => " (weekly window)".into(),
    w if w.contains("PerDay") || w.contains("Daily") => " (daily quota)".into(),
    w => format!(" ({})", w.replace('_', " ")),
  };
  let reset = u("resets_at").parse::<Timestamp>().ok();
  match reset {
    Some(at) if at > now => format!(
      "{label}: usage limit reached{window}; it resets {} ({}). Wait until then, or {switch}",
      when(at, now, tz),
      wait(at.duration_since(now).as_secs())
    ),
    Some(at) => format!(
      "{label}: usage limit reached{window}; it was to reset {}: try again, or {switch}",
      when(at, now, tz)
    ),
    None => format!("{label}: usage limit reached{window}. Wait for it to reset, or {switch}"),
  }
}

/// `at 18:00`, `tomorrow at 09:30`, `on Fri 24 Oct at 18:00`.
fn when(at: Timestamp, now: Timestamp, tz: &TimeZone) -> String {
  let (at, now) = (at.to_zoned(tz.clone()), now.to_zoned(tz.clone()));
  let utc = if *tz == TimeZone::UTC { " UTC" } else { "" };
  let time = at.strftime("%H:%M").to_string();
  let (day, today) = (at.date(), now.date());
  if day == today {
    format!("at {time}{utc}")
  } else if today.tomorrow().ok() == Some(day) {
    format!("tomorrow at {time}{utc}")
  } else {
    format!("on {} at {time}{utc}", at.strftime("%a %-d %b"))
  }
}

/// `in 25 min`, `in 2 h 10 min`, `in 3 days`.
fn wait(secs: i64) -> String {
  let mins = (secs + 59) / 60;
  match mins {
    m if m < 60 => format!("in {m} min"),
    m if m < 48 * 60 => match m % 60 {
      0 => format!("in {} h", m / 60),
      r => format!("in {} h {r} min", m / 60),
    },
    m => format!("in {} days", m / (24 * 60)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
  }

  #[test]
  fn a_usage_limit_says_when_it_resets_and_what_to_do() {
    let tz = TimeZone::get("Europe/Rome").unwrap();
    let now = at("2026-10-22T13:50:00Z");
    let e = ProviderError::OutOfUsage {
      provider: "anthropic",
      status: 429,
      what: Exhausted::Limit,
      message: "rate limited".into(),
      resets_at: Some(at("2026-10-22T16:00:00Z").as_second()),
      window: Some("five_hour".into()),
    };
    let Event::Other(v) = event(&e, "Anthropic (Claude subscription)") else {
      unreachable!()
    };
    let said = shown_at(&v, now, &tz);
    assert_eq!(
      said,
      "Anthropic (Claude subscription): usage limit reached (5-hour window); it resets at 18:00 \
       (in 2 h 10 min). Wait until then, or switch to another model with /model and go on \
       (send a message, or Resume). Stitch does not switch by itself."
    );
    let next = shown_at(&v, at("2026-10-21T20:00:00Z"), &tz);
    assert!(
      next.contains("resets tomorrow at 18:00 (in 20 h)"),
      "{next}"
    );
    let past = shown_at(&v, at("2026-10-22T16:30:00Z"), &tz);
    assert!(past.contains("was to reset at 18:00: try again"), "{past}");
  }

  #[test]
  fn credit_and_other_failures() {
    let tz = TimeZone::UTC;
    let now = at("2026-10-22T13:50:00Z");
    let e = ProviderError::OutOfUsage {
      provider: "anthropic",
      status: 400,
      what: Exhausted::Credit,
      message: "Your credit balance is too low to access the Anthropic API.".into(),
      resets_at: None,
      window: None,
    };
    let Event::Other(v) = event(&e, "Anthropic (API key)") else {
      unreachable!()
    };
    assert!(shown_at(&v, now, &tz).starts_with(
      "Anthropic (API key): out of credit or quota (Your credit balance is too low to access \
         the Anthropic API). Add credit"
    ));
    let e = ProviderError::Api {
      provider: "openai",
      status: 503,
      kind: "server_error".into(),
      message: "down".into(),
    };
    let Event::Other(v) = event(&e, "OpenAI (API key)") else {
      unreachable!()
    };
    assert_eq!(
      shown_at(&v, now, &tz),
      "The model call failed: openai: 503 server_error: down"
    );
    assert_eq!(wait(25 * 60), "in 25 min");
    assert_eq!(wait(3 * 86400), "in 3 days");
  }
}
