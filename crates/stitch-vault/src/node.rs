//! Opt-in adapter between a node's store and the vault (phase 4). Nothing calls it
//! unless a caller wires it up; the node does not. Two directions:
//!
//! - `backup`: every record and transcript segment of a record root, and **every
//!   original blob** they attach (eagerly, not as fetched for display), become vault
//!   items; files that disappeared (purged threads) become tombstones. A record whose
//!   blob is not on this node makes the backup *incomplete*, and says which.
//! - `preview`: a recovery becomes an isolated, inert directory: fresh record IDs
//!   (with every reference rewritten), kinds that act (schedules, assignments, claims,
//!   calls, settings, peers, ...) renamed to `inert-<kind>` so no node derives work from
//!   them, blobs verified against their hashes, and a `PREVIEW.json` that says so. It
//!   never writes into a live store, starts anything, or touches secrets (which are not
//!   records and are never backed up).
use crate::*;
use std::{
  collections::{BTreeMap, HashMap},
  fs,
  io::Read,
  path::Path,
};
use stitch_records::{BlobHash, NodeName, Record, RecordId, Segment};
use stitch_store::{BlobStore, ItemPath, RecordRoot};

/// Record kinds a preview keeps as they are: conversations and their content. Every
/// other kind (known or not: schedules, assignments, apps, SSH and peer trust,
/// settings, memory, places, ...) is renamed `inert-<kind>`, so no node derives work,
/// trust or configuration from a recovery.
pub const PASSIVE_KINDS: &[&str] = &[
  "thread",
  "report",
  "review",
  "resolution",
  "verification",
  "question",
  "answer",
];
/// Kinds that hold secrets or their sealing keys (sealed or not): never backed up.
pub const SECRET_KINDS: &[&str] = &["secret", "seal-key", "variable"];
/// Record kinds known to make a node act, or to carry membership or configuration
/// (documentation; a preview neutralises every kind not in `PASSIVE_KINDS`).
pub const ACTIVE_KINDS: &[&str] = &[
  "assignment",
  "call",
  "capabilities",
  "claim",
  "close",
  "deletion",
  "handoff",
  "instructions",
  "peer",
  "place",
  "revoke",
  "rewind",
  "run",
  "schedule",
  "set",
  "task",
  "toolset",
];
/// Changes per commit when backing up (bounded manifests and retry cost).
pub const BACKUP_BATCH: usize = 512;

#[derive(Clone, Debug, Default, Serialize)]
pub struct BackupReport {
  pub uploaded: usize,
  pub unchanged: usize,
  pub deleted: usize,
  pub commits: usize,
  /// Records of secret kinds left out (`SECRET_KINDS`).
  pub secrets_skipped: usize,
  /// Attached blobs this node does not hold: the backup is incomplete until a node
  /// that has them backs them up.
  pub missing_blobs: Vec<String>,
}
impl BackupReport {
  pub fn complete(&self) -> bool {
    self.missing_blobs.is_empty()
  }
}

