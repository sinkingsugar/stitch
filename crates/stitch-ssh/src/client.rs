//! Connections: one per host, its channels shared by its sessions; host
//! keys checked (trusted on first use, a changed one refused); the user
//! authenticated with what this device has.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};

use crate::{Error, Result};

/// How long connecting (and authenticating) may take.
const CONNECT: Duration = Duration::from_secs(20);

/// A host, resolved: where it is, as whom, through which jump host, and
/// how to authenticate.
#[derive(Clone)]
pub struct Target {
  /// Its name (a host record's, or `~/.ssh/config`'s).
  pub name: String,
  pub address: String,
  pub port: u16,
  pub user: String,
  pub jump: Option<Box<Target>>,
  /// Tried in order until one is accepted.
  pub auth: Vec<Auth>,
}

impl std::fmt::Debug for Target {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Target")
      .field("name", &self.name)
      .field("address", &self.address)
      .field("port", &self.port)
      .field("user", &self.user)
      .field("jump", &self.jump)
      .field("auth", &self.auth)
      .finish()
  }
}

impl Target {
  /// Where host keys are kept for it: OpenSSH's `[address]:port` (the
  /// address alone on port 22), so two names of one machine agree.
  pub fn key_host(&self) -> String {
    key_host(&self.address, self.port)
  }
}

/// Where host keys of `address:port` are kept.
pub fn key_host(address: &str, port: u16) -> String {
  if port == 22 {
    address.to_owned()
  } else {
    format!("[{address}]:{port}")
  }
}

/// A way to authenticate.
#[derive(Clone)]
pub enum Auth {
  /// The SSH agent (`SSH_AUTH_SOCK`), every key it holds.
  Agent,
  /// A private key file (`~/.ssh/id_ed25519`); one with a passphrase is
  /// left to the agent.
  KeyFile(PathBuf),
  /// A password (a secret's value).
  Password(String),
  /// A key held in memory (this device's own, where it has no Secure
  /// Enclave).
  Key(Arc<PrivateKey>),
  /// A key that signs elsewhere (the Secure Enclave).
  Signer(Arc<dyn DeviceSigner>),
}

impl std::fmt::Debug for Auth {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Auth::Agent => f.write_str("Agent"),
      Auth::KeyFile(p) => write!(f, "KeyFile({})", p.display()),
      Auth::Password(_) => f.write_str("Password(…)"),
      Auth::Key(k) => write!(f, "Key({})", k.public_key().fingerprint(HashAlg::Sha256)),
      Auth::Signer(s) => write!(f, "Signer({})", s.public_key().fingerprint(HashAlg::Sha256)),
    }
  }
}

impl Auth {
  fn said(&self) -> String {
    match self {
      Auth::Agent => "the SSH agent".into(),
      Auth::KeyFile(p) => p.display().to_string(),
      Auth::Password(_) => "a password".into(),
      Auth::Key(_) | Auth::Signer(_) => "this device's key".into(),
    }
  }
}

/// A key whose private half never leaves where it is (the Secure
/// Enclave): it signs what authentication needs.
pub trait DeviceSigner: Send + Sync + 'static {
  fn public_key(&self) -> PublicKey;
  /// The SSH signature of `data` (its wire form: the algorithm's name,
  /// then the signature as the algorithm encodes it).
  fn sign(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String>;
}

/// What a host's key is checked against, and where a new one is kept.
pub trait HostKeys: Send + Sync + 'static {
  /// The keys trusted for `host` (`Target::key_host`).
  fn known(&self, host: &str) -> Vec<PublicKey>;
  /// Trusts `key` for `host`: it was the first seen there.
  fn trust(&self, host: &str, name: &str, key: &PublicKey);
}

/// Host keys in memory (tests, and a node without a home workspace).
#[derive(Default)]
pub struct MemoryHostKeys(pub Mutex<Vec<(String, PublicKey)>>);

impl HostKeys for MemoryHostKeys {
  fn known(&self, host: &str) -> Vec<PublicKey> {
    self
      .0
      .lock()
      .expect("keys lock")
      .iter()
      .filter(|(h, _)| h == host)
      .map(|(_, k)| k.clone())
      .collect()
  }

  fn trust(&self, host: &str, _: &str, key: &PublicKey) {
    self
      .0
      .lock()
      .expect("keys lock")
      .push((host.to_owned(), key.clone()));
  }
}

/// A key's fingerprint, as OpenSSH shows it (`SHA256:…`).
pub fn fingerprint(key: &PublicKey) -> String {
  key.fingerprint(HashAlg::Sha256).to_string()
}

/// What the check found, kept for the error.
#[derive(Default)]
struct Checked {
  changed: Option<(Vec<String>, String)>,
}

struct Client {
  host: String,
  name: String,
  keys: Arc<dyn HostKeys>,
  /// `~/.ssh/known_hosts`, consulted too where there is one.
  known_hosts: Option<PathBuf>,
  address: String,
  port: u16,
  checked: Arc<Mutex<Checked>>,
}

