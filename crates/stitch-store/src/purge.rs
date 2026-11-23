//! Deleting threads (design §2.5, "Deleting a thread"; docs/records.md §4,
//! `deletion`): the user's `deletion` record names threads; every node
//! removes each thread's record and segments, and the records that exist
//! only for it, and no sync brings them back.
//!
//! - **What goes with a thread:** a record of another kind than `thread`
//!   and `deletion` whose `thread`, `target`, `answers` or `claims`
//!   reference names it or another record going (its reports, questions
//!   and their answers, `set`s on it, its toolset changes, assignments for
//!   it and their claims), and a schedule that reports to it. Never another
//!   thread: the threads it spawned, its forks and the threads that report
//!   to it stay.
//! - **What a root refuses:** each root keeps the IDs it purged, and every
//!   deleted thread's, in `purged` (local, never synced). A file of a
//!   purged record, of a deleted thread's segments, or of a record that
//!   would go with one is not written, whether it comes by sync or from a
//!   loop still running here.

use std::collections::{BTreeSet, HashSet};
use std::fs;

use rusqlite::params;
use stitch_records::{BlobHash, Record, RecordId, kinds};

use crate::{BlobStore, Index, ItemPath, RecordRoot, Result};

/// Whether a record of `kind` goes with what its `rel` reference names.
pub fn goes_with(kind: &str, rel: &str) -> bool {
  if kind == kinds::THREAD || kind == kinds::DELETION {
    return false;
  }
  matches!(rel, "thread" | "target" | "answers" | "claims")
    || (kind == kinds::SCHEDULE && rel == "reports-to")
}

/// Whether `record` goes with one of `gone` (record IDs).
pub fn record_goes(record: &Record, gone: &HashSet<String>) -> bool {
  gone.contains(record.id.as_str())
    || record
      .refs
      .iter()
      .any(|(rel, t)| goes_with(record.kind.as_str(), rel) && gone.contains(t.id.as_str()))
}

/// What deleting threads takes, as the index knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
  /// The threads, and every record going with them (theirs included).
  pub records: BTreeSet<RecordId>,
  /// The files: `(repository, path)`, records and segments.
  pub files: Vec<(String, String)>,
  /// Blobs they reference (removed when nothing else does).
  pub blobs: BTreeSet<String>,
}

/// What a purge did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Purged {
  pub files: usize,
  pub blobs: usize,
}

