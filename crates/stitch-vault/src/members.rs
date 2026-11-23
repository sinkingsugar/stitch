//! Keys and membership (v3): the recovery kit, per-epoch content roots, device sealing
//! keys, and the hash-chained membership log that admits, revokes and rotates.
//! See docs/vault-protocol.md ("Keys and membership").
use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use x25519_dalek::{PublicKey, StaticSecret};

/// The recovery kit: one high-entropy secret, separate from every content root. It
/// derives the vault's **anchor** (an Ed25519 key that may sign membership statements)
/// and a sealing key that opens the kit's envelopes, which carry every epoch's root.
/// Ordinary devices never hold it, so they cannot sign membership. The vault ID is the
/// first 16 bytes of SHA256 of the first kit's anchor: knowing the vault ID is enough
/// to recognise the true genesis, so neither a replaced kit nor a device needs any
/// other trust anchor.
/// Text: `sv2:{vault}:{seed}:{checksum}` (checksum: first 8 bytes of SHA256 of the
/// rest; detects transcription errors only). Deliberately neither Debug nor Serialize.
pub struct Kit {
  vault: String,
  seed: Zeroizing<[u8; 32]>,
}
impl Kit {
  pub fn generate() -> Result<Self> {
    let mut kit = Self {
      vault: String::new(),
      seed: Zeroizing::new(random()?),
    };
    kit.vault = vault_of(&kit.anchor_public()?)?;
    Ok(kit)
  }
  /// A replacement kit for an existing vault (installed by `Membership::replace_kit`).
  pub fn replacement(vault: &str) -> Result<Self> {
    Ok(Self {
      vault: vault.into(),
      seed: Zeroizing::new(random()?),
    })
  }
  pub fn vault(&self) -> &str {
    &self.vault
  }
  pub fn text(&self) -> Zeroizing<String> {
    let prefix = Zeroizing::new(format!(
      "sv2:{}:{}",
      self.vault,
      hex::encode(self.seed.as_ref())
    ));
    Zeroizing::new(format!(
      "{}:{}",
      prefix.as_str(),
      &sha256(prefix.as_bytes())[..16]
    ))
  }
  pub fn parse(text: &str) -> Result<Self> {
    let parts: Vec<_> = text.split(':').collect();
    if text.len() != 118
      || parts.len() != 4
      || parts[0] != "sv2"
      || !is_hex(parts[1], 16)
      || !is_hex(parts[2], 32)
      || !is_hex(parts[3], 8)
    {
      return Err(Error::Invalid("recovery kit"));
    }
    let prefix = Zeroizing::new(parts[..3].join(":"));
    if sha256(prefix.as_bytes())[..16] != *parts[3] {
      return Err(Error::Invalid("kit checksum"));
    }
    let mut seed = Zeroizing::new([0; 32]);
    hex::decode_to_slice(parts[2], seed.as_mut()).map_err(|_| Error::Invalid("kit"))?;
    Ok(Self {
      vault: parts[1].into(),
      seed,
    })
  }
  // Independent of the vault ID (which is derived from the anchor).
  fn derive(&self, purpose: &str) -> Result<Zeroizing<[u8; 32]>> {
    let prk =
      hkdf::Salt::new(hkdf::HKDF_SHA256, b"stitch vault kit v3").extract(self.seed.as_ref());
    let info = [purpose.as_bytes()];
    let okm = prk
      .expand(&info, hkdf::HKDF_SHA256)
      .map_err(|_| Error::Crypto)?;
    let mut key = Zeroizing::new([0; 32]);
    okm.fill(key.as_mut()).map_err(|_| Error::Crypto)?;
    Ok(key)
  }
  pub(crate) fn anchor(&self) -> Result<signature::Ed25519KeyPair> {
    signature::Ed25519KeyPair::from_seed_unchecked(self.derive("kit anchor")?.as_ref())
      .map_err(|_| Error::Crypto)
  }
  pub fn anchor_public(&self) -> Result<String> {
    Ok(hex::encode(self.anchor()?.public_key().as_ref()))
  }
  fn seal_secret(&self) -> Result<StaticSecret> {
    Ok(StaticSecret::from(*self.derive("kit seal")?))
  }
  pub fn seal_public(&self) -> Result<String> {
    Ok(hex::encode(
      PublicKey::from(&self.seal_secret()?).as_bytes(),
    ))
  }
}
/// Destructive membership actions (deleting the vault, shortening retention, adding or
/// revoking an admin, replacing the kit) take effect this long after they are signed,
/// unless the vault requires two admins and two distinct admins (or one and the kit)
/// co-sign; an admin added by the kit is admitted at once.
pub const DELAY_MS: u64 = 7 * 86_400_000;
/// How far a statement's time may be from the service's clock (and ahead of a
/// verifier's).
pub const SKEW_MS: u64 = 5 * 60_000;
/// The commitment to an epoch's root, carried by its `genesis` or `epoch` statement:
/// holders check every root they open against it, so a signer cannot hand some
/// holders a different root than the others.
pub fn root_commitment(vault: &str, epoch: u32, root: &[u8; 32]) -> String {
  sha256(
    &[
      format!("stitch vault root v3\n{vault}\n{epoch}\n").as_bytes(),
      root,
    ]
    .concat(),
  )
}
static CLOCK_OFFSET: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
/// Milliseconds since the Unix epoch, plus the test offset (`set_clock_offset`).
pub fn now_ms() -> u64 {
  let real = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_millis() as i64);
  (real + CLOCK_OFFSET.load(std::sync::atomic::Ordering::Relaxed)).max(0) as u64
}
/// Accelerated-clock tests only: shift what `now_ms` returns.
pub fn set_clock_offset(ms: i64) {
  CLOCK_OFFSET.store(ms, std::sync::atomic::Ordering::Relaxed);
}

/// The vault ID a genesis anchor commits to.
pub fn vault_of(anchor: &str) -> Result<String> {
  let bytes = hex::decode(anchor).map_err(|_| Error::Invalid("anchor"))?;
  Ok(sha256(&bytes)[..32].to_string())
}
pub(crate) fn derive(vault: &str, ikm: &[u8], purpose: &str) -> Result<Zeroizing<[u8; 32]>> {
  let salt = hex::decode(vault).map_err(|_| Error::Invalid("vault id"))?;
  let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(ikm);
  let info = format!("stitch vault v3 {purpose}");
  let infos = [info.as_bytes()];
  let okm = prk
    .expand(&infos, hkdf::HKDF_SHA256)
    .map_err(|_| Error::Crypto)?;
  let mut key = Zeroizing::new([0; 32]);
  okm.fill(key.as_mut()).map_err(|_| Error::Crypto)?;
  Ok(key)
}

/// Every content root a holder can open, by epoch, and the anchor it trusts.
pub struct Keyring {
  vault: String,
  anchor: String,
  keys: BTreeMap<u32, VaultKey>,
}
impl Keyring {
  pub fn vault(&self) -> &str {
    &self.vault
  }
  pub fn anchor(&self) -> &str {
    &self.anchor
  }
  pub fn epochs(&self) -> Vec<u32> {
    self.keys.keys().copied().collect()
  }
  /// The newest epoch's key: what new content is sealed with.
  pub fn current(&self) -> Result<&VaultKey> {
    self.keys.values().last().ok_or(Error::Invalid("no key"))
  }
  pub fn get(&self, epoch: u32) -> Result<&VaultKey> {
    self
      .keys
      .get(&epoch)
      .ok_or(Error::Invalid("no key for epoch"))
  }
}

