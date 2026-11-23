//! Local subscription credentials. A per-provider OS lock serializes refresh,
//! login and logout across CLI and node processes. Tokens never enter records.
use crate::{Error, Result, keys::Store, secret};
use serde::{Deserialize, Serialize};
use std::{
  future::Future,
  path::{Path, PathBuf},
  pin::Pin,
  sync::Arc,
  time::Duration,
};
use stitch_providers::oauth::{Credentials, Login, Service, TokenFuture, TokenSource, Tokens};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  net::TcpListener,
};

fn file(data: &Path, service: Service) -> PathBuf {
  data.join(format!("oauth-{}.json", service.name()))
}
const SECRET_SERVICE: &str = "stitch-oauth";
/// The keychain account of a service's tokens in `data` (`secret::scope`).
fn account(data: &Path, service: Service) -> Result<String> {
  Ok(
    match secret::scope(data).map_err(Error::io(data.display()))? {
      Some(id) => format!("{}:{id}", service.name()),
      None => service.name().to_owned(),
    },
  )
}
async fn lock(data: &Path, service: Service) -> Result<std::fs::File> {
  std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
  let path = data.join(format!("oauth-{}.lock", service.name()));
  let f = std::fs::OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(false)
    .open(&path)
    .map_err(Error::io(path.display()))?;
  tokio::time::timeout(Duration::from_secs(660), async {
    loop {
      match f.try_lock() {
        Ok(()) => return Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => {
          tokio::time::sleep(Duration::from_millis(50)).await
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(Error::io(path.display())(e)),
      }
    }
  })
  .await
  .map_err(|_| Error::Config("OAuth credentials are busy; try again".into()))?
}
// A tombstone prevents stale keychain credentials from reappearing after logout,
// including when the keychain is temporarily unavailable.
#[derive(Serialize, Deserialize)]
struct FileState {
  #[serde(default)]
  keychain: bool,
  credentials: Option<Credentials>,
}
fn load(data: &Path, service: Service) -> Result<Option<(Credentials, Store)>> {
  let path = file(data, service);
  match std::fs::read(&path) {
    Ok(bytes) => {
      let state: FileState = serde_json::from_slice(&bytes)
        .map_err(|_| Error::Config(format!("invalid OAuth credentials in {}", path.display())))?;
      if !state.keychain {
        return Ok(state.credentials.map(|c| (c, Store::File)));
      }
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(e) => return Err(Error::io(path.display())(e)),
  }
  match secret::get(SECRET_SERVICE, &account(data, service)?) {
    Ok(Some(text)) => Ok(Some((
      serde_json::from_str(&text)
        .map_err(|_| Error::Config("invalid OAuth credentials in keychain".into()))?,
      Store::Keychain,
    ))),
    Ok(None) => Ok(None),
    Err(secret::Unavailable) => Err(Error::Config(
      "OAuth keychain unavailable; unlock it or use stitch auth login --file".into(),
    )),
  }
}
fn write_file(
  data: &Path,
  service: Service,
  credentials: Option<&Credentials>,
  keychain: bool,
) -> Result<()> {
  use std::io::Write;
  let path = file(data, service);
  let mut tmp = tempfile::NamedTempFile::new_in(data).map_err(Error::io(data.display()))?;
  let bytes = serde_json::to_vec(&FileState {
    keychain,
    credentials: credentials.cloned(),
  })
  .map_err(|_| Error::Config("cannot encode OAuth credentials".into()))?;
  tmp.write_all(&bytes).map_err(Error::io(path.display()))?;
  tmp
    .as_file()
    .sync_all()
    .map_err(Error::io(path.display()))?;
  tmp
    .persist(&path)
    .map_err(|e| Error::io(path.display())(e.error))?;
  Ok(())
}
fn save(data: &Path, service: Service, credentials: &Credentials, store: Store) -> Result<Store> {
  if store == Store::Keychain {
    let text = serde_json::to_string(credentials)
      .map_err(|_| Error::Config("cannot encode OAuth credentials".into()))?;
    if let Ok(account) = account(data, service)
      && secret::set(SECRET_SERVICE, &account, &text).is_ok()
    {
      write_file(data, service, None, true)?;
      return Ok(Store::Keychain);
    }
  }
  write_file(data, service, Some(credentials), false)?;
  Ok(Store::File)
}

/// The signed-in credentials, refreshed first when they near expiry.
pub async fn credentials(data: &Path, service: Service) -> Result<Option<Credentials>> {
  credentials_using(data, service, None, refresh_with_service).await
}

/// Credentials to use instead of the access token `stale`, which the
/// provider refused: refreshed if the stored token is still `stale`, else
/// the stored ones, which another request refreshed meanwhile. Under the
/// lock, so concurrent turns refused at once refresh only once.
pub async fn credentials_forced(
  data: &Path,
  service: Service,
  stale: &str,
) -> Result<Option<Credentials>> {
  credentials_using(data, service, Some(stale), refresh_with_service).await
}

async fn refresh_with_service(
  service: Service,
  c: Credentials,
) -> stitch_providers::Result<Credentials> {
  service.refresh(&c).await
}

async fn credentials_using<F, Fut>(
  data: &Path,
  service: Service,
  stale: Option<&str>,
  refresh: F,
) -> Result<Option<Credentials>>
where
  F: FnOnce(Service, Credentials) -> Fut,
  Fut: Future<Output = stitch_providers::Result<Credentials>>,
{
  let _guard = lock(data, service).await?;
  let Some((mut c, store)) = load(data, service)? else {
    return Ok(None);
  };
  if c.needs_refresh() || stale == Some(c.access_token.as_str()) {
    c = refresh(service, c).await?;
    save(data, service, &c, store)?;
  }
  Ok(Some(c))
}

type Refresher = Arc<
  dyn Fn(
      Service,
      Credentials,
    ) -> Pin<Box<dyn Future<Output = stitch_providers::Result<Credentials>> + Send>>
    + Send
    + Sync,
>;

/// The token source of a provider on a managed subscription: the stored
/// credentials, asked before each request (design §2.8).
struct Stored {
  data: PathBuf,
  service: Service,
  refresh: Refresher,
}

/// `service`'s subscription tokens in `data`, for a provider to ask per
/// request.
pub fn tokens(data: &Path, service: Service) -> Tokens {
  tokens_using(
    data,
    service,
    Arc::new(|s, c| Box::pin(refresh_with_service(s, c))),
  )
}

fn tokens_using(data: &Path, service: Service, refresh: Refresher) -> Tokens {
  Tokens(Arc::new(Stored {
    data: data.to_owned(),
    service,
    refresh,
  }))
}

impl Stored {
  async fn access_token(&self, stale: Option<&str>) -> stitch_providers::Result<String> {
    let refresh = self.refresh.clone();
    match credentials_using(&self.data, self.service, stale, move |s, c| refresh(s, c)).await {
      Ok(Some(c)) => Ok(c.access_token),
      Ok(None) => Err(stitch_providers::Error::Protocol {
        provider: "oauth",
        message: format!(
          "signed out of {0}; run stitch auth login {0}",
          self.service.name()
        ),
      }),
      Err(Error::Provider(e)) => Err(e),
      Err(e) => Err(stitch_providers::Error::Protocol {
        provider: "oauth",
        message: e.to_string(),
      }),
    }
  }
}

impl TokenSource for Stored {
  fn token(&self) -> TokenFuture<'_> {
    Box::pin(self.access_token(None))
  }
  fn refreshed<'a>(&'a self, stale: &'a str) -> TokenFuture<'a> {
    Box::pin(self.access_token(Some(stale)))
  }
}

pub async fn status(data: &Path, service: Service) -> Result<String> {
  let _guard = lock(data, service).await?;
  Ok(match load(data, service)? {
    None => "signed out".into(),
    Some((c, store)) => format!(
      "signed in ({}, {})",
      if c.needs_refresh() {
        "refresh needed"
      } else {
        "token current"
      },
      if store == Store::File {
        "private file"
      } else {
        "keychain"
      }
    ),
  })
}
/// Whether a subscription session is stored, without refreshing it.
pub async fn signed_in(data: &Path, service: Service) -> Result<bool> {
  let _guard = lock(data, service).await?;
  Ok(load(data, service)?.is_some())
}
pub async fn logout(data: &Path, service: Service) -> Result<()> {
  let _guard = lock(data, service).await?;
  write_file(data, service, None, false)?;
  if let Ok(account) = account(data, service) {
    secret::delete(SECRET_SERVICE, &account);
  }
  Ok(())
}

/// Bind before showing the URL; a conflicting CLI cannot steal the callback.
/// `show_url` belongs to the client UI. On a remote node, forward the printed port.
pub async fn login(
  data: &Path,
  service: Service,
  store: Store,
  show_url: impl FnOnce(&str),
) -> Result<Store> {
  let _guard = lock(data, service).await?;
  let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, service.port()))
    .await
    .map_err(Error::io("OAuth callback listener"))?;
  let flow = Login::new(service);
  show_url(&flow.authorize_url());
  let code = tokio::time::timeout(Duration::from_secs(600), callback(&listener, &flow))
    .await
    .map_err(|_| Error::Config("OAuth login timed out after 10 minutes".into()))??;
  let c = flow.exchange(&code).await?;
  save(data, service, &c, store)
}
async fn callback(listener: &TcpListener, flow: &Login) -> Result<String> {
  loop {
    let (mut stream, _) = listener
      .accept()
      .await
      .map_err(Error::io("OAuth callback"))?;
    let request = tokio::time::timeout(Duration::from_secs(5), async {
      let mut buf = Vec::new();
      loop {
        let mut chunk = [0u8; 1024];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
          break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 16 * 1024 {
          return Err(std::io::Error::other("callback too large"));
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
          break;
        }
      }
      Ok::<_, std::io::Error>(buf)
    })
    .await;
    let Ok(Ok(buf)) = request else { continue };
    let text = String::from_utf8_lossy(&buf);
    let mut first = text.lines().next().unwrap_or_default().split_whitespace();
    let method = first.next();
    let target = first.next().unwrap_or_default();
    let code = if method == Some("GET") {
      flow.callback_code(target).ok()
    } else {
      None
    };
    let (status, body) = if code.is_some() {
      (
        "200 OK",
        "Authorization received. Return to Stitch to finish signing in.",
      )
    } else {
      (
        "400 Bad Request",
        "Invalid OAuth callback. Continue sign-in in the original browser tab.",
      )
    };
    let response = format!(
      "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
      body.len()
    );
    let _ = tokio::time::timeout(
      Duration::from_secs(5),
      stream.write_all(response.as_bytes()),
    )
    .await;
    if let Some(code) = code {
      return Ok(code);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  /// A subscription signed in to wins over an API key (design §2.8).
  #[tokio::test]
  async fn a_subscription_wins_over_an_api_key() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
      dir.path().join("credentials.toml"),
      "anthropic = \"sk-key\"\n",
    )
    .unwrap();
    let config = crate::config::Config::default();
    let model = Some("anthropic/claude-x");
    assert_eq!(config.billing(dir.path(), "anthropic").await, "API key");
    let (p, _, _) = config
      .resolve_authenticated(dir.path(), model)
      .await
      .unwrap();
    let stitch_providers::Provider::Anthropic(a) = p else {
      panic!("anthropic")
    };
    assert!(!a.oauth);
    assert_eq!(a.api_key, "sk-key");
    let c = Credentials {
      access_token: "token".into(),
      refresh_token: "refresh".into(),
      expires_at: u64::MAX,
      account_id: None,
    };
    save(dir.path(), Service::Anthropic, &c, Store::File).unwrap();
    assert_eq!(
      config.billing(dir.path(), "anthropic").await,
      "subscription"
    );
    let (p, _, _) = config
      .resolve_authenticated(dir.path(), model)
      .await
      .unwrap();
    let stitch_providers::Provider::Anthropic(a) = p else {
      panic!("anthropic")
    };
    assert!(a.oauth);
    assert_eq!(a.api_key, "token");
  }
  #[tokio::test]
  async fn private_file_status_and_logout() {
    let dir = tempfile::tempdir().unwrap();
    let c = Credentials {
      access_token: "secret".into(),
      refresh_token: "refresh".into(),
      expires_at: u64::MAX,
      account_id: None,
    };
    save(dir.path(), Service::Anthropic, &c, Store::File).unwrap();
    assert_eq!(
      credentials(dir.path(), Service::Anthropic)
        .await
        .unwrap()
        .unwrap()
        .access_token,
      "secret"
    );
    assert!(
      !status(dir.path(), Service::Anthropic)
        .await
        .unwrap()
        .contains("secret")
    );
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        std::fs::metadata(file(dir.path(), Service::Anthropic))
          .unwrap()
          .permissions()
          .mode()
          & 0o777,
        0o600
      );
    }
    logout(dir.path(), Service::Anthropic).await.unwrap();
    assert!(
      credentials(dir.path(), Service::Anthropic)
        .await
        .unwrap()
        .is_none()
    );
  }
  #[tokio::test]
  async fn locks_serialize_refresh_and_corruption_is_not_signed_out() {
    let dir = tempfile::tempdir().unwrap();
    let guard = lock(dir.path(), Service::Grok).await.unwrap();
    assert!(
      tokio::time::timeout(Duration::from_millis(100), lock(dir.path(), Service::Grok))
        .await
        .is_err()
    );
    drop(guard);
    let _guard = lock(dir.path(), Service::Grok).await.unwrap();
    std::fs::write(file(dir.path(), Service::Grok), "bad").unwrap();
    assert!(load(dir.path(), Service::Grok).is_err());
  }

  fn signed_in(data: &Path, token: &str, expires_in: u64) {
    let c = Credentials {
      access_token: token.into(),
      refresh_token: "refresh".into(),
      expires_at: now() + expires_in,
      account_id: None,
    };
    save(data, Service::Anthropic, &c, Store::File).unwrap();
  }
  fn now() -> u64 {
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_secs()
  }
  /// A refresher that counts its calls and hands out `fresh-<n>`, a while
  /// later (as a token server does).
  fn counting(count: &Arc<std::sync::atomic::AtomicUsize>) -> Refresher {
    let count = count.clone();
    Arc::new(move |_, c: Credentials| {
      let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
      Box::pin(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(Credentials {
          access_token: format!("fresh-{n}"),
          expires_at: now() + 8 * 3600,
          ..c
        })
      })
    })
  }

  #[tokio::test]
  async fn a_token_expiring_mid_turn_is_refreshed_between_steps() {
    // A turn's provider on the stored subscription, against a server that
    // records each request's bearer token.
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path(), "first", 8 * 3600);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    tokio::spawn(async move {
      loop {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        // The head, then the body its content-length says.
        let (head, len) = loop {
          let n = sock.read(&mut chunk).await.unwrap();
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            let len: usize = head
              .lines()
              .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                  .then(|| v.trim().parse().unwrap())
              })
              .unwrap_or(0);
            break (head, end + 4 + len);
          }
        };
        while buf.len() < len {
          let n = sock.read(&mut chunk).await.unwrap();
          buf.extend_from_slice(&chunk[..n]);
        }
        log.lock().unwrap().push(
          head
            .lines()
            .find_map(|l| {
              let (k, v) = l.split_once(':')?;
              k.eq_ignore_ascii_case("authorization")
                .then(|| v.trim().to_owned())
            })
            .unwrap_or_default(),
        );
        let body = concat!(
          "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1}}}\n\n",
          "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
          "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
          "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
          "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
          "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let response = format!(
          "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
        );
        sock.write_all(response.as_bytes()).await.unwrap();
        sock.shutdown().await.unwrap();
      }
    });
    let mut a = stitch_providers::Anthropic::new("first");
    a.base_url = base;
    a.oauth = true;
    a.tokens = Some(tokens_using(
      dir.path(),
      Service::Anthropic,
      counting(&count),
    ));
    let provider = stitch_providers::Provider::Anthropic(a);
    let request = stitch_providers::Request::new(
      "m",
      vec![stitch_providers::Message::user(vec![
        stitch_providers::Content::Text("x".into()),
      ])],
    );
    provider.stream(&request, &mut |_| {}).await.unwrap();
    // Hours later, in the same turn: the stored token nears expiry.
    signed_in(dir.path(), "first", 60);
    provider.stream(&request, &mut |_| {}).await.unwrap();
    provider.stream(&request, &mut |_| {}).await.unwrap();
    assert_eq!(
      *seen.lock().unwrap(),
      ["Bearer first", "Bearer fresh-1", "Bearer fresh-1"]
    );
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    // Saved, so the next turn (and other processes) use it too.
    assert_eq!(
      load(dir.path(), Service::Anthropic)
        .unwrap()
        .unwrap()
        .0
        .access_token,
      "fresh-1"
    );
  }

  #[tokio::test]
  async fn the_refresh_margin_is_minutes_wide() {
    let dir = tempfile::tempdir().unwrap();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let source = tokens_using(dir.path(), Service::Anthropic, counting(&count));
    signed_in(dir.path(), "current", 3600);
    assert_eq!(source.0.token().await.unwrap(), "current");
    signed_in(dir.path(), "near", 10 * 60);
    assert_eq!(source.0.token().await.unwrap(), "fresh-1");
    logout(dir.path(), Service::Anthropic).await.unwrap();
    let err = source.0.token().await.unwrap_err();
    assert!(err.to_string().contains("signed out"), "{err}");
  }

  #[tokio::test]
  async fn concurrent_forced_refreshes_refresh_once() {
    // Several turns refused the same token at once: one refreshes, the
    // others get its token rather than refreshing again.
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path(), "stale", 8 * 3600);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let source = tokens_using(dir.path(), Service::Anthropic, counting(&count));
    let mut turns = Vec::new();
    for _ in 0..8 {
      let source = source.clone();
      turns.push(tokio::spawn(async move {
        source.0.refreshed("stale").await.unwrap()
      }));
    }
    for turn in turns {
      assert_eq!(turn.await.unwrap(), "fresh-1");
    }
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    // A refusal of a token that is not the stored one refreshes nothing.
    assert_eq!(source.0.refreshed("older").await.unwrap(), "fresh-1");
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The stored one refused again refreshes again.
    assert_eq!(source.0.refreshed("fresh-1").await.unwrap(), "fresh-2");
  }
}

#[cfg(test)]
mod callback_tests {
  use super::*;
  #[tokio::test]
  async fn invalid_callbacks_do_not_consume_login() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let flow = Login::new(Service::Grok);
    let url = flow.authorize_url();
    let state = url
      .split("state=")
      .nth(1)
      .unwrap()
      .split('&')
      .next()
      .unwrap()
      .to_owned();
    let task = tokio::spawn(async move { callback(&listener, &flow).await });
    for (target, status) in [
      ("/favicon.ico".to_owned(), "400"),
      ("/callback?code=wrong&state=wrong".to_owned(), "400"),
      (format!("/callback?code=right&state={state}"), "200"),
    ] {
      let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
      stream
        .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
        .await
        .unwrap();
      let mut result = String::new();
      stream.read_to_string(&mut result).await.unwrap();
      assert!(result.starts_with(&format!("HTTP/1.1 {status}")));
      assert!(!result.contains(&state));
    }
    assert_eq!(task.await.unwrap().unwrap(), "right");
  }
  #[tokio::test]
  async fn empty_directory_is_signed_out_without_keychain_access() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
      status(dir.path(), Service::OpenAi).await.unwrap(),
      "signed out"
    );
  }
}
