//! Secrets and variables (design §2.19).
//!
//! **Secrets** are kept where provider keys always were (`keys`: the
//! keychain of this data directory, or `credentials.toml`): a global
//! secret under its bare name, so a provider's key *is* the secret of the
//! provider's name (`openai`, `github`), and one of a place under
//! `<scope>/<name>` (`project:p-…/openai`). Which secrets this node has,
//! and where each came from, is `<data>/secrets.toml`: names and scopes,
//! never values. Keys stored before secrets existed are this device's.
//!
//! **Synced sealed:** unless it is this device's only, setting a secret
//! writes a `secret` record in the home workspace: its name, scope, when
//! its value was set, and the value sealed for each trusted node (this one
//! included) with that node's X25519 key, as pushes are sealed
//! (`apns::seal_with`, info `stitch secret v1`). A node's key is made the
//! first time it has a secret to sync or sees one; its private half is
//! kept as a secret is, its public half published as a `seal-key` record.
//! After a sync each node opens what was sealed for it and keeps the value
//! (`reconcile`); a node that holds the current value re-seals it for a
//! trusted node the record left out (paired later). The latest record of
//! a name and scope stands (by when its value was set); `removed = true`
//! removes it everywhere. Nodes removed since a record was sealed for
//! them are listed as a reason to rotate the secret, until it is set anew.
//!
//! **Variables** are `variable` records in the home workspace (name,
//! scope, value), the latest of a name and scope standing.
//!
//! **Used, never shown:** shells get the variables of their place (and the
//! global ones) by name, then what `[env]` (config.toml, then the place's
//! settings) maps: `OPENAI_API_KEY = "secret:openai"`. Every value this
//! node holds is replaced by `«secret:<name>»` in tool output before a
//! model or a record sees it (`Vault::redact`, and `StreamRedactor` for
//! output read in chunks).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, Draft, NodeName};
use stitch_store::{Index, IndexedRecord, RecordRoot};

use crate::api::{SecretInfo, VarInfo};
use crate::config::Config;
use crate::keys::{self, Store};
use crate::place::Place;
use crate::{Error, Result};

/// The record kinds.
pub const SECRET: &str = "secret";
pub const VARIABLE: &str = "variable";
pub const SEAL_KEY: &str = "seal-key";

/// HKDF's info for a secret's key: what is sealed for a push never opens
/// as a secret, nor the other way round.
const INFO: &[u8] = b"stitch secret v1";

/// This node's list of its secrets.
const LOCAL: &str = "secrets.toml";

/// Where this node's sealing key is kept (`@` is in no secret's name).
const SEAL_ENTRY: &str = "seal-key@stitch";

/// Values shorter than this are not redacted: they would match everywhere.
pub const MIN_REDACTED: usize = 6;

/// Names looked for among the stored keys when listing, so that keys
/// stored before secrets existed (or by `stitch key`, `stitch auth login
/// github`, `stitch apns set`) are listed as secrets.
const KNOWN: &[&str] = &[
  "anthropic",
  "openai",
  "gemini",
  "grok",
  "kimi",
  "deepseek",
  "openrouter",
  "brave",
  "perplexity",
  "github",
  "apns",
];

const B64: base64::engine::general_purpose::GeneralPurpose =
  base64::engine::general_purpose::STANDARD;

/// A secret's or variable's name: letters, digits, `_`, `-`, `.`, starting
/// with a letter or digit, at most 64.
pub fn check_name(name: &str) -> Result<()> {
  let ok = !name.is_empty()
    && name.len() <= 64
    && name
      .chars()
      .next()
      .is_some_and(|c| c.is_ascii_alphanumeric())
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
  if ok {
    Ok(())
  } else {
    Err(Error::Config(format!(
      "`{name}` is not a name for a secret or variable: letters, digits, `_`, `-` and `.`, \
       starting with a letter or digit"
    )))
  }
}

/// A scope as written (`global`, `project:<repo>`, `folder:<key>`), checked
/// and in its usual form.
pub fn check_scope(scope: &str) -> Result<String> {
  Place::of_key(scope).map(|p| p.key()).ok_or_else(|| {
    Error::Config(format!(
      "`{scope}` is not a scope: `global`, `project:<repo>` or `folder:<key>`"
    ))
  })
}

/// Where a secret's value is kept: a global one under its name (a
/// provider's key), a place's under `<scope>/<name>`.
fn entry(scope: &str, name: &str) -> String {
  if scope == "global" {
    name.to_owned()
  } else {
    format!("{scope}/{name}")
  }
}

fn now() -> jiff::Timestamp {
  stitch_records::millis(jiff::Timestamp::now())
}

// ---------------------------------------------------------------------
// This node's list.

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Local {
  #[serde(default, rename = "secret", skip_serializing_if = "Vec::is_empty")]
  secrets: Vec<LocalSecret>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LocalSecret {
  name: String,
  scope: String,
  #[serde(default)]
  device_only: bool,
  /// The `secret` record this value is (none: never synced).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  record: Option<String>,
  /// When the value was set (RFC 3339), and on which node.
  updated: String,
  from: String,
}

impl Local {
  fn load(data: &Path) -> Local {
    std::fs::read_to_string(data.join(LOCAL))
      .ok()
      .and_then(|t| toml::from_str(&t).ok())
      .unwrap_or_default()
  }

  fn save(&self, data: &Path) -> Result<()> {
    std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
    let path = data.join(LOCAL);
    let tmp = data.join("secrets.toml.tmp");
    let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp).map_err(Error::io(tmp.display()))?;
    f.write_all(text.as_bytes())
      .map_err(Error::io(tmp.display()))?;
    f.sync_all().map_err(Error::io(tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))
  }

  fn find(&self, scope: &str, name: &str) -> Option<&LocalSecret> {
    self
      .secrets
      .iter()
      .find(|s| s.scope == scope && s.name == name)
  }

  fn put(&mut self, s: LocalSecret) {
    self
      .secrets
      .retain(|e| !(e.scope == s.scope && e.name == s.name));
    self.secrets.push(s);
    self
      .secrets
      .sort_by(|a, b| (&a.scope, &a.name).cmp(&(&b.scope, &b.name)));
  }

  fn drop_one(&mut self, scope: &str, name: &str) {
    self
      .secrets
      .retain(|e| !(e.scope == scope && e.name == name));
  }
}

