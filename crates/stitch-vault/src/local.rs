//! A node's own vault (opt-in): what `stitch vault` drives. Everything lives in
//! `<data>/vault/`: `config.json` (endpoint, vault ID, this device's public key),
//! `device.key` (the device's secrets, owner-only, like `node.key`), `state.json`
//! (what is backed up: per repository the index fingerprint and file hashes, the blobs
//! sent, known chunks, the device's chain head) and `unpublished.json` (a signed
//! commit not yet known to be accepted). The recovery kit is never stored here: it is
//! shown once when the vault is created and the user keeps it.
//!
//! Backups are incremental through the store's index (the same fingerprints and item
//! sets that drive sync and its change feed): a repository whose fingerprint did not
//! change is not read at all; otherwise only files new since the last backup are read,
//! files gone become tombstones, and attached original blobs are sent once.
//! Recovery stages into a new directory and writes an inert preview (`node::preview`);
//! `import` copies chosen conversations from a preview into a live record root.
use crate::node::{BACKUP_BATCH, SECRET_KINDS};
use crate::*;
use std::{
  collections::{BTreeMap, BTreeSet},
  fs,
  io::{Read, Write},
  path::{Path, PathBuf},
};
use stitch_records::{BlobHash, Record, RecordId};
use stitch_store::{BlobStore, Index, ItemPath, RecordRoot};

/// The node's vault configuration (`config.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
  pub version: u32,
  pub endpoint: String,
  pub vault: String,
  /// This device's signing key (public).
  pub device: String,
  /// This device's sealing key (public): what an admin needs to admit it.
  pub seal: String,
  pub node: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoState {
  /// The index fingerprint at the last complete backup of the repository.
  fingerprint: String,
  /// Root-relative path → blake3 (hex) of what was backed up.
  files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalState {
  sequence: u64,
  previous: String,
  repos: BTreeMap<String, RepoState>,
  /// `blob/{hash}` names already in the vault.
  blobs: BTreeSet<String>,
  known: Known,
  last: Option<Summary>,
}
/// What one backup did.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
  /// When it finished (ms since the epoch).
  pub at: u64,
  pub repos: usize,
  /// Repositories not read at all (fingerprint unchanged).
  pub unchanged_repos: usize,
  pub uploaded: usize,
  pub deleted: usize,
  pub blobs: usize,
  pub commits: usize,
  pub secrets_skipped: usize,
  /// Attached blobs this node does not hold: the backup is incomplete without them.
  pub missing_blobs: Vec<String>,
}
impl Summary {
  pub fn complete(&self) -> bool {
    self.missing_blobs.is_empty()
  }
}
/// `stitch vault status`.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
  pub config: LocalConfig,
  /// This device's role in the verified log (`None`: not admitted yet).
  pub role: Option<String>,
  pub frozen: bool,
  pub revoked: bool,
  pub epoch: u32,
  pub retention_days: u64,
  pub two_admins: bool,
  pub deleted: bool,
  /// Destructive actions waiting for their time: what the user must look at.
  pub pending: Vec<Pending>,
  /// A membership statement refused while verifying (the log is suspect).
  pub refused: Option<String>,
  /// Statements whose kit envelope does not check out (the kit could not recover
  /// those epochs).
  pub kit_envelope_problems: Vec<u64>,
  pub last_backup: Option<Summary>,
  pub repos: usize,
  pub files: usize,
  /// A commit waiting to be published again.
  pub unpublished: bool,
}
/// What `import` did.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Imported {
  pub records: usize,
  pub segments: usize,
  pub blobs: usize,
  pub present: usize,
  pub not_found: Vec<String>,
}

/// One change of a backup: the vault change (an empty name records a file without
/// sending it), the file it records (root-relative path and hash; an empty hash: the
/// file is gone), or the blob name it sends.
type Queued = (Change, Option<(String, String)>, Option<String>);

pub struct Local {
  dir: PathBuf,
  pub config: LocalConfig,
  device: Device,
  client: Client,
  state: LocalState,
}

