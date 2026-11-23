//! GitHub sign-in (design §2.8): the OAuth device flow (RFC 8628) of an
//! OAuth App, as edge talk's, so there is no client secret, no redirect and
//! no server of ours, and it works on a headless node or a phone alike.
//! The token is the key `github` (`keys`), so an MCP server's
//! `key = "github"` uses it; the account it belongs to is kept beside it
//! in `github.json`, which holds nothing secret. Each node signs in on its
//! own: tokens never sync.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::keys::{self, Store};
use crate::{Error, Result};

/// edge talk's OAuth App, for now (design §2.8: to be replaced by one
/// registered for Stitch). Its token expiration is off: refreshing an
/// OAuth App's token needs its client secret, which a device-flow client
/// does not hold.
const CLIENT_ID: &str = "Ov23liZBMqn9tfgy2tzu";

/// edge talk's scopes: what the GitHub MCP tools need, and GitHub has no
/// read-only repository scope. No admin or delete scopes.
const SCOPES: &str =
  "repo user gist notifications workflow read:org project security_events read:audit_log";

/// The key the token is stored as.
pub const KEY: &str = "github";

/// Where the flow's requests go: GitHub's, or a test's.
#[derive(Clone, Debug)]
pub struct Endpoints {
  pub device_code: String,
  pub access_token: String,
  pub api: String,
}

impl Default for Endpoints {
  fn default() -> Self {
    Endpoints {
      device_code: "https://github.com/login/device/code".into(),
      access_token: "https://github.com/login/oauth/access_token".into(),
      api: "https://api.github.com".into(),
    }
  }
}

/// What the user does to sign in: enter `user_code` at `verification_uri`.
#[derive(Clone, Debug)]
pub struct Prompt {
  pub user_code: String,
  pub verification_uri: String,
  /// How long the code stays valid.
  pub expires_in: Duration,
}

/// The account a token belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
  pub login: String,
  /// GitHub's number for the account: commits are authored as
  /// `<id>+<login>@users.noreply.github.com`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  /// The scopes GitHub granted.
  #[serde(default)]
  pub scopes: String,
}

fn account_file(data: &Path) -> PathBuf {
  data.join("github.json")
}

pub(crate) fn http() -> reqwest::Client {
  // Errors when a provider is installed already, which is fine.
  let _ = rustls::crypto::ring::default_provider().install_default();
  reqwest::Client::builder()
    .user_agent(concat!("stitch/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(Duration::from_secs(20))
    .timeout(Duration::from_secs(60))
    .build()
    .expect("an HTTP client with default settings builds")
}

/// A form body, every value percent-encoded (edge talk sent scopes and the
/// device code raw).
fn form(fields: &[(&str, &str)]) -> String {
  fields
    .iter()
    .map(|(k, v)| format!("{k}={}", encode(v)))
    .collect::<Vec<_>>()
    .join("&")
}

fn encode(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
      _ => out.push_str(&format!("%{b:02X}")),
    }
  }
  out
}

async fn post(client: &reqwest::Client, url: &str, body: String) -> Result<serde_json::Value> {
  let resp = client
    .post(url)
    .header("Accept", "application/json")
    .header("Content-Type", "application/x-www-form-urlencoded")
    .body(body)
    .send()
    .await
    .map_err(|e| Error::Config(format!("GitHub: {e}")))?;
  let status = resp.status();
  let text = resp
    .text()
    .await
    .map_err(|e| Error::Config(format!("GitHub: {e}")))?;
  serde_json::from_str(&text).map_err(|_| {
    Error::Config(format!(
      "GitHub answered {status} with something other than JSON"
    ))
  })
}

#[derive(Deserialize)]
struct DeviceCode {
  device_code: String,
  user_code: String,
  verification_uri: String,
  expires_in: u64,
  #[serde(default = "five")]
  interval: u64,
}

fn five() -> u64 {
  5
}

/// Signs in to GitHub: shows the user a code to enter (`show`), waits for
/// them to, and stores the token and the account. Cancelled by dropping it.
pub async fn login(data: &Path, store: Store, show: impl FnOnce(&Prompt)) -> Result<Account> {
  login_with(data, store, &Endpoints::default(), show).await
}

