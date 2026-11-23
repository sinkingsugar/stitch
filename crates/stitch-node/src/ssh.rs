//! SSH on a node (design §2.20): hosts, host keys and this device's key,
//! and shell sessions on hosts for the shell tools (`stitch_ssh`).
//!
//! - **Hosts** are `ssh-host` records in the home workspace (`name`,
//!   `address`, `port`, `user`, `jump`, `auth`, `approval`, `tmux`;
//!   `removed = true`), the latest of a name standing. A desktop node
//!   also reads `~/.ssh/config`: its hosts are listed, and fill what a
//!   record of the same name leaves out.
//! - **Host keys** are `ssh-host-key` records (`host` as `[address]:port`,
//!   `name`, `key`, `fingerprint`; `removed = true` forgets the keys of a
//!   host): trusted on first use by any node, so every node knows them; a
//!   changed key fails. A desktop also checks the user's `known_hosts`.
//! - **This device's key:** on an iPhone a Secure Enclave P-256 key; else
//!   an Ed25519 key kept as a secret is (`ssh-key@stitch`, never synced).
//!   Its public half is an `ssh-device-key` record (`node`, `key`), so
//!   another node can authorize it.
//! - **Authorizing** a key on a host: done here when this node reaches the
//!   host; else an `ssh-authorize` request record (`host`, `key`), which
//!   the nodes that reach the host take after a sync, answering with an
//!   `ssh-authorized` record (`request`, `ok`, `said`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde_json::Value;
use stitch_records::{Author, Draft};
use stitch_ssh::{Auth, HostKeys, PrivateKey, PublicKey, Target};
use stitch_store::IndexedRecord;

use crate::api::{Reply, SshAuthorization, SshHost, SshHostKey, SshOp};
use crate::node::Shared;
use crate::rules::HostApproval;
use crate::{Error, Result};

pub const HOST: &str = "ssh-host";
pub const HOST_KEY: &str = "ssh-host-key";
pub const DEVICE_KEY: &str = "ssh-device-key";
pub const AUTHORIZE: &str = "ssh-authorize";
pub const AUTHORIZED: &str = "ssh-authorized";

/// Where this device's software key is kept (`@` is in no secret's name).
const KEY_ENTRY: &str = "ssh-key@stitch";

/// How long authorizing a key may take.
const AUTHORIZING: Duration = Duration::from_secs(30);

fn str_of<'a>(r: &'a IndexedRecord, k: &str) -> Option<&'a str> {
  r.fields.get(k).and_then(Value::as_str)
}

fn removed(r: &IndexedRecord) -> bool {
  r.fields.get("removed").and_then(Value::as_bool) == Some(true)
}

/// A host name: letters, digits, `.`, `-`, `_` (at most 64).
pub fn check_name(name: &str) -> Result<()> {
  let ok = !name.is_empty()
    && name.len() <= 64
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
  if ok {
    Ok(())
  } else {
    Err(Error::Config(format!(
      "`{name}`: a host's name is letters, digits, `.`, `-` and `_`"
    )))
  }
}

fn host_of_record(r: &IndexedRecord) -> Option<SshHost> {
  Some(SshHost {
    name: str_of(r, "name")?.to_owned(),
    address: str_of(r, "address").unwrap_or("").to_owned(),
    port: r
      .fields
      .get("port")
      .and_then(Value::as_u64)
      .and_then(|p| u16::try_from(p).ok())
      .unwrap_or(0),
    user: str_of(r, "user").unwrap_or("").to_owned(),
    jump: str_of(r, "jump").unwrap_or("").to_owned(),
    auth: str_of(r, "auth").unwrap_or("").to_owned(),
    approval: str_of(r, "approval").unwrap_or("").to_owned(),
    no_tmux: r.fields.get("tmux").and_then(Value::as_bool) == Some(false),
    from: "record".into(),
  })
}

/// The `~/.ssh/config` of this node's user (none on a phone).
fn ssh_config() -> Option<stitch_ssh::config::SshConfig> {
  if !stitch_shell::CAN_SPAWN {
    return None;
  }
  let home = std::env::var_os("HOME")?;
  Some(stitch_ssh::config::SshConfig::load(&PathBuf::from(home)))
}