/// The names worth looking for among stored keys: the known ones, the
/// configured providers', MCP servers' keys, and `credentials.toml`'s.
fn candidates(data: &Path, config: &Config) -> BTreeSet<String> {
  let mut out: BTreeSet<String> = KNOWN.iter().map(|s| (*s).to_owned()).collect();
  out.extend(config.providers.keys().cloned());
  if let Some(mcp) = &config.mcp {
    for s in mcp.servers.values() {
      if let Some(k) = &s.key {
        out.insert(k.strip_prefix("secret:").unwrap_or(k).to_owned());
      }
    }
  }
  out.extend(
    keys::file_entries(data)
      .into_iter()
      .filter(|e| !e.contains('/') && !e.contains('@')),
  );
  out.retain(|n| check_name(n).is_ok());
  out
}

/// Keys stored here that the list does not have yet (stored before
/// secrets were, or by `stitch key`): listed as global secrets of this
/// device only, so a value another node holds under the same name is
/// never overwritten by them, nor they by it; sharing one is the user's
/// choice. Whether the list changed.
fn discover(data: &Path, config: &Config, node: &str, local: &mut Local) -> bool {
  let mut changed = false;
  for name in candidates(data, config) {
    if local.find("global", &name).is_some() {
      continue;
    }
    if keys::stored(data, &name).is_some() {
      local.put(LocalSecret {
        name,
        scope: "global".into(),
        device_only: true,
        record: None,
        updated: now().to_string(),
        from: node.to_owned(),
      });
      changed = true;
    }
  }
  changed
}

// ---------------------------------------------------------------------
// Sealing keys.

/// This node's sealing key's private half; made and kept when `create`.
fn seal_private(data: &Path, create: bool) -> Result<Option<[u8; 32]>> {
  if let Some(k) = keys::stored(data, SEAL_ENTRY)
    && let Ok(bytes) = B64.decode(k.trim())
    && let Ok(k) = <[u8; 32]>::try_from(bytes.as_slice())
  {
    return Ok(Some(k));
  }
  if !create {
    return Ok(None);
  }
  use ring::rand::SecureRandom;
  let mut k = [0u8; 32];
  ring::rand::SystemRandom::new()
    .fill(&mut k)
    .map_err(|_| Error::Config("no randomness for a sealing key".into()))?;
  keys::set(data, SEAL_ENTRY, &B64.encode(k), keys::store_for(data))?;
  Ok(Some(k))
}

/// The nodes' sealing keys (public, base64), as their latest `seal-key`
/// records say.
fn sealing_keys(index: &Index) -> BTreeMap<String, String> {
  let mut out = BTreeMap::new();
  for r in index.records(Some(SEAL_KEY)).unwrap_or_default() {
    if let Some(k) = r.fields.get("key").and_then(Value::as_str) {
      out.insert(r.node.clone(), k.to_owned());
    }
  }
  out
}

/// What is sealed: the value with its name and scope, so a ciphertext
/// moved to another record does not open as that one.
#[derive(Serialize, Deserialize)]
struct Sealed {
  name: String,
  scope: String,
  value: String,
}

fn open(private: &[u8; 32], wire: &str, name: &str, scope: &str) -> Option<String> {
  let bytes = crate::apns::open_with(INFO, private, wire)?;
  let s: Sealed = serde_json::from_slice(&bytes).ok()?;
  (s.name == name && s.scope == scope).then_some(s.value)
}

// ---------------------------------------------------------------------
// Records.

/// A `secret` record as read.
#[derive(Clone, Debug)]
struct SecretRecord {
  id: String,
  node: String,
  name: String,
  scope: String,
  set_at: jiff::Timestamp,
  created: jiff::Timestamp,
  removed: bool,
  sealed: BTreeMap<String, String>,
  rotate: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
  v.and_then(Value::as_array)
    .map(|a| {
      a.iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
    })
    .unwrap_or_default()
}

fn secret_record(r: &IndexedRecord) -> Option<SecretRecord> {
  let s = |k: &str| r.fields.get(k).and_then(Value::as_str);
  let created: jiff::Timestamp = r.created;
  Some(SecretRecord {
    id: r.id.to_string(),
    node: r.node.clone(),
    name: s("name")?.to_owned(),
    scope: s("scope")?.to_owned(),
    set_at: s("set_at").and_then(|t| t.parse().ok()).unwrap_or(created),
    created,
    removed: r.fields.get("removed").and_then(Value::as_bool) == Some(true),
    sealed: r
      .fields
      .get("sealed")
      .and_then(Value::as_object)
      .map(|m| {
        m.iter()
          .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
          .collect()
      })
      .unwrap_or_default(),
    rotate: strings(r.fields.get("rotate")),
  })
}

/// The standing record of each secret, by (scope, name): the one whose
/// value was set last.
fn latest_secrets(index: &Index) -> BTreeMap<(String, String), SecretRecord> {
  let mut out: BTreeMap<(String, String), SecretRecord> = BTreeMap::new();
  for r in index.records(Some(SECRET)).unwrap_or_default() {
    let Some(s) = secret_record(&r) else { continue };
    let key = (s.scope.clone(), s.name.clone());
    let newer = out
      .get(&key)
      .is_none_or(|o| (s.set_at, s.created, &s.id) > (o.set_at, o.created, &o.id));
    if newer {
      out.insert(key, s);
    }
  }
  out
}

/// A variable as its standing record says.
#[derive(Clone, Debug)]
struct VarRecord {
  name: String,
  scope: String,
  value: String,
  removed: bool,
  node: String,
  created: jiff::Timestamp,
}

fn latest_vars(index: &Index) -> BTreeMap<(String, String), VarRecord> {
  let mut out = BTreeMap::new();
  // Oldest first: a later record replaces an earlier.
  for r in index.records(Some(VARIABLE)).unwrap_or_default() {
    let s = |k: &str| r.fields.get(k).and_then(Value::as_str);
    let (Some(name), Some(scope)) = (s("name"), s("scope")) else {
      continue;
    };
    out.insert(
      (scope.to_owned(), name.to_owned()),
      VarRecord {
        name: name.to_owned(),
        scope: scope.to_owned(),
        value: s("value").unwrap_or("").to_owned(),
        removed: r.fields.get("removed").and_then(Value::as_bool) == Some(true),
        node: r.node.clone(),
        created: r.created,
      },
    );
  }
  out
}

// ---------------------------------------------------------------------
// The node's view: values held, for redaction and injection.

