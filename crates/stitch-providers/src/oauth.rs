//! Subscription OAuth protocols, matching edge-talk's sign-in managers.
//! Credentials are deliberately not Debug: tokens must never enter logs.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{future::Future, pin::Pin, sync::Arc};

use crate::{Error, Result, http};

/// The Claude Code release a subscription's requests say they come from:
/// the API refuses new models to older ones (Fable 5.2 wants 2.1.290 or
/// newer), so it follows Claude Code's releases (npm
/// `@anthropic-ai/claude-code`; 2.1.321 on 2026-11-09).
pub const CLAUDE_USER_AGENT: &str = "claude-cli/2.1.321";
pub const GROK_SURFACE: &str = "grok-build";
pub const GROK_VERSION: &str = "0.2.112";
/// How long before expiry a token is refreshed. Wide, so that no request
/// starts with a token about to lapse; a 401 still refreshes (`TokenSource`).
pub const REFRESH_MARGIN_SECS: u64 = 15 * 60;

/// What a token source answers with.
pub type TokenFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// Where a subscription provider gets its access token. It is asked before
/// every request, so a turn outlives the token it started with: the source
/// refreshes it when it nears expiry, and again when the provider refused
/// it (401).
pub trait TokenSource: Send + Sync {
  /// The current access token, refreshed first when it nears expiry.
  fn token(&self) -> TokenFuture<'_>;
  /// A token to use instead of `stale`, which the provider refused:
  /// refreshed, unless another request already replaced it.
  fn refreshed<'a>(&'a self, stale: &'a str) -> TokenFuture<'a>;
}

/// A provider's token source (`Provider::stream` asks it per request).
#[derive(Clone)]
pub struct Tokens(pub Arc<dyn TokenSource>);

impl std::fmt::Debug for Tokens {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("Tokens")
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
  Anthropic,
  OpenAi,
  Grok,
}

impl Service {
  pub fn parse(name: &str) -> Result<Self> {
    match name {
      "anthropic" => Ok(Self::Anthropic),
      "openai" | "codex" => Ok(Self::OpenAi),
      "grok" | "xai" => Ok(Self::Grok),
      _ => Err(Error::protocol(
        "oauth",
        "choose anthropic, openai, or grok",
      )),
    }
  }
  pub fn name(self) -> &'static str {
    match self {
      Self::Anthropic => "anthropic",
      Self::OpenAi => "openai",
      Self::Grok => "grok",
    }
  }
  fn client_id(self) -> &'static str {
    match self {
      Self::Anthropic => "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
      Self::OpenAi => "app_EMoamEEZ73f0CkXaXp7hrann",
      Self::Grok => "b1a00492-073a-47ea-816f-4c329264a828",
    }
  }
  pub fn redirect_uri(self) -> &'static str {
    match self {
      Self::Anthropic => "http://localhost:53692/callback",
      Self::OpenAi => "http://localhost:1455/auth/callback",
      Self::Grok => "http://127.0.0.1:53694/callback",
    }
  }
  pub fn port(self) -> u16 {
    match self {
      Self::Anthropic => 53692,
      Self::OpenAi => 1455,
      Self::Grok => 53694,
    }
  }
  pub fn callback_path(self) -> &'static str {
    match self {
      Self::OpenAi => "/auth/callback",
      _ => "/callback",
    }
  }
  fn scope(self) -> &'static str {
    match self {
      Self::Anthropic => {
        "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
      }
      Self::OpenAi => "openid profile email offline_access",
      Self::Grok => "openid profile email offline_access grok-cli:access api:access",
    }
  }
  fn authorize(self) -> &'static str {
    match self {
      Self::Anthropic => "https://claude.ai/oauth/authorize",
      Self::OpenAi => "https://auth.openai.com/oauth/authorize",
      Self::Grok => "https://auth.x.ai/oauth2/authorize",
    }
  }
  fn token_url(self) -> &'static str {
    match self {
      Self::Anthropic => "https://platform.claude.com/v1/oauth/token",
      Self::OpenAi => "https://auth.openai.com/oauth/token",
      Self::Grok => "https://auth.x.ai/oauth2/token",
    }
  }
  async fn tokens(self, fields: Value, previous: Option<&Credentials>) -> Result<Credentials> {
    self.tokens_at(self.token_url(), fields, previous).await
  }

  async fn tokens_at(
    self,
    url: &str,
    fields: Value,
    previous: Option<&Credentials>,
  ) -> Result<Credentials> {
    let mut req = http::client()
      .post(url)
      .timeout(std::time::Duration::from_secs(60))
      .header("accept", "application/json");
    if self == Self::Anthropic {
      req = req.header("user-agent", CLAUDE_USER_AGENT).json(&fields);
    } else {
      let mut form = Url::parse("https://localhost/").expect("static URL");
      for (k, v) in fields.as_object().expect("token fields").iter() {
        form
          .query_pairs_mut()
          .append_pair(k, v.as_str().expect("string field"));
      }
      req = req
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form.query().unwrap_or_default().to_owned());
      if self == Self::OpenAi {
        req = req.header("user-agent", "codex_cli_rs/0.50.0");
      }
    }
    let response = req.send().await?;
    if !response.status().is_success() {
      // Never echo an auth server's body: it may contain credential material.
      return Err(Error::protocol(
        "oauth",
        format!(
          "{} token exchange: HTTP {}; run stitch auth login {} if refresh fails",
          self.name(),
          response.status(),
          self.name()
        ),
      ));
    }
    let value: Value = response.json().await?;
    self.credentials(&value, previous, now())
  }
  fn credentials(self, v: &Value, previous: Option<&Credentials>, now: u64) -> Result<Credentials> {
    let required = |key: &str| -> Result<String> {
      v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::protocol("oauth", format!("missing {key}")))
    };
    let access_token = required("access_token")?;
    let refresh_token =
      required("refresh_token").or_else(|e| previous.map(|p| p.refresh_token.clone()).ok_or(e))?;
    let expires_in = v["expires_in"]
      .as_u64()
      .or_else(|| (self == Self::Grok).then_some(3600))
      .filter(|n| *n > 0)
      .ok_or_else(|| Error::protocol("oauth", "missing expires_in"))?;
    let account_id = if self == Self::OpenAi {
      Some(
        account_id(&access_token)
          .ok_or_else(|| Error::protocol("oauth", "access token missing chatgpt_account_id"))?,
      )
    } else {
      None
    };
    Ok(Credentials {
      access_token,
      refresh_token,
      expires_at: now.saturating_add(expires_in),
      account_id,
    })
  }
  pub async fn refresh(self, current: &Credentials) -> Result<Credentials> {
    let mut fields = json!({"grant_type":"refresh_token", "refresh_token":current.refresh_token, "client_id":self.client_id()});
    if self == Self::OpenAi {
      fields["scope"] = json!(self.scope());
    }
    self.tokens(fields, Some(current)).await
  }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
  pub access_token: String,
  pub refresh_token: String,
  pub expires_at: u64,
  pub account_id: Option<String>,
}
impl Credentials {
  pub fn needs_refresh(&self) -> bool {
    self.expires_at <= now().saturating_add(REFRESH_MARGIN_SECS)
  }
}
fn now() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}
fn account_id(token: &str) -> Option<String> {
  let payload = B64
    .decode(token.split('.').nth(1)?.trim_end_matches('='))
    .ok()?;
  let v: Value = serde_json::from_slice(&payload).ok()?;
  v.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")?
    .as_str()
    .filter(|s| !s.is_empty())
    .map(str::to_owned)
}