/// Who signs a statement: the kit's anchor or an admin device.
pub enum Signer<'a> {
  Kit(&'a Kit),
  Device(&'a Device),
}
impl Signer<'_> {
  fn public(&self) -> Result<String> {
    match self {
      Signer::Kit(k) => k.anchor_public(),
      Signer::Device(d) => Ok(d.public()),
    }
  }
  fn sign(&self, text: &[u8]) -> Result<String> {
    Ok(hex::encode(match self {
      Signer::Kit(k) => k.anchor()?.sign(text).as_ref().to_vec(),
      Signer::Device(d) => d.sign(text).as_ref().to_vec(),
    }))
  }
}
/// Whose envelopes to open: the kit, or an enrolled device of `vault` (the vault ID
/// commits to the genesis anchor, so a service cannot substitute a whole log).
pub enum Credential<'a> {
  Kit(&'a Kit),
  Device { device: &'a Device, vault: &'a str },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
  /// Statement 1: the anchor, the kit's sealing key, epoch 1's root sealed to the kit.
  Genesis,
  /// Admit a device with a role and its sealing key; seals it the current keyring.
  Add,
  /// End a device's access; `through` is its last valid commit sequence. Delayed when
  /// the device is an admin.
  Revoke,
  /// Rotate: a new root, sealed with all earlier ones to every current device and the kit.
  Epoch,
  /// Replace the kit (new anchor and sealing key), sealing it the keyring. Delayed.
  Anchor,
  /// Vault settings: `through` = retention days (1..=3650), `role` = `approvals1` or
  /// `approvals2`. Delayed when it shortens retention.
  Settings,
  /// Delete the whole vault (account closure). Delayed.
  Delete,
  /// Cancel the pending action of statement `target`.
  Cancel,
  /// Suspend a device at once (reversible): no access, no signing, left out of
  /// rotations; `through` = its last valid commit sequence while frozen.
  Freeze,
  /// Lift a freeze, sealing the device the current keyring. A freeze by the kit can only
  /// be lifted by the kit.
  Unfreeze,
}
impl Action {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Genesis => "genesis",
      Self::Add => "add",
      Self::Revoke => "revoke",
      Self::Epoch => "epoch",
      Self::Anchor => "anchor",
      Self::Settings => "settings",
      Self::Delete => "delete",
      Self::Cancel => "cancel",
      Self::Freeze => "freeze",
      Self::Unfreeze => "unfreeze",
    }
  }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
  /// A device's public key, or `kit`.
  pub to: String,
  pub sealed: String,
}
/// One membership statement. Unused fields are empty/0 so the signed text is one shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Statement {
  pub version: u32,
  pub vault: String,
  pub seq: u64,
  pub previous: String,
  pub action: Action,
  pub signer: String,
  pub device: String,
  pub role: String,
  pub seal: String,
  pub through: u64,
  pub epoch: u32,
  pub anchor: String,
  /// When it was signed (ms); the service refuses times more than 5 minutes off and
  /// times before the previous statement's. Delayed actions apply at `at + DELAY_MS`.
  pub at: u64,
  /// The statement a `cancel` cancels (0 otherwise).
  pub target: u64,
  /// An optional second admin (or the kit) signing the same text.
  pub cosigner: String,
  /// `genesis` and `epoch`: the commitment to the epoch's root (`root_commitment`), so
  /// a holder can tell a sealed root is the one every holder got; empty otherwise.
  pub commitment: String,
  pub envelopes: Vec<Envelope>,
  pub signature: String,
  pub cosignature: String,
}
fn word(s: &str) -> bool {
  s.len() <= 128 && s.bytes().all(|c| c.is_ascii_alphanumeric())
}
impl Statement {
  pub fn signing_bytes(&self) -> Result<Vec<u8>> {
    if self.version != VERSION
      || !is_hex(&self.vault, 16)
      || self.seq == 0
      || !is_hex(&self.previous, 32)
      || !is_hex(&self.signer, 32)
      || ![&self.device, &self.role, &self.seal, &self.anchor]
        .iter()
        .all(|s| word(s))
      || self.envelopes.len() > MAX_MEMBERS
      || !(self.cosigner.is_empty() || is_hex(&self.cosigner, 32))
      || !(self.commitment.is_empty() || is_hex(&self.commitment, 32))
      || (self.cosigner.is_empty() && !self.cosignature.is_empty())
    {
      return Err(Error::Invalid("membership statement"));
    }
    let mut text = format!(
      "stitch vault member v3\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
      self.vault,
      self.seq,
      self.previous,
      self.action.as_str(),
      self.signer,
      self.device,
      self.role,
      self.seal,
      self.through,
      self.epoch,
      self.anchor,
      self.at,
      self.target,
      self.cosigner,
      self.commitment
    );
    let mut last = "";
    for e in &self.envelopes {
      if !word(&e.to)
        || e.to.as_str() <= last
        || e.sealed.len() > 8192
        || !e
          .sealed
          .bytes()
          .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
      {
        return Err(Error::Invalid("membership envelopes"));
      }
      last = &e.to;
      text.push_str(&format!("{}\t{}\n", e.to, e.sealed));
    }
    Ok(text.into_bytes())
  }
  pub fn hash(&self) -> Result<String> {
    Ok(sha256(&self.signing_bytes()?))
  }
  fn verify_signature(&self) -> Result<()> {
    let text = self.signing_bytes()?;
    crate::checkpoint::verify_ed25519(&self.signer, &self.signature, &text)?;
    if !self.cosigner.is_empty() {
      if self.cosigner == self.signer {
        return Err(Error::Invalid("cosigner must differ"));
      }
      crate::checkpoint::verify_ed25519(&self.cosigner, &self.cosignature, &text)?;
    }
    Ok(())
  }
}