/// What this node holds, in memory: the secrets' values (for redaction
/// and injection) and the variables.
#[derive(Default)]
pub struct Vault {
  snap: RwLock<Snapshot>,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
  /// (scope, name, value).
  secrets: Vec<(String, String, String)>,
  vars: Vec<(String, String, String)>,
  /// What redaction replaces, longest value first: (value, label).
  redact: Arc<Vec<(String, String)>>,
}

impl Vault {
  /// Reads again what this node holds: its list's values, the keys of
  /// providers it uses from anywhere (the environment too), and the
  /// variables in `index`.
  pub fn reload(&self, data: &Path, config: &Config, index: Option<&Index>) {
    let local = Local::load(data);
    let mut secrets = Vec::new();
    for s in &local.secrets {
      if let Some(v) = keys::stored(data, &entry(&s.scope, &s.name)) {
        secrets.push((s.scope.clone(), s.name.clone(), v));
      }
    }
    let mut redact: Vec<(String, String)> = secrets
      .iter()
      .map(|(_, n, v)| (v.clone(), n.clone()))
      .collect();
    // Provider keys from the environment are as secret as stored ones.
    for (name, kind) in KNOWN
      .iter()
      .map(|n| ((*n).to_owned(), (*n).to_owned()))
      .chain(
        config
          .providers
          .iter()
          .map(|(n, p)| (n.clone(), p.kind.clone())),
      )
    {
      if let Some(v) = keys::get(data, &name, &kind) {
        redact.push((v, name));
      }
    }
    redact.retain(|(v, _)| v.chars().count() >= MIN_REDACTED);
    redact.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.cmp(b)));
    redact.dedup_by(|a, b| a.0 == b.0);
    let redact = redact
      .into_iter()
      .map(|(v, n)| (v, format!("«secret:{n}»")))
      .collect();
    let vars = index
      .map(|i| {
        latest_vars(i)
          .into_values()
          .filter(|v| !v.removed)
          .map(|v| (v.scope, v.name, v.value))
          .collect()
      })
      .unwrap_or_default();
    *self.snap.write().expect("vault lock") = Snapshot {
      secrets,
      vars,
      redact: Arc::new(redact),
    };
  }

  /// Reads the variables again (another node may have set some).
  pub fn reload_vars(&self, index: &Index) {
    let vars = latest_vars(index)
      .into_values()
      .filter(|v| !v.removed)
      .map(|v| (v.scope, v.name, v.value))
      .collect();
    self.snap.write().expect("vault lock").vars = vars;
  }

  /// `text` with every secret value this node holds replaced by
  /// `«secret:<name>»`.
  pub fn redact(&self, text: &str) -> String {
    let pairs = self.snap.read().expect("vault lock").redact.clone();
    redact_with(&pairs, text)
  }

  /// A redactor for output read in chunks.
  pub fn stream(&self) -> StreamRedactor {
    StreamRedactor::new(&self.snap.read().expect("vault lock").redact)
  }

  /// The secret `name` as `place` (a place's key) gets it: the place's own,
  /// else the global one.
  pub fn secret(&self, place: &str, name: &str) -> Option<String> {
    let snap = self.snap.read().expect("vault lock");
    let find = |scope: &str| {
      snap
        .secrets
        .iter()
        .find(|(s, n, _)| s == scope && n == name)
        .map(|(_, _, v)| v.clone())
    };
    find(place).or_else(|| find("global"))
  }

  /// The variable `name` as `place` gets it.
  pub fn var(&self, place: &str, name: &str) -> Option<String> {
    let snap = self.snap.read().expect("vault lock");
    let find = |scope: &str| {
      snap
        .vars
        .iter()
        .find(|(s, n, _)| s == scope && n == name)
        .map(|(_, _, v)| v.clone())
    };
    find(place).or_else(|| find("global"))
  }

  /// A value as `[env]` writes it: `secret:<name>`, `var:<name>`, or as
  /// written.
  pub fn resolve(&self, place: &str, value: &str) -> std::result::Result<String, String> {
    if let Some(name) = value.strip_prefix("secret:") {
      self
        .secret(place, name)
        .ok_or_else(|| format!("no secret `{name}` here"))
    } else if let Some(name) = value.strip_prefix("var:") {
      self
        .var(place, name)
        .ok_or_else(|| format!("no variable `{name}`"))
    } else {
      Ok(value.to_owned())
    }
  }

  /// What a shell in `place` gets: the global variables and the place's
  /// (by their names, those that are names of environment variables),
  /// then each of `maps` (config.toml's `[env]`, then the place's) in
  /// order. What cannot be resolved is left out, said in the second list.
  pub fn env_for(
    &self,
    place: &str,
    maps: &[&BTreeMap<String, String>],
  ) -> (Vec<(String, String)>, Vec<String>) {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    {
      let snap = self.snap.read().expect("vault lock");
      for scope in ["global", place] {
        for (s, n, v) in &snap.vars {
          if s == scope && is_env_name(n) {
            env.insert(n.clone(), v.clone());
          }
        }
      }
    }
    let mut problems = Vec::new();
    for map in maps {
      for (k, v) in *map {
        if !is_env_name(k) {
          problems.push(format!("`{k}` is not the name of an environment variable"));
          continue;
        }
        match self.resolve(place, v) {
          Ok(v) => {
            env.insert(k.clone(), v);
          }
          Err(e) => problems.push(format!("{k}: {e}")),
        }
      }
    }
    (env.into_iter().collect(), problems)
  }
}

