//! The HTTP side shared by the adapters: one client (rustls with ring, the
//! provider iroh uses too), a streamed POST, and retries before the stream
//! starts. Once output has streamed, a failure is returned, not retried:
//! the caller has already shown part of it.

use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use serde_json::Value;

use crate::sse::{SseEvent, SseParser};
use crate::{Error, Exhausted, Result};

const MAX_RETRIES: u32 = 3;

tokio::task_local! {
  /// Headers every request made inside `with_attribution` carries: who a
  /// call is for, for a key-holding proxy in front of the provider to
  /// account by (`x-stitch-end-user`, `x-stitch-thread`; design §2.21).
  static ATTRIBUTION: std::cell::RefCell<Vec<(String, String)>>;
}

/// Runs `f` with `headers` added to every provider request it makes.
pub async fn with_attribution<F: std::future::Future>(
  headers: Vec<(String, String)>,
  f: F,
) -> F::Output {
  ATTRIBUTION.scope(std::cell::RefCell::new(headers), f).await
}

/// Adds (or replaces) a header of the attribution in effect; nothing
/// outside `with_attribution`.
pub fn attribute(name: &str, value: &str) {
  let _ = ATTRIBUTION.try_with(|a| {
    let mut a = a.borrow_mut();
    a.retain(|(k, _)| k != name);
    a.push((name.to_owned(), value.to_owned()));
  });
}

/// The attribution headers in effect here (none outside `with_attribution`).
pub fn attribution() -> Vec<(String, String)> {
  ATTRIBUTION
    .try_with(|a| a.borrow().clone())
    .unwrap_or_default()
}

/// `headers` with the attribution headers in effect added.
fn attributed(mut headers: HeaderMap) -> HeaderMap {
  for (k, v) in attribution() {
    if let (Ok(k), Ok(v)) = (
      reqwest::header::HeaderName::from_bytes(k.as_bytes()),
      reqwest::header::HeaderValue::from_str(&v),
    ) {
      headers.insert(k, v);
    }
  }
  headers
}

pub(crate) fn client() -> reqwest::Client {
  static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
  CLIENT
    .get_or_init(|| {
      // Errors when another provider is installed already, which is fine.
      let _ = rustls::crypto::ring::default_provider().install_default();
      reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        // A stream may pause while the model thinks; not this long.
        .read_timeout(Duration::from_secs(600))
        .build()
        .expect("an HTTP client with default settings builds")
    })
    .clone()
}

pub(crate) fn api_error(provider: &'static str, status: u16, body: &str) -> Error {
  let parsed: Option<Value> = serde_json::from_str(body).ok();
  let err = parsed.as_ref().and_then(|v| v.get("error"));
  let field = |k: &str| {
    err
      .and_then(|e| e.get(k))
      .and_then(Value::as_str)
      .map(str::to_owned)
  };
  Error::Api {
    provider,
    status,
    // Google's errors name their kind `status` (`INVALID_ARGUMENT`).
    kind: field("type")
      .or_else(|| field("code"))
      .or_else(|| field("status"))
      .unwrap_or_default(),
    message: field("message").unwrap_or_else(|| body.chars().take(500).collect()),
  }
}

/// A refusal the provider answered with an error status, as `Error`: an
/// account out of usage (`Error::OutOfUsage`, never retried) when the
/// status, headers or body say so, else `api_error`.
pub(crate) fn failure(
  provider: &'static str,
  status: u16,
  headers: &HeaderMap,
  body: &str,
) -> Error {
  let err = api_error(provider, status, body);
  out_of_usage(provider, status, headers, body, now_secs()).unwrap_or(err)
}

pub(crate) fn now_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs() as i64)
}

/// A `retry-after` longer than this is no rate limit to wait out in a
/// request: the account is out of usage until then.
const LONG_WAIT: i64 = 10 * 60;