/// Seal `plain` to an X25519 public key: ephemeral key agreement, HKDF, ChaCha20-Poly1305.
fn seal_to(vault: &str, to: &str, public: &str, plain: &[u8]) -> Result<String> {
  seal_with(vault, to, public, plain, random::<32>()?, random::<12>()?)
}
/// The kit's envelope, deterministic from the epoch root it carries, the statement and
/// the plaintext: every holder of those roots can recompute it and so check that the
/// kit was really sealed what the devices got (`kit_envelope_problems`). A different
/// plaintext gives a different ephemeral key and nonce.
fn seal_to_kit(
  vault: &str,
  to: &str,
  public: &str,
  plain: &[u8],
  root: &[u8; 32],
  seq: u64,
) -> Result<String> {
  let info = format!("kit envelope {to} {seq} {}", sha256(plain));
  let ephemeral = *derive(vault, root, &format!("{info} key"))?;
  let nonce: [u8; 12] = derive(vault, root, &format!("{info} nonce"))?[..12]
    .try_into()
    .map_err(|_| Error::Crypto)?;
  seal_with(vault, to, public, plain, ephemeral, nonce)
}
fn seal_with(
  vault: &str,
  to: &str,
  public: &str,
  plain: &[u8],
  ephemeral: [u8; 32],
  nonce: [u8; 12],
) -> Result<String> {
  let theirs: [u8; 32] = hex::decode(public)
    .ok()
    .and_then(|b| b.try_into().ok())
    .ok_or(Error::Invalid("sealing key"))?;
  let theirs = PublicKey::from(theirs);
  let ephemeral = StaticSecret::from(ephemeral);
  let ours = PublicKey::from(&ephemeral);
  let shared = ephemeral.diffie_hellman(&theirs);
  if !shared.was_contributory() {
    return Err(Error::Invalid("sealing key"));
  }
  let key = envelope_key(
    vault,
    to,
    shared.as_bytes(),
    ours.as_bytes(),
    theirs.as_bytes(),
  )?;
  let mut body = plain.to_vec();
  key
    .seal_in_place_append_tag(
      aead::Nonce::assume_unique_for_key(nonce),
      aead::Aad::from(format!("stitch vault envelope v3\n{vault}\n{to}\n").as_bytes()),
      &mut body,
    )
    .map_err(|_| Error::Crypto)?;
  Ok(hex::encode(
    [ours.as_bytes().as_slice(), &nonce, &body].concat(),
  ))
}
fn open_from(
  vault: &str,
  to: &str,
  secret: &StaticSecret,
  sealed: &str,
) -> Result<Zeroizing<Vec<u8>>> {
  let bytes = hex::decode(sealed).map_err(|_| Error::Crypto)?;
  if bytes.len() < 32 + 12 + 16 {
    return Err(Error::Crypto);
  }
  let theirs: [u8; 32] = bytes[..32].try_into().map_err(|_| Error::Crypto)?;
  let theirs = PublicKey::from(theirs);
  let shared = secret.diffie_hellman(&theirs);
  let ours = PublicKey::from(secret);
  let key = envelope_key(
    vault,
    to,
    shared.as_bytes(),
    theirs.as_bytes(),
    ours.as_bytes(),
  )?;
  let nonce: [u8; 12] = bytes[32..44].try_into().map_err(|_| Error::Crypto)?;
  let mut body = Zeroizing::new(bytes[44..].to_vec());
  let len = key
    .open_in_place(
      aead::Nonce::assume_unique_for_key(nonce),
      aead::Aad::from(format!("stitch vault envelope v3\n{vault}\n{to}\n").as_bytes()),
      &mut body,
    )
    .map_err(|_| Error::Crypto)?
    .len();
  body.truncate(len);
  Ok(body)
}
fn envelope_key(
  vault: &str,
  to: &str,
  shared: &[u8],
  ephemeral: &[u8],
  recipient: &[u8],
) -> Result<aead::LessSafeKey> {
  let ikm = [shared, ephemeral, recipient].concat();
  let key = derive(vault, &ikm, &format!("envelope {to}"))?;
  Ok(aead::LessSafeKey::new(
    aead::UnboundKey::new(&aead::CHACHA20_POLY1305, key.as_ref()).map_err(|_| Error::Crypto)?,
  ))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Roots {
  roots: Vec<(u32, String)>,
}

/// A device's standing in the verified log.
#[derive(Clone, Debug)]
pub struct MemberState {
  pub role: String,
  pub seal: String,
  /// Set when revoked: its last valid commit sequence.
  pub through: Option<u64>,
  /// Set while frozen: who froze it (`kit` or `admin`).
  pub frozen: Option<String>,
  /// While frozen: its last valid commit sequence.
  pub frozen_through: u64,
}
impl MemberState {
  pub fn writes(&self) -> bool {
    self.role == "admin" || self.role == "writer"
  }
  /// Neither revoked nor frozen.
  pub fn current(&self) -> bool {
    self.through.is_none() && self.frozen.is_none()
  }
}
/// A destructive action waiting for its time. Applied at `effective` only if not
/// cancelled and its signer is then still the anchor or a current admin.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Pending {
  pub seq: u64,
  pub action: Action,
  pub signer: String,
  pub device: String,
  pub role: String,
  pub seal: String,
  pub through: u64,
  pub anchor: String,
  pub at: u64,
  pub effective: u64,
}
/// The state a verified membership log describes at a time, and the next statement's
/// position.
#[derive(Clone, Debug, Default)]
pub struct Membership {
  pub vault: String,
  pub anchor: String,
  pub kit_seal: String,
  pub epoch: u32,
  pub devices: BTreeMap<String, MemberState>,
  pub seq: u64,
  pub head: String,
  /// Rolling history kept by GC, in days (30 until a settings statement says otherwise).
  pub retention_days: u64,
  /// 2: destructive actions co-signed by two admins apply at once (else after the delay).
  pub approvals: u8,
  /// Destructive actions not yet in effect, visible to every device.
  pub pending: Vec<Pending>,
  /// The vault was deleted (a delete took effect).
  pub deleted: bool,
  /// The newest statement's time.
  pub last_at: u64,
  /// Epoch → its root's commitment, from `genesis` and `epoch` statements.
  pub commitments: BTreeMap<u32, String>,
}
impl Membership {
  /// Verify a log as of now (`now_ms`).
  pub fn verify(vault: &str, log: &[Statement]) -> (Self, Option<String>) {
    Self::verify_at(vault, log, now_ms())
  }
  /// Verify a log from the start and apply the pending actions due by `now`; returns the
  /// state of its longest valid prefix and the reason the next statement (if any) was
  /// refused. The genesis must name the anchor the vault ID commits to.
  pub fn verify_at(vault: &str, log: &[Statement], now: u64) -> (Self, Option<String>) {
    let mut state = Self {
      vault: vault.into(),
      head: ZERO.into(),
      ..Self::default()
    };
    for s in log {
      // A statement dated past the verifier's clock (beyond the service's skew) is
      // refused: it would otherwise apply pending actions early (`due(s.at)`).
      let early = s.action != Action::Genesis && s.at > now.saturating_add(SKEW_MS);
      let result = if early {
        Err(Error::Invalid("time in the future"))
      } else {
        state.apply(s)
      };
      if let Err(e) = result {
        state.due(now);
        return (
          state,
          Some(format!("membership statement {} invalid: {e}", s.seq)),
        );
      }
    }
    state.due(now);
    (state, None)
  }
  /// The kit replacement waiting for its time, if any.
  pub fn pending_anchor(&self) -> Option<&Pending> {
    self.pending.iter().find(|p| p.action == Action::Anchor)
  }
  /// The anchor, or a current admin device.
  pub fn admin_ok(&self, key: &str) -> bool {
    key == self.anchor
      || self
        .devices
        .get(key)
        .is_some_and(|m| m.role == "admin" && m.current())
  }
  /// Apply pending actions whose time has come (in statement order).
  fn due(&mut self, t: u64) {
    if self.deleted {
      return;
    }
    let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
      .into_iter()
      .partition(|p| p.effective <= t);
    self.pending = later;
    for p in now {
      if self.admin_ok(&p.signer) {
        self.effect(&p);
      }
    }
  }
  fn effect(&mut self, p: &Pending) {
    match p.action {
      Action::Delete => self.deleted = true,
      Action::Settings => self.retention_days = p.through,
      Action::Revoke => {
        if let Some(m) = self.devices.get_mut(&p.device) {
          m.through = Some(p.through);
        }
      }
      Action::Anchor => {
        self.anchor = p.anchor.clone();
        self.kit_seal = p.seal.clone();
      }
      // The promotion of a delayed admin: only if it is still current (a freeze or a
      // revocation meanwhile leaves it a writer, or revoked).
      Action::Add => {
        if let Some(m) = self.devices.get_mut(&p.device).filter(|m| m.current()) {
          m.role = "admin".into();
        }
      }
      _ => {}
    }
  }
  /// Check one statement against this state and apply it.
  /// A refused statement leaves the state as it was (applied on a copy).
  pub fn apply(&mut self, s: &Statement) -> Result<()> {
    let mut next = self.clone();
    next.apply_in_place(s)?;
    *self = next;
    Ok(())
  }
  fn apply_in_place(&mut self, s: &Statement) -> Result<()> {
    if s.vault != self.vault || s.seq != self.seq + 1 || s.previous != self.head {
      return Err(Error::Invalid("chain"));
    }
    s.verify_signature()?;
    if self.deleted {
      return Err(Error::Invalid("vault deleted"));
    }
    if s.action != Action::Genesis {
      if s.at < self.last_at {
        return Err(Error::Invalid("time before the previous statement"));
      }
      self.due(s.at);
      if self.deleted {
        return Err(Error::Invalid("vault deleted"));
      }
      if !self.admin_ok(&s.signer) {
        return Err(Error::Invalid("signer is not the anchor or an admin"));
      }
    }
    if !s.cosigner.is_empty() && !self.admin_ok(&s.cosigner) {
      return Err(Error::Invalid("cosigner is not the anchor or an admin"));
    }
    let immediate = self.approvals >= 2 && !s.cosigner.is_empty();
    let pending = Pending {
      seq: s.seq,
      action: s.action,
      signer: s.signer.clone(),
      device: s.device.clone(),
      role: s.role.clone(),
      seal: s.seal.clone(),
      through: s.through,
      anchor: s.anchor.clone(),
      at: s.at,
      effective: s.at.saturating_add(DELAY_MS),
    };
    let to: Vec<&str> = s.envelopes.iter().map(|e| e.to.as_str()).collect();
    let empty = |v: &[&str]| v.iter().all(|f| f.is_empty());
    let plain = s.target == 0 || s.action == Action::Cancel;
    let commits = matches!(s.action, Action::Genesis | Action::Epoch);
    if !plain
      || (s.action != Action::Epoch && s.epoch != self.epoch && s.action != Action::Genesis)
      || commits != is_hex(&s.commitment, 32)
    {
      return Err(Error::Invalid("statement fields"));
    }
    match s.action {
      Action::Genesis => {
        if s.seq != 1
          || s.signer != s.anchor
          || vault_of(&s.anchor)? != self.vault
          || s.epoch != 1
          || !is_hex(&s.seal, 32)
          || !empty(&[&s.device, &s.role, &s.cosigner])
          || s.through != 0
          || to != ["kit"]
        {
          return Err(Error::Invalid("genesis"));
        }
        self.anchor = s.anchor.clone();
        self.kit_seal = s.seal.clone();
        self.epoch = 1;
        self.commitments.insert(1, s.commitment.clone());
        self.retention_days = 30;
        self.approvals = 1;
      }
      Action::Add => {
        if !is_hex(&s.device, 32)
          || !["admin", "writer", "reader"].contains(&s.role.as_str())
          || !is_hex(&s.seal, 32)
          || !s.anchor.is_empty()
          || s.through != 0
          || to != [s.device.as_str()]
          || self.devices.contains_key(&s.device)
          || self
            .devices
            .values()
            .filter(|m| m.through.is_none())
            .count()
            >= MAX_MEMBERS
        {
          return Err(Error::Invalid("add"));
        }
        // A new admin is admitted at once as a writer and promoted at its time, unless
        // the kit signs or co-signs (bootstrap, recovery: the kit can already freeze
        // and unfreeze anyone at once) or two admins co-sign under the two-admin
        // setting. A stolen admin device so cannot mint admins the kit must race.
        let kit = s.signer == self.anchor || s.cosigner == self.anchor;
        let delayed = s.role == "admin" && !immediate && !kit;
        self.devices.insert(
          s.device.clone(),
          MemberState {
            role: if delayed {
              "writer".into()
            } else {
              s.role.clone()
            },
            seal: s.seal.clone(),
            through: None,
            frozen: None,
            frozen_through: 0,
          },
        );
        if delayed {
          self.pending.push(pending);
        }
      }
      Action::Revoke => {
        let target = self.devices.get(&s.device).filter(|m| m.through.is_none());
        if target.is_none() || !empty(&[&s.role, &s.seal, &s.anchor]) || !to.is_empty() {
          return Err(Error::Invalid("revoke"));
        }
        if target.is_some_and(|m| m.role == "admin") && !immediate {
          self.pending.push(pending);
        } else {
          self.effect(&pending);
        }
      }
      Action::Epoch => {
        let mut expected: BTreeSet<&str> = self
          .devices
          .iter()
          .filter(|(_, m)| m.current())
          .map(|(d, _)| d.as_str())
          .collect();
        expected.insert("kit");
        // While a kit replacement waits, new epochs are sealed to the next kit too.
        if self.pending_anchor().is_some() {
          expected.insert("nextkit");
        }
        if s.epoch != self.epoch + 1
          || !empty(&[&s.device, &s.role, &s.seal, &s.anchor])
          || s.through != 0
          || to.iter().copied().collect::<BTreeSet<_>>() != expected
        {
          return Err(Error::Invalid(
            "epoch: must seal to every current device and the kit",
          ));
        }
        self.epoch = s.epoch;
        self.commitments.insert(s.epoch, s.commitment.clone());
      }
      Action::Anchor => {
        if !is_hex(&s.anchor, 32)
          || !is_hex(&s.seal, 32)
          || !empty(&[&s.device, &s.role])
          || s.through != 0
          || to != ["kit"]
          || self.pending_anchor().is_some()
        {
          return Err(Error::Invalid("anchor (one replacement at a time)"));
        }
        if immediate {
          self.effect(&pending);
        } else {
          self.pending.push(pending);
        }
      }
      Action::Settings => {
        if !(1..=3650).contains(&s.through)
          || !["approvals1", "approvals2"].contains(&s.role.as_str())
          || !empty(&[&s.device, &s.seal, &s.anchor])
          || !to.is_empty()
        {
          return Err(Error::Invalid("settings"));
        }
        // The approvals setting applies at once (it never removes data); only a
        // shorter retention waits.
        self.approvals = if s.role == "approvals2" { 2 } else { 1 };
        if s.through < self.retention_days && !immediate {
          self.pending.push(pending);
        } else {
          self.retention_days = s.through;
        }
      }
      Action::Delete => {
        if !empty(&[&s.device, &s.role, &s.seal, &s.anchor]) || s.through != 0 || !to.is_empty() {
          return Err(Error::Invalid("delete"));
        }
        if immediate {
          self.effect(&pending);
        } else {
          self.pending.push(pending);
        }
      }
      Action::Cancel => {
        if !empty(&[&s.device, &s.role, &s.seal, &s.anchor])
          || !to.is_empty()
          || !self.pending.iter().any(|p| p.seq == s.target)
        {
          return Err(Error::Invalid("cancel: no such pending action"));
        }
        self.pending.retain(|p| p.seq != s.target);
      }
      Action::Freeze => {
        let by = if s.signer == self.anchor {
          "kit"
        } else {
          "admin"
        };
        match self.devices.get_mut(&s.device) {
          Some(m)
            if m.current()
              && s.role.is_empty()
              && s.seal.is_empty()
              && s.anchor.is_empty()
              && to.is_empty() =>
          {
            m.frozen = Some(by.into());
            m.frozen_through = s.through;
          }
          _ => return Err(Error::Invalid("freeze")),
        }
      }
      Action::Unfreeze => {
        let kit = s.signer == self.anchor;
        match self.devices.get_mut(&s.device) {
          Some(m)
            if m.through.is_none()
              && m.frozen.as_deref().is_some_and(|f| kit || f == "admin")
              && to == [s.device.as_str()]
              && s.through == 0 =>
          {
            m.frozen = None;
            m.frozen_through = 0;
          }
          _ => return Err(Error::Invalid("unfreeze")),
        }
      }
    }
    self.seq = s.seq;
    self.head = s.hash()?;
    self.last_at = s.at;
    Ok(())
  }
  /// Build, seal and sign the next statement.
  fn next(
    &self,
    signer: &Signer,
    action: Action,
    fill: impl FnOnce(&mut Statement),
  ) -> Result<Statement> {
    self.next_cosigned(signer, None, action, fill)
  }
  fn next_cosigned(
    &self,
    signer: &Signer,
    cosigner: Option<&Signer>,
    action: Action,
    fill: impl FnOnce(&mut Statement),
  ) -> Result<Statement> {
    let mut s = Statement {
      version: VERSION,
      vault: self.vault.clone(),
      seq: self.seq + 1,
      previous: self.head.clone(),
      action,
      signer: signer.public()?,
      device: String::new(),
      role: String::new(),
      seal: String::new(),
      through: 0,
      epoch: self.epoch,
      anchor: String::new(),
      at: now_ms().max(self.last_at),
      target: 0,
      cosigner: match cosigner {
        Some(c) => c.public()?,
        None => String::new(),
      },
      envelopes: vec![],
      signature: String::new(),
      cosignature: String::new(),
      commitment: String::new(),
    };
    fill(&mut s);
    s.envelopes.sort_by(|a, b| a.to.cmp(&b.to));
    let text = s.signing_bytes()?;
    s.signature = signer.sign(&text)?;
    if let Some(c) = cosigner {
      s.cosignature = c.sign(&text)?;
    }
    Ok(s)
  }
  fn roots(keyring: &Keyring) -> Result<Zeroizing<Vec<u8>>> {
    let roots = keyring
      .keys
      .iter()
      .map(|(e, k)| (*e, hex::encode(k.root.as_ref())))
      .collect();
    Ok(Zeroizing::new(serde_json::to_vec(&Roots { roots })?))
  }
  /// Statement 1 of a new vault: returns it and the keyring with epoch 1's fresh root.
  pub fn genesis(kit: &Kit) -> Result<(Statement, Keyring)> {
    if vault_of(&kit.anchor_public()?)? != kit.vault {
      return Err(Error::Invalid("a replacement kit cannot start a vault"));
    }
    let state = Self {
      vault: kit.vault.clone(),
      head: ZERO.into(),
      ..Self::default()
    };
    let keyring = Keyring {
      vault: kit.vault.clone(),
      anchor: kit.anchor_public()?,
      keys: BTreeMap::from([(1, VaultKey::new(&kit.vault, 1, random()?))]),
    };
    let plain = Self::roots(&keyring)?;
    let root = keyring.get(1)?.root_bytes();
    let sealed = seal_to_kit(&kit.vault, "kit", &kit.seal_public()?, &plain, &root, 1)?;
    let commitment = root_commitment(&kit.vault, 1, &root);
    let s = state.next(&Signer::Kit(kit), Action::Genesis, |s| {
      s.anchor = keyring.anchor.clone();
      s.seal = kit.seal_public().unwrap_or_default();
      s.epoch = 1;
      s.commitment = commitment;
      s.envelopes = vec![Envelope {
        to: "kit".into(),
        sealed,
      }];
    })?;
    Ok((s, keyring))
  }
  pub fn add(
    &self,
    signer: &Signer,
    keyring: &Keyring,
    device: &str,
    seal: &str,
    role: Role,
  ) -> Result<Statement> {
    self.add_cosigned(signer, None, keyring, device, seal, role)
  }
  /// Admit a device. Writers and readers at once; an admin is admitted as a writer and
  /// promoted after the delay, unless the kit signs or co-signs, or two admins co-sign
  /// under the two-admin setting.
  pub fn add_cosigned(
    &self,
    signer: &Signer,
    cosigner: Option<&Signer>,
    keyring: &Keyring,
    device: &str,
    seal: &str,
    role: Role,
  ) -> Result<Statement> {
    let sealed = seal_to(&self.vault, device, seal, &Self::roots(keyring)?)?;
    self.next_cosigned(signer, cosigner, Action::Add, |s| {
      s.device = device.into();
      s.role = role.as_str().into();
      s.seal = seal.into();
      s.envelopes = vec![Envelope {
        to: device.into(),
        sealed,
      }];
    })
  }
  /// Revoke a device: at once for writers and readers; for an admin, after the delay
  /// (or at once when the vault requires two admins and `cosigner` signs too).
  pub fn revoke(
    &self,
    signer: &Signer,
    cosigner: Option<&Signer>,
    device: &str,
    through: u64,
  ) -> Result<Statement> {
    self.next_cosigned(signer, cosigner, Action::Revoke, |s| {
      s.device = device.into();
      s.through = through;
    })
  }
  /// Rotate after a revocation: a fresh root for the next epoch, sealed (with every
  /// earlier root, so history stays readable) to each current device and the kit.
  /// `keyring` gains the new epoch.
  pub fn rotate(&self, signer: &Signer, keyring: &mut Keyring) -> Result<Statement> {
    let epoch = self.epoch + 1;
    keyring
      .keys
      .insert(epoch, VaultKey::new(&self.vault, epoch, random()?));
    let plain = Self::roots(keyring)?;
    let root = keyring.get(epoch)?.root_bytes();
    let seq = self.seq + 1;
    let mut envelopes = vec![Envelope {
      to: "kit".into(),
      sealed: seal_to_kit(&self.vault, "kit", &self.kit_seal, &plain, &root, seq)?,
    }];
    if let Some(next) = self.pending_anchor() {
      envelopes.push(Envelope {
        to: "nextkit".into(),
        sealed: seal_to_kit(&self.vault, "nextkit", &next.seal, &plain, &root, seq)?,
      });
    }
    let commitment = root_commitment(&self.vault, epoch, &root);
    for (device, m) in self.devices.iter().filter(|(_, m)| m.current()) {
      envelopes.push(Envelope {
        to: device.clone(),
        sealed: seal_to(&self.vault, device, &m.seal, &plain)?,
      });
    }
    self.next(signer, Action::Epoch, |s| {
      s.epoch = epoch;
      s.commitment = commitment;
      s.envelopes = envelopes;
    })
  }
  /// Retention (days) and whether destructive operations need two admins.
  pub fn settings(
    &self,
    signer: &Signer,
    cosigner: Option<&Signer>,
    retention_days: u64,
    two_admins: bool,
  ) -> Result<Statement> {
    self.next_cosigned(signer, cosigner, Action::Settings, |s| {
      s.through = retention_days;
      s.role = if two_admins {
        "approvals2"
      } else {
        "approvals1"
      }
      .into();
    })
  }
  /// Replace the kit (lost or exposed): the new kit becomes the anchor and gets the keyring.
  /// Delete the whole vault (after the delay, or at once with two admins' approval).
  pub fn delete(&self, signer: &Signer, cosigner: Option<&Signer>) -> Result<Statement> {
    self.next_cosigned(signer, cosigner, Action::Delete, |_| {})
  }
  /// Cancel pending statement `seq` (any admin or the kit).
  pub fn cancel(&self, signer: &Signer, seq: u64) -> Result<Statement> {
    self.next(signer, Action::Cancel, |s| s.target = seq)
  }
  /// Freeze a device at once; `through` = its last valid commit sequence.
  pub fn freeze(&self, signer: &Signer, device: &str, through: u64) -> Result<Statement> {
    self.next(signer, Action::Freeze, |s| {
      s.device = device.into();
      s.through = through;
    })
  }
  /// Lift a freeze and seal the device the keyring (rotations skipped it).
  pub fn unfreeze(&self, signer: &Signer, keyring: &Keyring, device: &str) -> Result<Statement> {
    let seal = &self
      .devices
      .get(device)
      .ok_or(Error::Invalid("unknown device"))?
      .seal;
    let sealed = seal_to(&self.vault, device, seal, &Self::roots(keyring)?)?;
    self.next(signer, Action::Unfreeze, |s| {
      s.device = device.into();
      s.envelopes = vec![Envelope {
        to: device.into(),
        sealed,
      }];
    })
  }
  pub fn replace_kit(
    &self,
    signer: &Signer,
    cosigner: Option<&Signer>,
    keyring: &Keyring,
    new: &Kit,
  ) -> Result<Statement> {
    let sealed = seal_to_kit(
      &self.vault,
      "kit",
      &new.seal_public()?,
      &Self::roots(keyring)?,
      &keyring.get(self.epoch)?.root_bytes(),
      self.seq + 1,
    )?;
    let (anchor, seal) = (new.anchor_public()?, new.seal_public()?);
    self.next_cosigned(signer, cosigner, Action::Anchor, |s| {
      s.anchor = anchor;
      s.seal = seal;
      s.envelopes = vec![Envelope {
        to: "kit".into(),
        sealed,
      }];
    })
  }
}
/// The keyring a credential can open from a verified log: every envelope addressed to
/// it, in order (the newest envelope carries all roots the holder may read).
pub fn keyring(
  log: &[Statement],
  credential: &Credential,
) -> Result<(Keyring, Membership, Option<String>)> {
  let vault = match credential {
    Credential::Kit(kit) => kit.vault.clone(),
    Credential::Device { vault, .. } => vault.to_string(),
  };
  let (membership, refused) = Membership::verify(&vault, log);
  let mut keys = BTreeMap::new();
  let (names, secret) = match credential {
    // A kit opens what was sealed to it as the kit, or as the next kit while its
    // replacement of another kit was pending.
    Credential::Kit(kit) => (
      vec!["kit".to_string(), "nextkit".into()],
      kit.seal_secret()?,
    ),
    Credential::Device { device, .. } => (vec![device.public()], device.seal_secret()),
  };
  for s in log.iter().take(membership.seq as usize) {
    for e in s.envelopes.iter().filter(|e| names.contains(&e.to)) {
      // Envelopes sealed to another kit (a replacement) do not open: skipped.
      let Ok(plain) = open_from(&vault, &e.to, &secret, &e.sealed) else {
        continue;
      };
      let roots: Roots = serde_json::from_slice(&plain)?;
      let mut opened = BTreeMap::new();
      let mut honest = true;
      for (epoch, root) in roots.roots {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(&root, &mut bytes).map_err(|_| Error::Crypto)?;
        // Every root must be the one its epoch committed to, and never replaces a
        // different one already held: a signer cannot poison a holder's keys.
        honest &= membership.commitments.get(&epoch)
          == Some(&root_commitment(&vault, epoch, &bytes))
          && keys
            .get(&epoch)
            .is_none_or(|k: &VaultKey| k.root_bytes() == bytes);
        opened.insert(epoch, VaultKey::new(&vault, epoch, bytes));
        bytes.iter_mut().for_each(|b| *b = 0);
      }
      if honest {
        keys.extend(opened);
      }
    }
  }
  if keys.is_empty() {
    return Err(Error::Invalid("no envelope for this credential"));
  }
  Ok((
    Keyring {
      vault,
      anchor: membership.anchor.clone(),
      keys,
    },
    membership,
    refused,
  ))
}