/// The hosts: the standing records, then `~/.ssh/config`'s that no record
/// names; by name.
pub(crate) fn hosts(s: &Shared) -> Vec<SshHost> {
  let mut out: BTreeMap<String, SshHost> = BTreeMap::new();
  {
    let index = s.state().index.lock().expect("index lock");
    for r in index.records(Some(HOST)).unwrap_or_default() {
      let Some(h) = host_of_record(&r) else {
        continue;
      };
      if removed(&r) {
        out.remove(&h.name);
      } else {
        out.insert(h.name.clone(), h);
      }
    }
  }
  if let Some(cfg) = ssh_config() {
    for name in cfg.hosts() {
      if out.contains_key(&name) {
        continue;
      }
      let Some(e) = cfg.entry(&name) else { continue };
      // Git forges give no shell.
      let forge = |h: &str| {
        [
          "github.com",
          "gitlab.com",
          "bitbucket.org",
          "ssh.dev.azure.com",
          "codeberg.org",
        ]
        .iter()
        .any(|f| h == *f || h.ends_with(&format!(".{f}")))
      };
      if e.user.as_deref() == Some("git")
        || forge(&name)
        || e.host_name.as_deref().is_some_and(forge)
      {
        continue;
      }
      out.insert(
        name.clone(),
        SshHost {
          name,
          address: e.host_name.unwrap_or_default(),
          port: e.port.unwrap_or(0),
          user: e.user.unwrap_or_default(),
          jump: e.proxy_jump.unwrap_or_default(),
          from: "ssh-config".into(),
          ..SshHost::default()
        },
      );
    }
  }
  out.into_values().collect()
}

pub(crate) fn host(s: &Shared, name: &str) -> Option<SshHost> {
  hosts(s).into_iter().find(|h| h.name == name)
}

/// The approval a host's commands get.
pub(crate) fn approval_of(h: &SshHost) -> HostApproval {
  HostApproval::parse(&h.approval).unwrap_or_default()
}

/// What `name` resolves to, its jump hosts and its ways to authenticate.
pub(crate) fn target(s: &Shared, name: &str) -> Result<Target> {
  target_at(s, name, 0)
}

fn target_at(s: &Shared, name: &str, depth: u32) -> Result<Target> {
  if depth > 4 {
    return Err(Error::Config(format!(
      "{name}: jump hosts go round (more than four deep)"
    )));
  }
  let Some(mut h) = host(s, name) else {
    let known: Vec<String> = hosts(s).into_iter().map(|h| h.name).collect();
    return Err(Error::Config(format!(
      "no SSH host `{name}`{}; add one with `stitch ssh host add` (or Settings → SSH Hosts)",
      if known.is_empty() {
        String::new()
      } else {
        format!(" (known: {})", known.join(", "))
      }
    )));
  };
  // What the config says fills what the record leaves out.
  let entry = ssh_config().and_then(|c| c.entry(name));
  if let Some(e) = &entry {
    if h.address.is_empty() {
      h.address = e.host_name.clone().unwrap_or_default();
    }
    if h.port == 0 {
      h.port = e.port.unwrap_or(0);
    }
    if h.user.is_empty() {
      h.user = e.user.clone().unwrap_or_default();
    }
    if h.jump.is_empty() {
      h.jump = e.proxy_jump.clone().unwrap_or_default();
    }
  }
  if h.address.is_empty() {
    h.address = h.name.clone();
  }
  if h.user.is_empty() {
    h.user = std::env::var("USER").unwrap_or_default();
  }
  if h.user.is_empty() {
    return Err(Error::Config(format!(
      "SSH host `{name}` has no user: set one (`stitch ssh host add {name} --user …`)"
    )));
  }
  let jump = match h.jump.split(',').next().map(str::trim) {
    Some(j) if !j.is_empty() => {
      // `user@host:port` as ProxyJump writes it, or a host's name.
      let j_name = j
        .rsplit_once('@')
        .map_or(j, |(_, h)| h)
        .split(':')
        .next()
        .unwrap_or(j);
      Some(Box::new(target_at(s, j_name, depth + 1)?))
    }
    _ => None,
  };
  let identity = entry.map(|e| e.identity_files).unwrap_or_default();
  Ok(Target {
    name: h.name.clone(),
    address: h.address.clone(),
    port: if h.port == 0 { 22 } else { h.port },
    user: h.user.clone(),
    jump,
    auth: auth_for(s, &h, identity)?,
  })
}

