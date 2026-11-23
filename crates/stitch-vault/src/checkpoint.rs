//! Protected checkpoints and the admin operations: checkpointing (with optional
//! repacking), GC, and posting membership statements (revocation, delayed actions).
//! There is no per-item erase: deleted items expire with the retention window.
//! See docs/vault-protocol.md ("Checkpoints, retention and GC").
use crate::client::Budget;
use crate::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The full live state at a global cursor, signed by an admin device (or the kit's
/// anchor): `manifest` is a chunk in one of `packs`, and `packs` hold every chunk the
/// state needs. The service protects these packs from GC while the checkpoint is
/// retained; recovery may start from it and replay only later commits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
  pub version: u32,
  pub vault: String,
  pub cursor: u64,
  pub manifest: String,
  pub manifest_epoch: u32,
  pub packs: Vec<PackRef>,
  pub signer: String,
  /// The membership log's length and head hash when it was signed: recovery judges
  /// the signer's authority there, so a checkpoint stays usable after its signer is
  /// revoked (a lost device), and one signed after a freeze never is.
  pub members: u64,
  pub members_head: String,
  pub signature: String,
}
impl Checkpoint {
  pub fn signing_bytes(&self) -> Result<Vec<u8>> {
    if self.version != VERSION
      || !is_hex(&self.vault, 16)
      || !is_hex(&self.manifest, 32)
      || self.manifest_epoch == 0
      || !is_hex(&self.signer, 32)
      || self.members == 0
      || !is_hex(&self.members_head, 32)
      || self.packs.is_empty()
      || self.packs.len() > MAX_CHECKPOINT_PACKS
    {
      return Err(Error::Invalid("checkpoint"));
    }
    let mut text = format!(
      "stitch vault checkpoint v3\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
      self.vault,
      self.cursor,
      self.manifest,
      self.manifest_epoch,
      self.signer,
      self.members,
      self.members_head
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
    Ok(text.into_bytes())
  }
  pub fn verify(&self, vault: &str) -> Result<()> {
    if self.vault != vault {
      return Err(Error::Invalid("checkpoint context"));
    }
    verify_ed25519(&self.signer, &self.signature, &self.signing_bytes()?)
  }
}
/// Most packs one checkpoint may protect (a 100 GB vault needs ~6,000).
pub const MAX_CHECKPOINT_PACKS: usize = 100_000;

pub(crate) fn verify_ed25519(public: &str, signature: &str, text: &[u8]) -> Result<()> {
  if !is_hex(public, 32) || !is_hex(signature, 64) {
    return Err(Error::Invalid("signature"));
  }
  let public = hex::decode(public).map_err(|_| Error::Crypto)?;
  let sig = hex::decode(signature).map_err(|_| Error::Crypto)?;
  signature::UnparsedPublicKey::new(&signature::ED25519, public)
    .verify(text, &sig)
    .map_err(|_| Error::Crypto)
}
impl Device {
  pub fn sign_checkpoint(&self, c: &mut Checkpoint) -> Result<()> {
    c.signer = self.public();
    c.signature = hex::encode(self.sign(&c.signing_bytes()?).as_ref());
    Ok(())
  }
}

/// What `Client::checkpoint` did.
#[derive(Clone, Debug, Serialize)]
pub struct CheckpointOutcome {
  pub cursor: u64,
  /// Packs the checkpoint protects.
  pub packs: usize,
  /// Packs whose live chunks were copied out (they become collectable once older
  /// checkpoints expire).
  pub repacked: Vec<String>,
  /// New packs written (repacked chunks and the checkpoint manifest).
  pub written: usize,
}

impl Client {
  /// Checkpoint the live state at the current head, signed by an admin device.
  /// Packs whose live fraction is below `repack_below` (e.g. 0.5), and every pack in
  /// `force`, have their live chunks copied into new packs first, so the checkpoint
  /// stops referencing them. Chunks keep their IDs (and epochs): manifests need no
  /// rewrite.
  pub async fn checkpoint(
    &self,
    credential: &Credential<'_>,
    admin: &Device,
    repack_below: f64,
    force: &HashSet<String>,
  ) -> Result<CheckpointOutcome> {
    let mut budget = Budget::default();
    let scan = self
      .scan(credential, &RecoverOptions::default(), &mut budget)
      .await?;
    // A checkpoint of an incomplete state would let GC drop what recovery could not
    // see (e.g. history only a revoked signer's checkpoint held): refuse instead.
    if !scan.report.complete() {
      return Err(Error::Invalid(
        "checkpoint refused: the scanned state is incomplete",
      ));
    }
    let live: BTreeMap<String, Item> = scan
      .state
      .into_iter()
      .filter_map(|(name, (item, _))| item.map(|i| (name, i)))
      .collect();
    // Live chunks per pack.
    let mut per_pack: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    for item in live.values() {
      for c in &item.chunks {
        if seen.insert(c.clone()) {
          let (pack, _, _) = scan
            .chunks
            .get(c)
            .and_then(|v| v.first())
            .ok_or(Error::Invalid("chunk location"))?;
          per_pack.entry(pack.clone()).or_default().push(c.clone());
        }
      }
    }
    let mut keep = vec![];
    let mut repack: Vec<(String, Vec<String>)> = vec![];
    // Candidates: forced packs first, then the sparsest; copying stops at the repack
    // limit (the service's repack room bounds what may go past the quota), and the
    // rest is kept as is until a later checkpoint.
    let mut candidates = vec![];
    for (pack, ids) in per_pack {
      let reference = scan
        .refs
        .get(&pack)
        .ok_or(Error::Invalid("pack reference"))?;
      let live_bytes: u64 = ids.iter().map(|c| scan.chunks[c][0].2.length).sum();
      let fraction = live_bytes as f64 / reference.size.max(1) as f64;
      if force.contains(&pack) || fraction < repack_below {
        candidates.push((!force.contains(&pack), fraction, live_bytes, pack, ids));
      } else {
        keep.push(reference.clone());
      }
    }
    candidates.sort_by(|a, b| {
      (a.0, a.1)
        .partial_cmp(&(b.0, b.1))
        .unwrap_or(Ordering::Equal)
    });
    let mut copied = 0u64;
    for (optional, _, live_bytes, pack, ids) in candidates {
      if optional && copied.saturating_add(live_bytes) > self.repack_limit() {
        keep.push(scan.refs[&pack].clone());
      } else {
        copied = copied.saturating_add(live_bytes);
        repack.push((pack, ids));
      }
    }
    // Copy repacked chunks under their own epoch's key; the manifest under the newest.
    let current = scan.keys.current()?.epoch();
    let mut by_epoch: BTreeMap<u32, Vec<(String, Vec<u8>)>> = BTreeMap::new();
    for (_, ids) in &repack {
      for id in ids {
        let (pack, epoch, at) = &scan.chunks[id][0];
        let sealed = self.range(pack, at.offset, at.length, &mut budget).await?;
        let plain = scan.keys.get(*epoch)?.open_chunk(*epoch, id, &sealed)?;
        by_epoch
          .entry(*epoch)
          .or_default()
          .push((id.clone(), plain));
      }
    }
    by_epoch.entry(current).or_default();
    let manifest = Manifest {
      version: VERSION,
      entries: live
        .into_iter()
        .map(|(name, item)| Entry {
          name,
          item: Some(item),
        })
        .collect(),
    };
    let mut written = vec![];
    let mut manifest_id = String::new();
    for (epoch, chunks) in by_epoch {
      let key = scan.keys.get(epoch)?;
      let mut known = Known::new();
      let mut packer = Packer::with_target(key, &mut known, self.pack_target());
      for (id, plain) in chunks {
        packer.put(id, &plain, true)?;
      }
      if epoch == current {
        let (id, packs, _) = packer.finish(&manifest)?;
        manifest_id = id;
        written.extend(packs);
      } else {
        written.extend(packer.finish_chunks()?);
      }
    }
    for pack in &written {
      self.put_pack_in(pack, true).await?;
    }
    let mut packs: Vec<PackRef> = keep;
    packs.extend(written.iter().map(|p| p.reference.clone()));
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    let mut c = Checkpoint {
      version: VERSION,
      vault: scan.keys.vault().into(),
      cursor: scan.report.cursor,
      manifest: manifest_id,
      manifest_epoch: current,
      packs,
      signer: String::new(),
      members: scan.members,
      members_head: scan.members_head.clone(),
      signature: String::new(),
    };
    admin.sign_checkpoint(&mut c)?;
    let response = self
      .request_with(
        reqwest::Method::POST,
        "/checkpoints",
        Some(serde_json::to_vec(&c)?),
      )
      .header("content-type", "application/json")
      .send()
      .await?;
    Self::bounded(response, 1024).await?;
    Ok(CheckpointOutcome {
      cursor: c.cursor,
      packs: c.packs.len(),
      repacked: repack.into_iter().map(|(p, _)| p).collect(),
      written: written.len(),
    })
  }
  /// Ask the service to collect unprotected packs (it refuses while GC is disabled).
  pub async fn gc(&self) -> Result<serde_json::Value> {
    let response = self.request(reqwest::Method::POST, "/gc").send().await?;
    Ok(serde_json::from_slice(
      &Self::bounded(response, 1 << 20).await?,
    )?)
  }
}

impl Client {
  /// Build a statement against the verified current membership and post it. The
  /// poster is this client's credential (an admin device, or the kit's anchor).
  pub async fn propose(
    &self,
    vault: &str,
    build: impl FnOnce(&Membership) -> Result<Statement>,
  ) -> Result<Statement> {
    let log = self.statements().await?;
    let (state, refused) = Membership::verify(vault, &log);
    if refused.is_some() {
      return Err(Error::Invalid("membership log invalid"));
    }
    let s = build(&state)?;
    self.post_statement(&s).await?;
    Ok(s)
  }
  /// Destructive actions waiting for their time, from the verified log (what a node
  /// would notify about), and whether the vault is deleted.
  pub async fn pending(&self, vault: &str) -> Result<(Vec<Pending>, bool, u64)> {
    let log = self.statements().await?;
    let (state, _) = Membership::verify(vault, &log);
    Ok((state.pending, state.deleted, state.retention_days))
  }
  /// Revoke a device and rotate. A writer or reader is revoked at once. An admin is
  /// frozen at once (no access, no signing, left out of the rotation), and its
  /// revocation proper is scheduled for after the delay, unless the vault requires two
  /// admins and `cosigner` signs too.
  pub async fn revoke(
    &self,
    admin: &Signer<'_>,
    cosigner: Option<&Signer<'_>>,
    vault: &str,
    device: &str,
    through: u64,
  ) -> Result<()> {
    let log = self.statements().await?;
    let (state, _) = Membership::verify(vault, &log);
    let is_admin = state.devices.get(device).is_some_and(|m| m.role == "admin");
    if is_admin {
      self
        .propose(vault, |m| m.freeze(admin, device, through))
        .await?;
    }
    self
      .propose(vault, |m| m.revoke(admin, cosigner, device, through))
      .await?;
    let credential = match admin {
      Signer::Kit(kit) => Credential::Kit(kit),
      Signer::Device(d) => Credential::Device { device: d, vault },
    };
    let (mut ring, _, _) = self.keyring(&credential).await?;
    self.propose(vault, |m| m.rotate(admin, &mut ring)).await?;
    Ok(())
  }
  /// Post vault settings (retention days, two-admin approval). Shortening retention is
  /// delayed unless two admins approve.
  pub async fn settings(
    &self,
    admin: &Device,
    vault: &str,
    days: u64,
    two_admins: bool,
  ) -> Result<()> {
    self
      .propose(vault, |m| {
        m.settings(&Signer::Device(admin), None, days, two_admins)
      })
      .await?;
    Ok(())
  }
}