/// Back up one record root and its attachments. `synced` (name → SHA256 of what the
/// vault holds) is the caller's durable state; it is updated as commits are accepted.
#[allow(clippy::too_many_arguments)]
pub async fn backup(
  client: &Client,
  key: &VaultKey,
  device: &mut Device,
  known: &mut Known,
  root: &RecordRoot,
  blobs: &BlobStore,
  synced: &mut BTreeMap<String, String>,
) -> Result<BackupReport> {
  let repo = root.repo().to_string();
  let mut report = BackupReport::default();
  let mut current: BTreeMap<String, (Kind, Vec<u8>)> = BTreeMap::new();
  let io = |e: stitch_store::Error| Error::Io(std::io::Error::other(e.to_string()));
  for item in root.scan().map_err(io)? {
    let bytes = root.read(&item).map_err(io)?;
    let kind = match &item {
      ItemPath::Record(_) => {
        if let Ok(record) = Record::parse(&String::from_utf8_lossy(&bytes)) {
          if SECRET_KINDS.contains(&record.kind.as_str()) {
            report.secrets_skipped += 1;
            continue;
          }
          for a in &record.attachments {
            let name = format!("blob/{}", a.hash.hex());
            if current.contains_key(&name) {
              continue;
            }
            if !blobs.contains(&a.hash) {
              report.missing_blobs.push(a.hash.hex());
              continue;
            }
            let mut data = vec![];
            blobs
              .open_blob(&a.hash)
              .map_err(io)?
              .read_to_end(&mut data)?;
            if BlobHash::of(&data) != a.hash {
              report.missing_blobs.push(a.hash.hex());
              continue;
            }
            current.insert(name, (Kind::Blob, data));
          }
        }
        Kind::Record
      }
      ItemPath::Segment { .. } => Kind::Transcript,
    };
    current.insert(format!("{repo}/{item}"), (kind, bytes));
  }
  report.missing_blobs.sort();
  report.missing_blobs.dedup();
  // Each change with the digest the vault will hold once it is accepted.
  let mut changes: Vec<(Change, Option<String>)> = vec![];
  for (name, (kind, bytes)) in current.iter() {
    let digest = sha256(bytes);
    if synced.get(name) == Some(&digest) {
      report.unchanged += 1;
    } else {
      changes.push(((name.clone(), Some((*kind, bytes.clone()))), Some(digest)));
    }
  }
  // Gone from this root since the last backup (a purged thread): a tombstone. Blobs
  // are shared across roots and repos; they are left to retention/GC, not deleted.
  let prefix = format!("{repo}/");
  for name in synced.keys() {
    if name.starts_with(&prefix) && !current.contains_key(name) {
      changes.push(((name.clone(), None), None));
    }
  }
  for batch in changes.chunks(BACKUP_BATCH) {
    let list: Vec<Change> = batch.iter().map(|(c, _)| c.clone()).collect();
    client.commit(key, device, known, &list).await?;
    report.commits += 1;
    for ((name, _), digest) in batch {
      match digest {
        Some(d) => {
          synced.insert(name.clone(), d.clone());
          report.uploaded += 1;
        }
        None => {
          synced.remove(name);
          report.deleted += 1;
        }
      }
    }
  }
  Ok(report)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Preview {
  pub records: usize,
  pub segments: usize,
  pub blobs: usize,
  /// Old ID → new ID.
  pub ids: BTreeMap<String, String>,
  /// New IDs of records whose kind was made inert.
  pub inert: Vec<String>,
  /// Entries not written, with the reason (unparseable, hash mismatch, unknown name).
  pub skipped: Vec<(String, String)>,
}

/// Replace every whole token (a maximal run of `[A-Za-z0-9-]`) found in `map`.
fn rewrite(text: &str, map: &HashMap<String, String>) -> String {
  let mut out = String::with_capacity(text.len());
  let mut token = String::new();
  let flush = |token: &mut String, out: &mut String| {
    out.push_str(
      map
        .get(token.as_str())
        .map_or(token.as_str(), String::as_str),
    );
    token.clear();
  };
  for c in text.chars() {
    if c.is_ascii_alphanumeric() || c == '-' {
      token.push(c);
    } else {
      flush(&mut token, &mut out);
      out.push(c);
    }
  }
  flush(&mut token, &mut out);
  out
}

/// Write a recovery into a new, isolated, inert directory (see the module comment).
pub fn preview(recovered: &Recovered, dest: &Path, node: &str) -> Result<Preview> {
  let node: NodeName = node.parse().map_err(|_| Error::Invalid("node name"))?;
  let mut preview = Preview::default();
  // Fresh IDs for every record and segment, keeping their times.
  let mut map: HashMap<String, String> = HashMap::new();
  let fresh = |id: &RecordId, map: &mut HashMap<String, String>| {
    map
      .entry(id.to_string())
      .or_insert_with(|| RecordId::random(id.timestamp(), &node).to_string())
      .clone()
  };
  let mut items = vec![];
  for name in recovered.entries().keys() {
    if let Some(hash) = name.strip_prefix("blob/") {
      items.push((name.clone(), None, Some(hash.to_string())));
      continue;
    }
    // The repository part must be a repository ID: a name like `../records/...`
    // would otherwise place a file outside the preview.
    let Some((repo, path)) = name
      .split_once('/')
      .filter(|(r, _)| r.parse::<stitch_records::RepoId>().is_ok())
    else {
      preview.skipped.push((name.clone(), "unknown name".into()));
      continue;
    };
    match ItemPath::parse(path) {
      Some(item) => {
        match &item {
          ItemPath::Record(id) => {
            fresh(id, &mut map);
          }
          ItemPath::Segment { thread, segment } => {
            fresh(thread, &mut map);
            fresh(segment, &mut map);
          }
        }
        items.push((name.clone(), Some((repo.to_string(), item)), None));
      }
      None => preview
        .skipped
        .push((name.clone(), "not a record path".into())),
    }
  }
  fs::create_dir(dest)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dest, fs::Permissions::from_mode(0o700))?;
  }
  for (name, item, hash) in items {
    let bytes = recovered.read(&name)?;
    if let Some(hash) = hash {
      if BlobHash::of(&bytes).hex() != hash {
        preview.skipped.push((name, "blob hash mismatch".into()));
        continue;
      }
      fs::create_dir_all(dest.join("blobs"))?;
      fs::write(dest.join("blobs").join(&hash), &bytes)?;
      preview.blobs += 1;
      continue;
    }
    let Some((repo, item)) = item else { continue };
    let text = rewrite(&String::from_utf8_lossy(&bytes), &map);
    let (path, text) = match item {
      ItemPath::Record(id) => {
        let Ok(mut record) = Record::parse(&text) else {
          preview.skipped.push((name, "unparseable record".into()));
          continue;
        };
        if !PASSIVE_KINDS.contains(&record.kind.as_str()) {
          record.kind = format!("inert-{}", record.kind)
            .parse()
            .map_err(|_| Error::Invalid("kind"))?;
          preview.inert.push(map[id.as_str()].clone());
        }
        preview.records += 1;
        let new: RecordId = map[id.as_str()].parse().map_err(|_| Error::Invalid("id"))?;
        (ItemPath::Record(new), record.to_text())
      }
      ItemPath::Segment { thread, segment } => {
        if Segment::parse(&text).is_err() {
          preview.skipped.push((name, "unparseable segment".into()));
          continue;
        }
        preview.segments += 1;
        let thread: RecordId = map[thread.as_str()]
          .parse()
          .map_err(|_| Error::Invalid("id"))?;
        let segment: RecordId = map[segment.as_str()]
          .parse()
          .map_err(|_| Error::Invalid("id"))?;
        (ItemPath::Segment { thread, segment }, text)
      }
    };
    let file = dest.join(&repo).join(path.to_string());
    if let Some(parent) = file.parent() {
      fs::create_dir_all(parent)?;
    }
    fs::write(file, text)?;
  }
  preview.ids = map.into_iter().collect();
  fs::write(
    dest.join("PREVIEW.json"),
    serde_json::to_vec_pretty(&serde_json::json!({
      "inert": true,
      "note": "A vault recovery preview: fresh IDs, acting kinds renamed inert-*, no secrets. Not a node store; nothing here runs.",
      "cursor": recovered.cursor(),
      "records": preview.records,
      "segments": preview.segments,
      "blobs": preview.blobs,
      "inert_records": preview.inert,
    }))?,
  )?;
  Ok(preview)
}