/// The ways to authenticate to a host, in order.
fn auth_for(s: &Shared, h: &SshHost, identity: Vec<PathBuf>) -> Result<Vec<Auth>> {
  let device = || -> Result<Auth> { device_key(s).map(|k| k.auth()) };
  let files = || -> Vec<Auth> {
    let mut out: Vec<Auth> = identity.iter().cloned().map(Auth::KeyFile).collect();
    if let Some(home) = std::env::var_os("HOME") {
      for f in ["id_ed25519", "id_ecdsa", "id_rsa"] {
        let p = PathBuf::from(&home).join(".ssh").join(f);
        if !identity.contains(&p) && p.exists() {
          out.push(Auth::KeyFile(p));
        }
      }
    }
    out
  };
  let auth = h.auth.trim();
  Ok(match auth {
    "" | "auto" => {
      let mut out = Vec::new();
      if stitch_shell::CAN_SPAWN {
        out.push(Auth::Agent);
        out.extend(files());
      }
      out.extend(device().ok());
      out
    }
    "agent" => vec![Auth::Agent],
    "device" => vec![device()?],
    a if a.starts_with("secret:") => {
      let name = &a["secret:".len()..];
      match s.vault.secret("global", name) {
        Some(pw) => vec![Auth::Password(pw)],
        None => {
          return Err(Error::Config(format!(
            "SSH host `{}` authenticates with the secret `{name}`, which this node does not hold",
            h.name
          )));
        }
      }
    }
    a if a.starts_with("key:") => vec![Auth::KeyFile(crate::place::expand_home(&a[4..]))],
    other => {
      return Err(Error::Config(format!(
        "SSH host `{}`: auth `{other}` is not auto, agent, device, secret:<name> or key:<path>",
        h.name
      )));
    }
  })
}

// ---------------------------------------------------------------------
// Host keys.

/// The host keys trusted, from the records: by host, the latest records
/// standing (a removal forgets what came before it).
fn host_keys(s: &Shared) -> Vec<SshHostKey> {
  let index = s.state().index.lock().expect("index lock");
  let mut out: Vec<SshHostKey> = Vec::new();
  for r in index.records(Some(HOST_KEY)).unwrap_or_default() {
    let Some(host) = str_of(&r, "host") else {
      continue;
    };
    if removed(&r) {
      out.retain(|k| k.host != host);
      continue;
    }
    let Some(key) = str_of(&r, "key") else {
      continue;
    };
    if out.iter().any(|k| k.host == host && k.key == key) {
      continue;
    }
    out.push(SshHostKey {
      host: host.to_owned(),
      name: str_of(&r, "name").unwrap_or("").to_owned(),
      fingerprint: str_of(&r, "fingerprint").unwrap_or("").to_owned(),
      key: key.to_owned(),
      trusted: r.created.to_string(),
      node: r.node.clone(),
    });
  }
  out
}

/// `HostKeys` over the records (in memory too, so a key trusted is known
/// before its record is indexed, and where there is no home workspace).
pub(crate) struct RecordHostKeys {
  shared: Weak<Shared>,
  memory: stitch_ssh::MemoryHostKeys,
}

impl HostKeys for RecordHostKeys {
  fn known(&self, host: &str) -> Vec<PublicKey> {
    let mut out = self.memory.known(host);
    if let Some(s) = self.shared.upgrade() {
      for k in host_keys(&s).into_iter().filter(|k| k.host == host) {
        if let Ok(key) = stitch_ssh::device::parse_public(&k.key)
          && !out.contains(&key)
        {
          out.push(key);
        }
      }
    }
    out
  }