fn is_env_name(s: &str) -> bool {
  !s.is_empty()
    && !s.starts_with(|c: char| c.is_ascii_digit())
    && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn redact_with(pairs: &[(String, String)], text: &str) -> String {
  let mut out = text.to_owned();
  for (value, label) in pairs {
    if out.contains(value.as_str()) {
      out = out.replace(value.as_str(), label);
    }
  }
  out
}

/// Redacts output that comes in chunks (a shell's terminal): a value split
/// between two chunks is still replaced. What could be the start of a
/// value is held back until the next chunk says, or `flush`.
pub struct StreamRedactor {
  pairs: Arc<Vec<(String, String)>>,
  pending: Vec<u8>,
}

impl StreamRedactor {
  fn new(pairs: &Arc<Vec<(String, String)>>) -> StreamRedactor {
    StreamRedactor {
      pairs: pairs.clone(),
      pending: Vec::new(),
    }
  }

  /// What of `chunk` (after what was held back) can be shown now.
  pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
    if self.pairs.is_empty() {
      return chunk.to_vec();
    }
    let mut buf = std::mem::take(&mut self.pending);
    buf.extend_from_slice(chunk);
    let mut out = Vec::with_capacity(buf.len());
    let mut i = 0;
    'scan: while i < buf.len() {
      for (value, label) in self.pairs.iter() {
        let v = value.as_bytes();
        if buf[i..].starts_with(v) {
          out.extend_from_slice(label.as_bytes());
          i += v.len();
          continue 'scan;
        }
      }
      // What is left could be the start of a value: held back.
      let rest = &buf[i..];
      if self
        .pairs
        .iter()
        .any(|(v, _)| v.len() > rest.len() && v.as_bytes().starts_with(rest))
      {
        self.pending = rest.to_vec();
        return out;
      }
      out.push(buf[i]);
      i += 1;
    }
    out
  }

  /// Whether something is held back.
  pub fn holding(&self) -> bool {
    !self.pending.is_empty()
  }

  /// What was held back: the output ended, or nothing came for a while.
  pub fn flush(&mut self) -> Vec<u8> {
    std::mem::take(&mut self.pending)
  }
}

// ---------------------------------------------------------------------
// Acting on them.

/// What the operations need of a node.
pub(crate) struct Ctx<'a> {
  pub data: &'a Path,
  pub config: &'a Config,
  pub home: Option<RecordRoot>,
  pub index: &'a Mutex<Index>,
  pub node: &'a NodeName,
  /// The other nodes this one trusts, by name.
  pub trusted: Vec<String>,
  pub vault: &'a Vault,
}

impl Ctx<'_> {
  fn home(&self) -> Result<&RecordRoot> {
    self
      .home
      .as_ref()
      .ok_or_else(|| Error::Config("no home workspace here".into()))
  }

  fn refresh(&self) {
    if let Some(home) = &self.home {
      let _ = self.index.lock().expect("index lock").refresh(home);
    }
  }

  fn reload(&self) {
    let index = self.index.lock().expect("index lock");
    self.vault.reload(self.data, self.config, Some(&index));
  }

  /// This node's sealing key, made when there is none, and published when
  /// its record is not the latest of this node.
  fn own_key(&self) -> Result<[u8; 32]> {
    let private = seal_private(self.data, true)?.expect("made");
    let public = B64.encode(crate::apns::public_of(&private));
    let published = sealing_keys(&self.index.lock().expect("index lock"))
      .get(self.node.as_str())
      .is_some_and(|k| *k == public);
    if !published && let Some(home) = &self.home {
      let draft = Draft::new(SEAL_KEY, Author::node(self.node.clone()))?
        .field("node", self.node.as_str())?
        .field("key", public.as_str())?
        .body(format!(
          "# {}'s sealing key\n\nWhat other nodes seal secrets for it with.\n",
          self.node
        ));
      home.publish(self.node, draft)?;
      self.refresh();
    }
    Ok(private)
  }

  /// Seals `value` for this node and every trusted node with a key: the
  /// record's `sealed` table.
  fn seal_all(&self, name: &str, scope: &str, value: &str) -> Result<toml::Table> {
    let content = serde_json::to_vec(&Sealed {
      name: name.into(),
      scope: scope.into(),
      value: value.into(),
    })
    .map_err(|e| Error::Config(e.to_string()))?;
    let keys = sealing_keys(&self.index.lock().expect("index lock"));
    let mut sealed = toml::Table::new();
    for (node, key) in keys {
      if node == self.node.as_str() || self.trusted.contains(&node) {
        sealed.insert(node, crate::apns::seal_with(INFO, &key, &content)?.into());
      }
    }
    Ok(sealed)
  }

  /// Writes a `secret` record: sealed, or (`None`) removed everywhere.
  fn publish_secret(
    &self,
    name: &str,
    scope: &str,
    set_at: jiff::Timestamp,
    sealed: Option<toml::Table>,
    rotate: &[String],
  ) -> Result<String> {
    let mut draft = Draft::new(SECRET, Author::node(self.node.clone()))?
      .field("name", name)?
      .field("scope", scope)?
      .field("set_at", set_at.to_string())?;
    draft = match sealed {
      Some(sealed) => draft.field("sealed", toml::Value::Table(sealed))?,
      None => draft.field("removed", true)?,
    };
    if !rotate.is_empty() {
      draft = draft.field(
        "rotate",
        toml::Value::Array(rotate.iter().map(|n| n.as_str().into()).collect()),
      )?;
    }
    let draft = draft.body(format!(
      "# Secret {name} ({scope})\n\nIts value is sealed for each node that holds it.\n"
    ));
    let r = self.home()?.publish(self.node, draft)?;
    self.refresh();
    Ok(r.id.to_string())
  }

  /// The nodes a record was sealed for that this node trusts no more,
  /// with those it already named.
  fn rotate_of(&self, r: &SecretRecord) -> Vec<String> {
    let mut out: BTreeSet<String> = r.rotate.iter().cloned().collect();
    for n in r.sealed.keys() {
      if n != self.node.as_str() && !self.trusted.contains(n) {
        out.insert(n.clone());
      }
    }
    out.into_iter().collect()
  }
}

