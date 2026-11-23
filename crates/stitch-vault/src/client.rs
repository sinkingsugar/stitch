use crate::*;
use futures_util::StreamExt;
use serde::Deserialize;
use std::{
  collections::{BTreeMap, HashMap, HashSet},
  fs,
  io::Write,
  path::{Path, PathBuf},
  time::Duration,
};
use zeroize::Zeroizing;

/// One name change in a batch: new content (kind and bytes), or `None` to delete.
pub type Change = (String, Option<(Kind, Vec<u8>)>);

/// Explicit authenticated transport, independent of node/P2P sync.
pub struct Client {
  http: reqwest::Client,
  base: String,
  vault: String,
  auth: Auth,
  pack_target: usize,
  repack_limit: u64,
}
/// The membership log's bounds, matching the service's budget (index.js: 6 MiB of
/// ordinary statements, 6 MiB signed by the kit, small cancel/freeze statements, at
/// most 3072 in all), so a stolen admin cannot grow the log past what clients read.
pub const MAX_LOG: usize = 3072;
pub const MAX_LOG_BYTES: usize = 24 << 20;
/// Live bytes one checkpoint copies out of sparse packs at most (the rest waits for the
/// next checkpoint): well inside the service's repack room (2% of a 100 GB quota).
pub const REPACK_LIMIT: u64 = 1 << 30;
/// How requests prove who sends them.
enum Auth {
  /// A bearer credential (the local prototype's enrolled tokens and bootstrap token).
  Bearer(Zeroizing<String>),
  /// Every request signed by a device key or the kit's anchor (production).
  Signed(RequestSigner),
}
/// Signs requests: `x-vault-auth: {public}:{unix ms}:{signature}` over
/// `stitch vault request v3\n{vault}\n{METHOD}\n{path?query}\n{unix ms}\n{body sha256 or empty}\n`.
pub struct RequestSigner {
  key: signature::Ed25519KeyPair,
}
impl RequestSigner {
  pub fn public(&self) -> String {
    hex::encode(self.key.public_key().as_ref())
  }
  /// The `x-vault-auth` value for one request (`path` below `/v1/vaults/{vault}`, with
  /// its query; `digest`: the body's SHA256 or empty). Times strictly increase in this
  /// process: the service spends each signed request once (replay protection), and
  /// Ed25519 signs the same text identically, so two identical requests in one
  /// millisecond must not share a time.
  pub fn header(&self, vault: &str, method: &str, path: &str, digest: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_or(0, |d| d.as_millis() as u64);
    let next = |last: u64| now.max(last.saturating_add(1));
    let at = next(
      LAST
        .fetch_update(SeqCst, SeqCst, |last| Some(next(last)))
        .unwrap_or_default(),
    );
    let text = format!(
      "stitch vault request v3\n{vault}\n{method}\n/v1/vaults/{vault}{path}\n{at}\n{digest}\n"
    );
    let signature = hex::encode(self.key.sign(text.as_bytes()).as_ref());
    format!("{}:{at}:{signature}", self.public())
  }
}
impl Device {
  pub fn request_signer(&self) -> Result<RequestSigner> {
    Ok(RequestSigner {
      key: signature::Ed25519KeyPair::from_pkcs8(self.pkcs8.as_ref()).map_err(|_| Error::Crypto)?,
    })
  }
}
impl Kit {
  /// The anchor signs requests that post membership statements (genesis, first admin).
  pub fn request_signer(&self) -> Result<RequestSigner> {
    Ok(RequestSigner {
      key: self.anchor()?,
    })
  }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogEntry {
  cursor: u64,
  // Parsed per commit, so one malformed commit is reported rather than fatal.
  commit: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
  head: u64,
  entries: Vec<LogEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Members {
  members: Vec<serde_json::Value>,
  // The service's view (clients recompute both from the verified log).
  #[serde(default)]
  #[allow(dead_code)]
  pending: Vec<serde_json::Value>,
  #[serde(default)]
  #[allow(dead_code)]
  deleted: bool,
}

/// What a recovery should reach and trust beyond the root key.
#[derive(Clone, Debug)]
pub struct RecoverOptions {
  /// Recover the state as of this global cursor (default: the current head).
  pub through: Option<u64>,
  /// A head the caller saw before: a log shorter than this is a rollback (error).
  pub minimum_cursor: u64,
  /// A membership log length the caller saw before (rollback of membership: error).
  pub minimum_members: u64,
  /// Device → commit sequence the caller knows it reached (e.g. its own device's
  /// durable head). A chain that stops short is reported as missing history.
  pub pins: BTreeMap<String, u64>,
  /// Bounds on the work and space a recovery may take.
  pub limits: Limits,
  /// Start from the newest valid checkpoint (default true; false replays the whole log,
  /// which fails for content whose packs GC has removed).
  pub checkpoints: bool,
}
/// Recovery stops with `Error::Limit` before exceeding any of these. They bound what a
/// misbehaving service or member can make a recovering device do, whatever the
/// payload size: commits read, manifest entries processed (including superseded and
/// deleted ones), name bytes held, bytes downloaded (manifests and objects, charged
/// before fetching) and plaintext bytes staged on disk.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
  pub commits: u64,
  /// Chunk locations held from pack indexes.
  pub chunks: u64,
  pub entries: u64,
  pub name_bytes: u64,
  /// Per run: a resumed recovery starts again from zero.
  pub download_bytes: u64,
  /// In all: what is already staged counts when resuming (a disk-space bound).
  pub staged_bytes: u64,
}
impl Limits {
  /// For phones and other small devices: bounded memory (about 1M names, 2M chunk
  /// locations) and 4 GiB downloaded per run, 8 GiB staged in all. A recovery that hits
  /// the download bound stops with `Error::Limit` and its staging directory is resumed
  /// by `Client::resume`, which does not fetch what is already staged: a large vault is
  /// recovered over several runs (or with the app suspended in between).
  pub fn phone() -> Self {
    Self {
      commits: 200_000,
      chunks: 2_000_000,
      entries: 1_000_000,
      name_bytes: 64 << 20,
      download_bytes: 4 << 30,
      staged_bytes: 8 << 30,
    }
  }
}
impl Default for Limits {
  /// Sized for the proposed 100 GB plan with headroom; phones use `Limits::phone`.
  fn default() -> Self {
    Self {
      commits: 1_000_000,
      chunks: 20_000_000,
      entries: 10_000_000,
      name_bytes: 1 << 30,
      download_bytes: 256 << 30,
      staged_bytes: 128 << 30,
    }
  }
}
#[derive(Default)]
pub(crate) struct Budget {
  limits: Limits,
  commits: u64,
  chunks: u64,
  entries: u64,
  name_bytes: u64,
  download_bytes: u64,
  staged_bytes: u64,
}
impl Budget {
  fn charge(used: &mut u64, amount: u64, limit: u64, what: &'static str) -> Result<()> {
    *used = used.saturating_add(amount);
    if *used > limit {
      return Err(Error::Limit(what));
    }
    Ok(())
  }
  fn commit(&mut self) -> Result<()> {
    Self::charge(&mut self.commits, 1, self.limits.commits, "commits")
  }
  fn chunk(&mut self) -> Result<()> {
    Self::charge(&mut self.chunks, 1, self.limits.chunks, "chunks")
  }
  fn entry(&mut self, name: &str) -> Result<()> {
    Self::charge(&mut self.entries, 1, self.limits.entries, "entries")?;
    Self::charge(
      &mut self.name_bytes,
      name.len() as u64,
      self.limits.name_bytes,
      "name bytes",
    )
  }
  fn download(&mut self, size: u64) -> Result<()> {
    Self::charge(
      &mut self.download_bytes,
      size,
      self.limits.download_bytes,
      "download bytes",
    )
  }
  fn stage(&mut self, size: u64) -> Result<()> {
    Self::charge(
      &mut self.staged_bytes,
      size,
      self.limits.staged_bytes,
      "staged bytes",
    )
  }
}
impl Default for RecoverOptions {
  fn default() -> Self {
    Self {
      through: None,
      minimum_cursor: 0,
      minimum_members: 0,
      pins: BTreeMap::new(),
      limits: Limits::default(),
      checkpoints: true,
    }
  }
}
/// One problem found, attributed to a cursor/device/name where it has one.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Issue {
  pub cursor: u64,
  pub device: String,
  pub name: String,
  pub reason: String,
}
/// What recovery accepted and what it could not, instead of failing the vault.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
  /// The global cursor recovered through.
  pub cursor: u64,
  /// Verified membership statements used.
  pub members: u64,
  /// The checkpoint recovery started from, if any.
  pub checkpoint: Option<u64>,
  /// Commits accepted into device chains.
  pub accepted: u64,
  /// Commits not applied: unauthentic, from non-members, after revocation, or
  /// replayed/forked/gapped chains (everything after a break in a device's chain).
  pub rejected: Vec<Issue>,
  /// Authentic commits whose manifests could not be read or were invalid: their
  /// changes are not applied, so later state may be stale.
  pub incomplete: Vec<Issue>,
  /// History that should exist and does not: pinned or revoked devices whose chain
  /// stops short, and live entries whose objects are unavailable.
  pub missing: Vec<Issue>,
}
impl Report {
  pub fn complete(&self) -> bool {
    self.rejected.is_empty() && self.incomplete.is_empty() && self.missing.is_empty()
  }
}