/// Whether a refusal says the account is out of usage, from what the
/// providers answer (2026-10):
/// - Anthropic, a Claude subscription's limit: 429 `rate_limit_error`
///   with `anthropic-ratelimit-unified-status: rejected`, the reset in
///   `anthropic-ratelimit-unified-reset` (seconds since the epoch), the
///   window in `…-representative-claim` (`five_hour`, `seven_day`, …);
///   long-context requests needing usage credits say "usage credits".
///   A spending cap: 429 with `error.details.error_code`
///   `enforced_spend_limit_reached`. No credit: 400 "Your credit balance
///   is too low…"; a negative balance, 402.
/// - OpenAI's API: 429 `insufficient_quota`. ChatGPT's Codex backend:
///   429 `usage_limit_reached` with `resets_at` / `resets_in_seconds`, or
///   `usage_not_included`.
/// - xAI: 403 or 429, "…has either used all available credits or reached
///   its monthly spending limit".
/// - Gemini: 429 `RESOURCE_EXHAUSTED` whose `QuotaFailure` names a
///   per-day quota (`…PerDay…`); a per-minute one is a rate limit, retried.
///   Prepaid credit used up: 402.
///
/// Per-minute rate limits (a 429 with a short `retry-after`) are not out
/// of usage: they are retried.
pub(crate) fn out_of_usage(
  provider: &'static str,
  status: u16,
  headers: &HeaderMap,
  body: &str,
  now: i64,
) -> Option<Error> {
  if !matches!(status, 400 | 402 | 403 | 429) {
    return None;
  }
  let header = |name: &str| {
    headers
      .get(name)
      .and_then(|v| v.to_str().ok())
      .map(str::trim)
      .filter(|v| !v.is_empty())
  };
  let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
  let err = parsed.get("error").unwrap_or(&Value::Null);
  let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
  let kind = text(err, "type");
  let code = match err.get("code") {
    Some(Value::String(c)) => c.clone(),
    _ => text(&parsed, "code"),
  };
  let detail = err
    .get("details")
    .and_then(|d| d.get("error_code"))
    .and_then(Value::as_str)
    .unwrap_or("");
  let message = match err {
    Value::String(m) => m.clone(),
    _ => match text(err, "message") {
      m if m.is_empty() => body.chars().take(500).collect(),
      m => m,
    },
  };
  let said = message.to_lowercase();
  let retry_after = header("retry-after").and_then(|v| v.parse::<i64>().ok());
  let unified = header("anthropic-ratelimit-unified-status") == Some("rejected");
  let mut resets_at = header("anthropic-ratelimit-unified-reset")
    .and_then(|v| v.parse::<i64>().ok())
    .or_else(|| err.get("resets_at").and_then(Value::as_i64))
    .or_else(|| {
      err
        .get("resets_in_seconds")
        .and_then(Value::as_i64)
        .map(|s| now + s)
    });
  let mut window = header("anthropic-ratelimit-unified-representative-claim").map(str::to_owned);
  // Gemini's quota that ran out, when it is a day's.
  let daily_quota = err
    .get("details")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|d| d.get("violations").and_then(Value::as_array))
    .flatten()
    .filter_map(|v| v.get("quotaId").and_then(Value::as_str))
    .find(|q| q.contains("PerDay") || q.contains("Daily"));
  let credit_said = [
    "credit balance is too low",
    "used all available credits",
    "spending limit",
    "run out of credits",
    "out of credits",
    "insufficient credits",
    "insufficient balance",
  ]
  .iter()
  .any(|m| said.contains(m));
  // In this order: a limit the provider names, credit it names, a day's
  // quota, credit said in words, a limit said in words or by a long wait.
  let limit =
    (status == 429 && unified) || kind == "usage_limit_reached" || code == "usage_limit_reached";
  // Moonshot's balance spent (a 429 it means as credit, not a limit).
  let credit = kind == "usage_not_included"
    || kind == "exceeded_current_quota_error"
    || kind == "insufficient_quota"
    || code == "insufficient_quota"
    || detail.contains("spend_limit")
    || status == 402;
  let limit_said = status == 429
    && (said.contains("usage limit")
      || said.contains("usage credits")
      || retry_after.is_some_and(|s| s > LONG_WAIT));
  let what = if limit {
    Exhausted::Limit
  } else if credit {
    Exhausted::Credit
  } else if status == 429
    && let Some(q) = daily_quota
  {
    window = Some(q.to_owned());
    Exhausted::Limit
  } else if credit_said {
    Exhausted::Credit
  } else if limit_said {
    Exhausted::Limit
  } else {
    return None;
  };
  if resets_at.is_none() && what == Exhausted::Limit {
    resets_at = retry_after.filter(|s| *s > 0).map(|s| now + s);
  }
  Some(Error::OutOfUsage {
    provider,
    status,
    what,
    message,
    resets_at,
    window,
  })
}

fn backoff(attempt: u32, headers: &HeaderMap) -> Duration {
  let hinted = headers
    .get(RETRY_AFTER)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.parse::<u64>().ok())
    .map(Duration::from_secs);
  hinted
    .unwrap_or(Duration::from_millis(1000 << attempt))
    .min(Duration::from_secs(60))
}