  fn trust(&self, host: &str, name: &str, key: &PublicKey) {
    self.memory.trust(host, name, key);
    let Some(s) = self.shared.upgrade() else {
      return;
    };
    let state = s.state();
    let Some(home) = state.home() else { return };
    let fingerprint = stitch_ssh::client::fingerprint(key);
    let draft = Draft::new(HOST_KEY, Author::node(state.node.clone())).and_then(|d| {
      d.field("host", host)?
        .field("name", name)?
        .field("key", stitch_ssh::device::authorized_line(key).as_str())?
        .field("fingerprint", fingerprint.as_str())
        .map(|d| {
          d.body(format!(
            "# Host key of {name} ({host})\n\n{fingerprint}, trusted on first use.\n"
          ))
        })
    });
    match draft
      .map_err(Error::from)
      .and_then(|d| Ok(home.publish(&state.node, d)?))
    {
      Ok(_) => {
        let _ = state.index.lock().expect("index lock").refresh(&home);
        let s2 = s.clone();
        tokio::spawn(async move { crate::node::sync_one(&s2, home.repo()).await });
      }
      Err(e) => eprintln!("ssh: keeping the host key of {name}: {e}"),
    }
  }
}

// ---------------------------------------------------------------------
// This device's key.

#[derive(Clone)]
#[cfg_attr(not(any(target_os = "ios", target_os = "visionos")), allow(dead_code))]
pub(crate) enum DeviceKey {
  Soft(Arc<PrivateKey>),
  Signer(Arc<dyn stitch_ssh::DeviceSigner>),
}

impl DeviceKey {
  pub(crate) fn public(&self) -> PublicKey {
    match self {
      DeviceKey::Soft(k) => k.public_key().clone(),
      DeviceKey::Signer(s) => s.public_key(),
    }
  }

  pub(crate) fn kind(&self) -> &'static str {
    match self {
      DeviceKey::Soft(_) => "keychain",
      DeviceKey::Signer(_) => "secure-enclave",
    }
  }

  fn auth(&self) -> Auth {
    match self {
      DeviceKey::Soft(k) => Auth::Key(k.clone()),
      DeviceKey::Signer(s) => Auth::Signer(s.clone()),
    }
  }
}

/// This device's key, made the first time (and its public half published).
pub(crate) fn device_key(s: &Shared) -> Result<DeviceKey> {
  let got = s
    .ssh
    .device
    .get_or_init(|| make_device_key(s).map_err(|e| e.to_string()))
    .clone()
    .map_err(Error::Config)?;
  publish_device_key(s, &got);
  Ok(got)
}

fn make_device_key(s: &Shared) -> Result<DeviceKey> {
  let comment = format!("stitch@{}", s.state().node);
  #[cfg(any(target_os = "ios", target_os = "visionos"))]
  match stitch_ssh::device::enclave::EnclaveKey::load_or_make(&comment) {
    Ok(k) => return Ok(DeviceKey::Signer(Arc::new(k))),
    // The simulator has no Secure Enclave: a key in the keychain.
    Err(e) => eprintln!("ssh: no Secure Enclave key ({e}); a key in the keychain instead"),
  }
  if let Some(text) = crate::keys::stored(&s.data, KEY_ENTRY)
    && let Ok(k) = stitch_ssh::device::from_openssh(&text)
  {
    return Ok(DeviceKey::Soft(Arc::new(k)));
  }
  let k = stitch_ssh::device::software_key(&comment).map_err(|e| Error::Config(e.to_string()))?;
  let text = stitch_ssh::device::to_openssh(&k).map_err(|e| Error::Config(e.to_string()))?;
  crate::keys::set(&s.data, KEY_ENTRY, &text, crate::keys::store_for(&s.data))?;
  Ok(DeviceKey::Soft(Arc::new(k)))
}

/// The keys the nodes published, by node: the latest of each.
fn device_keys(s: &Shared) -> BTreeMap<String, String> {
  let index = s.state().index.lock().expect("index lock");
  let mut out = BTreeMap::new();
  for r in index.records(Some(DEVICE_KEY)).unwrap_or_default() {
    if let (Some(n), Some(k)) = (str_of(&r, "node"), str_of(&r, "key")) {
      out.insert(n.to_owned(), k.to_owned());
    }
  }
  out
}