/// Sets a secret here (and, unless this device's only, for every node).
/// What to tell the user.
pub(crate) fn set(
  cx: &Ctx,
  name: &str,
  scope: &str,
  value: Option<&str>,
  device_only: Option<bool>,
  file: bool,
) -> Result<String> {
  check_name(name)?;
  let scope = check_scope(scope)?;
  let mut local = Local::load(cx.data);
  discover(cx.data, cx.config, cx.node.as_str(), &mut local);
  let before = local.find(&scope, name).cloned();
  let key = entry(&scope, name);
  let value = match value {
    Some(v) => {
      let v = v.trim();
      if v.is_empty() {
        return Err(Error::Config("an empty secret".into()));
      }
      Some(v.to_owned())
    }
    None => None,
  };
  let current = match &value {
    Some(v) => v.clone(),
    None => keys::stored(cx.data, &key)
      .ok_or_else(|| Error::Config(format!("no secret `{name}` ({scope}) here")))?,
  };
  let device_only = device_only
    .or(before.as_ref().map(|b| b.device_only))
    .unwrap_or(false);
  let store = if file {
    Store::File
  } else {
    keys::store_for(cx.data)
  };
  let mut stored = None;
  if value.is_some() {
    stored = Some(keys::set(cx.data, &key, &current, store)?);
  }
  let latest =
    latest_secrets(&cx.index.lock().expect("index lock")).remove(&(scope.clone(), name.to_owned()));
  let at = now();
  let record = if cx.home.is_none() {
    // No home workspace yet: sealed once there is one (`reconcile`).
    None
  } else if device_only {
    // Kept here alone: the other nodes drop the copy they were given.
    if let Some(l) = &latest
      && !l.removed
      && !before.as_ref().is_some_and(|b| b.device_only)
    {
      cx.publish_secret(name, &scope, at, None, &[])?;
    }
    None
  } else {
    cx.own_key()?;
    let sealed = cx.seal_all(name, &scope, &current)?;
    // A new value starts over; the same one keeps the nodes it reached.
    let rotate = match (&value, &latest) {
      (None, Some(l)) => cx.rotate_of(l),
      _ => Vec::new(),
    };
    Some(cx.publish_secret(name, &scope, at, Some(sealed), &rotate)?)
  };
  local.put(LocalSecret {
    name: name.to_owned(),
    scope: scope.clone(),
    device_only,
    record,
    updated: if value.is_some() {
      at.to_string()
    } else {
      before
        .as_ref()
        .map_or_else(|| at.to_string(), |b| b.updated.clone())
    },
    from: cx.node.to_string(),
  });
  local.save(cx.data)?;
  cx.reload();
  let place = if scope == "global" {
    String::new()
  } else {
    format!(" ({scope})")
  };
  let where_ = match stored {
    Some(Store::File) => " in credentials.toml",
    Some(Store::Keychain) => " in the keychain",
    None => "",
  };
  Ok(if device_only {
    format!("secret {name}{place} kept{where_}, on this device only")
  } else {
    format!("secret {name}{place} kept{where_}, and sealed for the user's other nodes")
  })
}

/// A secret's value here.
pub(crate) fn get(cx: &Ctx, name: &str, scope: &str) -> Result<String> {
  check_name(name)?;
  let scope = check_scope(scope)?;
  keys::stored(cx.data, &entry(&scope, name))
    .ok_or_else(|| Error::Config(format!("no secret `{name}` ({scope}) here")))
}

/// Removes a secret here and, unless it was this device's only, from every
/// node.
pub(crate) fn remove(cx: &Ctx, name: &str, scope: &str) -> Result<String> {
  check_name(name)?;
  let scope = check_scope(scope)?;
  let mut local = Local::load(cx.data);
  let before = local.find(&scope, name).cloned();
  let latest =
    latest_secrets(&cx.index.lock().expect("index lock")).remove(&(scope.clone(), name.to_owned()));
  let here = keys::stored(cx.data, &entry(&scope, name)).is_some();
  if before.is_none() && !here && latest.as_ref().is_none_or(|l| l.removed) {
    return Err(Error::Config(format!("no secret `{name}` ({scope})")));
  }
  keys::remove(cx.data, &entry(&scope, name))?;
  local.drop_one(&scope, name);
  local.save(cx.data)?;
  let everywhere =
    !before.as_ref().is_some_and(|b| b.device_only) && latest.as_ref().is_some_and(|l| !l.removed);
  if everywhere {
    cx.publish_secret(name, &scope, now(), None, &[])?;
  }
  cx.reload();
  Ok(if everywhere {
    format!("secret {name} removed here and from the other nodes")
  } else {
    format!("secret {name} removed")
  })
}

/// Every secret this node holds or a record names.
pub(crate) fn list(cx: &Ctx, label: &dyn Fn(&str) -> String) -> Vec<SecretInfo> {
  let mut local = Local::load(cx.data);
  if discover(cx.data, cx.config, cx.node.as_str(), &mut local) {
    let _ = local.save(cx.data);
    cx.reload();
  }
  let (latest, places) = {
    let index = cx.index.lock().expect("index lock");
    (latest_secrets(&index), place_envs(&index))
  };
  let mut out = Vec::new();
  let mut seen = BTreeSet::new();
  for l in &local.secrets {
    seen.insert((l.scope.clone(), l.name.clone()));
    let record = latest
      .get(&(l.scope.clone(), l.name.clone()))
      .filter(|_| !l.device_only);
    out.push(SecretInfo {
      name: l.name.clone(),
      scope: l.scope.clone(),
      scope_label: label(&l.scope),
      device_only: l.device_only,
      here: keys::stored(cx.data, &entry(&l.scope, &l.name)).is_some(),
      nodes: match record {
        Some(r) if !r.removed => r.sealed.keys().cloned().collect(),
        _ => vec![cx.node.to_string()],
      },
      used: used(cx.config, &places, &l.scope, &l.name, label),
      updated: l.updated.clone(),
      from: l.from.clone(),
      rotate: record.map(|r| cx.rotate_of(r)).unwrap_or_default(),
    });
  }
  // Sealed for other nodes only (this one is waiting to be re-sealed for).
  for ((scope, name), r) in &latest {
    if r.removed || seen.contains(&(scope.clone(), name.clone())) {
      continue;
    }
    out.push(SecretInfo {
      name: name.clone(),
      scope: scope.clone(),
      scope_label: label(scope),
      device_only: false,
      here: false,
      nodes: r.sealed.keys().cloned().collect(),
      used: used(cx.config, &places, scope, name, label),
      updated: r.set_at.to_string(),
      from: r.node.clone(),
      rotate: cx.rotate_of(r),
    });
  }
  out.sort_by(|a, b| {
    (&a.scope != "global", &a.scope, &a.name).cmp(&(&b.scope != "global", &b.scope, &b.name))
  });
  out
}

/// Each place's `[env]`, as its latest record of any node says.
fn place_envs(index: &Index) -> BTreeMap<String, BTreeMap<String, String>> {
  let mut out = BTreeMap::new();
  for r in index
    .records(Some(stitch_records::kinds::PLACE))
    .unwrap_or_default()
  {
    let Some(place) = r.fields.get("place").and_then(Value::as_str) else {
      continue;
    };
    let env: BTreeMap<String, String> = r
      .fields
      .get("env")
      .and_then(Value::as_object)
      .map(|m| {
        m.iter()
          .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
          .collect()
      })
      .unwrap_or_default();
    out.insert(place.to_owned(), env);
  }
  out
}