/// A live name's verified plaintext, staged once per object.
#[derive(Clone, Debug, Serialize)]
pub struct Staged {
  /// The item's content key: also the staged file's name (`items/<key>`), never the
  /// entry name.
  pub item: String,
  pub kind: Kind,
  /// Plaintext bytes.
  pub size: u64,
  /// The cursor of the commit that set it.
  pub cursor: u64,
}
/// A recovery staged on disk: `items/<key>` (verified plaintext, owner-only) and
/// `index.json` (report and entries). Plain inert files: no store import, scheduling,
/// keychain or execution effects; names are never used as paths. A separate, future
/// preview/import step is the only way anything leaves the staging area.
pub struct Recovered {
  staging: PathBuf,
  report: Report,
  entries: BTreeMap<String, Staged>,
}
impl Recovered {
  #[cfg(test)]
  pub(crate) fn for_test(staging: PathBuf, entries: BTreeMap<String, Staged>) -> Self {
    Self {
      staging,
      report: Report::default(),
      entries,
    }
  }
  pub fn cursor(&self) -> u64 {
    self.report.cursor
  }
  pub fn report(&self) -> &Report {
    &self.report
  }
  pub fn staging(&self) -> &Path {
    &self.staging
  }
  /// Live names (tombstoned names are absent).
  pub fn entries(&self) -> &BTreeMap<String, Staged> {
    &self.entries
  }
  /// The staged plaintext of a live name.
  pub fn read(&self, name: &str) -> Result<Vec<u8>> {
    let staged = self
      .entries
      .get(name)
      .ok_or(Error::Invalid("no such entry"))?;
    Ok(fs::read(self.staging.join("items").join(&staged.item))?)
  }
}
/// A staging directory's target, written before any item: a resumed run recovers the
/// same cursor.
const PROGRESS: &str = "progress.json";
/// One line per staged item (`{"item", "sha256"}`), appended and synced after its
/// file is in place: what a resumed run may keep.
const JOURNAL: &str = "staged.jsonl";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
  version: u32,
  vault: String,
  cursor: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journalled {
  item: String,
  sha256: String,
}
/// Owner-only directory that must not exist yet: recovery never merges into or
/// overwrites anything.
fn private_dir(path: &Path) -> Result<()> {
  let mut builder = fs::DirBuilder::new();
  #[cfg(unix)]
  std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
  builder.create(path)?;
  Ok(())
}
/// Write then rename, owner-only, so a crash never leaves a partial file under its final name.
fn private_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
  let partial = dir.join(format!(".{name}.partial"));
  let mut options = fs::OpenOptions::new();
  options.write(true).create_new(true);
  #[cfg(unix)]
  std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
  let mut file = options.open(&partial)?;
  file.write_all(bytes)?;
  file.sync_all()?;
  fs::rename(partial, dir.join(name))?;
  Ok(())
}