fn publish_device_key(s: &Shared, key: &DeviceKey) {
  let state = s.state();
  let line = stitch_ssh::device::authorized_line(&key.public());
  if device_keys(s).get(state.node.as_str()) == Some(&line) {
    return;
  }
  let Some(home) = state.home() else { return };
  let draft = Draft::new(DEVICE_KEY, Author::node(state.node.clone())).and_then(|d| {
    d.field("node", state.node.as_str())?
      .field("key", line.as_str())?
      .field("held", key.kind())
      .map(|d| {
        d.body(format!(
          "# {}'s SSH key\n\nWhat a host's `authorized_keys` takes to let it in.\n",
          state.node
        ))
      })
  });
  match draft
    .map_err(Error::from)
    .and_then(|d| Ok(home.publish(&state.node, d)?))
  {
    Ok(_) => {
      let _ = state.index.lock().expect("index lock").refresh(&home);
    }
    Err(e) => eprintln!("ssh: publishing this device's key: {e}"),
  }
}

// ---------------------------------------------------------------------
// Sessions.

/// The node's SSH: connections, sessions' far ends, this device's key.
#[derive(Default)]
pub(crate) struct Ssh {
  pool: stitch_ssh::Pool,
  /// By session key: the far end of each remote session.
  links: Mutex<HashMap<String, Arc<stitch_ssh::Link>>>,
  /// Hosts whose lack of tmux was said.
  plain_said: Mutex<HashSet<String>>,
  device: OnceLock<std::result::Result<DeviceKey, String>>,
  keys: OnceLock<Arc<RecordHostKeys>>,
}

impl Ssh {
  fn keys(&self, s: &Arc<Shared>) -> Arc<RecordHostKeys> {
    self
      .keys
      .get_or_init(|| {
        Arc::new(RecordHostKeys {
          shared: Arc::downgrade(s),
          memory: stitch_ssh::MemoryHostKeys::default(),
        })
      })
      .clone()
  }

  /// Whether the remote session of `key` ended because its connection
  /// dropped (so it is made again when next used).
  pub(crate) fn dropped(&self, key: &str) -> bool {
    self
      .links
      .lock()
      .expect("links lock")
      .get(key)
      .is_some_and(|l| l.dropped())
  }

  pub(crate) fn forget(&self, key: &str) {
    self.links.lock().expect("links lock").remove(key);
  }

  pub(crate) async fn close_all(&self) {
    self.pool.close_all().await;
  }
}

fn known_hosts_file() -> Option<PathBuf> {
  if !stitch_shell::CAN_SPAWN {
    return None;
  }
  let p = PathBuf::from(std::env::var_os("HOME")?).join(".ssh/known_hosts");
  p.exists().then_some(p)
}

/// What a session on a host is opened with.
pub(crate) struct Open<'a> {
  pub thread: &'a str,
  pub host: &'a str,
  pub session: &'a str,
  /// The session's key (`jobs::session_key`).
  pub key: &'a str,
  pub redact: Option<stitch_shell::Redact>,
}

/// Opens a session on a host (connecting when there is no connection, or
/// it dropped); what its result says of it, if anything.
pub(crate) async fn open(
  s: &Arc<Shared>,
  o: Open<'_>,
) -> Result<(stitch_shell::Session, Option<String>)> {
  let h = host(s, o.host).ok_or_else(|| Error::Config(format!("no SSH host `{}`", o.host)))?;
  let target = {
    let s = s.clone();
    let name = o.host.to_owned();
    // Reading the device key may touch the keychain.
    tokio::task::spawn_blocking(move || target(&s, &name))
      .await
      .map_err(|e| Error::Config(e.to_string()))??
  };
  let reconnecting = s.ssh.dropped(o.key);
  let (conn, _) = s
    .ssh
    .pool
    .get(&target, s.ssh.keys(s), known_hosts_file())
    .await
    .map_err(|e| Error::Config(e.to_string()))?;
  let opened = stitch_ssh::open(
    conn,
    stitch_ssh::ShellOptions {
      name: stitch_ssh::session_name(o.thread, o.session),
      rows: 40,
      cols: 120,
      max_output: 30_000,
      redact: o.redact,
      tmux: !h.no_tmux,
    },
  )
  .await
  .map_err(|e| Error::Config(e.to_string()))?;
  let link = opened.link.clone();
  s.ssh
    .links
    .lock()
    .expect("links lock")
    .insert(o.key.to_owned(), link.clone());
  let mut said = Vec::new();
  if reconnecting {
    said.push(if link.resumed() {
      format!(
        "The connection to {} had dropped; reconnected: the same tmux session, picked up where it was.",
        o.host
      )
    } else if link.in_tmux() {
      format!(
        "The connection to {} had dropped; reconnected, but its tmux session was gone: a new shell (fresh directory and environment).",
        o.host
      )
    } else {
      format!(
        "The connection to {} had dropped; reconnected: a new shell (without tmux the old one ended with the connection).",
        o.host
      )
    });
  } else if link.resumed() {
    said.push(format!(
      "Picked up session \"{}\" on {} where it was (its tmux session was still there).",
      o.session, o.host
    ));
  }
  if !link.in_tmux()
    && !h.no_tmux
    && s
      .ssh
      .plain_said
      .lock()
      .expect("said lock")
      .insert(h.name.clone())
  {
    said.push(format!(
      "{} has no tmux: sessions there are plain shells, which end if the connection drops.",
      o.host
    ));
  }
  let note = (!said.is_empty()).then(|| format!("({})", said.join(" ")));
  Ok((opened.session, note))
}