/// Where a secret is used, as far as this node can tell.
fn used(
  config: &Config,
  places: &BTreeMap<String, BTreeMap<String, String>>,
  scope: &str,
  name: &str,
  label: &dyn Fn(&str) -> String,
) -> Vec<String> {
  let reference = format!("secret:{name}");
  let mut out = Vec::new();
  if scope == "global" {
    if config.providers.contains_key(name)
      || [
        "anthropic",
        "openai",
        "gemini",
        "grok",
        "kimi",
        "deepseek",
        "openrouter",
      ]
      .contains(&name)
    {
      out.push(format!("provider {name}"));
    }
    match name {
      "brave" | "perplexity" => out.push("web search".into()),
      "github" => out.push("GitHub".into()),
      "apns" => out.push("push notifications".into()),
      _ => {}
    }
    for (k, v) in &config.env {
      if *v == reference {
        out.push(format!("env {k} (this node)"));
      }
    }
    if let Some(mcp) = &config.mcp {
      for (server, s) in &mcp.servers {
        let key = s
          .key
          .as_deref()
          .map(|k| k.strip_prefix("secret:").unwrap_or(k));
        if key == Some(name) || s.env.values().any(|v| *v == reference) {
          out.push(format!("mcp {server}"));
        }
      }
    }
  }
  for (place, env) in places {
    // A place's own secret serves it alone; a global one any place
    // without its own.
    if scope != "global" && place != scope {
      continue;
    }
    for (k, v) in env {
      if *v == reference {
        out.push(format!("env {k} ({})", label(place)));
      }
    }
  }
  out
}

/// Sets a variable (for every node).
pub(crate) fn set_var(cx: &Ctx, name: &str, scope: &str, value: &str) -> Result<()> {
  check_name(name)?;
  let scope = check_scope(scope)?;
  let draft = Draft::new(VARIABLE, Author::user(cx.node.clone()))?
    .field("name", name)?
    .field("scope", scope.as_str())?
    .field("value", value)?
    .body(format!("# Variable {name} ({scope})\n"));
  cx.home()?.publish(cx.node, draft)?;
  cx.refresh();
  cx.reload();
  Ok(())
}

pub(crate) fn remove_var(cx: &Ctx, name: &str, scope: &str) -> Result<()> {
  check_name(name)?;
  let scope = check_scope(scope)?;
  let exists = latest_vars(&cx.index.lock().expect("index lock"))
    .get(&(scope.clone(), name.to_owned()))
    .is_some_and(|v| !v.removed);
  if !exists {
    return Err(Error::Config(format!("no variable `{name}` ({scope})")));
  }
  let draft = Draft::new(VARIABLE, Author::user(cx.node.clone()))?
    .field("name", name)?
    .field("scope", scope.as_str())?
    .field("removed", true)?
    .body(format!("# Variable {name} ({scope}) removed\n"));
  cx.home()?.publish(cx.node, draft)?;
  cx.refresh();
  cx.reload();
  Ok(())
}

pub(crate) fn list_vars(cx: &Ctx, label: &dyn Fn(&str) -> String) -> Vec<VarInfo> {
  let mut out: Vec<VarInfo> = latest_vars(&cx.index.lock().expect("index lock"))
    .into_values()
    .filter(|v| !v.removed)
    .map(|v| VarInfo {
      scope_label: label(&v.scope),
      name: v.name,
      scope: v.scope,
      value: v.value,
      updated: v.created.to_string(),
      from: v.node,
    })
    .collect();
  out.sort_by(|a, b| {
    (&a.scope != "global", &a.scope, &a.name).cmp(&(&b.scope != "global", &b.scope, &b.name))
  });
  out
}

/// After records arrived (and when the node starts): keys stored before
/// secrets were (or by other commands) listed; what other nodes sealed for
/// this one opened, what they removed dropped, and what this node holds
/// re-sealed for trusted nodes a record left out. Whether it wrote records
/// (to be synced).
pub(crate) fn reconcile(cx: &Ctx) -> Result<bool> {
  let mut local = Local::load(cx.data);
  let mut changed = discover(cx.data, cx.config, cx.node.as_str(), &mut local);
  let latest = latest_secrets(&cx.index.lock().expect("index lock"));
  let synced_here = local.secrets.iter().any(|s| !s.device_only);
  let any = latest.values().any(|r| !r.removed);
  if (!any && !synced_here) || cx.home.is_none() {
    if changed {
      local.save(cx.data)?;
      cx.reload();
    }
    return Ok(false);
  }
  // Secrets seen, or some to sync: this node needs its key.
  let mine_before = sealing_keys(&cx.index.lock().expect("index lock")).remove(cx.node.as_str());
  let private = cx.own_key()?;
  let keys_now = sealing_keys(&cx.index.lock().expect("index lock"));
  let mut wrote = keys_now.get(cx.node.as_str()) != mine_before.as_ref();
  for ((scope, name), r) in &latest {
    let mine = local.find(scope, name).cloned();
    if mine.as_ref().is_some_and(|m| m.device_only) {
      continue;
    }
    if r.removed {
      if mine.is_some() {
        keys::remove(cx.data, &entry(scope, name))?;
        local.drop_one(scope, name);
        changed = true;
      }
      continue;
    }
    let holds = mine.as_ref().and_then(|m| m.record.as_deref()) == Some(r.id.as_str());
    if !holds {
      // Opened, or not this node's to re-seal: what it holds (if anything)
      // is an older value.
      let Some(value) = r
        .sealed
        .get(cx.node.as_str())
        .and_then(|w| open(&private, w, name, scope))
      else {
        // Not sealed for this node (yet): a node that holds it re-seals.
        continue;
      };
      keys::set(
        cx.data,
        &entry(scope, name),
        &value,
        keys::store_for(cx.data),
      )?;
      local.put(LocalSecret {
        name: name.clone(),
        scope: scope.clone(),
        device_only: false,
        record: Some(r.id.clone()),
        updated: r.set_at.to_string(),
        from: r.node.clone(),
      });
      changed = true;
    }
    // A trusted node with a key the record left out (paired since): this
    // node holds the record's value now, opened or its own.
    let missing = keys_now
      .keys()
      .any(|n| cx.trusted.contains(n) && !r.sealed.contains_key(n));
    if missing && let Some(value) = keys::stored(cx.data, &entry(scope, name)) {
      let sealed = cx.seal_all(name, scope, &value)?;
      let id = cx.publish_secret(name, scope, r.set_at, Some(sealed), &cx.rotate_of(r))?;
      if let Some(mut m) = local.find(scope, name).cloned() {
        m.record = Some(id);
        local.put(m);
      }
      changed = true;
      wrote = true;
    }
  }
  // Set to sync here before there was a record (no home workspace then):
  // sealed now.
  for m in local.secrets.clone() {
    if m.device_only
      || m.record.is_some()
      || latest.contains_key(&(m.scope.clone(), m.name.clone()))
    {
      continue;
    }
    if let Some(value) = keys::stored(cx.data, &entry(&m.scope, &m.name)) {
      let sealed = cx.seal_all(&m.name, &m.scope, &value)?;
      let at = m.updated.parse().unwrap_or_else(|_| now());
      let id = cx.publish_secret(&m.name, &m.scope, at, Some(sealed), &[])?;
      local.put(LocalSecret {
        record: Some(id),
        ..m
      });
      changed = true;
      wrote = true;
    }
  }
  if changed {
    local.save(cx.data)?;
    cx.reload();
  }
  Ok(wrote)
}

