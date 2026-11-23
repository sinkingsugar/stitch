//! v3 object format: items are cut into content-defined chunks, chunks are padded and
//! sealed individually, and sealed chunks are packed into ~16 MiB objects that end in
//! an encrypted index. See docs/vault-protocol.md ("Object format").
use crate::*;
use std::collections::{BTreeSet, HashMap};

/// FastCDC bounds (the largest the v2020 algorithm allows): items up to 1 MiB are one
/// chunk; larger ones split where content decides, so an edit re-uploads only the
/// chunks around it.
pub const CHUNK_MIN: usize = 1 << 20;
pub const CHUNK_AVG: usize = 4 << 20;
pub const CHUNK_MAX: usize = 16 << 20;
/// A pack is sealed once adding the next chunk would pass this (one chunk may exceed it).
pub const PACK_TARGET: usize = 16 << 20;
/// Largest pack object: one maximal chunk plus a full pack's worth, padded.
pub const MAX_PACK: u64 = 40 << 20;
/// Per sealed chunk or index: 12-byte nonce + 16-byte tag.
pub const SEAL_OVERHEAD: usize = 28;

/// Padmé (Nikitin et al., PETS 2019): rounds up so that a length leaks O(log log n)
/// bits; at most ~12% larger, 1.3% on Stitch's measured size mix.
pub fn padme(n: u64) -> u64 {
  if n < 2 {
    return n;
  }
  let e = 63 - n.leading_zeros() as u64;
  let s = 64 - e.leading_zeros() as u64;
  let mask = (1u64 << (e - s)) - 1;
  (n + mask) & !mask
}
/// Plaintext frame: data, 0x80, zeros up to the Padmé length (ISO/IEC 7816-4 style).
pub(crate) fn pad(data: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(padme(data.len() as u64 + 1) as usize);
  out.extend_from_slice(data);
  out.push(0x80);
  out.resize(padme(data.len() as u64 + 1) as usize, 0);
  out
}
pub(crate) fn unpad(mut frame: Vec<u8>) -> Result<Vec<u8>> {
  while frame.last() == Some(&0) {
    frame.pop();
  }
  if frame.pop() != Some(0x80) {
    return Err(Error::Crypto);
  }
  Ok(frame)
}

/// Content-defined cut points of an item.
pub fn chunks(data: &[u8]) -> Vec<&[u8]> {
  if data.len() <= CHUNK_MIN {
    return vec![data];
  }
  fastcdc::v2020::FastCDC::new(data, CHUNK_MIN, CHUNK_AVG, CHUNK_MAX)
    .map(|c| &data[c.offset..c.offset + c.length])
    .collect()
}

/// What a commit says about a pack it introduces. Server-visible.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackRef {
  /// Random: packs are arbitrary groupings, their IDs say nothing about content.
  pub id: String,
  /// SHA256 of the whole pack object.
  pub digest: String,
  pub size: u64,
  /// Where the sealed index sits, for one ranged read.
  pub index_offset: u64,
  pub index_length: u64,
  pub epoch: u32,
}
impl PackRef {
  pub fn validate(&self) -> Result<()> {
    if !is_hex(&self.id, 32)
      || !is_hex(&self.digest, 32)
      || !(SEAL_OVERHEAD as u64..=MAX_PACK).contains(&self.size)
      || self.index_length < SEAL_OVERHEAD as u64
      || self.index_offset.checked_add(self.index_length) != Some(self.size)
      || self.epoch == 0
    {
      return Err(Error::Invalid("pack reference"));
    }
    Ok(())
  }
}
/// One sealed chunk inside a pack (from the decrypted index).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Located {
  pub id: String,
  pub offset: u64,
  pub length: u64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackIndex {
  pub version: u32,
  pub chunks: Vec<Located>,
}

/// An item's plaintext as an ordered list of chunk IDs. Inside encrypted manifests only.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Item {
  pub kind: Kind,
  pub size: u64,
  pub chunks: Vec<String>,
}
impl Item {
  /// Stable local identity of the content (names its staged file): SHA256 over the
  /// chunk IDs, which are vault-keyed, so it reveals nothing outside the vault.
  pub fn key(&self) -> String {
    sha256(self.chunks.join("\n").as_bytes())
  }
}

pub struct Pack {
  pub reference: PackRef,
  pub bytes: Vec<u8>,
  pub chunks: Vec<Located>,
}