impl Index {
  /// The threads the user deleted: what `deletes` names in `deletion`
  /// records the user wrote (one a node or an agent's run wrote counts
  /// for nothing).
  pub fn deleted_threads(&self) -> Result<BTreeSet<RecordId>> {
    let mut st = self.conn.prepare(
      "SELECT DISTINCT e.dst FROM edges e JOIN records r ON r.id = e.src
       WHERE r.kind = ?1 AND r.by_user = 1 AND e.rel = 'deletes' AND e.derived = 0",
    )?;
    let rows = st.query_map([kinds::DELETION], |r| r.get::<_, String>(0))?;
    Ok(
      rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|s| s.parse().ok())
        .collect(),
    )
  }

  /// Whether the user deleted this thread.
  pub fn is_deleted(&self, thread: &RecordId) -> Result<bool> {
    let n: i64 = self.conn.query_row(
      "SELECT COUNT(*) FROM edges e JOIN records r ON r.id = e.src
       WHERE e.dst = ?1 AND e.rel = 'deletes' AND e.derived = 0
         AND r.kind = ?2 AND r.by_user = 1",
      params![thread.as_str(), kinds::DELETION],
      |r| r.get(0),
    )?;
    Ok(n > 0)
  }

  /// What deleting `threads` (with `more`, records the caller adds) takes
  /// of what this node holds.
  pub fn purge_plan(&self, threads: &BTreeSet<RecordId>, more: &[RecordId]) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut frontier: Vec<String> = threads.iter().chain(more).map(|t| t.to_string()).collect();
    let mut seen: HashSet<String> = frontier.iter().cloned().collect();
    let mut deps = self.conn.prepare(
      "SELECT e.src, r.kind, e.rel FROM edges e JOIN records r ON r.id = e.src
       WHERE e.dst = ?1 AND e.derived = 0",
    )?;
    while let Some(id) = frontier.pop() {
      let rows = deps
        .query_map([&id], |r| {
          Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
          ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
      for (src, kind, rel) in rows {
        // A schedule reporting to a thread goes with the thread only.
        let thread = threads.iter().any(|t| t.as_str() == id);
        if goes_with(&kind, &rel) && (rel != "reports-to" || thread) && seen.insert(src.clone()) {
          frontier.push(src);
        }
      }
    }
    for id in &seen {
      let Ok(rid) = id.parse::<RecordId>() else {
        continue;
      };
      let found: Option<(String, String)> = self
        .conn
        .query_row("SELECT repo, path FROM records WHERE id = ?1", [id], |r| {
          Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
      if let Some(f) = found {
        plan.files.push(f);
      }
      plan.records.insert(rid);
    }
    let mut segs = self
      .conn
      .prepare("SELECT id, repo, path FROM segments WHERE thread = ?1")?;
    let mut owners: Vec<String> = seen.iter().cloned().collect();
    for t in threads {
      let rows = segs
        .query_map([t.as_str()], |r| {
          Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
          ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
      for (id, repo, path) in rows {
        owners.push(id);
        plan.files.push((repo, path));
      }
    }
    let mut blobs = self
      .conn
      .prepare("SELECT hash FROM blob_refs WHERE owner = ?1")?;
    for o in &owners {
      for h in blobs.query_map([o], |r| r.get::<_, String>(0))? {
        plan.blobs.insert(h?);
      }
    }
    plan.files.sort();
    Ok(plan)
  }

  /// Whether any record or segment still references a blob.
  fn blob_referenced(&self, hash: &str) -> Result<bool> {
    let n: i64 = self.conn.query_row(
      "SELECT COUNT(*) FROM blob_refs WHERE hash = ?1",
      [hash],
      |r| r.get(0),
    )?;
    Ok(n > 0)
  }
}

impl RecordRoot {
  fn purged_path(&self) -> std::path::PathBuf {
    self.dir().join("purged")
  }

  /// The IDs this root refuses (see the module's docs).
  pub fn purged(&self) -> HashSet<String> {
    fs::read_to_string(self.purged_path())
      .map(|t| t.lines().map(str::to_owned).collect())
      .unwrap_or_default()
  }

  /// Adds IDs to those this root refuses.
  pub fn add_purged<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let have = self.purged();
    let new: Vec<&str> = ids
      .into_iter()
      .filter(|i| !have.contains(*i))
      .collect::<BTreeSet<_>>()
      .into_iter()
      .collect();
    if new.is_empty() {
      return Ok(());
    }
    use std::io::Write;
    let path = self.purged_path();
    let mut f = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&path)
      .map_err(|source| crate::Error::Io {
        path: path.clone(),
        source,
      })?;
    for id in new {
      writeln!(f, "{id}").map_err(|source| crate::Error::Io {
        path: path.clone(),
        source,
      })?;
    }
    Ok(())
  }

  /// Whether this root refuses `item` (`record`: its content, when it is
  /// one).
  pub fn refuses(&self, item: &ItemPath, record: Option<&Record>) -> bool {
    let gone = self.purged();
    if gone.is_empty() {
      return false;
    }
    match item {
      ItemPath::Segment { thread, .. } => gone.contains(thread.as_str()),
      ItemPath::Record(id) => {
        gone.contains(id.as_str()) || record.is_some_and(|r| record_goes(r, &gone))
      }
    }
  }
}

/// Deletes `threads` on this node (with `more`, records the caller found
/// to go too: memory records only of them): every file of the plan, their
/// directories and open segments, then the blobs nothing references any
/// more. Every root refuses them from then on, also roots that held
/// none of it (a record for them may come to any). The roots are
/// refreshed in the index.
pub fn purge(
  index: &mut Index,
  roots: &[RecordRoot],
  blobs: &BlobStore,
  threads: &BTreeSet<RecordId>,
  more: &[RecordId],
) -> Result<Purged> {
  let plan = index.purge_plan(threads, more)?;
  let ids: Vec<&str> = plan.records.iter().map(RecordId::as_str).collect();
  for root in roots {
    root.add_purged(ids.iter().copied())?;
  }
  let mut out = Purged::default();
  for (repo, path) in &plan.files {
    let Some(root) = roots.iter().find(|r| r.repo().as_str() == repo) else {
      continue;
    };
    let Some(item) = ItemPath::parse(path) else {
      continue;
    };
    if fs::remove_file(root.path_of(&item)).is_ok() {
      out.files += 1;
    }
  }
  for root in roots {
    for t in threads {
      for dir in [
        root.dir().join("threads").join(t.as_str()),
        root.dir().join("journal").join(t.as_str()),
      ] {
        if dir.exists() {
          let _ = fs::remove_dir_all(&dir);
        }
      }
    }
    index.refresh(root)?;
  }
  for hash in &plan.blobs {
    if index.blob_referenced(hash)? {
      continue;
    }
    if let Ok(h) = hash.parse::<BlobHash>()
      && fs::remove_file(blobs.path(&h)).is_ok()
    {
      out.blobs += 1;
    }
  }
  Ok(out)
}