pub async fn login_with(
  data: &Path,
  store: Store,
  at: &Endpoints,
  show: impl FnOnce(&Prompt),
) -> Result<Account> {
  let client = http();
  let v = post(
    &client,
    &at.device_code,
    form(&[("client_id", CLIENT_ID), ("scope", SCOPES)]),
  )
  .await?;
  if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
    return Err(said(e, &v));
  }
  let code: DeviceCode =
    serde_json::from_value(v).map_err(|e| Error::Config(format!("GitHub's device code: {e}")))?;
  show(&Prompt {
    user_code: code.user_code.clone(),
    verification_uri: code.verification_uri.clone(),
    expires_in: Duration::from_secs(code.expires_in),
  });
  let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
  let mut interval = Duration::from_secs(code.interval);
  let token = loop {
    tokio::time::sleep(interval).await;
    if tokio::time::Instant::now() >= deadline {
      return Err(Error::Config(
        "the GitHub code expired before it was entered; sign in again".into(),
      ));
    }
    let v = post(
      &client,
      &at.access_token,
      form(&[
        ("client_id", CLIENT_ID),
        ("device_code", &code.device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
      ]),
    )
    .await?;
    if let Some(t) = v.get("access_token").and_then(|t| t.as_str()) {
      // No `expires_in`: a token that does not expire (edge talk assumed
      // eight hours, then thought valid tokens had expired).
      if v.get("expires_in").is_some() {
        eprintln!(
          "github: an expiring token: the OAuth App's token expiration is on, and it cannot be refreshed without the app's secret"
        );
      }
      break (
        t.to_owned(),
        v.get("scope")
          .and_then(|s| s.as_str())
          .unwrap_or_default()
          .to_owned(),
      );
    }
    match v.get("error").and_then(|e| e.as_str()) {
      Some("authorization_pending") => {}
      Some("slow_down") => interval += Duration::from_secs(5),
      Some(e) => return Err(said(e, &v)),
      None => {
        return Err(Error::Config(
          "GitHub answered with neither a token nor an error".into(),
        ));
      }
    }
  };
  let (token, scopes) = token;
  let mut account = user(&client, &at.api, &token).await?;
  account.scopes = scopes.replace(',', " ");
  keys::set(data, KEY, &token, store)?;
  let json = serde_json::to_vec_pretty(&account).map_err(|e| Error::Config(e.to_string()))?;
  let path = account_file(data);
  std::fs::write(&path, json).map_err(Error::io(path.display()))?;
  Ok(account)
}

fn said(error: &str, v: &serde_json::Value) -> Error {
  let what = match error {
    "access_denied" => "GitHub sign-in was declined".to_owned(),
    "expired_token" => "the GitHub code expired before it was entered; sign in again".to_owned(),
    "device_flow_disabled" => "the OAuth App does not allow the device flow".to_owned(),
    other => {
      let detail = v
        .get("error_description")
        .and_then(|d| d.as_str())
        .unwrap_or("");
      format!("GitHub: {other} {detail}").trim_end().to_owned()
    }
  };
  Error::Config(what)
}

pub(crate) async fn user(client: &reqwest::Client, api: &str, token: &str) -> Result<Account> {
  let resp = client
    .get(format!("{api}/user"))
    .bearer_auth(token)
    .header("Accept", "application/vnd.github+json")
    .header("X-GitHub-Api-Version", "2022-11-28")
    .send()
    .await
    .map_err(|e| Error::Config(format!("GitHub: {e}")))?;
  if !resp.status().is_success() {
    return Err(Error::Config(format!(
      "GitHub did not say whose token this is ({})",
      resp.status()
    )));
  }
  let v: serde_json::Value = resp
    .json()
    .await
    .map_err(|e| Error::Config(format!("GitHub: {e}")))?;
  Ok(Account {
    login: v
      .get("login")
      .and_then(|l| l.as_str())
      .ok_or_else(|| Error::Config("GitHub's /user has no login".into()))?
      .to_owned(),
    id: v.get("id").and_then(|n| n.as_u64()),
    name: v.get("name").and_then(|n| n.as_str()).map(str::to_owned),
    scopes: String::new(),
  })
}