fn io(e: impl std::fmt::Display) -> Error {
  Error::Io(std::io::Error::other(e.to_string()))
}
fn now() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_millis() as u64)
}
/// Write `bytes` to `dir/name` owner-only, through a temporary file and a rename.
fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
  let tmp = dir.join(format!(".{name}.tmp"));
  let _ = fs::remove_file(&tmp);
  let mut o = fs::OpenOptions::new();
  o.write(true).create_new(true);
  #[cfg(unix)]
  std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
  let mut f = o.open(&tmp)?;
  f.write_all(bytes)?;
  f.sync_all()?;
  fs::rename(tmp, dir.join(name))?;
  Ok(())
}

impl Local {
  pub fn dir_of(data: &Path) -> PathBuf {
    data.join("vault")
  }
  /// The node's vault, if one is set up.
  pub fn open(data: &Path) -> Result<Option<Self>> {
    let dir = Self::dir_of(data);
    if !dir.join("config.json").exists() {
      return Ok(None);
    }
    let config: LocalConfig = serde_json::from_slice(&fs::read(dir.join("config.json"))?)?;
    let mut device = Device::from_secret_text(&fs::read_to_string(dir.join("device.key"))?)?;
    if device.public() != config.device {
      return Err(Error::Invalid("device.key does not match config.json"));
    }
    let state: LocalState = match fs::read(dir.join("state.json")) {
      Ok(b) => serde_json::from_slice(&b)?,
      Err(_) => LocalState::default(),
    };
    if state.sequence > 0 {
      device.resume(state.sequence, &state.previous)?;
    }
    // A commit signed but not known to be accepted: the device's head is past it, and
    // the next commit publishes it again first.
    if let Ok(b) = fs::read(dir.join("unpublished.json")) {
      let c: Commit = serde_json::from_slice(&b)?;
      if c.sequence == state.sequence + 1 {
        device.resume(c.sequence, &sha256(&c.signing_bytes()?))?;
        device.set_unpublished(Some(c));
      }
    }
    let client = Client::signed(&config.endpoint, &config.vault, device.request_signer()?)?;
    Ok(Some(Self {
      dir,
      config,
      device,
      client,
      state,
    }))
  }
  fn save_new(data: &Path, config: &LocalConfig, device: &Device) -> Result<PathBuf> {
    let dir = Self::dir_of(data);
    if dir.join("config.json").exists() {
      return Err(Error::Invalid("a vault is already set up on this node"));
    }
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    write_private(&dir, "device.key", device.secret_text().as_bytes())?;
    write_private(&dir, "config.json", &serde_json::to_vec_pretty(config)?)?;
    Ok(dir)
  }
  fn config_for(endpoint: &str, vault: &str, node: &str, device: &Device) -> LocalConfig {
    LocalConfig {
      version: VERSION,
      endpoint: endpoint.trim_end_matches('/').into(),
      vault: vault.into(),
      device: device.public(),
      seal: device.seal_public(),
      node: node.into(),
    }
  }
  /// A new vault: a fresh recovery kit (returned once: the user must keep it), its
  /// genesis, and this device admitted by the kit as the first admin.
  pub async fn create(data: &Path, node: &str, endpoint: &str) -> Result<(Self, Kit)> {
    let kit = Kit::generate()?;
    let device = Device::generate()?;
    let bootstrap = Client::signed(endpoint, kit.vault(), kit.request_signer()?)?;
    let (genesis, ring) = Membership::genesis(&kit)?;
    bootstrap.post_statement(&genesis).await?;
    let (state, _) = Membership::verify(kit.vault(), std::slice::from_ref(&genesis));
    let add = state.add(
      &Signer::Kit(&kit),
      &ring,
      &device.public(),
      &device.seal_public(),
      Role::Admin,
    )?;
    bootstrap.post_statement(&add).await?;
    let config = Self::config_for(endpoint, kit.vault(), node, &device);
    Self::save_new(data, &config, &device)?;
    let local = Self::open(data)?.ok_or(Error::Invalid("vault setup"))?;
    Ok((local, kit))
  }
  /// Join an existing vault. With the kit, the kit admits this device at once (as
  /// `role`); without it, the device is set up and waits for an admin to run `admit`
  /// with the keys `config` shows.
  pub async fn join(
    data: &Path,
    node: &str,
    endpoint: &str,
    vault: &str,
    kit: Option<&Kit>,
    role: Role,
  ) -> Result<Self> {
    let device = Device::generate()?;
    if let Some(kit) = kit {
      if kit.vault() != vault {
        return Err(Error::Invalid("the kit is another vault's"));
      }
      let bootstrap = Client::signed(endpoint, vault, kit.request_signer()?)?;
      let (ring, _, _) = bootstrap.keyring(&Credential::Kit(kit)).await?;
      bootstrap
        .propose(vault, |m| {
          m.add(
            &Signer::Kit(kit),
            &ring,
            &device.public(),
            &device.seal_public(),
            role,
          )
        })
        .await?;
    }
    let config = Self::config_for(endpoint, vault, node, &device);
    Self::save_new(data, &config, &device)?;
    Self::open(data)?.ok_or(Error::Invalid("vault setup"))
  }
  /// This (admin) device admits another device by its public keys. An admin admitted
  /// this way is a writer for 7 days first (the delayed-admin rule).
  pub async fn admit(&self, device: &str, seal: &str, role: Role) -> Result<Statement> {
    let credential = self.credential();
    let (ring, _, _) = self.client.keyring(&credential).await?;
    let signer = Signer::Device(&self.device);
    self
      .client
      .propose(&self.config.vault, |m| {
        m.add(&signer, &ring, device, seal, role)
      })
      .await
  }
  /// Stop using the vault on this node: its directory is renamed aside (the device key
  /// is kept there, so nothing is lost by mistake). Nothing changes in the vault.
  pub fn disable(data: &Path) -> Result<PathBuf> {
    let dir = Self::dir_of(data);
    if !dir.exists() {
      return Err(Error::Invalid("no vault on this node"));
    }
    let aside = data.join(format!("vault.off-{}", now()));
    fs::rename(&dir, &aside)?;
    Ok(aside)
  }
  fn credential(&self) -> Credential<'_> {
    Credential::Device {
      device: &self.device,
      vault: &self.config.vault,
    }
  }
  fn save_state(&mut self) -> Result<()> {
    let (sequence, previous) = self.device.head();
    self.state.sequence = sequence;
    self.state.previous = previous.to_string();
    write_private(&self.dir, "state.json", &serde_json::to_vec(&self.state)?)?;
    match self.device.unpublished() {
      Some(c) => write_private(&self.dir, "unpublished.json", &serde_json::to_vec(c)?)?,
      None => {
        let _ = fs::remove_file(self.dir.join("unpublished.json"));
      }
    }
    Ok(())
  }
  pub async fn status(&self) -> Result<Status> {
    // Not admitted (or frozen, or revoked): the service does not let it read the log.
    let log = match self.client.statements().await {
      Err(Error::Status(401)) => vec![],
      other => other?,
    };
    let (m, refused) = Membership::verify(&self.config.vault, &log);
    let me = m.devices.get(&self.config.device);
    let problems = match keyring(&log, &self.credential()) {
      Ok((ring, _, _)) => kit_envelope_problems(&log, &ring),
      Err(_) => vec![],
    };
    Ok(Status {
      config: self.config.clone(),
      role: me.map(|d| d.role.clone()),
      frozen: me.is_some_and(|d| d.frozen.is_some()),
      revoked: me.is_some_and(|d| d.through.is_some()),
      epoch: m.epoch,
      retention_days: m.retention_days,
      two_admins: m.approvals >= 2,
      deleted: m.deleted,
      pending: m.pending.clone(),
      refused,
      kit_envelope_problems: problems,
      last_backup: self.state.last.clone(),
      repos: self.state.repos.len(),
      files: self.state.repos.values().map(|r| r.files.len()).sum(),
      unpublished: self.device.unpublished().is_some(),
    })
  }
  /// Back up these record roots (refreshed in `index` by the caller) and the original
  /// blobs their records attach, incrementally (see the module comment).
  pub async fn backup(
    &mut self,
    index: &mut Index,
    roots: &[RecordRoot],
    blobs: &BlobStore,
  ) -> Result<Summary> {
    let (ring, _, _) = self.client.keyring(&self.credential()).await?;
    let key = ring.current()?;
    let mut summary = Summary {
      repos: roots.len(),
      ..Summary::default()
    };
    for root in roots {
      let repo = root.repo().to_string();
      let fp = index.fingerprint(root.repo()).map_err(io)?;
      let fingerprint = format!("{}:{}", hex::encode(fp.xor), fp.count);
      let known = self.state.repos.get(&repo).cloned().unwrap_or_default();
      if known.fingerprint == fingerprint {
        summary.unchanged_repos += 1;
        continue;
      }
      let current: BTreeMap<String, String> = index
        .items(root.repo())
        .map_err(io)?
        .items()
        .iter()
        .map(|i| (i.key.clone(), hex::encode(i.hash)))
        .collect();
      let mut changes: Vec<Queued> = vec![];
      let mut queued_blobs = BTreeSet::new();
      for (path, hash) in &current {
        if known.files.get(path) == Some(hash) {
          continue;
        }
        let Some(item) = ItemPath::parse(path) else {
          continue;
        };
        let Ok(bytes) = root.read(&item) else {
          continue;
        };
        let kind = match &item {
          ItemPath::Record(_) => {
            if let Ok(record) = Record::parse(&String::from_utf8_lossy(&bytes)) {
              if SECRET_KINDS.contains(&record.kind.as_str()) {
                summary.secrets_skipped += 1;
                changes.push((
                  (String::new(), None),
                  Some((path.clone(), hash.clone())),
                  None,
                ));
                continue;
              }
              for a in &record.attachments {
                let name = format!("blob/{}", a.hash.hex());
                if self.state.blobs.contains(&name) || !queued_blobs.insert(name.clone()) {
                  continue;
                }
                let mut data = vec![];
                let read = blobs
                  .open_blob(&a.hash)
                  .map_err(io)
                  .and_then(|mut f| f.read_to_end(&mut data).map_err(Error::Io));
                if read.is_err() || BlobHash::of(&data) != a.hash {
                  summary.missing_blobs.push(a.hash.hex());
                  continue;
                }
                changes.push(((name.clone(), Some((Kind::Blob, data))), None, Some(name)));
              }
            }
            Kind::Record
          }
          ItemPath::Segment { .. } => Kind::Transcript,
        };
        changes.push((
          (format!("{repo}/{path}"), Some((kind, bytes))),
          Some((path.clone(), hash.clone())),
          None,
        ));
      }
      for path in known.files.keys() {
        if !current.contains_key(path) {
          changes.push(((format!("{repo}/{path}"), None), None, None));
          // Recorded below as removed.
          changes.last_mut().unwrap().1 = Some((path.clone(), String::new()));
        }
      }
      let mut state = known.clone();
      for batch in changes.chunks(BACKUP_BATCH) {
        let list: Vec<Change> = batch
          .iter()
          .filter(|(c, _, _)| !c.0.is_empty())
          .map(|(c, _, _)| c.clone())
          .collect();
        if !list.is_empty() {
          let dir = self.dir.clone();
          let mut journal = |c: &Commit| -> Result<()> {
            write_private(&dir, "unpublished.json", &serde_json::to_vec(c)?)
          };
          let result = self
            .client
            .commit_journaled(
              key,
              &mut self.device,
              &mut self.state.known,
              &list,
              &mut journal,
            )
            .await;
          if let Err(e) = result {
            self.state.repos.insert(repo.clone(), state);
            self.save_state()?;
            return Err(e);
          }
          summary.commits += 1;
        }
        for ((name, change), file, blob) in batch {
          if let Some(b) = blob {
            self.state.blobs.insert(b.clone());
            summary.blobs += 1;
          }
          match file {
            Some((path, hash)) if hash.is_empty() => {
              state.files.remove(path);
              summary.deleted += 1;
            }
            Some((path, hash)) => {
              state.files.insert(path.clone(), hash.clone());
              if !name.is_empty() && change.is_some() {
                summary.uploaded += 1;
              }
            }
            None => {}
          }
        }
        // Accepted: the commit is no longer waiting.
        self.device.set_unpublished(None);
        self.state.repos.insert(repo.clone(), state.clone());
        self.save_state()?;
      }
      // A repository whose blobs are incomplete is read again next time.
      if summary.missing_blobs.is_empty() {
        state.fingerprint = fingerprint;
      }
      self.state.repos.insert(repo, state);
      self.save_state()?;
    }
    summary.missing_blobs.sort();
    summary.missing_blobs.dedup();
    summary.at = now();
    self.state.last = Some(summary.clone());
    self.save_state()?;
    Ok(summary)
  }
  /// Recover into `staging` (new, or resumed) and write an inert preview into
  /// `preview` (new). With `kit`, recovery uses the kit (every device lost).
  #[allow(clippy::too_many_arguments)]
  pub async fn recover(
    &self,
    staging: &Path,
    preview: &Path,
    through: Option<u64>,
    resume: bool,
    phone: bool,
  ) -> Result<(Report, node::Preview)> {
    recover_with(
      &self.client,
      &self.credential(),
      &self.config.node,
      staging,
      preview,
      through,
      resume,
      phone,
    )
    .await
  }
}

