/// What an account ran out of (`Error::OutOfUsage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
  /// A plan's usage limit: a subscription's 5-hour or weekly window, a
  /// daily quota. It comes back by itself, at `resets_at` when known.
  Limit,
  /// Credit or quota that does not come back by itself: prepaid credit
  /// used up, a spending cap, a plan that does not include the model.
  Credit,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The provider answered with an error status (or an `error` event).
  #[error("{provider}: {status} {kind}: {message}")]
  Api {
    provider: &'static str,
    status: u16,
    kind: String,
    message: String,
  },
  /// The account is out of usage (design §2.8, 2026-10-22): a plan's
  /// limit reached, or credit or quota used up. Never retried: asking again
  /// fails the same way until it resets or the user adds credit.
  #[error("{provider}: {status} out of usage: {message}")]
  OutOfUsage {
    provider: &'static str,
    status: u16,
    what: Exhausted,
    message: String,
    /// When it comes back, in seconds since the epoch, when the provider
    /// said.
    resets_at: Option<i64>,
    /// The limit that was hit, as the provider names it (`five_hour`,
    /// `seven_day`, a quota's ID), when it did.
    window: Option<String>,
  },
  #[error("{0}: no API key configured")]
  NoKey(&'static str),
  #[error("http: {0}")]
  Http(#[from] reqwest::Error),
  /// The stream did not follow the provider's protocol.
  #[error("{provider}: {message}")]
  Protocol {
    provider: &'static str,
    message: String,
  },
  /// The stream stopped before the reply was whole: the connection closed
  /// without the provider's last event.
  #[error("{provider}: {message}")]
  Cut {
    provider: &'static str,
    message: String,
  },
}

impl Error {
  /// Worth retrying: rate limits, overload and server errors.
  pub fn retryable(&self) -> bool {
    match self {
      Error::Api { status, .. } => {
        *status == 408 || *status == 409 || *status == 429 || *status >= 500
      }
      Error::Http(e) => e.is_connect() || e.is_timeout(),
      _ => false,
    }
  }

  /// The provider refused the credentials (401): an access token that
  /// expired or was revoked.
  pub fn is_unauthorized(&self) -> bool {
    matches!(self, Error::Api { status: 401, .. })
  }

  /// The network failed the call: no connection, or one that dropped or
  /// stalled mid-stream, or a stream cut short. Sending the request again
  /// may work; nothing the provider said refused it.
  pub fn is_network(&self) -> bool {
    match self {
      Error::Cut { .. } => true,
      Error::Http(e) => {
        !e.is_status()
          && !e.is_builder()
          && (e.is_connect() || e.is_timeout() || e.is_body() || e.is_request() || e.is_decode())
      }
      _ => false,
    }
  }

  pub(crate) fn protocol(provider: &'static str, message: impl std::fmt::Display) -> Self {
    Error::Protocol {
      provider,
      message: message.to_string(),
    }
  }

  pub(crate) fn cut(provider: &'static str, message: impl std::fmt::Display) -> Self {
    Error::Cut {
      provider,
      message: message.to_string(),
    }
  }
}