// ---------------------------------------------------------------------
// Authorizing a key on a host.

/// The line that adds `key` to `authorized_keys` there, once.
fn authorize_script(line: &str) -> String {
  let q = stitch_ssh::tmux::quote(line);
  format!(
    "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && chmod 700 ~/.ssh && \
     chmod 600 ~/.ssh/authorized_keys && (grep -qxF {q} ~/.ssh/authorized_keys || \
     printf '%s\\n' {q} >> ~/.ssh/authorized_keys) && echo stitch:authorized"
  )
}

/// Adds `line` to `host`'s `authorized_keys`, from this node.
async fn authorize_here(s: &Arc<Shared>, host: &str, line: &str) -> Result<String> {
  let target = {
    let s = s.clone();
    let name = host.to_owned();
    tokio::task::spawn_blocking(move || target(&s, &name))
      .await
      .map_err(|e| Error::Config(e.to_string()))??
  };
  let (conn, _) = s
    .ssh
    .pool
    .get(&target, s.ssh.keys(s), known_hosts_file())
    .await
    .map_err(|e| Error::Config(e.to_string()))?;
  let out = conn
    .script(&authorize_script(line), AUTHORIZING)
    .await
    .map_err(|e| Error::Config(e.to_string()))?;
  if out.stdout_text().contains("stitch:authorized") {
    Ok(format!(
      "{} added to {}@{}'s authorized_keys (by {})",
      key_said(line),
      target.user,
      host,
      s.state().node
    ))
  } else {
    Err(Error::Config(format!(
      "adding the key on {host} failed: {}",
      String::from_utf8_lossy(&out.stderr).trim()
    )))
  }
}

fn key_said(line: &str) -> String {
  match stitch_ssh::device::parse_public(line) {
    Ok(k) => {
      let c = k.comment();
      if c.is_empty() {
        stitch_ssh::client::fingerprint(&k)
      } else {
        format!("{c} ({})", stitch_ssh::client::fingerprint(&k))
      }
    }
    Err(_) => "the key".into(),
  }
}

/// The requests and their answers.
fn authorizations(s: &Shared) -> Vec<SshAuthorization> {
  let index = s.state().index.lock().expect("index lock");
  let mut out: Vec<SshAuthorization> = index
    .records(Some(AUTHORIZE))
    .unwrap_or_default()
    .into_iter()
    .filter_map(|r| {
      Some(SshAuthorization {
        id: r.id.to_string(),
        host: str_of(&r, "host")?.to_owned(),
        key: key_said(str_of(&r, "key")?),
        asked: format!("{} by {}", r.created, r.node),
        state: "pending".into(),
        said: Vec::new(),
      })
    })
    .collect();
  for r in index.records(Some(AUTHORIZED)).unwrap_or_default() {
    let Some(a) = str_of(&r, "request").and_then(|id| out.iter_mut().find(|a| a.id == id)) else {
      continue;
    };
    let ok = r.fields.get("ok").and_then(Value::as_bool) == Some(true);
    if ok {
      a.state = "done".into();
    } else if a.state == "pending" {
      a.state = "failed".into();
    }
    a.said
      .push(format!("{}: {}", r.node, str_of(&r, "said").unwrap_or("")));
  }
  out
}