/// Chunk ID -> (pack, epoch, location), from the indexes of accepted commits' packs.
/// Every known location: a chunk may sit in several packs (concurrent writers, repacks).
pub(crate) type Chunks = HashMap<String, Vec<(String, u32, Located)>>;
/// What a scan found: the verified live state and where its chunks are.
pub(crate) struct Scan {
  pub report: Report,
  pub keys: Keyring,
  /// Name → (item or tombstone, cursor that set it).
  pub state: BTreeMap<String, (Option<Item>, u64)>,
  pub chunks: Chunks,
  /// Every pack the scanned commits introduced.
  pub refs: HashMap<String, PackRef>,
  /// The verified membership log's length and head (what a new checkpoint names).
  pub members: u64,
  pub members_head: String,
}
/// Whether a checkpoint's signer was the anchor or a current admin at the membership
/// position it names (within the verified prefix of `log`, of length `verified`).
fn checkpoint_signer_ok(log: &[Statement], c: &Checkpoint, verified: u64) -> bool {
  if c.members == 0 || c.members > verified {
    return false;
  }
  let prefix = &log[..c.members as usize];
  let Some(last) = prefix.last() else {
    return false;
  };
  let (m, refused) = Membership::verify_at(&c.vault, prefix, last.at);
  refused.is_none() && m.head == c.members_head && m.admin_ok(&c.signer)
}
struct Context<'a> {
  keys: &'a Keyring,
  chunks: &'a mut Chunks,
  refs: &'a HashMap<String, PackRef>,
  indexed: &'a mut HashSet<String>,
  /// Packs a starting checkpoint vouches for.
  protected: &'a HashSet<String>,
  budget: &'a mut Budget,
}
fn issue(cursor: u64, device: &str, name: &str, reason: impl Into<String>) -> Issue {
  Issue {
    cursor,
    device: device.into(),
    name: name.into(),
    reason: reason.into(),
  }
}