/// Chunk ID → the pack that holds it, for chunks already in the vault.
pub type Known = HashMap<String, String>;

/// Accumulates sealed chunks into packs of about `target` bytes.
pub struct Packer<'a> {
  key: &'a VaultKey,
  /// Chunks already stored (in the vault, or earlier in this batch): never stored twice.
  known: &'a mut Known,
  target: usize,
  current_id: String,
  current: Vec<u8>,
  located: Vec<Located>,
  /// Existing packs this batch's items reuse chunks from (a commit's `uses`).
  used: BTreeSet<String>,
  done: Vec<Pack>,
}
impl<'a> Packer<'a> {
  pub fn new(key: &'a VaultKey, known: &'a mut Known) -> Self {
    Self::with_target(key, known, PACK_TARGET)
  }
  /// A client setting: 16 MiB by default, 4–8 MiB for phones and metered links.
  pub fn with_target(key: &'a VaultKey, known: &'a mut Known, target: usize) -> Self {
    Self {
      key,
      known,
      target: target.clamp(1 << 20, 32 << 20),
      current_id: String::new(),
      current: vec![],
      located: vec![],
      used: BTreeSet::new(),
      done: vec![],
    }
  }
  /// Chunk, seal and pack an item's new chunks; returns the item (its chunk list).
  pub fn add(&mut self, kind: Kind, data: &[u8]) -> Result<Item> {
    let mut ids = vec![];
    for chunk in chunks(data) {
      let id = self.key.chunk_id(chunk)?;
      ids.push(self.put(id, chunk, false)?);
    }
    Ok(Item {
      kind,
      size: data.len() as u64,
      chunks: ids,
    })
  }
  /// Store a chunk under its known ID (repacking: the plaintext was already verified).
  pub fn put(&mut self, id: String, plain: &[u8], always: bool) -> Result<String> {
    if !always && let Some(pack) = self.known.get(&id) {
      if *pack != self.current_id && !self.done.iter().any(|p| p.reference.id == *pack) {
        self.used.insert(pack.clone());
      }
      return Ok(id);
    }
    let sealed = self.key.seal_chunk(&id, plain)?;
    if !self.current.is_empty() && self.current.len() + sealed.len() > self.target {
      self.seal_pack()?;
    }
    if self.current_id.is_empty() {
      self.current_id = hex::encode(random::<32>()?);
    }
    self.located.push(Located {
      id: id.clone(),
      offset: self.current.len() as u64,
      length: sealed.len() as u64,
    });
    self.current.extend_from_slice(&sealed);
    self.known.insert(id.clone(), self.current_id.clone());
    Ok(id)
  }
  fn seal_pack(&mut self) -> Result<()> {
    if self.current_id.is_empty() {
      return Ok(());
    }
    let id = std::mem::take(&mut self.current_id);
    let chunks = std::mem::take(&mut self.located);
    let index = self.key.seal_index(
      &id,
      &serde_json::to_vec(&PackIndex {
        version: VERSION,
        chunks: chunks.clone(),
      })?,
    )?;
    let mut bytes = std::mem::take(&mut self.current);
    // Zero filler before the index pads the whole object to a Padmé length.
    let raw = (bytes.len() + index.len()) as u64;
    bytes.resize((padme(raw) - index.len() as u64) as usize, 0);
    let index_offset = bytes.len() as u64;
    bytes.extend_from_slice(&index);
    let reference = PackRef {
      id,
      digest: sha256(&bytes),
      size: bytes.len() as u64,
      index_offset,
      index_length: index.len() as u64,
      epoch: self.key.epoch,
    };
    self.done.push(Pack {
      reference,
      bytes,
      chunks,
    });
    Ok(())
  }
  /// Close the open pack without a manifest (repacking an older epoch's chunks).
  pub fn finish_chunks(mut self) -> Result<Vec<Pack>> {
    self.seal_pack()?;
    Ok(self.done)
  }
  /// Seal the manifest as the batch's last chunk and close the open pack. Returns the
  /// manifest's chunk ID, the new packs, and the existing packs the batch reuses.
  pub fn finish(mut self, manifest: &Manifest) -> Result<(String, Vec<Pack>, Vec<String>)> {
    let bytes = serde_json::to_vec(manifest)?;
    // The manifest is always stored, so its commit's (or checkpoint's) packs contain it.
    let id = self.key.chunk_id(&bytes)?;
    let id = self.put(id, &bytes, true)?;
    self.seal_pack()?;
    let new: BTreeSet<_> = self.done.iter().map(|p| p.reference.id.clone()).collect();
    let used = self.used.into_iter().filter(|p| !new.contains(p)).collect();
    Ok((id, self.done, used))
  }
}