/// A GET returning JSON (a model list), with a short timeout.
pub(crate) async fn get_json(
  provider: &'static str,
  url: &str,
  headers: HeaderMap,
) -> Result<Value> {
  json_reply(provider, client().get(url).headers(attributed(headers))).await
}

/// A POST of `body` returning JSON (a model's details), with a short
/// timeout.
pub(crate) async fn post_json(
  provider: &'static str,
  url: &str,
  headers: HeaderMap,
  body: &Value,
) -> Result<Value> {
  json_reply(
    provider,
    client().post(url).headers(attributed(headers)).json(body),
  )
  .await
}

/// A POST of `body` returning JSON, within `timeout` (connecting
/// included), sent once.
pub(crate) async fn post_json_within(
  provider: &'static str,
  url: &str,
  headers: HeaderMap,
  body: &Value,
  timeout: Duration,
) -> Result<Value> {
  json_reply_within(
    provider,
    client().post(url).headers(attributed(headers)).json(body),
    timeout,
  )
  .await
}

async fn json_reply(provider: &'static str, request: reqwest::RequestBuilder) -> Result<Value> {
  json_reply_within(provider, request, Duration::from_secs(15)).await
}

async fn json_reply_within(
  provider: &'static str,
  request: reqwest::RequestBuilder,
  timeout: Duration,
) -> Result<Value> {
  let r = request.timeout(timeout).send().await?;
  let status = r.status().as_u16();
  let headers = r.headers().clone();
  let text = r.text().await?;
  if !(200..300).contains(&status) {
    return Err(failure(provider, status, &headers, &text));
  }
  serde_json::from_str(&text).map_err(|_| Error::protocol(provider, "a reply that is not JSON"))
}