impl Client {
  pub fn new(endpoint: &str, vault: &str, token: String) -> Result<Self> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| Error::Invalid("endpoint"))?;
    if !is_hex(vault, 16)
      || !url.username().is_empty()
      || url.password().is_some()
      || url.query().is_some()
      || url.fragment().is_some()
      || url.path() != "/"
      || !(url.scheme() == "https"
        || (url.scheme() == "http"
          && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
    {
      return Err(Error::Invalid("HTTPS endpoint required (except loopback)"));
    }
    // Like stitch-providers: standalone clients must also have a TLS provider.
    // Respect one already selected by the embedding host.
    let _ = rustls::crypto::ring::default_provider().install_default();
    Ok(Self {
      http: reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?,
      base: format!("{}/v1/vaults/{vault}", endpoint.trim_end_matches('/')),
      vault: vault.into(),
      auth: Auth::Bearer(Zeroizing::new(token)),
      pack_target: PACK_TARGET,
      repack_limit: REPACK_LIMIT,
    })
  }
  /// A client whose every request is signed (no bearer credential exists).
  pub fn signed(endpoint: &str, vault: &str, signer: RequestSigner) -> Result<Self> {
    let mut client = Self::new(endpoint, vault, String::new())?;
    client.auth = Auth::Signed(signer);
    Ok(client)
  }
  pub fn pack_target(&self) -> usize {
    self.pack_target
  }
  /// Pack size for uploads: 16 MiB by default; 4–8 MiB suits phones and metered links.
  pub fn repack_limit(&self) -> u64 {
    self.repack_limit
  }
  /// At most this many live bytes copied by one checkpoint's repacking (sparsest
  /// packs first; forced packs always).
  pub fn with_repack_limit(mut self, bytes: u64) -> Self {
    self.repack_limit = bytes;
    self
  }
  pub fn with_pack_target(mut self, bytes: usize) -> Self {
    self.pack_target = bytes.clamp(1 << 20, 32 << 20);
    self
  }
  pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    self.request_with(method, path, None)
  }
  /// A request with an optional body; the body's SHA256 goes in `x-content-sha256` and,
  /// when signing, into the signature, so the service can check the body it reads.
  pub(crate) fn request_with(
    &self,
    method: reqwest::Method,
    path: &str,
    body: Option<Vec<u8>>,
  ) -> reqwest::RequestBuilder {
    let digest = body.as_deref().map(sha256).unwrap_or_default();
    let mut builder = self
      .http
      .request(method.clone(), format!("{}{path}", self.base));
    builder = match &self.auth {
      Auth::Bearer(token) => builder.bearer_auth(token.as_str()),
      Auth::Signed(signer) => builder.header(
        "x-vault-auth",
        signer.header(&self.vault, method.as_str(), path, &digest),
      ),
    };
    if let Some(body) = body {
      builder = builder.header("x-content-sha256", digest).body(body);
    }
    builder
  }
  pub(crate) async fn bounded(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
      return Err(Error::Status(response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
      let chunk = chunk?;
      if chunk.len() > limit.saturating_sub(bytes.len()) {
        return Err(Error::Invalid("response too large"));
      }
      bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
  }
  /// Upload one sealed pack (create-only; the same pack again is a retry).
  pub async fn put_pack(&self, pack: &Pack) -> Result<()> {
    self.put_pack_in(pack, false).await
  }
  /// Upload one pack; `repack`: an admin's checkpoint repacking, which may use the
  /// service's repack room above the quota.
  pub(crate) async fn put_pack_in(&self, pack: &Pack, repack: bool) -> Result<()> {
    let room = if repack { "?room=repack" } else { "" };
    let mut attempt = 0;
    loop {
      let response = self
        .request_with(
          reqwest::Method::PUT,
          &format!("/objects/{}{room}", pack.reference.id),
          Some(pack.bytes.clone()),
        )
        .send()
        .await?;
      // 503: a reservation for this ID is in flight; it settles within its lease.
      if response.status().as_u16() != 503 || attempt == 5 {
        Self::bounded(response, 1024).await?;
        return Ok(());
      }
      attempt += 1;
      tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
    }
  }
  /// Pack, upload and publish one batch of name changes (`None` deletes a name) as the
  /// device's next commit. `known` holds chunk IDs already in the vault; it is updated
  /// only when the commit is accepted, so a failed batch never claims chunks that
  /// were not stored.
  pub async fn commit(
    &self,
    key: &VaultKey,
    device: &mut Device,
    known: &mut Known,
    changes: &[Change],
  ) -> Result<(u64, Commit)> {
    self
      .commit_journaled(key, device, known, changes, &mut |_| Ok(()))
      .await
  }
  /// `commit`, with `journal` called with the signed commit before it is published, so
  /// a caller can make it durable: after a crash it publishes the same commit again
  /// (idempotent) instead of signing a different one at the same sequence (a fork).
  pub async fn commit_journaled(
    &self,
    key: &VaultKey,
    device: &mut Device,
    known: &mut Known,
    changes: &[Change],
    journal: &mut (dyn FnMut(&Commit) -> Result<()> + Send),
  ) -> Result<(u64, Commit)> {
    if key.vault() != self.vault {
      return Err(Error::Invalid("client/key vault mismatch"));
    }
    // A commit whose publication failed before: the same commit again, first.
    if let Some(waiting) = device.unpublished.take() {
      match self.publish(&waiting).await {
        Ok(_) => {}
        Err(Error::Status(409)) => device.resume(waiting.sequence - 1, &waiting.previous)?,
        Err(e) => {
          device.unpublished = Some(waiting);
          return Err(e);
        }
      }
    }
    let mut seen = known.clone();
    let mut packer = Packer::with_target(key, &mut seen, self.pack_target);
    let mut entries = vec![];
    for (name, change) in changes {
      let item = match change {
        Some((kind, bytes)) => Some(packer.add(*kind, bytes)?),
        None => None,
      };
      entries.push(Entry {
        name: name.clone(),
        item,
      });
    }
    let (manifest, packs, uses) = packer.finish(&Manifest {
      version: VERSION,
      entries,
    })?;
    for pack in &packs {
      self.put_pack(pack).await?;
    }
    let (sequence, previous) = (device.head().0, device.head().1.to_string());
    let commit = device.commit(
      key.vault(),
      &manifest,
      packs.into_iter().map(|p| p.reference).collect(),
      uses,
    )?;
    if let Err(e) = journal(&commit) {
      device.resume(sequence, &previous)?;
      return Err(e);
    }
    let cursor = match self.publish(&commit).await {
      Ok(cursor) => cursor,
      Err(e) => {
        // 409 is a definitive refusal (e.g. a reused pack was collected): the commit
        // was never accepted, so the chain continues from the previous head. Any other
        // failure may hide an acceptance: keep the head and retry the same commit.
        if matches!(e, Error::Status(409)) {
          device.resume(sequence, &previous)?;
        } else {
          device.unpublished = Some(commit);
        }
        return Err(e);
      }
    };
    *known = seen;
    Ok((cursor, commit))
  }
  /// One ranged read of exactly `length` bytes.
  pub(crate) async fn range(
    &self,
    pack: &str,
    offset: u64,
    length: u64,
    budget: &mut Budget,
  ) -> Result<Vec<u8>> {
    if length == 0 {
      return Err(Error::Invalid("empty range"));
    }
    budget.download(length)?;
    let response = self
      .request(reqwest::Method::GET, &format!("/objects/{pack}"))
      .header("range", format!("bytes={offset}-{}", offset + length - 1))
      .send()
      .await?;
    if response.status().as_u16() != 206 {
      return Err(Error::Status(response.status().as_u16()));
    }
    let bytes = Self::bounded(response, length as usize).await?;
    if bytes.len() as u64 != length {
      return Err(Error::Invalid("short range"));
    }
    Ok(bytes)
  }
  /// Publish a commit; retrying the same commit is idempotent (same cursor).
  pub async fn publish(&self, commit: &Commit) -> Result<u64> {
    commit.verify(&self.vault)?;
    let bytes = serde_json::to_vec(commit)?;
    if bytes.len() > MAX_COMMIT {
      return Err(Error::Invalid("commit too large"));
    }
    let response = self
      .request_with(reqwest::Method::POST, "/commits", Some(bytes))
      .header("content-type", "application/json")
      .send()
      .await?;
    #[derive(Deserialize)]
    struct Receipt {
      cursor: u64,
    }
    let cursor = serde_json::from_slice::<Receipt>(&Self::bounded(response, 1024).await?)?.cursor;
    if cursor == 0 {
      return Err(Error::Invalid("publication receipt"));
    }
    Ok(cursor)
  }
  /// The membership log as the service stores it (unverified).
  pub async fn statements(&self) -> Result<Vec<Statement>> {
    let response = self
      .request(reqwest::Method::GET, "/members")
      .send()
      .await?;
    let log: Members = serde_json::from_slice(&Self::bounded(response, MAX_LOG_BYTES).await?)?;
    if log.members.len() > MAX_LOG {
      return Err(Error::Invalid("membership log too long"));
    }
    // A malformed statement ends the usable prefix, like an invalid one.
    Ok(
      log
        .members
        .into_iter()
        .map_while(|v| serde_json::from_value(v).ok())
        .collect(),
    )
  }
  /// Append a signed statement (posted by an admin device's credential; the service
  /// checks it and stores it, it never authors one).
  pub async fn post_statement(&self, statement: &Statement) -> Result<()> {
    let response = self
      .request_with(
        reqwest::Method::POST,
        "/members",
        Some(serde_json::to_vec(statement)?),
      )
      .header("content-type", "application/json")
      .send()
      .await?;
    Self::bounded(response, 1024).await?;
    Ok(())
  }
  /// Enroll a device's service credential (`{token_hash, role}`), by an admin
  /// device's credential (or the local bootstrap token for the first admin).
  pub async fn put_device(&self, device: &str, enrollment: &serde_json::Value) -> Result<()> {
    let response = self
      .request_with(
        reqwest::Method::PUT,
        &format!("/devices/{device}"),
        Some(serde_json::to_vec(enrollment)?),
      )
      .header("content-type", "application/json")
      .send()
      .await?;
    Self::bounded(response, 1024).await?;
    Ok(())
  }
  /// Move the service's clock (local prototype with TEST_CLOCK only; bootstrap token).
  pub async fn set_clock(&self, offset_ms: i64) -> Result<()> {
    let response = self
      .request_with(
        reqwest::Method::POST,
        "/clock",
        Some(serde_json::to_vec(
          &serde_json::json!({"offset_ms": offset_ms}),
        )?),
      )
      .header("content-type", "application/json")
      .send()
      .await?;
    Self::bounded(response, 1024).await?;
    Ok(())
  }
  /// Verify the log and open this credential's envelopes.
  pub async fn keyring(
    &self,
    credential: &Credential<'_>,
  ) -> Result<(Keyring, Membership, Option<String>)> {
    let log = self.statements().await?;
    let (ring, membership, refused) = members::keyring(&log, credential)?;
    if ring.vault() != self.vault {
      return Err(Error::Invalid("client/key vault mismatch"));
    }
    Ok((ring, membership, refused))
  }
  /// Verify membership and the commit log through a cursor, starting from the newest
  /// valid checkpoint, and return the live state (no item content fetched).
  pub(crate) async fn scan(
    &self,
    credential: &Credential<'_>,
    options: &RecoverOptions,
    budget: &mut Budget,
  ) -> Result<Scan> {
    let mut report = Report::default();
    let log = self.statements().await?;
    let (keys, membership, refused) = members::keyring(&log, credential)?;
    if keys.vault() != self.vault {
      return Err(Error::Invalid("client/key vault mismatch"));
    }
    if let Some(reason) = refused {
      report.rejected.push(issue(0, "", "", reason));
    }
    report.members = membership.seq;
    let members = &membership.devices;
    if report.members < options.minimum_members {
      return Err(Error::Invalid("membership rollback below pinned length"));
    }
    // Device → (sequence, hash) of its last accepted commit.
    let mut heads: BTreeMap<String, (u64, String)> = BTreeMap::new();
    // Name → (latest item or tombstone, cursor that set it).
    let mut state: BTreeMap<String, (Option<Item>, u64)> = BTreeMap::new();
    // Chunk ID → (pack, location), from the indexes of accepted commits' packs.
    let mut chunks: Chunks = HashMap::new();
    // Every pack any commit introduced (metadata only), to read `uses` indexes on demand.
    let mut refs: HashMap<String, PackRef> = HashMap::new();
    let mut indexed: HashSet<String> = HashSet::new();
    // Start from the newest valid checkpoint at or below the target, if any.
    let mut from = 0u64;
    let mut protected: HashSet<String> = HashSet::new();
    if options.checkpoints {
      // A checkpoint counts if its signer was an admin at the membership position it
      // names. Those whose signer is still current come first (newest first), then the
      // others: after a lost admin is revoked its checkpoints still serve, but a new
      // admin's checkpoint is preferred once there is one.
      let mut list = self.checkpoints(options.through).await?;
      list.sort_by_key(|c| (!membership.admin_ok(&c.signer), std::cmp::Reverse(c.cursor)));
      for c in list {
        if options.through.is_some_and(|t| c.cursor > t) {
          continue;
        }
        let signer_ok = checkpoint_signer_ok(&log, &c, membership.seq);
        let mut seeded = HashMap::new();
        let result = async {
          c.verify(keys.vault())?;
          if !signer_ok {
            return Err(Error::Invalid("checkpoint signer is not an admin"));
          }
          let mut state = BTreeMap::new();
          for pack in &c.packs {
            self.index(&keys, pack, &mut seeded, budget).await?;
          }
          let (pack, epoch, at) = seeded
            .get(&c.manifest)
            .and_then(|v| v.first())
            .cloned()
            .ok_or(Error::Invalid("checkpoint manifest"))?;
          let sealed = self.range(&pack, at.offset, at.length, budget).await?;
          let manifest: Manifest =
            serde_json::from_slice(&keys.get(epoch)?.open_chunk(epoch, &c.manifest, &sealed)?)?;
          for entry in manifest.entries {
            budget.entry(&entry.name)?;
            let Some(item) = entry.item else { continue };
            if item.chunks.iter().any(|ch| !seeded.contains_key(ch)) {
              return Err(Error::Invalid("checkpoint closure"));
            }
            state.insert(entry.name, (Some(item), c.cursor));
          }
          Ok(state)
        }
        .await;
        match result {
          Ok(seed) => {
            state = seed;
            chunks = seeded;
            from = c.cursor;
            indexed = c.packs.iter().map(|p| p.id.clone()).collect();
            for p in &c.packs {
              refs.insert(p.id.clone(), p.clone());
            }
            protected = indexed.clone();
            report.checkpoint = Some(c.cursor);
            break;
          }
          Err(e @ Error::Limit(_)) => return Err(e),
          Err(e) => {
            report
              .incomplete
              .push(issue(c.cursor, &c.signer, "", format!("checkpoint: {e}")))
          }
        }
      }
    }
    let mut target = options.through;
    let mut cursor = 0u64;
    loop {
      let through = target
        .map(|head| format!("&through={head}"))
        .unwrap_or_default();
      let response = self
        .request(
          reqwest::Method::GET,
          &format!("/commits?after={cursor}{through}"),
        )
        .send()
        .await?;
      let page: Page = serde_json::from_slice(&Self::bounded(response, 100 * MAX_COMMIT).await?)?;
      if page.entries.len() > 100
        || page.head < cursor
        || target.is_some_and(|head| head != page.head)
      {
        return Err(Error::Invalid("page bounds/head"));
      }
      target = Some(page.head);
      if page.entries.is_empty() {
        if cursor != page.head {
          return Err(Error::Invalid("truncated prefix"));
        }
        break;
      }
      for log in page.entries {
        if log.cursor != cursor + 1 || log.cursor > page.head {
          return Err(Error::Invalid("nonconsecutive cursor"));
        }
        cursor = log.cursor;
        budget.commit()?;
        let Ok(commit) = serde_json::from_value::<Commit>(log.commit) else {
          report
            .rejected
            .push(issue(cursor, "", "", "malformed commit"));
          continue;
        };
        let device = commit.device.clone();
        let reject = |report: &mut Report, reason: &str| {
          report.rejected.push(issue(cursor, &device, "", reason));
        };
        if commit.verify(keys.vault()).is_err() {
          reject(&mut report, "bad signature");
          continue;
        }
        let Some(member) = members.get(&device) else {
          reject(&mut report, "not a member");
          continue;
        };
        if !member.writes() {
          reject(&mut report, "not a writer");
          continue;
        }
        if member.through.is_some_and(|t| commit.sequence > t) {
          reject(&mut report, "after revocation");
          continue;
        }
        if member.frozen.is_some() && commit.sequence > member.frozen_through {
          reject(&mut report, "frozen");
          continue;
        }
        let (last, last_hash) = heads.get(&device).cloned().unwrap_or((0, ZERO.to_string()));
        if commit.sequence <= last {
          reject(&mut report, "replayed or forked");
          continue;
        }
        if commit.sequence > last + 1 {
          reject(&mut report, "sequence gap");
          continue;
        }
        if commit.previous != last_hash {
          reject(&mut report, "broken chain");
          continue;
        }
        heads.insert(device.clone(), (commit.sequence, commit.hash()?));
        report.accepted += 1;
        for p in &commit.packs {
          refs.insert(p.id.clone(), p.clone());
        }
        // Covered by the checkpoint: chains are verified, content is already in it.
        if cursor <= from {
          continue;
        }
        let mut ctx = Context {
          keys: &keys,
          chunks: &mut chunks,
          refs: &refs,
          indexed: &mut indexed,
          protected: &protected,
          budget,
        };
        match self.changes(&commit, &mut ctx).await {
          Ok(changes) => {
            for (name, item) in changes {
              state.insert(name, (item, cursor));
            }
          }
          Err(e @ Error::Limit(_)) => return Err(e),
          Err(e) => report
            .incomplete
            .push(issue(cursor, &device, "", format!("content: {e}"))),
        }
      }
    }
    report.cursor = cursor;
    if cursor < options.minimum_cursor {
      return Err(Error::Invalid("rollback below pinned cursor"));
    }
    let head = |d: &str| heads.get(d).map_or(0, |h| h.0);
    for (device, pin) in &options.pins {
      if head(device) < *pin {
        report.missing.push(issue(
          cursor,
          device,
          "",
          format!("pinned device behind: {} < {pin}", head(device)),
        ));
      }
    }
    for (device, m) in members {
      if let Some(t) = m.through
        && head(device) < t
      {
        report.missing.push(issue(
          cursor,
          device,
          "",
          format!("revoked device behind: {} < {t}", head(device)),
        ));
      }
    }
    if from > cursor {
      return Err(Error::Invalid("checkpoint beyond the log"));
    }
    Ok(Scan {
      report,
      keys,
      state,
      chunks,
      refs,
      members: membership.seq,
      members_head: membership.head.clone(),
    })
  }
  /// Reconstruct the state as of a global cursor from verified device chains into a
  /// new staging directory (which must not exist). Problems with individual commits
  /// or objects are reported, not fatal; transport failures, malformed pages, rollback
  /// below the caller's pins and exceeded limits are errors. Kit-only recovery cannot
  /// prove freshness beyond the pins it is given. On error the staging directory may
  /// hold verified plaintext and has no `index.json`: the caller removes it.
  pub async fn recover(
    &self,
    credential: &Credential<'_>,
    options: &RecoverOptions,
    staging: &Path,
  ) -> Result<Recovered> {
    self.stage(credential, options, staging, false).await
  }
  /// Continue a recovery into `staging` that stopped before it finished (a limit, the
  /// network, the process killed): the same cursor (from its `progress.json`, so the
  /// state is the one the first run chose), items already staged and verified against
  /// their journalled digest are kept, the rest fetched. A finished staging directory
  /// (with `index.json`) is refused.
  pub async fn resume(
    &self,
    credential: &Credential<'_>,
    options: &RecoverOptions,
    staging: &Path,
  ) -> Result<Recovered> {
    self.stage(credential, options, staging, true).await
  }
  async fn stage(
    &self,
    credential: &Credential<'_>,
    options: &RecoverOptions,
    staging: &Path,
    resume: bool,
  ) -> Result<Recovered> {
    let items = staging.join("items");
    let mut options = options.clone();
    let mut done: HashMap<String, String> = HashMap::new();
    if resume {
      if staging.join("index.json").exists() {
        return Err(Error::Invalid("staging already complete"));
      }
      let progress: Progress = serde_json::from_slice(&fs::read(staging.join(PROGRESS))?)?;
      if progress.version != VERSION || progress.vault != self.vault {
        return Err(Error::Invalid("staging of another vault or version"));
      }
      options.through = Some(progress.cursor);
      options.minimum_cursor = options.minimum_cursor.max(progress.cursor);
      // Partial writes never became items; journal lines are whole or ignored.
      for e in fs::read_dir(&items)? {
        let e = e?;
        if e.file_name().to_string_lossy().ends_with(".partial") {
          fs::remove_file(e.path())?;
        }
      }
      let journal = fs::read(staging.join(JOURNAL)).unwrap_or_default();
      for line in journal.split(|b| *b == b'\n') {
        if let Ok(j) = serde_json::from_slice::<Journalled>(line) {
          done.insert(j.item, j.sha256);
        }
      }
    }
    let mut budget = Budget {
      limits: options.limits,
      ..Budget::default()
    };
    let Scan {
      mut report,
      keys,
      state,
      chunks,
      ..
    } = self.scan(credential, &options, &mut budget).await?;
    if resume && Some(report.cursor) != options.through {
      return Err(Error::Invalid("resumed recovery reached another cursor"));
    }
    // Stage the live state: each item fetched, verified and written once (the staged
    // set is the verified-item cache); superseded and deleted items are never fetched.
    if !resume {
      private_dir(staging)?;
      private_dir(&items)?;
      let progress = Progress {
        version: VERSION,
        vault: self.vault.clone(),
        cursor: report.cursor,
      };
      private_file(staging, PROGRESS, &serde_json::to_vec(&progress)?)?;
    }
    let mut journal = {
      let mut o = fs::OpenOptions::new();
      o.append(true).create(true);
      #[cfg(unix)]
      std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
      o.open(staging.join(JOURNAL))?
    };
    let mut entries = BTreeMap::new();
    let mut staged: HashSet<String> = HashSet::new();
    let mut failed: HashSet<String> = HashSet::new();
    for (name, (item, set_at)) in state {
      let Some(item) = item else { continue };
      let id = item.key();
      if failed.contains(&id) {
        report
          .missing
          .push(issue(set_at, "", &name, "item unavailable"));
        continue;
      }
      if !staged.contains(&id) {
        // Staged by an earlier run: kept if its bytes still match the journal.
        if let Some(digest) = done.get(&id) {
          match fs::read(items.join(&id)) {
            Ok(bytes) if bytes.len() as u64 == item.size && sha256(&bytes) == *digest => {
              budget.stage(bytes.len() as u64)?;
              staged.insert(id.clone());
            }
            _ => {
              let _ = fs::remove_file(items.join(&id));
            }
          }
        }
      }
      if !staged.contains(&id) {
        match self.fetch_item(&keys, &item, &chunks, &mut budget).await {
          Ok(bytes) => {
            budget.stage(bytes.len() as u64)?;
            private_file(&items, &id, &bytes)?;
            let line = serde_json::to_vec(&Journalled {
              item: id.clone(),
              sha256: sha256(&bytes),
            })?;
            journal.write_all(&[line.as_slice(), b"\n"].concat())?;
            journal.sync_data()?;
            staged.insert(id.clone());
          }
          Err(e @ Error::Limit(_)) => return Err(e),
          Err(e) => {
            failed.insert(id);
            report
              .missing
              .push(issue(set_at, "", &name, format!("item unavailable: {e}")));
            continue;
          }
        }
      }
      entries.insert(
        name,
        Staged {
          item: id,
          kind: item.kind,
          size: item.size,
          cursor: set_at,
        },
      );
    }
    let index = serde_json::json!({"version": VERSION, "complete": report.complete(), "report": &report, "entries": &entries});
    private_file(staging, "index.json", &serde_json::to_vec_pretty(&index)?)?;
    Ok(Recovered {
      staging: staging.into(),
      report,
      entries,
    })
  }
  /// The protected checkpoints the service keeps (unverified; recovery verifies).
  pub async fn checkpoints(&self, through: Option<u64>) -> Result<Vec<Checkpoint>> {
    let path = match through {
      Some(t) => format!("/checkpoints?through={t}"),
      None => "/checkpoints".into(),
    };
    let response = self.request(reqwest::Method::GET, &path).send().await?;
    #[derive(Deserialize)]
    struct List {
      checkpoints: Vec<serde_json::Value>,
    }
    let list: List = serde_json::from_slice(&Self::bounded(response, 64 << 20).await?)?;
    let mut out: Vec<Checkpoint> = list
      .checkpoints
      .into_iter()
      .filter_map(|v| serde_json::from_value(v).ok())
      .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.cursor));
    Ok(out)
  }
  /// Read one pack's index (a ranged read) and record its chunks' locations.
  pub(crate) async fn index(
    &self,
    keys: &Keyring,
    pack: &PackRef,
    chunks: &mut Chunks,
    budget: &mut Budget,
  ) -> Result<()> {
    let sealed = self
      .range(&pack.id, pack.index_offset, pack.index_length, budget)
      .await?;
    for located in keys.get(pack.epoch)?.open_index(pack, &sealed)? {
      budget.chunk()?;
      chunks
        .entry(located.id.clone())
        .or_default()
        .push((pack.id.clone(), pack.epoch, located));
    }
    Ok(())
  }
  /// The name changes one accepted commit declares: read the indexes of its packs and
  /// of the packs it `uses` (one ranged read each, once), then its manifest. Every
  /// chunk an item names must lie in one of those packs (or a starting checkpoint's).
  async fn changes(
    &self,
    commit: &Commit,
    ctx: &mut Context<'_>,
  ) -> Result<Vec<(String, Option<Item>)>> {
    for pack in &commit.packs {
      if ctx.indexed.insert(pack.id.clone()) {
        self.index(ctx.keys, pack, ctx.chunks, ctx.budget).await?;
      }
    }
    for id in &commit.uses {
      if !ctx.indexed.contains(id) {
        let pack = ctx
          .refs
          .get(id)
          .ok_or(Error::Invalid("uses an unknown pack"))?;
        self.index(ctx.keys, pack, ctx.chunks, ctx.budget).await?;
        ctx.indexed.insert(id.clone());
      }
    }
    let allowed = |pack: &str| {
      commit.packs.iter().any(|p| p.id == pack)
        || commit.uses.iter().any(|u| u == pack)
        || ctx.protected.contains(pack)
    };
    let (pack, epoch, at) = ctx
      .chunks
      .get(&commit.manifest)
      .and_then(|v| {
        v.iter()
          .find(|(p, _, _)| commit.packs.iter().any(|c| c.id == *p))
      })
      .cloned()
      .ok_or(Error::Invalid("manifest not in its commit's packs"))?;
    let sealed = self.range(&pack, at.offset, at.length, ctx.budget).await?;
    let manifest: Manifest = serde_json::from_slice(&ctx.keys.get(epoch)?.open_chunk(
      epoch,
      &commit.manifest,
      &sealed,
    )?)?;
    if manifest.version != VERSION {
      return Err(Error::Invalid("manifest"));
    }
    let mut changes = vec![];
    let mut names = HashSet::new();
    for entry in manifest.entries {
      ctx.budget.entry(&entry.name)?;
      if entry.name.is_empty()
        || entry.name.len() > MAX_NAME
        || !names.insert(entry.name.clone())
        || entry.item.as_ref().is_some_and(|i| {
          i.chunks.is_empty() && i.size != 0
            || i.chunks.iter().any(|c| {
              !ctx
                .chunks
                .get(c)
                .is_some_and(|v| v.iter().any(|(p, _, _)| allowed(p)))
            })
        })
      {
        return Err(Error::Invalid("manifest closure"));
      }
      changes.push((entry.name, entry.item));
    }
    Ok(changes)
  }
  /// An item's plaintext: its chunks in order, consecutive chunks of one pack in one
  /// ranged read, each chunk authenticated against its keyed ID.
  async fn fetch_item(
    &self,
    keys: &Keyring,
    item: &Item,
    chunks: &Chunks,
    budget: &mut Budget,
  ) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(item.size.min(64 << 20) as usize);
    let mut i = 0;
    while i < item.chunks.len() {
      let (pack, epoch, first) = chunks
        .get(&item.chunks[i])
        .and_then(|v| v.first())
        .ok_or(Error::Invalid("chunk location"))?;
      let key = keys.get(*epoch)?;
      let mut end = first.offset + first.length;
      let mut run = vec![(&item.chunks[i], first)];
      while let Some((p, _, next)) = item
        .chunks
        .get(i + run.len())
        .and_then(|c| chunks.get(c))
        .and_then(|v| v.first())
      {
        if p != pack || next.offset != end || end - first.offset >= 32 << 20 {
          break;
        }
        end = next.offset + next.length;
        run.push((&item.chunks[i + run.len()], next));
      }
      // The run in one ranged read; if it fails (a damaged or unavailable pack, or an
      // index that claims a chunk it does not hold), each chunk is tried at every
      // location known for it, so one bad copy does not hide a good one.
      let read = self
        .range(pack, first.offset, end - first.offset, budget)
        .await;
      let opened: Result<Vec<Vec<u8>>> = match read {
        Ok(bytes) => run
          .iter()
          .map(|(id, at)| {
            let from = (at.offset - first.offset) as usize;
            key.open_chunk(*epoch, id, &bytes[from..from + at.length as usize])
          })
          .collect(),
        Err(e @ Error::Limit(_)) => return Err(e),
        Err(e) => Err(e),
      };
      let opened = match opened {
        Ok(v) => v,
        Err(first_error) => {
          let mut v = vec![];
          for (id, _) in &run {
            let mut found = None;
            for (p, e, at) in chunks.get(*id).into_iter().flatten() {
              let Ok(k) = keys.get(*e) else { continue };
              match self.range(p, at.offset, at.length, budget).await {
                Ok(b) => {
                  if let Ok(plain) = k.open_chunk(*e, id, &b) {
                    found = Some(plain);
                    break;
                  }
                }
                Err(l @ Error::Limit(_)) => return Err(l),
                Err(_) => {}
              }
            }
            v.push(found.ok_or(first_error.clone_status())?);
          }
          v
        }
      };
      for plain in opened {
        out.extend(plain);
        if out.len() as u64 > item.size {
          return Err(Error::Invalid("item size"));
        }
      }
      i += run.len();
    }
    if out.len() as u64 != item.size {
      return Err(Error::Invalid("item size"));
    }
    Ok(out)
  }
}