fn publish(s: &Shared, draft: std::result::Result<Draft, stitch_records::Error>) -> Result<String> {
  let state = s.state();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here".into()))?;
  let r = home.publish(&state.node, draft?)?;
  let _ = state.index.lock().expect("index lock").refresh(&home);
  Ok(r.id.to_string())
}

fn sync_soon(s: &Arc<Shared>) {
  if let Some(home) = s.state().home() {
    let s = s.clone();
    tokio::spawn(async move {
      crate::node::sync_one(&s, home.repo()).await;
    });
  }
}

/// After a sync: the requests this node can do, done (each tried once by
/// each node).
pub(crate) async fn after_sync(s: &Arc<Shared>) {
  if !stitch_shell::CAN_SPAWN {
    return;
  }
  let me = s.state().node.to_string();
  let pending: Vec<(String, String, String)> = {
    let index = s.state().index.lock().expect("index lock");
    let tried: HashSet<String> = index
      .records(Some(AUTHORIZED))
      .unwrap_or_default()
      .into_iter()
      .filter(|r| r.node == me || r.fields.get("ok").and_then(Value::as_bool) == Some(true))
      .filter_map(|r| str_of(&r, "request").map(str::to_owned))
      .collect();
    index
      .records(Some(AUTHORIZE))
      .unwrap_or_default()
      .into_iter()
      .filter(|r| !tried.contains(&r.id.to_string()) && r.node != me)
      .filter_map(|r| {
        Some((
          r.id.to_string(),
          str_of(&r, "host")?.to_owned(),
          str_of(&r, "key")?.to_owned(),
        ))
      })
      .collect()
  };
  let mut wrote = false;
  for (id, host_name, key) in pending {
    if host(s, &host_name).is_none() {
      continue;
    }
    let (ok, said) = match authorize_here(s, &host_name, &key).await {
      Ok(said) => (true, said),
      Err(e) => (false, e.to_string()),
    };
    let draft = Draft::new(AUTHORIZED, Author::node(s.state().node.clone())).and_then(|d| {
      d.field("request", id.as_str())?
        .field("ok", ok)?
        .field("said", said.as_str())
        .map(|d| d.body(format!("# Key authorized on {host_name}?\n\n{said}\n")))
    });
    match publish(s, draft) {
      Ok(_) => wrote = true,
      Err(e) => eprintln!("ssh: answering an authorization: {e}"),
    }
  }
  if wrote {
    sync_soon(s);
  }
}

// ---------------------------------------------------------------------
// What the user asks (`stitch ssh`, the app).