/// Statements whose kit envelope is not what the kit should have been sealed: each
/// one is recomputed (`seal_to_kit` is deterministic) from the roots `keyring` holds.
/// A device runs this so that a kit envelope a signer garbled (the kit could not
/// recover those epochs) is noticed while devices still hold the roots. Statements
/// the keyring cannot check (missing roots) are skipped.
pub fn kit_envelope_problems(log: &[Statement], keyring: &Keyring) -> Vec<u64> {
  let mut state = Membership {
    vault: keyring.vault.clone(),
    head: ZERO.into(),
    ..Membership::default()
  };
  let mut bad = vec![];
  for s in log {
    let before = state.clone();
    let epoch = match s.action {
      Action::Genesis | Action::Epoch => s.epoch,
      _ => before.epoch,
    };
    for e in s
      .envelopes
      .iter()
      .filter(|e| e.to == "kit" || e.to == "nextkit")
    {
      let public = match (s.action, e.to.as_str()) {
        (Action::Genesis | Action::Anchor, "kit") => s.seal.clone(),
        (_, "kit") => before.kit_seal.clone(),
        _ => match before.pending_anchor() {
          Some(p) => p.seal.clone(),
          None => continue,
        },
      };
      let ring = Keyring {
        vault: keyring.vault.clone(),
        anchor: String::new(),
        keys: keyring
          .keys
          .iter()
          .filter(|(k, _)| **k <= epoch)
          .map(|(k, v)| (*k, VaultKey::new(&keyring.vault, *k, v.root_bytes())))
          .collect(),
      };
      if ring.keys.len() != epoch as usize {
        continue;
      }
      let ok = ring.get(epoch).and_then(|k| {
        let plain = Membership::roots(&ring)?;
        seal_to_kit(&ring.vault, &e.to, &public, &plain, &k.root_bytes(), s.seq)
      });
      if ok.ok().as_deref() != Some(e.sealed.as_str()) {
        bad.push(s.seq);
      }
    }
    if state.apply(s).is_err() {
      break;
    }
  }
  bad
}