/// POSTs `body` and feeds each server-sent event to `on_event`.
pub(crate) async fn post_sse(
  provider: &'static str,
  url: &str,
  headers: HeaderMap,
  body: &Value,
  mut on_event: impl FnMut(SseEvent) -> Result<()>,
) -> Result<()> {
  let client = client();
  let headers = attributed(headers);
  let mut attempt = 0;
  let response = loop {
    let sent = client
      .post(url)
      .headers(headers.clone())
      .json(body)
      .send()
      .await;
    let (err, wait) = match sent {
      Ok(r) if r.status().is_success() => break r,
      Ok(r) => {
        let status = r.status().as_u16();
        let h = r.headers().clone();
        let text = r.text().await.unwrap_or_default();
        // An account out of usage is not retried (`retryable`): asking
        // again only fails the same way, and looks like a frozen chat.
        (failure(provider, status, &h, &text), backoff(attempt, &h))
      }
      Err(e) => (Error::Http(e), backoff(attempt, &HeaderMap::new())),
    };
    if !err.retryable() || attempt >= MAX_RETRIES {
      return Err(err);
    }
    tokio::time::sleep(wait).await;
    attempt += 1;
  };
  let mut parser = SseParser::default();
  let mut bytes = response.bytes_stream();
  while let Some(chunk) = bytes.next().await {
    for event in parser.feed(&chunk?) {
      on_event(event)?;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use reqwest::header::HeaderValue;
  use serde_json::json;

  const NOW: i64 = 1_800_000_000;

  fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
      h.insert(*k, HeaderValue::from_str(v).unwrap());
    }
    h
  }

  fn usage(status: u16, h: &[(&'static str, &str)], body: Value) -> Option<Error> {
    out_of_usage("p", status, &headers(h), &body.to_string(), NOW)
  }

  fn what(e: &Option<Error>) -> Option<(Exhausted, Option<i64>, Option<String>)> {
    match e {
      Some(Error::OutOfUsage {
        what,
        resets_at,
        window,
        ..
      }) => Some((*what, *resets_at, window.clone())),
      _ => None,
    }
  }

  #[test]
  fn anthropic_subscription_limits_are_out_of_usage_with_their_reset() {
    let body = json!({"type": "error", "error": {"type": "rate_limit_error", "message": "This request would exceed your account's rate limit. Please try again later."}});
    let e = usage(
      429,
      &[
        ("anthropic-ratelimit-unified-status", "rejected"),
        ("anthropic-ratelimit-unified-reset", "1800003600"),
        (
          "anthropic-ratelimit-unified-representative-claim",
          "five_hour",
        ),
        ("retry-after", "3600"),
      ],
      body.clone(),
    );
    assert_eq!(
      what(&e),
      Some((
        Exhausted::Limit,
        Some(1_800_003_600),
        Some("five_hour".into())
      ))
    );
    assert!(!e.unwrap().retryable());
    // Only a long `retry-after`: out of usage until then.
    let e = usage(429, &[("retry-after", "7200")], body.clone());
    assert_eq!(what(&e), Some((Exhausted::Limit, Some(NOW + 7200), None)));
    // A per-minute rate limit is waited out, not reported.
    assert!(usage(429, &[("retry-after", "20")], body.clone()).is_none());
    assert!(usage(429, &[], body).is_none());
    // Long context on a subscription needs usage credits.
    let e = usage(
      429,
      &[],
      json!({"error": {"type": "rate_limit_error", "message": "Usage credits are required for long context requests."}}),
    );
    assert_eq!(what(&e).map(|w| w.0), Some(Exhausted::Limit));
  }

  #[test]
  fn credit_and_quota_used_up_are_out_of_usage() {
    let credit = |status, body: Value| what(&usage(status, &[], body)).map(|w| w.0);
    assert_eq!(
      credit(
        400,
        json!({"type": "error", "error": {"type": "invalid_request_error", "message": "Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}})
      ),
      Some(Exhausted::Credit)
    );
    assert_eq!(
      credit(
        429,
        json!({"error": {"type": "rate_limit_error", "message": "spend cap", "details": {"error_code": "enforced_spend_limit_reached"}}})
      ),
      Some(Exhausted::Credit)
    );
    assert_eq!(
      credit(
        429,
        json!({"error": {"message": "You exceeded your current quota, please check your plan and billing details.", "type": "insufficient_quota", "param": null, "code": "insufficient_quota"}})
      ),
      Some(Exhausted::Credit)
    );
    assert_eq!(
      credit(
        403,
        json!({"code": "permission-denied", "error": "Your team abc has either used all available credits or reached its monthly spending limit."})
      ),
      Some(Exhausted::Credit)
    );
    assert_eq!(
      credit(
        402,
        json!({"error": {"message": "prepay credits depleted"}})
      ),
      Some(Exhausted::Credit)
    );
    // Moonshot's balance spent, and DeepSeek's (a 402).
    assert_eq!(
      credit(
        429,
        json!({"error": {"type": "exceeded_current_quota_error", "message": "Account balance is insufficient or the account has been disabled"}})
      ),
      Some(Exhausted::Credit)
    );
    assert_eq!(
      credit(
        402,
        json!({"error": {"message": "Insufficient Balance", "type": "unknown_error"}})
      ),
      Some(Exhausted::Credit)
    );
    // Moonshot's rate limits are retried, not said.
    assert_eq!(
      credit(
        429,
        json!({"error": {"type": "rate_limit_reached_error", "message": "Organization-level RPM limit reached"}})
      ),
      None
    );
    // An ordinary bad request is not.
    assert_eq!(
      credit(
        400,
        json!({"error": {"type": "invalid_request_error", "message": "max_tokens: too large"}})
      ),
      None
    );
    assert_eq!(
      credit(500, json!({"error": {"type": "insufficient_quota"}})),
      None
    );
  }

  #[test]
  fn codex_and_gemini_limits() {
    let e = usage(
      429,
      &[],
      json!({"error": {"type": "usage_limit_reached", "message": "The usage limit has been reached", "plan_type": "plus", "resets_at": 1_800_004_000, "resets_in_seconds": 4000}}),
    );
    assert_eq!(
      what(&e),
      Some((Exhausted::Limit, Some(1_800_004_000), None))
    );
    let e = usage(
      429,
      &[],
      json!({"error": {"type": "usage_limit_reached", "message": "The usage limit has been reached", "resets_in_seconds": 60}}),
    );
    assert_eq!(what(&e), Some((Exhausted::Limit, Some(NOW + 60), None)));
    let quota = |id: &str| {
      json!({"error": {"code": 429, "message": "You exceeded your current quota, please check your plan and billing details.", "status": "RESOURCE_EXHAUSTED", "details": [
        {"@type": "type.googleapis.com/google.rpc.QuotaFailure", "violations": [{"quotaId": id}]},
        {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "34s"}
      ]}})
    };
    let e = usage(
      429,
      &[],
      quota("GenerateRequestsPerDayPerProjectPerModel-FreeTier"),
    );
    assert_eq!(
      what(&e),
      Some((
        Exhausted::Limit,
        None,
        Some("GenerateRequestsPerDayPerProjectPerModel-FreeTier".into())
      ))
    );
    // A minute's quota is a rate limit: retried.
    assert!(
      usage(
        429,
        &[],
        quota("GenerateRequestsPerMinutePerProjectPerModel")
      )
      .is_none()
    );
  }
}