/// Recovery with the kit alone (no device of the vault left): the kit signs the
/// requests and opens the keys.
#[allow(clippy::too_many_arguments)]
pub async fn recover_with_kit(
  endpoint: &str,
  kit: &Kit,
  node: &str,
  staging: &Path,
  preview: &Path,
  through: Option<u64>,
  resume: bool,
  phone: bool,
) -> Result<(Report, node::Preview)> {
  let client = Client::signed(endpoint, kit.vault(), kit.request_signer()?)?;
  recover_with(
    &client,
    &Credential::Kit(kit),
    node,
    staging,
    preview,
    through,
    resume,
    phone,
  )
  .await
}

#[allow(clippy::too_many_arguments)]
async fn recover_with(
  client: &Client,
  credential: &Credential<'_>,
  node: &str,
  staging: &Path,
  preview: &Path,
  through: Option<u64>,
  resume: bool,
  phone: bool,
) -> Result<(Report, node::Preview)> {
  let options = RecoverOptions {
    through,
    limits: if phone {
      Limits::phone()
    } else {
      Limits::default()
    },
    ..RecoverOptions::default()
  };
  let recovered = if resume {
    client.resume(credential, &options, staging).await?
  } else {
    client.recover(credential, &options, staging).await?
  };
  let p = node::preview(&recovered, preview, node)?;
  Ok((recovered.report().clone(), p))
}