impl client::Handler for Client {
  type Error = russh::Error;

  async fn check_server_key(
    &mut self,
    key: &russh::keys::PublicKeyOrCertificate,
  ) -> std::result::Result<bool, Self::Error> {
    // Host keys only: no certificate algorithm is advertised (russh's
    // default), and a host certificate needs trusted authorities we do not
    // have.
    let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
      return Ok(false);
    };
    let known = self.keys.known(&self.host);
    if known.iter().any(|k| k.key_data() == key.key_data()) {
      return Ok(true);
    }
    // The user's known_hosts: a key it records for the host wins over a
    // first use; one that differs is a change.
    if let Some(path) = &self.known_hosts {
      match russh::keys::check_known_hosts_path(&self.address, self.port, key, path) {
        Ok(_) => {}
        Err(russh::keys::Error::KeyChanged { line }) => {
          self.checked.lock().expect("checked lock").changed = Some((
            vec![format!("{} (line {line})", path.display())],
            fingerprint(key),
          ));
          return Ok(false);
        }
        Err(_) => {}
      }
    }
    if !known.is_empty() {
      self.checked.lock().expect("checked lock").changed =
        Some((known.iter().map(fingerprint).collect(), fingerprint(key)));
      return Ok(false);
    }
    // Trusted on first use (or as the user's known_hosts has it).
    self.keys.trust(&self.host, &self.name, key);
    Ok(true)
  }
}

/// An authenticated connection to a host.
pub struct Connection {
  handle: Handle<Client>,
  pub target: Target,
  /// The jump host's connection this one goes through.
  _jump: Option<Arc<Connection>>,
}

/// What a command run with `exec` did.
#[derive(Debug, Default)]
pub struct Exec {
  pub status: Option<u32>,
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
}

impl Exec {
  pub fn ok(&self) -> bool {
    self.status == Some(0)
  }

  pub fn stdout_text(&self) -> String {
    String::from_utf8_lossy(&self.stdout).into_owned()
  }
}

fn config() -> Arc<client::Config> {
  Arc::new(client::Config {
    inactivity_timeout: None,
    // A dropped connection is noticed within a minute and a half.
    keepalive_interval: Some(Duration::from_secs(30)),
    keepalive_max: 3,
    nodelay: true,
    ..Default::default()
  })
}

impl Connection {
  /// Connects to `target` (through its jump host), checks its host key
  /// and authenticates.
  pub async fn connect(
    target: &Target,
    keys: Arc<dyn HostKeys>,
    known_hosts: Option<PathBuf>,
  ) -> Result<Arc<Connection>> {
    Box::pin(Self::connect_inner(target, keys, known_hosts)).await
  }

  async fn connect_inner(
    target: &Target,
    keys: Arc<dyn HostKeys>,
    known_hosts: Option<PathBuf>,
  ) -> Result<Arc<Connection>> {
    let jump = match &target.jump {
      Some(j) => Some(Self::connect(j, keys.clone(), known_hosts.clone()).await?),
      None => None,
    };
    let checked = Arc::new(Mutex::new(Checked::default()));
    let handler = Client {
      host: target.key_host(),
      name: target.name.clone(),
      keys,
      known_hosts,
      address: target.address.clone(),
      port: target.port,
      checked: checked.clone(),
    };
    let connecting = async {
      match &jump {
        Some(j) => {
          let channel = j
            .handle
            .channel_open_direct_tcpip(
              target.address.clone(),
              u32::from(target.port),
              "127.0.0.1",
              0,
            )
            .await
            .map_err(|e| Error::Connect {
              host: target.name.clone(),
              why: format!("through {}: {e}", j.target.name),
            })?;
          client::connect_stream(config(), channel.into_stream(), handler).await
        }
        None => client::connect(config(), (target.address.as_str(), target.port), handler).await,
      }
      .map_err(|e| {
        if let Some((was, now)) = checked.lock().expect("checked lock").changed.take() {
          return Error::HostKeyChanged {
            host: target.name.clone(),
            at: target.key_host(),
            was,
            now,
          };
        }
        Error::Connect {
          host: target.name.clone(),
          why: e.to_string(),
        }
      })
    };
    let mut handle = tokio::time::timeout(CONNECT, connecting)
      .await
      .map_err(|_| Error::Connect {
        host: target.name.clone(),
        why: format!("no answer within {}s", CONNECT.as_secs()),
      })??;
    authenticate(&mut handle, target).await?;
    Ok(Arc::new(Connection {
      handle,
      target: target.clone(),
      _jump: jump,
    }))
  }

  /// Whether it has dropped.
  pub fn is_closed(&self) -> bool {
    self.handle.is_closed()
  }

  /// A session channel.
  pub async fn channel(&self) -> Result<russh::Channel<client::Msg>> {
    self
      .handle
      .channel_open_session()
      .await
      .map_err(|e| self.dropped(e))
  }

  fn dropped(&self, e: russh::Error) -> Error {
    Error::Channel {
      host: self.target.name.clone(),
      why: e.to_string(),
    }
  }