/// For tests and tools: this data directory's list's path.
pub fn local_path(data: &Path) -> PathBuf {
  data.join(LOCAL)
}

// ---------------------------------------------------------------------
// On a running node.

use crate::api::{Reply, SecretOp};
use crate::node::Shared;

/// Held through each operation: one at a time reads and writes a node's
/// list (and two nodes in one process, in tests, wait for each other).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Runs `f` with what the operations need of the node `s`.
fn with_ctx<T>(s: &Shared, f: impl FnOnce(&Ctx, &dyn Fn(&str) -> String) -> T) -> T {
  let _one = ONE_AT_A_TIME
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner);
  let state = s.state();
  let me = state.node.to_string();
  let trusted = state
    .known_peers()
    .unwrap_or_default()
    .into_iter()
    .map(|p| p.name)
    .filter(|n| *n != me)
    .collect();
  let cx = Ctx {
    data: &s.data,
    config: &s.config,
    home: state.home(),
    index: &state.index,
    node: &state.node,
    trusted,
    vault: &s.vault,
  };
  let places = s.places();
  let label = |scope: &str| match Place::of_key(scope) {
    Some(p) => p.label(&me, &places),
    None => scope.to_owned(),
  };
  f(&cx, &label)
}

/// `Request::Secret`: what the user asks of secrets and variables.
pub(crate) async fn request(s: &Arc<Shared>, op: SecretOp, cwd: Option<String>) -> Result<Reply> {
  let scope_of = |scope: &str| -> String {
    if scope == "here" {
      s.place_of_dir(cwd.as_deref()).0.key()
    } else {
      scope.to_owned()
    }
  };
  let op = match op {
    SecretOp::Set {
      name,
      scope,
      value,
      device_only,
      file,
    } => SecretOp::Set {
      name,
      scope: scope_of(&scope),
      value,
      device_only,
      file,
    },
    SecretOp::Get { name, scope } => SecretOp::Get {
      name,
      scope: scope_of(&scope),
    },
    SecretOp::Remove { name, scope } => SecretOp::Remove {
      name,
      scope: scope_of(&scope),
    },
    SecretOp::VarSet { name, scope, value } => SecretOp::VarSet {
      name,
      scope: scope_of(&scope),
      value,
    },
    SecretOp::VarRemove { name, scope } => SecretOp::VarRemove {
      name,
      scope: scope_of(&scope),
    },
    op => op,
  };
  let writes = matches!(
    op,
    SecretOp::Set { .. }
      | SecretOp::Remove { .. }
      | SecretOp::VarSet { .. }
      | SecretOp::VarRemove { .. }
  );
  // The keychain may take a while (or ask): off the runtime's threads.
  let s2 = s.clone();
  let reply = tokio::task::spawn_blocking(move || {
    with_ctx(&s2, |cx, label| -> Result<Reply> {
      Ok(match op {
        SecretOp::Set {
          name,
          scope,
          value,
          device_only,
          file,
        } => Reply::Lines(vec![set(
          cx,
          &name,
          &scope,
          value.as_deref(),
          device_only,
          file,
        )?]),
        SecretOp::Get { name, scope } => Reply::Secret(get(cx, &name, &scope)?),
        SecretOp::List => Reply::Secrets(list(cx, label)),
        SecretOp::Remove { name, scope } => Reply::Lines(vec![remove(cx, &name, &scope)?]),
        SecretOp::VarSet { name, scope, value } => {
          set_var(cx, &name, &scope, &value)?;
          Reply::Ok
        }
        SecretOp::VarRemove { name, scope } => {
          remove_var(cx, &name, &scope)?;
          Reply::Ok
        }
        SecretOp::VarList => Reply::Vars(list_vars(cx, label)),
      })
    })
  })
  .await
  .map_err(|e| Error::Config(e.to_string()))??;
  if writes && let Some(home) = s.state().home() {
    // The other nodes have it at once, when they can be reached.
    let s = s.clone();
    tokio::spawn(async move {
      crate::node::sync_one(&s, home.repo()).await;
    });
  }
  Ok(reply)
}