#[cfg(test)]
mod tests {
  use super::*;
  /// A recovery whose names try to leave the preview, and kinds outside the passive
  /// list (2026-10-28 review).
  #[test]
  fn preview_stays_inside_and_neutralises_unknown_kinds() -> Result<()> {
    use stitch_records::{Author, Draft};
    let base = std::env::temp_dir().join(format!("stitch-vault-preview-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("staging/items"))?;
    let record = |kind: &str, n: u32| -> Vec<u8> {
      Draft::new(kind, Author::user("a".parse().unwrap()))
        .unwrap()
        .body("text\n")
        .into_record(
          format!("2026-10-27T12000{n}Z-a-abc{}", (b'c' + n as u8) as char)
            .parse()
            .unwrap(),
          "2026-10-27T12:00:00Z".parse().unwrap(),
        )
        .to_text()
        .into_bytes()
    };
    let mut entries = BTreeMap::new();
    let mut put = |name: String, bytes: Vec<u8>| -> Result<()> {
      let item = sha256(name.as_bytes());
      fs::write(base.join("staging/items").join(&item), &bytes)?;
      let staged = crate::Staged {
        item,
        kind: Kind::Record,
        size: bytes.len() as u64,
        cursor: 1,
      };
      entries.insert(name, staged);
      Ok(())
    };
    put(
      "../records/2026-10/2026-10-27T120001Z-a-abcd.md".into(),
      record("note", 1),
    )?;
    put(
      "home/records/2026-10/2026-10-27T120002Z-a-abce.md".into(),
      record("thread", 2),
    )?;
    put(
      "home/records/2026-10/2026-10-27T120003Z-a-abcf.md".into(),
      record("ssh-host", 3),
    )?;
    let recovered = crate::client::Recovered::for_test(base.join("staging"), entries);
    let dest = base.join("preview");
    let p = preview(&recovered, &dest, "b")?;
    assert_eq!(p.records, 2);
    assert_eq!(p.skipped.len(), 1, "{:?}", p.skipped);
    assert!(
      !base.join("records").exists(),
      "nothing outside the preview"
    );
    let kinds: Vec<String> = fs::read_dir(dest.join("home/records/2026-10"))?
      .map(|e| {
        let text = fs::read_to_string(e.unwrap().path()).unwrap();
        Record::parse(&text).unwrap().kind.to_string()
      })
      .collect();
    let mut kinds = kinds;
    kinds.sort();
    assert_eq!(kinds, ["inert-ssh-host", "thread"]);
    fs::remove_dir_all(&base)?;
    Ok(())
  }
  #[test]
  fn rewrite_replaces_whole_tokens_only() {
    let map = HashMap::from([("2026-10-27T120000Z-a-abcd".to_string(), "NEW".to_string())]);
    assert_eq!(
      rewrite(
        "x 2026-10-27T120000Z-a-abcd, \"2026-10-27T120000Z-a-abcd\" 2026-10-27T120000Z-a-abcde",
        &map
      ),
      "x NEW, \"NEW\" 2026-10-27T120000Z-a-abcde"
    );
  }
}