/// The token, from wherever keys are (`stitch key set github` too).
pub fn token(data: &Path) -> Option<String> {
  keys::get(data, KEY, KEY)
}

/// The account signed in, when there is a token: `None` signed out.
pub fn account(data: &Path) -> Option<Account> {
  token(data)?;
  std::fs::read(account_file(data))
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .or_else(|| {
      // A token set as a key, never signed in: no account known.
      Some(Account {
        login: "(a token set as a key)".into(),
        id: None,
        name: None,
        scopes: String::new(),
      })
    })
}

/// `signed out`, or who is signed in.
pub fn status(data: &Path) -> String {
  match account(data) {
    None => "signed out".into(),
    Some(a) => format!("signed in as {}", a.login),
  }
}

/// Removes the token and the account. GitHub keeps the authorization until
/// the user revokes it (Settings, Applications).
pub fn logout(data: &Path) -> Result<()> {
  keys::remove(data, KEY)?;
  match std::fs::remove_file(account_file(data)) {
    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
      Err(Error::io(account_file(data).display())(e))
    }
    _ => Ok(()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::{Arc, Mutex};
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  /// A GitHub of three endpoints: the device code, a token after one
  /// pending poll, and the user.
  async fn mock() -> (Endpoints, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
      let mut polls = 0;
      loop {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        loop {
          n += s.read(&mut buf[n..]).await.unwrap();
          let text = String::from_utf8_lossy(&buf[..n]);
          if let Some(end) = text.find("\r\n\r\n") {
            let len = text
              .lines()
              .find_map(|l| {
                l.to_ascii_lowercase()
                  .strip_prefix("content-length: ")
                  .map(|v| v.trim().parse::<usize>().unwrap())
              })
              .unwrap_or(0);
            if n >= end + 4 + len {
              break;
            }
          }
        }
        let req = String::from_utf8_lossy(&buf[..n]).into_owned();
        log.lock().unwrap().push(req.clone());
        let body = if req.starts_with("POST /device") {
          r#"{"device_code":"d c&1","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","expires_in":900,"interval":0}"#.to_owned()
        } else if req.starts_with("POST /token") {
          polls += 1;
          if polls == 1 {
            r#"{"error":"authorization_pending"}"#.to_owned()
          } else {
            r#"{"access_token":"gho_test","token_type":"bearer","scope":"repo,user"}"#.to_owned()
          }
        } else {
          r#"{"login":"octo","id":583231,"name":"Octo Cat"}"#.to_owned()
        };
        let resp = format!(
          "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
          body.len()
        );
        s.write_all(resp.as_bytes()).await.unwrap();
      }
    });
    (
      Endpoints {
        device_code: format!("{base}/device"),
        access_token: format!("{base}/token"),
        api: base,
      },
      seen,
    )
  }

  #[tokio::test]
  async fn the_device_flow_signs_in_and_out() {
    let dir = tempfile::tempdir().unwrap();
    let (at, seen) = mock().await;
    let mut shown = None;
    let account = login_with(dir.path(), Store::File, &at, |p| {
      shown = Some(p.user_code.clone())
    })
    .await
    .unwrap();
    assert_eq!(shown.as_deref(), Some("ABCD-1234"));
    assert_eq!(account.login, "octo");
    assert_eq!(account.id, Some(583231));
    assert_eq!(account.scopes, "repo user");
    assert_eq!(token(dir.path()).as_deref(), Some("gho_test"));
    assert_eq!(status(dir.path()), "signed in as octo");
    let seen = seen.lock().unwrap();
    // Scopes and the device code percent-encoded; the token sent to /user.
    assert!(seen[0].contains("scope=repo%20user%20gist"), "{}", seen[0]);
    assert!(seen[1].contains("device_code=d%20c%261"), "{}", seen[1]);
    assert!(
      seen[3].contains("authorization: Bearer gho_test")
        || seen[3].contains("Authorization: Bearer gho_test")
    );
    drop(seen);
    logout(dir.path()).unwrap();
    assert_eq!(status(dir.path()), "signed out");
    assert!(!account_file(dir.path()).exists());
  }

  #[test]
  fn values_are_percent_encoded() {
    assert_eq!(encode("read:org repo"), "read%3Aorg%20repo");
    assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
  }
}