pub struct Login {
  service: Service,
  verifier: String,
  state: String,
}
impl Login {
  pub fn new(service: Service) -> Self {
    let verifier = B64.encode(rand::random::<[u8; 32]>());
    let state = if service == Service::Anthropic {
      verifier.clone()
    } else {
      B64.encode(rand::random::<[u8; 32]>())
    };
    Self {
      service,
      verifier,
      state,
    }
  }
  pub fn authorize_url(&self) -> String {
    let mut url = Url::parse(self.service.authorize()).expect("static OAuth URL");
    url.query_pairs_mut().extend_pairs([
      ("client_id", self.service.client_id()),
      ("response_type", "code"),
      ("redirect_uri", self.service.redirect_uri()),
      ("scope", self.service.scope()),
      (
        "code_challenge",
        &B64.encode(Sha256::digest(self.verifier.as_bytes())),
      ),
      ("code_challenge_method", "S256"),
      ("state", &self.state),
    ]);
    if self.service == Service::Anthropic {
      url.query_pairs_mut().append_pair("code", "true");
    }
    if self.service == Service::OpenAi {
      url.query_pairs_mut().extend_pairs([
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli"),
      ]);
    }
    url.into()
  }
  /// Validates the path, state and code before any network request.
  pub fn callback_code(&self, target: &str) -> Result<String> {
    if !target.starts_with('/') || target.starts_with("//") {
      return Err(Error::protocol("oauth", "invalid callback target"));
    }
    let url = Url::parse(&format!("http://localhost{target}"))
      .map_err(|_| Error::protocol("oauth", "invalid callback"))?;
    if url.path() != self.service.callback_path() {
      return Err(Error::protocol("oauth", "wrong callback path"));
    }
    let fields: Vec<_> = url.query_pairs().collect();
    let one = |key: &str| {
      let mut values = fields
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_ref());
      let value = values.next();
      if values.next().is_some() { None } else { value }
    };
    if one("state") != Some(self.state.as_str()) {
      return Err(Error::protocol("oauth", "callback state mismatch"));
    }
    if fields.iter().any(|(k, _)| k == "error") {
      return Err(Error::protocol("oauth", "authorization denied"));
    }
    one("code")
      .filter(|s| !s.is_empty())
      .map(str::to_owned)
      .ok_or_else(|| Error::protocol("oauth", "missing callback code"))
  }
  pub async fn exchange(&self, code: &str) -> Result<Credentials> {
    let mut fields = json!({"grant_type":"authorization_code", "code":code,
      "redirect_uri":self.service.redirect_uri(), "client_id":self.service.client_id(), "code_verifier":self.verifier});
    if self.service == Service::Anthropic {
      fields["state"] = json!(self.state);
    }
    self.service.tokens(fields, None).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn pkce_and_callback_validation() {
    for service in [Service::Anthropic, Service::OpenAi, Service::Grok] {
      let login = Login::new(service);
      let url = Url::parse(&login.authorize_url()).unwrap();
      let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
      assert_eq!(
        fields["code_challenge"],
        B64.encode(Sha256::digest(login.verifier.as_bytes()))
      );
      assert_eq!(login.state == login.verifier, service == Service::Anthropic);
      let target = format!(
        "{}?code=abc%2B123&state={}",
        service.callback_path(),
        login.state
      );
      assert_eq!(login.callback_code(&target).unwrap(), "abc+123");
      assert!(
        login
          .callback_code(&format!("{target}&state=evil"))
          .is_err()
      );
      assert!(
        login
          .callback_code(&target.replace(&login.state, "wrong"))
          .is_err()
      );
      assert!(
        login
          .callback_code(&target.replace(service.callback_path(), "/wrong"))
          .is_err()
      );
      assert!(
        login
          .callback_code(&format!("{target}&error=denied"))
          .is_err()
      );
    }
  }
  #[test]
  fn token_rotation_and_account_claim() {
    let jwt = format!(
      "header.{}.signature",
      B64.encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account"}}"#)
    );
    let first = Service::OpenAi
      .credentials(
        &json!({"access_token":jwt,"refresh_token":"one","expires_in":3600}),
        None,
        100,
      )
      .unwrap();
    assert_eq!(first.account_id.as_deref(), Some("account"));
    let second = Service::OpenAi
      .credentials(
        &json!({"access_token":jwt,"refresh_token":"two","expires_in":3600}),
        Some(&first),
        200,
      )
      .unwrap();
    assert_eq!(second.refresh_token, "two");
    assert_eq!(second.expires_at, 3800);
    let grok = Service::Grok
      .credentials(&json!({"access_token":"new"}), Some(&second), 200)
      .unwrap();
    assert_eq!(grok.refresh_token, "two");
    assert_eq!(grok.expires_at, 3800);
    assert!(
      Service::OpenAi
        .credentials(
          &json!({"access_token":"bad","refresh_token":"r","expires_in":3600}),
          None,
          0
        )
        .is_err()
    );
  }
}

