//! Experimental vault v3. Recovery is inert bytes, never an executable node store.
//! See docs/vault-protocol.md. Not a production backup/security promise.
mod checkpoint;
mod client;
mod members;

pub mod local;
pub mod node;
mod pack;
pub use checkpoint::*;
pub use client::{
  Change, Client, Issue, Limits, RecoverOptions, Recovered, Report, RequestSigner, Staged,
};
pub use members::*;
pub use pack::*;

use ring::{
  aead, digest, hkdf, hmac,
  rand::{SecureRandom, SystemRandom},
  signature::{self, KeyPair},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Largest signed commit JSON (up to `MAX_PACKS` pack lines).
pub const MAX_COMMIT: usize = 256 * 1024;
/// Packs a single commit may introduce.
pub const MAX_PACKS: usize = 1024;
pub const MAX_MEMBERS: usize = 256;
pub const MAX_NAME: usize = 1024;
/// Wire format version of commits, membership statements and manifests.
pub const VERSION: u32 = 3;
/// The only key epoch this slice accepts: rotation is designed, not implemented.
pub const EPOCH: u32 = 1;
/// `previous` of the first commit of a device and of the first membership statement.
pub const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("invalid vault data: {0}")]
  Invalid(&'static str),
  #[error("vault cryptographic verification failed")]
  Crypto,
  #[error("vault HTTP status {0}")]
  Status(u16),
  #[error("vault recovery limit: {0}")]
  Limit(&'static str),
  #[error("vault recovery staging: {0}")]
  Io(#[from] std::io::Error),
  #[error(transparent)]
  Http(#[from] reqwest::Error),
  #[error(transparent)]
  Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
  /// A copy where one can be made (errors wrapping I/O or HTTP become `Invalid`).
  pub(crate) fn clone_status(&self) -> Error {
    match self {
      Error::Invalid(s) => Error::Invalid(s),
      Error::Crypto => Error::Crypto,
      Error::Status(c) => Error::Status(*c),
      Error::Limit(s) => Error::Limit(s),
      _ => Error::Invalid("chunk unavailable"),
    }
  }
}

pub fn sha256(bytes: &[u8]) -> String {
  hex::encode(digest::digest(&digest::SHA256, bytes))
}
fn random<const N: usize>() -> Result<[u8; N]> {
  let mut bytes = [0; N];
  SystemRandom::new()
    .fill(&mut bytes)
    .map_err(|_| Error::Crypto)?;
  Ok(bytes)
}
pub(crate) fn is_hex(s: &str, bytes: usize) -> bool {
  s.len() == bytes * 2
    && s
      .bytes()
      .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// One epoch's content root. Deliberately neither Debug nor Serialize: roots travel only
/// inside sealed envelopes (members.rs).
pub struct VaultKey {
  vault: String,
  epoch: u32,
  root: Zeroizing<[u8; 32]>,
}
impl VaultKey {
  pub(crate) fn root_bytes(&self) -> [u8; 32] {
    *self.root
  }
  pub(crate) fn new(vault: &str, epoch: u32, root: [u8; 32]) -> Self {
    Self {
      vault: vault.into(),
      epoch,
      root: Zeroizing::new(root),
    }
  }
  #[cfg(test)]
  pub(crate) fn generate() -> Result<Self> {
    Ok(Self::new(&hex::encode(random::<16>()?), 1, random()?))
  }
  pub fn vault(&self) -> &str {
    &self.vault
  }
  pub fn epoch(&self) -> u32 {
    self.epoch
  }
  pub(crate) fn subkey(&self, purpose: &str) -> Result<Zeroizing<[u8; 32]>> {
    members::derive(&self.vault, self.root.as_ref(), purpose)
  }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
  Record,
  Transcript,
  Blob,
  File,
}
/// A manifest line: `item` sets the name, `None` deletes it (a tombstone).
/// Later commits (by global cursor) supersede earlier ones.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
  pub name: String,
  pub item: Option<Item>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
  pub version: u32,
  pub entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
  /// May sign membership statements (admit, revoke, rotate) and write.
  Admin,
  Writer,
  Reader,
}
impl Role {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Admin => "admin",
      Self::Writer => "writer",
      Self::Reader => "reader",
    }
  }
}

/// A device signing key and the head of its commit chain. Commits must be published
/// in the order they are made: a made commit is retried until accepted.
pub struct Device {
  key: signature::Ed25519KeyPair,
  /// The key's PKCS#8 document, to make request signers.
  pkcs8: Zeroizing<Vec<u8>>,
  /// X25519 secret that opens the envelopes sealed to this device.
  seal: x25519_dalek::StaticSecret,
  sequence: u64,
  previous: String,
  /// A signed commit whose publication failed with an error that may hide its
  /// acceptance: published again before any new commit (never a different commit at
  /// the same sequence, which would fork the chain).
  unpublished: Option<Commit>,
}
impl Device {
  /// The commit waiting to be published again, for the caller's durable state.
  pub fn unpublished(&self) -> Option<&Commit> {
    self.unpublished.as_ref()
  }
  /// Restore a commit waiting to be published (from durable state, with `resume`).
  pub fn set_unpublished(&mut self, commit: Option<Commit>) {
    self.unpublished = commit;
  }
  pub fn generate() -> Result<Self> {
    let pkcs8 =
      signature::Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| Error::Crypto)?;
    Ok(Self {
      key: signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| Error::Crypto)?,
      pkcs8: Zeroizing::new(pkcs8.as_ref().to_vec()),
      seal: Device::new_seal()?,
      sequence: 0,
      previous: ZERO.into(),
      unpublished: None,
    })
  }
  pub fn public(&self) -> String {
    hex::encode(self.key.public_key().as_ref())
  }
  /// The device's secrets as one line (`sd3:{pkcs8}:{x25519}`, hex) for an owner-only
  /// key file. The chain head is not in it: callers keep it with their durable state.
  pub fn secret_text(&self) -> Zeroizing<String> {
    Zeroizing::new(format!(
      "sd3:{}:{}",
      hex::encode(self.pkcs8.as_slice()),
      hex::encode(self.seal.to_bytes())
    ))
  }
  pub fn from_secret_text(text: &str) -> Result<Self> {
    let bad = || Error::Invalid("device key");
    let mut parts = text.trim().split(':');
    if parts.next() != Some("sd3") {
      return Err(bad());
    }
    let pkcs8 = Zeroizing::new(hex::decode(parts.next().ok_or_else(bad)?).map_err(|_| bad())?);
    let seal: [u8; 32] = hex::decode(parts.next().ok_or_else(bad)?)
      .map_err(|_| bad())?
      .try_into()
      .map_err(|_| bad())?;
    if parts.next().is_some() {
      return Err(bad());
    }
    Ok(Self {
      key: signature::Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| bad())?,
      pkcs8,
      seal: x25519_dalek::StaticSecret::from(seal),
      sequence: 0,
      previous: ZERO.into(),
      unpublished: None,
    })
  }
  pub(crate) fn sign(&self, text: &[u8]) -> signature::Signature {
    self.key.sign(text)
  }
  /// Sequence and hash of the last commit made (0 and `ZERO` before the first).
  pub fn head(&self) -> (u64, &str) {
    (self.sequence, &self.previous)
  }
  /// Continue a chain from durable local state (never from the service's word alone).
  pub fn resume(&mut self, sequence: u64, previous: &str) -> Result<()> {
    if !is_hex(previous, 32) || (sequence == 0) != (previous == ZERO) {
      return Err(Error::Invalid("device head"));
    }
    self.sequence = sequence;
    self.previous = previous.into();
    Ok(())
  }
  /// Sign the next commit: the manifest chunk and the packs this batch introduced.
  pub fn commit(
    &mut self,
    vault: &str,
    manifest: &str,
    mut packs: Vec<PackRef>,
    mut uses: Vec<String>,
  ) -> Result<Commit> {
    uses.sort();
    uses.dedup();
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    let epoch = packs.first().map_or(EPOCH, |p| p.epoch);
    let mut commit = Commit {
      version: VERSION,
      vault: vault.into(),
      epoch,
      device: self.public(),
      sequence: self.sequence + 1,
      previous: self.previous.clone(),
      manifest: manifest.into(),
      packs,
      uses,
      signature: String::new(),
    };
    let text = commit.signing_bytes()?;
    commit.signature = hex::encode(self.key.sign(&text).as_ref());
    self.sequence = commit.sequence;
    self.previous = sha256(&text);
    Ok(commit)
  }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commit {
  pub version: u32,
  pub vault: String,
  pub epoch: u32,
  pub device: String,
  pub sequence: u64,
  pub previous: String,
  /// Chunk ID of the encrypted manifest, stored in one of `packs`.
  pub manifest: String,
  pub packs: Vec<PackRef>,
  /// Existing packs this commit's items reuse chunks from: the service keeps them
  /// alive (GC) and refuses the commit if one is gone.
  pub uses: Vec<String>,
  pub signature: String,
}
impl Commit {
  pub fn signing_bytes(&self) -> Result<Vec<u8>> {
    if self.version != VERSION
      || self.epoch == 0
      || self.packs.iter().any(|p| p.epoch != self.epoch)
      || !is_hex(&self.vault, 16)
      || !is_hex(&self.device, 32)
      || self.sequence == 0
      || !is_hex(&self.previous, 32)
      || (self.sequence == 1) != (self.previous == ZERO)
      || !is_hex(&self.manifest, 32)
      || self.packs.is_empty()
      || self.packs.len() > MAX_PACKS
      || self.uses.len() > MAX_PACKS
    {
      return Err(Error::Invalid("commit header"));
    }
    let mut text = format!(
      "stitch vault commit v3\n{}\n{}\n{}\n{}\n{}\n{}\n",
      self.vault, self.epoch, self.device, self.sequence, self.previous, self.manifest
    );
    let mut previous = "";
    for p in &self.packs {
      p.validate()?;
      if p.id.as_str() <= previous {
        return Err(Error::Invalid("unsorted/duplicate packs"));
      }
      previous = &p.id;
      text.push_str(&format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        p.id, p.digest, p.size, p.index_offset, p.index_length, p.epoch
      ));
    }
    let mut previous = "";
    for u in &self.uses {
      if !is_hex(u, 32) || u.as_str() <= previous || self.packs.iter().any(|p| p.id == *u) {
        return Err(Error::Invalid("unsorted/duplicate uses"));
      }
      previous = u;
      text.push_str(&format!("uses\t{u}\n"));
    }
    Ok(text.into_bytes())
  }
  /// Chain identity: the next commit of the same device names it as `previous`.
  pub fn hash(&self) -> Result<String> {
    Ok(sha256(&self.signing_bytes()?))
  }
  /// Self-consistency only: whether the signer was a member is the membership log's job.
  pub fn verify(&self, vault: &str) -> Result<()> {
    if self.vault != vault || !is_hex(&self.signature, 64) {
      return Err(Error::Invalid("commit context"));
    }
    let public = hex::decode(&self.device).map_err(|_| Error::Crypto)?;
    let sig = hex::decode(&self.signature).map_err(|_| Error::Crypto)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public)
      .verify(&self.signing_bytes()?, &sig)
      .map_err(|_| Error::Crypto)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn transport_rejects_credential_leaks_and_insecure_endpoints() {
    let vault = "11".repeat(16);
    for endpoint in [
      "http://example.com",
      "https://user:password@example.com",
      "https://example.com/?token=secret",
      "https://example.com/#secret",
      "https://example.com/other",
    ] {
      assert!(Client::new(endpoint, &vault, "token".into()).is_err());
    }
    assert!(Client::new("https://example.invalid", &vault, "token".into()).is_ok());
    assert!(Client::new("http://127.0.0.1:12345", &vault, "token".into()).is_ok());
  }
  #[test]
  fn independent_known_answer() -> Result<()> {
    #[derive(Deserialize)]
    struct Vector {
      kit: String,
      vault: String,
      anchor: String,
      kit_seal: String,
      root: String,
      plaintext: String,
      chunk: String,
      sealed: String,
    }
    let v: Vector = serde_json::from_str(include_str!("../fixtures/crypto-v3.json"))?;
    let kit = Kit::parse(&v.kit)?;
    assert_eq!(kit.anchor_public()?, v.anchor);
    assert_eq!(vault_of(&v.anchor)?, v.vault);
    assert_eq!(kit.vault(), v.vault);
    assert_eq!(kit.seal_public()?, v.kit_seal);
    let mut root = [0u8; 32];
    hex::decode_to_slice(&v.root, &mut root).unwrap();
    let key = VaultKey::new(&v.vault, 1, root);
    let plain = hex::decode(v.plaintext).unwrap();
    assert_eq!(key.chunk_id(&plain)?, v.chunk);
    let sealed = hex::decode(v.sealed).unwrap();
    assert_eq!(key.open_chunk(1, &v.chunk, &sealed)?, plain);
    Ok(())
  }
  #[test]
  fn chunks_are_vault_keyed_and_bound() -> Result<()> {
    let key = VaultKey::generate()?;
    let other = VaultKey::generate()?;
    let id = key.chunk_id(b"same bytes")?;
    assert_ne!(other.chunk_id(b"same bytes")?, id);
    let a = key.seal_chunk(&id, b"same bytes")?;
    let b = key.seal_chunk(&id, b"same bytes")?;
    assert_ne!(a, b, "random nonces");
    assert_eq!(key.open_chunk(1, &id, &b)?, b"same bytes");
    assert!(other.open_chunk(1, &id, &a).is_err());
    assert!(key.open_chunk(2, &id, &a).is_err(), "bound to its epoch");
    let mut tampered = a.clone();
    tampered[12] ^= 1;
    assert!(key.open_chunk(1, &id, &tampered).is_err());
    Ok(())
  }
  #[test]
  fn commits_chain_and_bind_their_packs() -> Result<()> {
    let key = VaultKey::generate()?;
    let mut known = Known::new();
    let packer = Packer::new(&key, &mut known);
    let (manifest, packs, _) = packer.finish(&Manifest {
      version: VERSION,
      entries: vec![],
    })?;
    let refs: Vec<_> = packs.into_iter().map(|p| p.reference).collect();
    let mut device = Device::generate()?;
    let first = device.commit(key.vault(), &manifest, refs.clone(), vec![])?;
    assert_eq!((first.sequence, first.previous.as_str()), (1, ZERO));
    let second = device.commit(key.vault(), &manifest, refs, vec!["ab".repeat(32)])?;
    assert_eq!(second.previous, first.hash()?);
    assert_eq!(device.head(), (2, second.hash()?.as_str()));
    second.verify(key.vault())?;
    let mut c = second.clone();
    c.packs[0].index_offset -= 1;
    c.packs[0].index_length += 1;
    assert!(c.verify(key.vault()).is_err(), "pack layout is signed");
    c = second.clone();
    c.uses.clear();
    assert!(c.verify(key.vault()).is_err(), "uses are signed");
    c = second.clone();
    c.manifest = "ff".repeat(32);
    assert!(c.verify(key.vault()).is_err());
    c = second;
    c.previous = ZERO.into();
    assert!(
      c.verify(key.vault()).is_err(),
      "a later commit cannot claim to be first"
    );
    Ok(())
  }
}