impl Device {
  pub fn seal_public(&self) -> String {
    hex::encode(PublicKey::from(&self.seal).as_bytes())
  }
  fn seal_secret(&self) -> StaticSecret {
    self.seal.clone()
  }
  pub(crate) fn new_seal() -> Result<StaticSecret> {
    Ok(StaticSecret::from(random::<32>()?))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn kit_text_round_trips_and_checks() -> Result<()> {
    let kit = Kit::generate()?;
    let again = Kit::parse(&kit.text())?;
    assert_eq!(again.anchor_public()?, kit.anchor_public()?);
    let mut bad = kit.text().to_string();
    bad.replace_range(10..11, if &bad[10..11] == "0" { "1" } else { "0" });
    assert!(Kit::parse(&bad).is_err());
    Ok(())
  }
  #[test]
  fn admission_rotation_and_kit_replacement() -> Result<()> {
    let kit = Kit::generate()?;
    let (genesis, mut ring) = Membership::genesis(&kit)?;
    let mut log = vec![genesis];
    let state = |log: &[Statement]| Membership::verify(kit.vault(), log);
    let admin = Device::generate()?;
    let writer = Device::generate()?;
    let lost = Device::generate()?;
    log.push(state(&log).0.add(
      &Signer::Kit(&kit),
      &ring,
      &admin.public(),
      &admin.seal_public(),
      Role::Admin,
    )?);
    log.push(state(&log).0.add(
      &Signer::Device(&admin),
      &ring,
      &writer.public(),
      &writer.seal_public(),
      Role::Writer,
    )?);
    log.push(state(&log).0.add(
      &Signer::Device(&admin),
      &ring,
      &lost.public(),
      &lost.seal_public(),
      Role::Writer,
    )?);
    assert_eq!(state(&log).1, None);
    // An ordinary writer cannot sign membership.
    let forged = state(&log).0.add(
      &Signer::Device(&writer),
      &ring,
      &Device::generate()?.public(),
      &writer.seal_public(),
      Role::Admin,
    )?;
    let mut attempt = log.clone();
    attempt.push(forged);
    assert!(
      state(&attempt)
        .1
        .unwrap()
        .contains("not the anchor or an admin")
    );
    // Revoke and rotate: the lost device keeps epoch 1 only.
    log.push(
      state(&log)
        .0
        .revoke(&Signer::Device(&admin), None, &lost.public(), 0)?,
    );
    log.push(state(&log).0.rotate(&Signer::Device(&admin), &mut ring)?);
    let (s, refused) = state(&log);
    assert_eq!((s.epoch, refused), (2, None));
    let vault = kit.vault().to_string();
    let epochs = |c: Credential| keyring(&log, &c).map(|r| r.0.epochs());
    assert_eq!(epochs(Credential::Kit(&kit))?, vec![1, 2]);
    assert_eq!(
      epochs(Credential::Device {
        device: &writer,
        vault: &vault
      })?,
      vec![1, 2]
    );
    assert_eq!(
      epochs(Credential::Device {
        device: &lost,
        vault: &vault
      })?,
      vec![1]
    );
    // A substituted log (another anchor's genesis) is not this vault's.
    let other = Kit::generate()?;
    let (fake, _) = Membership::genesis(&other)?;
    let (s, refused) = Membership::verify(
      &vault,
      &[Statement {
        vault: vault.clone(),
        ..fake
      }],
    );
    assert_eq!(s.seq, 0);
    assert!(refused.is_some());
    // Epoch 2 content is unreadable with epoch 1's key.
    let k2 = ring.get(2)?;
    let id = k2.chunk_id(b"after rotation")?;
    let sealed = k2.seal_chunk(&id, b"after rotation")?;
    assert!(ring.get(1)?.open_chunk(2, &id, &sealed).is_err());
    // Replace the kit: a delayed action. Until its time the old kit still rules (and
    // could cancel); afterwards the old anchor signs nothing.
    let new_kit = Kit::replacement(kit.vault())?;
    assert!(Membership::genesis(&new_kit).is_err());
    log.push(
      state(&log)
        .0
        .replace_kit(&Signer::Device(&admin), None, &ring, &new_kit)?,
    );
    let (now, _) = state(&log);
    assert_eq!(now.anchor, kit.anchor_public()?);
    assert_eq!(now.pending.len(), 1);
    let later = now_ms() + DELAY_MS + 1;
    let (then, _) = Membership::verify_at(kit.vault(), &log, later);
    assert_eq!(then.anchor, new_kit.anchor_public()?);
    assert!(
      !then.admin_ok(&kit.anchor_public()?),
      "the old anchor no longer signs"
    );
    let (r, _, _) = keyring(&log, &Credential::Kit(&new_kit))?;
    assert_eq!(r.epochs(), vec![1, 2]);
    Ok(())
  }
  #[test]
  fn adding_an_admin_waits_unless_the_kit_or_two_admins_sign() -> Result<()> {
    let kit = Kit::generate()?;
    let (genesis, ring) = Membership::genesis(&kit)?;
    let mut log = vec![genesis];
    let v = kit.vault().to_string();
    let at = |log: &[Statement], t: u64| Membership::verify_at(&v, log, t);
    let state = |log: &[Statement]| at(log, now_ms()).0;
    let later = || now_ms() + DELAY_MS + 1000;
    let [a, b, c, d, e, f] = std::array::from_fn(|_| Device::generate().unwrap());
    let add = |log: &[Statement], by: &Signer, co: Option<&Signer>, x: &Device| {
      state(log).add_cosigned(by, co, &ring, &x.public(), &x.seal_public(), Role::Admin)
    };
    let kit_signer = Signer::Kit(&kit);
    // The kit admits admins at once (bootstrap, recovery).
    log.push(add(&log, &kit_signer, None, &a)?);
    log.push(add(&log, &kit_signer, None, &b)?);
    assert!(state(&log).admin_ok(&a.public()) && state(&log).admin_ok(&b.public()));
    assert!(state(&log).pending.is_empty());
    let (sa, sb) = (Signer::Device(&a), Signer::Device(&b));
    // An admin device alone: a writer at once, an admin only at its time.
    log.push(add(&log, &sa, None, &c)?);
    let now = state(&log);
    assert_eq!(now.devices[&c.public()].role, "writer");
    assert!(!now.admin_ok(&c.public()));
    assert_eq!(
      now.pending.iter().map(|p| p.action).collect::<Vec<_>>(),
      [Action::Add]
    );
    // Before its time it cannot sign membership.
    let mut early = log.clone();
    early.push(add(&early, &Signer::Device(&c), None, &f)?);
    assert!(
      at(&early, now_ms())
        .1
        .unwrap()
        .contains("not the anchor or an admin")
    );
    assert!(
      at(&log, later()).0.admin_ok(&c.public()),
      "promoted at its time"
    );
    // Cancelled by the kit: stays a writer for good.
    log.push(add(&log, &sa, None, &d)?);
    let seq = state(&log).seq;
    log.push(state(&log).cancel(&kit_signer, seq)?);
    let after = at(&log, later() + DELAY_MS).0;
    assert_eq!(after.devices[&d.public()].role, "writer");
    // Frozen while pending: not promoted.
    log.push(add(&log, &sa, None, &e)?);
    log.push(state(&log).freeze(&sb, &e.public(), 0)?);
    let after = at(&log, later() + DELAY_MS).0;
    assert_eq!(after.devices[&e.public()].role, "writer");
    // A co-signature without the two-admin setting changes nothing; with it, two admins
    // admit an admin at once.
    let mut cosigned = log.clone();
    cosigned.push(add(&cosigned, &sa, Some(&sb), &f)?);
    assert!(!state(&cosigned).admin_ok(&f.public()));
    log.push(state(&log).settings(&sa, None, 30, true)?);
    log.push(add(&log, &sa, Some(&sb), &f)?);
    assert!(state(&log).admin_ok(&f.public()), "two admins: at once");
    Ok(())
  }
  /// Findings of the 2026-10-28 review (the internal review of phases 1-5).
  #[test]
  fn refused_statements_change_nothing_and_roots_are_committed() -> Result<()> {
    let kit = Kit::generate()?;
    let (genesis, mut ring) = Membership::genesis(&kit)?;
    let mut log = vec![genesis];
    let v = kit.vault().to_string();
    let state = |log: &[Statement]| Membership::verify(&v, log).0;
    let resign = |s: &mut Statement, by: &Signer| -> Result<()> {
      s.signature = by.sign(&s.signing_bytes()?)?;
      Ok(())
    };
    let [a, x, d] = std::array::from_fn(|_| Device::generate().unwrap());
    let (sk, sa) = (Signer::Kit(&kit), Signer::Device(&a));
    log.push(state(&log).add(&sk, &ring, &a.public(), &a.seal_public(), Role::Admin)?);
    log.push(state(&log).add(&sa, &ring, &x.public(), &x.seal_public(), Role::Admin)?);
    log.push(state(&log).add(&sa, &ring, &d.public(), &d.seal_public(), Role::Writer)?);
    assert_eq!(state(&log).devices[&x.public()].role, "writer");
    // A statement far in the future, by a stranger or by a real admin: refused, and
    // the pending promotion stays pending (it used to apply through `due(s.at)`).
    let stranger = Device::generate()?;
    for by in [Signer::Device(&stranger), Signer::Device(&a)] {
      let mut junk = state(&log).settings(&by, None, 30, false)?;
      junk.at = u64::MAX / 2;
      resign(&mut junk, &by)?;
      let mut forged = log.clone();
      forged.push(junk);
      let (m, refused) = Membership::verify(&v, &forged);
      assert!(refused.is_some());
      assert_eq!(m.devices[&x.public()].role, "writer");
      assert_eq!(m.pending.len(), 1);
    }
    // A cosignature without a cosigner, and a freeze carrying an anchor, are refused.
    let mut odd = state(&log).delete(&sa, None)?;
    odd.cosignature = "ab".repeat(64);
    assert!(odd.signing_bytes().is_err());
    let mut f = state(&log).freeze(&sa, &d.public(), 0)?;
    f.anchor = "ab".repeat(32);
    resign(&mut f, &sa)?;
    let mut forged = log.clone();
    forged.push(f);
    assert!(Membership::verify(&v, &forged).1.is_some());
    // Poisoned keys: a freeze, then an unfreeze sealing bogus roots, leaves the device
    // its real roots.
    log.push(state(&log).freeze(&sa, &d.public(), 0)?);
    let bogus = Keyring {
      vault: v.clone(),
      anchor: String::new(),
      keys: BTreeMap::from([(1, VaultKey::new(&v, 1, random()?))]),
    };
    log.push(state(&log).unfreeze(&sa, &bogus, &d.public())?);
    let dev = |log: &[Statement], who: &Device| {
      keyring(
        log,
        &Credential::Device {
          device: who,
          vault: &v,
        },
      )
      .map(|r| r.0)
    };
    assert_eq!(
      dev(&log, &d)?.get(1)?.root_bytes(),
      ring.get(1)?.root_bytes()
    );
    // A rotation whose commitment does not match the root it seals: nobody takes it.
    let mut ring2 = Keyring {
      vault: v.clone(),
      anchor: ring.anchor.clone(),
      keys: ring
        .keys
        .iter()
        .map(|(e, k)| (*e, VaultKey::new(&v, *e, k.root_bytes())))
        .collect(),
    };
    let mut lying = state(&log).rotate(&sa, &mut ring2)?;
    lying.commitment = "cd".repeat(32);
    resign(&mut lying, &sa)?;
    let mut forged = log.clone();
    forged.push(lying);
    assert_eq!(dev(&forged, &d)?.epochs(), vec![1]);
    // An honest rotation checks out for the kit; a garbled kit envelope is noticed by a
    // device that holds the roots.
    log.push(state(&log).rotate(&sa, &mut ring)?);
    assert!(kit_envelope_problems(&log, &dev(&log, &a)?).is_empty());
    let mut garbled = log.clone();
    let last = garbled.last_mut().unwrap();
    let kit_env = last.envelopes.iter_mut().find(|e| e.to == "kit").unwrap();
    kit_env.sealed = seal_to(&v, "kit", &state(&log).kit_seal, b"{\"roots\":[]}")?;
    resign(last, &sa)?;
    let seq = last.seq;
    assert_eq!(kit_envelope_problems(&garbled, &dev(&log, &a)?), vec![seq]);
    // A kit replacement pending: rotations seal to the next kit too, so it reads the
    // epochs made while it waited; only one replacement at a time.
    let next = Kit::replacement(&v)?;
    log.push(state(&log).replace_kit(&sa, None, &ring, &next)?);
    assert!(
      state(&log)
        .replace_kit(&sa, None, &ring, &Kit::replacement(&v)?)
        .and_then(|s| {
          let mut l = log.clone();
          l.push(s);
          Membership::verify(&v, &l)
            .1
            .map_or(Ok(()), |_| Err(Error::Crypto))
        })
        .is_err()
    );
    log.push(state(&log).rotate(&sa, &mut ring)?);
    let (nk, _, _) = keyring(&log, &Credential::Kit(&next))?;
    assert_eq!(nk.epochs(), vec![1, 2, 3]);
    assert!(kit_envelope_problems(&log, &dev(&log, &a)?).is_empty());
    // The approvals setting applies at once even when the retention change waits.
    log.push(state(&log).settings(&sa, None, 10, true)?);
    let m = state(&log);
    assert_eq!((m.approvals, m.retention_days), (2, 30));
    Ok(())
  }
  #[test]
  fn destructive_actions_wait_unless_two_admins_approve() -> Result<()> {
    let kit = Kit::generate()?;
    let (genesis, ring) = Membership::genesis(&kit)?;
    let mut log = vec![genesis];
    let v = kit.vault().to_string();
    let at = |log: &[Statement], t: u64| Membership::verify_at(&v, log, t);
    let state = |log: &[Statement]| at(log, now_ms()).0;
    let a = Device::generate()?;
    let b = Device::generate()?;
    let w = Device::generate()?;
    for (d, role) in [(&a, Role::Admin), (&b, Role::Admin), (&w, Role::Writer)] {
      log.push(state(&log).add(
        &Signer::Kit(&kit),
        &ring,
        &d.public(),
        &d.seal_public(),
        role,
      )?);
    }
    let (sa, sb) = (Signer::Device(&a), Signer::Device(&b));
    let later = || now_ms() + DELAY_MS + 1000;
    // Shortening retention: pending until its time; lengthening is immediate.
    log.push(state(&log).settings(&sa, None, 10, false)?);
    assert_eq!(
      (state(&log).retention_days, state(&log).pending.len()),
      (30, 1)
    );
    assert_eq!(at(&log, later()).0.retention_days, 10);
    log.push(state(&log).settings(&sa, None, 60, false)?);
    assert_eq!(state(&log).retention_days, 60);
    // Delete: pending, then cancelled by the kit; it never applies.
    log.push(state(&log).delete(&sa, None)?);
    let seq = state(&log)
      .pending
      .iter()
      .find(|p| p.action == Action::Delete)
      .unwrap()
      .seq;
    log.push(state(&log).cancel(&Signer::Kit(&kit), seq)?);
    assert!(
      !at(&log, later() + DELAY_MS).0.deleted,
      "a cancelled action never applies"
    );
    // A writer cannot schedule or cancel.
    let mut forged = log.clone();
    forged.push(state(&log).delete(&Signer::Device(&w), None)?);
    assert!(
      at(&forged, now_ms())
        .1
        .unwrap()
        .contains("not the anchor or an admin")
    );
    // Co-signing changes nothing unless the vault requires two admins.
    let mut cosigned = log.clone();
    cosigned.push(state(&log).delete(&sa, Some(&sb))?);
    assert!(!state(&cosigned).deleted);
    log.push(state(&log).settings(&sa, None, 60, true)?);
    let mut now = log.clone();
    now.push(state(&log).delete(&sa, Some(&Signer::Kit(&kit)))?);
    assert!(state(&now).deleted, "two admins (one the kit): at once");
    // Revoking an admin: frozen at once, revoked only at its time.
    log.push(state(&log).freeze(&sa, &b.public(), 0)?);
    log.push(state(&log).revoke(&sa, None, &b.public(), 0)?);
    let m = state(&log);
    assert!(!m.admin_ok(&b.public()) && m.devices[&b.public()].through.is_none());
    assert_eq!(at(&log, later()).0.devices[&b.public()].through, Some(0));
    // A frozen admin cannot act; an admin's freeze is lifted by another admin, a kit
    // freeze only by the kit.
    let mut by_b = log.clone();
    by_b.push(state(&log).delete(&sb, None)?);
    assert!(at(&by_b, now_ms()).1.is_some());
    log.push(state(&log).freeze(&Signer::Kit(&kit), &a.public(), 0)?);
    let mut self_thaw = log.clone();
    self_thaw.push(state(&log).unfreeze(&sb, &ring, &a.public())?);
    assert!(
      at(&self_thaw, now_ms()).1.is_some(),
      "frozen b cannot unfreeze a"
    );
    log.push(state(&log).unfreeze(&Signer::Kit(&kit), &ring, &a.public())?);
    assert!(state(&log).admin_ok(&a.public()));
    let (r, _, _) = keyring(
      &log,
      &Credential::Device {
        device: &a,
        vault: &v,
      },
    )?;
    assert_eq!(r.epochs(), vec![1]);
    Ok(())
  }
}