#[cfg(test)]
mod transport_tests {
  use super::*;
  use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
  };
  #[tokio::test]
  async fn token_encoding_and_redirect_rejection() {
    for service in [Service::Anthropic, Service::Grok] {
      let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
      let url = format!("http://{}", listener.local_addr().unwrap());
      let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let request = loop {
          let mut chunk = [0u8; 4096];
          let n = stream.read(&mut chunk).await.unwrap();
          assert_ne!(n, 0);
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|s| s == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]);
            let len: usize = head
              .lines()
              .find_map(|l| {
                l.to_ascii_lowercase()
                  .strip_prefix("content-length:")
                  .map(|s| s.trim().parse().unwrap())
              })
              .unwrap();
            if buf.len() >= end + 4 + len {
              break String::from_utf8(buf).unwrap();
            }
          }
        };
        let body = r#"{"access_token":"new","refresh_token":"rotated","expires_in":3600}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        request
      });
      let c = service
        .tokens_at(
          &url,
          json!({"refresh_token":"a+b&c", "grant_type":"refresh_token"}),
          None,
        )
        .await
        .unwrap();
      assert_eq!(c.refresh_token, "rotated");
      let request = task.await.unwrap();
      let (head, body) = request.split_once("\r\n\r\n").unwrap();
      if service == Service::Anthropic {
        assert!(head.contains("application/json"));
        assert!(head.contains(CLAUDE_USER_AGENT));
        assert_eq!(
          serde_json::from_str::<Value>(body).unwrap()["refresh_token"],
          "a+b&c"
        );
      } else {
        assert!(head.contains("application/x-www-form-urlencoded"));
        assert!(body.contains("refresh_token=a%2Bb%26c"));
      }
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
      let (mut s, _) = listener.accept().await.unwrap();
      let mut buf = [0u8; 4096];
      let _ = s.read(&mut buf).await;
      s.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: https://example.invalid/token\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let err = Service::Grok
      .tokens_at(&url, json!({"refresh_token":"never-print-me"}), None)
      .await
      .err()
      .unwrap()
      .to_string();
    assert!(err.contains("307"));
    assert!(!err.contains("never-print-me"));
  }
}