impl VaultKey {
  /// Vault-keyed: equal plaintext chunks share an ID only inside this vault.
  pub fn chunk_id(&self, plain: &[u8]) -> Result<String> {
    let key = self.subkey("dedup")?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, key.as_ref());
    let mut ctx = hmac::Context::with_key(&key);
    ctx.update(b"chunk\0");
    ctx.update(digest::digest(&digest::SHA256, plain).as_ref());
    Ok(hex::encode(ctx.sign()))
  }
  fn derived(&self, label: &str) -> Result<aead::LessSafeKey> {
    let base = self.subkey("aead")?;
    let base = hmac::Key::new(hmac::HMAC_SHA256, base.as_ref());
    let tag = hmac::sign(&base, label.as_bytes());
    Ok(aead::LessSafeKey::new(
      aead::UnboundKey::new(&aead::CHACHA20_POLY1305, tag.as_ref()).map_err(|_| Error::Crypto)?,
    ))
  }
  fn seal_with(&self, key: aead::LessSafeKey, aad: String, plain: &[u8]) -> Result<Vec<u8>> {
    let nonce = random::<12>()?;
    let mut body = pad(plain);
    key
      .seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(aad.as_bytes()),
        &mut body,
      )
      .map_err(|_| Error::Crypto)?;
    Ok([nonce.as_slice(), &body].concat())
  }
  fn open_with(&self, key: aead::LessSafeKey, aad: String, sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < SEAL_OVERHEAD {
      return Err(Error::Crypto);
    }
    let nonce: [u8; 12] = sealed[..12].try_into().map_err(|_| Error::Crypto)?;
    let mut body = sealed[12..].to_vec();
    let plain = key
      .open_in_place(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(aad.as_bytes()),
        &mut body,
      )
      .map_err(|_| Error::Crypto)?;
    let len = plain.len();
    body.truncate(len);
    unpad(body)
  }
  fn chunk_key(&self, epoch: u32, id: &str) -> Result<(aead::LessSafeKey, String)> {
    Ok((
      self.derived(&format!("stitch vault chunk key v3\n{epoch}\n{id}\n"))?,
      format!("stitch vault chunk v3\n{}\n{epoch}\n{id}\n", self.vault),
    ))
  }
  /// One key per chunk identity: random nonces can only repeat across encryptions of
  /// the same plaintext (harmless), never across the vault.
  pub fn seal_chunk(&self, id: &str, plain: &[u8]) -> Result<Vec<u8>> {
    if plain.len() > CHUNK_MAX {
      return Err(Error::Invalid("chunk too large"));
    }
    let (key, aad) = self.chunk_key(self.epoch, id)?;
    self.seal_with(key, aad, plain)
  }
  /// Decrypt, unpad and check the keyed ID: a chunk cannot be swapped for another.
  pub fn open_chunk(&self, epoch: u32, id: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    if epoch != self.epoch || !is_hex(id, 32) {
      return Err(Error::Invalid("chunk reference"));
    }
    let (key, aad) = self.chunk_key(epoch, id)?;
    let plain = self.open_with(key, aad, sealed)?;
    if self.chunk_id(&plain)? != id {
      return Err(Error::Crypto);
    }
    Ok(plain)
  }
  fn index_key(&self, epoch: u32, pack: &str) -> Result<(aead::LessSafeKey, String)> {
    Ok((
      self.derived(&format!(
        "stitch vault pack index key v3\n{epoch}\n{pack}\n"
      ))?,
      format!(
        "stitch vault pack index v3\n{}\n{epoch}\n{pack}\n",
        self.vault
      ),
    ))
  }
  fn seal_index(&self, pack: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let (key, aad) = self.index_key(self.epoch, pack)?;
    self.seal_with(key, aad, plain)
  }
  /// A pack's chunk table, checked to lie inside the pack before its index.
  pub fn open_index(&self, pack: &PackRef, sealed: &[u8]) -> Result<Vec<Located>> {
    pack.validate()?;
    if pack.epoch != self.epoch {
      return Err(Error::Invalid("pack epoch"));
    }
    let (key, aad) = self.index_key(pack.epoch, &pack.id)?;
    let index: PackIndex = serde_json::from_slice(&self.open_with(key, aad, sealed)?)?;
    if index.version != VERSION {
      return Err(Error::Invalid("pack index"));
    }
    for c in &index.chunks {
      if !is_hex(&c.id, 32)
        || c.length < SEAL_OVERHEAD as u64
        || c
          .offset
          .checked_add(c.length)
          .is_none_or(|end| end > pack.index_offset)
      {
        return Err(Error::Invalid("pack index"));
      }
    }
    Ok(index.chunks)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn padme_matches_the_paper_and_frames_round_trip() -> Result<()> {
    assert_eq!(
      [0, 1, 2, 9, 100, 1000, 1_000_000].map(padme),
      [0, 1, 2, 10, 104, 1024, 1_015_808]
    );
    for n in [0usize, 1, 127, 128, 4096, 70_001] {
      let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8 + 1).collect();
      let mut zeros = data.clone();
      zeros.extend([0, 0]); // trailing zeros in the data survive
      assert_eq!(unpad(pad(&data))?, data);
      assert_eq!(unpad(pad(&zeros))?, zeros);
      assert_eq!(pad(&data).len() as u64, padme(n as u64 + 1));
    }
    Ok(())
  }
  #[test]
  fn an_edit_changes_only_nearby_chunks() {
    let mut state = 7u64;
    let data: Vec<u8> = (0..40 << 20)
      .map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as u8
      })
      .collect();
    let before: Vec<_> = chunks(&data).iter().map(|c| sha256(c)).collect();
    let mut edited = data.clone();
    edited.splice(
      20 << 20..20 << 20,
      b"inserted in the middle".iter().copied(),
    );
    let after: Vec<_> = chunks(&edited).iter().map(|c| sha256(c)).collect();
    let changed = after.iter().filter(|c| !before.contains(c)).count();
    assert!(before.len() >= 4, "{} chunks", before.len());
    assert!(changed <= 2, "{changed} of {} chunks changed", after.len());
    assert!(chunks(&data).iter().all(|c| c.len() <= CHUNK_MAX));
  }
  #[test]
  fn packs_seal_chunks_and_an_index_and_deduplicate() -> Result<()> {
    let key = VaultKey::generate()?;
    let mut known = Known::new();
    let mut packer = Packer::new(&key, &mut known);
    let a = packer.add(Kind::Record, b"record a")?;
    let again = packer.add(Kind::Blob, b"record a")?;
    assert_eq!(a.chunks, again.chunks, "same bytes, one chunk, any kind");
    let big = vec![9u8; 3 << 20];
    let b = packer.add(Kind::Blob, &big)?;
    let (manifest, packs, _) = packer.finish(&Manifest {
      version: VERSION,
      entries: vec![],
    })?;
    assert_eq!(packs.len(), 1);
    let pack = &packs[0];
    pack.reference.validate()?;
    assert_eq!(pack.reference.size, padme(pack.reference.size));
    let r = &pack.reference;
    let index = key.open_index(r, &pack.bytes[r.index_offset as usize..])?;
    assert_eq!(index.len(), 3, "a, the big chunk, the manifest");
    assert!(index.iter().any(|c| c.id == manifest));
    let located = index.iter().find(|c| c.id == b.chunks[0]).unwrap();
    let sealed = &pack.bytes[located.offset as usize..(located.offset + located.length) as usize];
    assert_eq!(key.open_chunk(EPOCH, &b.chunks[0], sealed)?, big);
    assert!(
      key.open_chunk(EPOCH, &a.chunks[0], sealed).is_err(),
      "IDs are bound"
    );
    let other = VaultKey::generate()?;
    assert!(
      other
        .open_index(r, &pack.bytes[r.index_offset as usize..])
        .is_err()
    );
    Ok(())
  }
  #[test]
  fn packs_split_near_the_target() -> Result<()> {
    let key = VaultKey::generate()?;
    let mut known = Known::new();
    let mut packer = Packer::new(&key, &mut known);
    for i in 0..40u8 {
      // 40 distinct 1 MiB items: about 40 MiB in packs of about 16 MiB.
      packer.add(Kind::Blob, &vec![i; 1 << 20])?;
    }
    let (_, packs, _) = packer.finish(&Manifest {
      version: VERSION,
      entries: vec![],
    })?;
    assert_eq!(packs.len(), 3);
    assert!(packs.iter().all(|p| p.reference.size <= MAX_PACK));
    assert!(packs[0].reference.size > 15 << 20);
    Ok(())
  }
}