  /// Runs `command` (by the user's shell there), with `stdin`, to its end
  /// or `timeout`.
  pub async fn exec(&self, command: &str, stdin: Option<&[u8]>, timeout: Duration) -> Result<Exec> {
    let mut channel = self.channel().await?;
    channel
      .exec(true, command)
      .await
      .map_err(|e| self.dropped(e))?;
    if let Some(data) = stdin {
      channel.data(data).await.map_err(|e| self.dropped(e))?;
    }
    channel.eof().await.map_err(|e| self.dropped(e))?;
    let mut out = Exec::default();
    let run = async {
      while let Some(msg) = channel.wait().await {
        match msg {
          ChannelMsg::Data { data } => out.stdout.extend_from_slice(&data),
          ChannelMsg::ExtendedData { data, .. } => out.stderr.extend_from_slice(&data),
          ChannelMsg::ExitStatus { exit_status } => out.status = Some(exit_status),
          _ => {}
        }
      }
    };
    if tokio::time::timeout(timeout, run).await.is_err() {
      let _ = channel.close().await;
      return Err(Error::Channel {
        host: self.target.name.clone(),
        why: format!("`{command}` did not end within {}s", timeout.as_secs()),
      });
    }
    Ok(out)
  }

  /// Runs a POSIX `sh` script there, given as `sh -s`'s input: the user's
  /// login shell (fish, csh) only ever sees `sh -s`.
  pub async fn script(&self, script: &str, timeout: Duration) -> Result<Exec> {
    self.exec("sh -s", Some(script.as_bytes()), timeout).await
  }

  /// Says goodbye.
  pub async fn disconnect(&self) {
    let _ = self
      .handle
      .disconnect(russh::Disconnect::ByApplication, "", "en")
      .await;
  }
}

/// A key file's key: unencrypted ones only (an encrypted one is the
/// agent's to use).
fn key_file(path: &PathBuf) -> Option<PrivateKey> {
  russh::keys::load_secret_key(path, None).ok()
}

struct Signing(Arc<dyn DeviceSigner>);

impl russh::Signer for Signing {
  type Error = SignError;

  fn auth_sign(
    &mut self,
    _: &russh::keys::agent::AgentIdentity,
    _: Option<HashAlg>,
    mut to_sign: Vec<u8>,
  ) -> impl std::future::Future<Output = std::result::Result<Vec<u8>, Self::Error>> + Send {
    let signer = self.0.clone();
    async move {
      let sig = signer.sign(&to_sign).map_err(SignError::Sign)?;
      // As the agent's signer appends it: the signature as an SSH string.
      to_sign.extend_from_slice(&(sig.len() as u32).to_be_bytes());
      to_sign.extend_from_slice(&sig);
      Ok(to_sign)
    }
  }
}

#[derive(Debug)]
pub enum SignError {
  Send,
  Sign(String),
}

impl From<russh::SendError> for SignError {
  fn from(_: russh::SendError) -> Self {
    SignError::Send
  }
}

async fn authenticate(handle: &mut Handle<Client>, t: &Target) -> Result<()> {
  let mut tried: Vec<String> = Vec::new();
  let rsa_hash = handle
    .best_supported_rsa_hash()
    .await
    .ok()
    .flatten()
    .flatten();
  for auth in &t.auth {
    let ok = match auth {
      Auth::Password(p) => handle
        .authenticate_password(&t.user, p)
        .await
        .is_ok_and(|r| r.success()),
      Auth::KeyFile(path) => match key_file(path) {
        Some(key) => {
          let hash = rsa_hash.filter(|_| key.algorithm().is_rsa());
          handle
            .authenticate_publickey(&t.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .is_ok_and(|r| r.success())
        }
        None => continue,
      },
      Auth::Key(key) => {
        let hash = rsa_hash.filter(|_| key.algorithm().is_rsa());
        handle
          .authenticate_publickey(&t.user, PrivateKeyWithHashAlg::new(key.clone(), hash))
          .await
          .is_ok_and(|r| r.success())
      }
      Auth::Signer(signer) => {
        let mut s = Signing(signer.clone());
        handle
          .authenticate_publickey_with(&t.user, signer.public_key(), None, &mut s)
          .await
          .is_ok_and(|r| r.success())
      }
      Auth::Agent => {
        let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_env().await else {
          continue;
        };
        let ids = agent.request_identities().await.unwrap_or_default();
        let mut ok = false;
        for id in ids {
          let key = id.public_key().into_owned();
          let hash = rsa_hash.filter(|_| key.algorithm().is_rsa());
          if handle
            .authenticate_publickey_with(&t.user, key, hash, &mut agent)
            .await
            .is_ok_and(|r| r.success())
          {
            ok = true;
            break;
          }
        }
        ok
      }
    };
    if ok {
      return Ok(());
    }
    tried.push(auth.said());
  }
  Err(Error::Auth {
    host: t.name.clone(),
    user: t.user.clone(),
    tried,
  })
}