pub(crate) async fn request(s: &Arc<Shared>, op: SshOp) -> Result<Reply> {
  match op {
    SshOp::Hosts => Ok(Reply::SshHosts(hosts(s))),
    SshOp::HostSet(h) => {
      check_name(&h.name)?;
      if !h.jump.is_empty() {
        check_name(&h.jump)?;
      }
      if HostApproval::parse(&h.approval).is_none() {
        return Err(Error::Config(format!(
          "approval `{}`: relaxed, ask or allow",
          h.approval
        )));
      }
      let mut d = Draft::new(HOST, Author::user(s.state().node.clone()))?
        .field("name", h.name.as_str())?
        .field("address", h.address.as_str())?
        .field("port", i64::from(h.port))?
        .field("user", h.user.as_str())?
        .field("jump", h.jump.as_str())?
        .field("auth", h.auth.as_str())?
        .field("approval", h.approval.as_str())?;
      if h.no_tmux {
        d = d.field("tmux", false)?;
      }
      let d = d.body(format!("# SSH host {}\n", h.name));
      publish(s, Ok(d))?;
      sync_soon(s);
      Ok(Reply::Ok)
    }
    SshOp::HostRemove { name } => {
      let exists = hosts(s)
        .iter()
        .any(|h| h.name == name && h.from == "record");
      if !exists {
        return Err(Error::Config(format!("no SSH host record `{name}`")));
      }
      let d = Draft::new(HOST, Author::user(s.state().node.clone()))?
        .field("name", name.as_str())?
        .field("removed", true)?
        .body(format!("# SSH host {name} removed\n"));
      publish(s, Ok(d))?;
      sync_soon(s);
      Ok(Reply::Ok)
    }
    SshOp::HostKeys => Ok(Reply::SshHostKeys(host_keys(s))),
    SshOp::HostKeyRemove { host: which } => {
      // A host's name stands for where it is.
      let at = match target(s, &which) {
        Ok(t) => t.key_host(),
        Err(_) => which.clone(),
      };
      if !host_keys(s).iter().any(|k| k.host == at) {
        return Err(Error::Config(format!("no host key kept for {at}")));
      }
      let d = Draft::new(HOST_KEY, Author::user(s.state().node.clone()))?
        .field("host", at.as_str())?
        .field("removed", true)?
        .body(format!("# Host keys of {at} forgotten\n"));
      publish(s, Ok(d))?;
      if let Some(k) = s.ssh.keys.get() {
        k.memory
          .0
          .lock()
          .expect("keys lock")
          .retain(|(h, _)| *h != at);
      }
      sync_soon(s);
      Ok(Reply::Lines(vec![format!(
        "forgot the host keys of {at}: the next connection trusts the key it finds"
      )]))
    }
    SshOp::Key => {
      let s2 = s.clone();
      let key = tokio::task::spawn_blocking(move || device_key(&s2))
        .await
        .map_err(|e| Error::Config(e.to_string()))??;
      let public = key.public();
      Ok(Reply::SshKey {
        line: stitch_ssh::device::authorized_line(&public),
        fingerprint: stitch_ssh::client::fingerprint(&public),
        kind: key.kind().into(),
      })
    }
    SshOp::Authorize {
      host: name,
      key,
      node,
    } => {
      let line = match (key, node) {
        (Some(k), _) => {
          stitch_ssh::device::parse_public(&k).map_err(|e| Error::Config(e.to_string()))?;
          k.trim().to_owned()
        }
        (None, Some(n)) if n != s.state().node.as_str() => device_keys(s)
          .get(&n)
          .cloned()
          .ok_or_else(|| Error::Config(format!("{n} has published no SSH key yet")))?,
        _ => {
          let s2 = s.clone();
          let k = tokio::task::spawn_blocking(move || device_key(&s2))
            .await
            .map_err(|e| Error::Config(e.to_string()))??;
          stitch_ssh::device::authorized_line(&k.public())
        }
      };
      if host(s, &name).is_none() {
        return Err(Error::Config(format!("no SSH host `{name}`")));
      }
      // Here, when this node gets in; else asked of those that do.
      let here = authorize_here(s, &name, &line).await;
      match here {
        Ok(said) => Ok(Reply::Lines(vec![said])),
        Err(e) => {
          let d = Draft::new(AUTHORIZE, Author::user(s.state().node.clone()))?
            .field("host", name.as_str())?
            .field("key", line.as_str())?
            .body(format!(
              "# Authorize a key on {name}\n\n{}, asked of the nodes that reach {name}.\n",
              key_said(&line)
            ));
          publish(s, Ok(d))?;
          sync_soon(s);
          Ok(Reply::Lines(vec![format!(
            "this node could not do it ({e}); asked the nodes that reach {name} (see `stitch ssh authorizations`)"
          )]))
        }
      }
    }
    SshOp::Authorizations => Ok(Reply::SshAuthorizations(authorizations(s))),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_are_checked() {
    assert!(check_name("devbox-1").is_ok());
    assert!(check_name("pi.lan").is_ok());
    assert!(check_name("a b").is_err());
    assert!(check_name("").is_err());
  }

  #[test]
  fn the_authorize_script_adds_a_key_once() {
    let line = "ssh-ed25519 AAAA stitch@iphone";
    let script = authorize_script(line);
    assert!(script.contains("grep -qxF 'ssh-ed25519 AAAA stitch@iphone'"));
    assert!(script.contains(">> ~/.ssh/authorized_keys"));
  }
}