/// Copy chosen records from an inert preview into a live record root: each ID's
/// record, and for a thread its transcript segments, plus the original blobs they
/// attach. IDs are the preview's (fresh) ones, kinds stay as the preview made them
/// (acting kinds are `inert-*`), so nothing imported starts work. Files already
/// present are left alone.
pub fn import(
  preview: &Path,
  ids: &[String],
  root: &RecordRoot,
  blobs: &BlobStore,
) -> Result<Imported> {
  let marker: serde_json::Value = serde_json::from_slice(&fs::read(preview.join("PREVIEW.json"))?)?;
  if marker["inert"] != serde_json::Value::Bool(true) {
    return Err(Error::Invalid("not a vault preview"));
  }
  let mut out = Imported::default();
  let repos: Vec<PathBuf> = fs::read_dir(preview)?
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "blobs"))
    .collect();
  for id in ids {
    let Ok(rid) = id.parse::<RecordId>() else {
      out.not_found.push(id.clone());
      continue;
    };
    let path = ItemPath::Record(rid.clone()).to_string();
    let Some(repo) = repos.iter().find(|r| r.join(&path).is_file()) else {
      out.not_found.push(id.clone());
      continue;
    };
    let bytes = fs::read(repo.join(&path))?;
    let record = Record::parse(&String::from_utf8_lossy(&bytes)).map_err(io)?;
    for a in &record.attachments {
      let file = preview.join("blobs").join(a.hash.hex());
      if let Ok(data) = fs::read(&file)
        && BlobHash::of(&data) == a.hash
        && !blobs.contains(&a.hash)
      {
        blobs.put_bytes(&data).map_err(io)?;
        out.blobs += 1;
      }
    }
    match root.insert(&path, &bytes).map_err(io)? {
      stitch_store::Inserted::New => out.records += 1,
      _ => out.present += 1,
    }
    let segments = repo.join("threads").join(id);
    if let Ok(entries) = fs::read_dir(&segments) {
      for e in entries.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        let seg = format!("threads/{id}/{name}");
        if ItemPath::parse(&seg).is_none() {
          continue;
        }
        match root.insert(&seg, &fs::read(e.path())?).map_err(io)? {
          stitch_store::Inserted::New => out.segments += 1,
          _ => out.present += 1,
        }
      }
    }
  }
  Ok(out)
}