/// After a sync, and when the node starts: `reconcile`, and what it wrote
/// synced at once.
pub(crate) async fn after_sync(s: &Arc<Shared>) {
  let s2 = s.clone();
  let wrote = tokio::task::spawn_blocking(move || {
    with_ctx(&s2, |cx, _| {
      cx.vault.reload_vars(&cx.index.lock().expect("index lock"));
      reconcile(cx)
    })
  })
  .await
  .map_err(|e| Error::Config(e.to_string()))
  .and_then(|r| r);
  match wrote {
    Ok(true) => {
      if let Some(home) = s.state().home() {
        crate::node::sync_one(s, home.repo()).await;
      }
    }
    Ok(false) => {}
    Err(e) => eprintln!("secrets: {e}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pairs(values: &[(&str, &str)]) -> Arc<Vec<(String, String)>> {
    let mut v: Vec<(String, String)> = values
      .iter()
      .map(|(v, n)| ((*v).to_owned(), format!("«secret:{n}»")))
      .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
    Arc::new(v)
  }

  #[test]
  fn values_are_replaced_by_their_names() {
    let p = pairs(&[("sk-abcdef123456", "openai"), ("hunter2hunter2", "db")]);
    assert_eq!(
      redact_with(
        &p,
        "key=sk-abcdef123456 pw=hunter2hunter2; again sk-abcdef123456"
      ),
      "key=«secret:openai» pw=«secret:db»; again «secret:openai»"
    );
    assert_eq!(redact_with(&p, "nothing here"), "nothing here");
  }

  #[test]
  fn a_longer_value_wins_over_one_it_contains() {
    let p = pairs(&[("abcdefgh", "short"), ("abcdefgh-and-more", "long")]);
    assert_eq!(
      redact_with(&p, "x abcdefgh-and-more y abcdefgh"),
      "x «secret:long» y «secret:short»"
    );
  }

  /// Every way of cutting the output in two, and in single bytes.
  #[test]
  fn values_split_across_chunks_are_replaced() {
    let p = pairs(&[("sk-abcdef123456", "openai"), ("pässwörd-ü", "umlaut")]);
    let text = "export KEY=sk-abcdef123456\npw pässwörd-ü end sk-abc";
    let want = "export KEY=«secret:openai»\npw «secret:umlaut» end sk-abc";
    let bytes = text.as_bytes();
    for cut in 0..=bytes.len() {
      let mut r = StreamRedactor::new(&p);
      let mut out = r.push(&bytes[..cut]);
      out.extend(r.push(&bytes[cut..]));
      out.extend(r.flush());
      assert_eq!(String::from_utf8(out).unwrap(), want, "cut at {cut}");
    }
    let mut r = StreamRedactor::new(&p);
    let mut out = Vec::new();
    for b in bytes {
      out.extend(r.push(std::slice::from_ref(b)));
      // What was shown never holds a value's whole.
      assert!(!String::from_utf8_lossy(&out).contains("sk-abcdef123456"));
    }
    out.extend(r.flush());
    assert_eq!(String::from_utf8(out).unwrap(), want);
  }

  #[test]
  fn only_what_may_start_a_value_is_held_back() {
    let p = pairs(&[("sk-abcdef123456", "openai")]);
    let mut r = StreamRedactor::new(&p);
    assert_eq!(r.push(b"hello s"), b"hello ");
    assert!(r.holding());
    assert_eq!(r.push(b"ome"), b"some");
    assert!(!r.holding());
    // Nothing to redact: passed through.
    let mut none = StreamRedactor::new(&Arc::new(Vec::new()));
    assert_eq!(none.push(b"sk-"), b"sk-");
  }

  #[test]
  fn names_and_scopes_are_checked() {
    assert!(check_name("openai").is_ok());
    assert!(check_name("my_key-2.v1").is_ok());
    for bad in ["", "-x", "a b", "a/b", "seal-key@stitch", &"x".repeat(65)] {
      assert!(check_name(bad).is_err(), "{bad}");
    }
    assert_eq!(check_scope("global").unwrap(), "global");
    assert_eq!(check_scope("folder:~/Mail").unwrap(), "folder:~/Mail");
    assert!(check_scope("nowhere").is_err());
  }

  #[test]
  fn sealed_values_open_for_their_name_only() {
    let private = [9u8; 32];
    let public = B64.encode(crate::apns::public_of(&private));
    let content = serde_json::to_vec(&Sealed {
      name: "openai".into(),
      scope: "global".into(),
      value: "sk-1".into(),
    })
    .unwrap();
    let wire = crate::apns::seal_with(INFO, &public, &content).unwrap();
    assert_eq!(
      open(&private, &wire, "openai", "global").as_deref(),
      Some("sk-1")
    );
    assert_eq!(open(&private, &wire, "github", "global"), None);
    assert_eq!(open(&[1u8; 32], &wire, "openai", "global"), None);
    // A push's sealing does not open as a secret's.
    let push = crate::apns::seal(&public, &content).unwrap();
    assert_eq!(open(&private, &push, "openai", "global"), None);
  }

  #[test]
  fn keys_stored_before_are_listed_as_this_devices_secrets() {
    let dir = tempfile::tempdir().unwrap();
    keys::set(dir.path(), "openai", "sk-before", Store::File).unwrap();
    keys::set(dir.path(), "custom-thing", "v-before", Store::File).unwrap();
    let mut local = Local::default();
    assert!(discover(dir.path(), &Config::default(), "mbp", &mut local));
    let names: Vec<_> = local.secrets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["custom-thing", "openai"]);
    assert!(
      local
        .secrets
        .iter()
        .all(|s| s.device_only && s.scope == "global")
    );
    // The value stays where it was: nothing moved, nothing lost.
    assert_eq!(
      keys::get(dir.path(), "openai", "openai").as_deref(),
      Some("sk-before")
    );
    assert!(!discover(dir.path(), &Config::default(), "mbp", &mut local));
  }

  #[test]
  fn shells_get_variables_and_what_env_maps() {
    let v = Vault::default();
    *v.snap.write().unwrap() = Snapshot {
      secrets: vec![
        ("global".into(), "openai".into(), "sk-global".into()),
        ("project:p-1".into(), "openai".into(), "sk-project".into()),
      ],
      vars: vec![
        ("global".into(), "HF_HOME".into(), "/data/hf".into()),
        ("project:p-1".into(), "REGION".into(), "eu".into()),
        ("global".into(), "lower.case".into(), "x".into()),
      ],
      redact: Arc::new(Vec::new()),
    };
    let map: BTreeMap<String, String> = [
      ("OPENAI_API_KEY", "secret:openai"),
      ("MISSING", "secret:nope"),
      ("PLAIN", "as is"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect();
    let (env, problems) = v.env_for("project:p-1", &[&map]);
    let env: BTreeMap<_, _> = env.into_iter().collect();
    assert_eq!(env["OPENAI_API_KEY"], "sk-project");
    assert_eq!(env["HF_HOME"], "/data/hf");
    assert_eq!(env["REGION"], "eu");
    assert_eq!(env["PLAIN"], "as is");
    assert!(!env.contains_key("MISSING") && !env.contains_key("lower.case"));
    assert_eq!(problems, ["MISSING: no secret `nope` here"]);
    let (env, _) = v.env_for("folder:~/x", &[&map]);
    let env: BTreeMap<_, _> = env.into_iter().collect();
    assert_eq!(env["OPENAI_API_KEY"], "sk-global");
    assert!(!env.contains_key("REGION"));
  }
}
