//! The local index (docs/records.md §8, §10): SQLite, derived from the record
//! roots a node holds and rebuildable at any time. It holds the files and
//! their hashes (for sync), the records, the graph's edges, segments and
//! usage, and answers the derived-status questions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value as Json;
use stitch_records::{
  Attachment, BlobHash, Error as FormatError, Record, RecordId, Ref, RepoId, Segment, kinds,
};

use crate::artefacts;
use crate::changes::{Change, ChangeFeed};
use crate::sync::{Fingerprint, Item, ItemSet};
use crate::{Error, ItemPath, RecordRoot, Result};

/// Bumped when the schema changes; an index with another version is rebuilt.
const SCHEMA: i64 = 5;

const TABLES: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE roots(repo TEXT PRIMARY KEY, dir TEXT NOT NULL);
-- Every file of every root, parsed or not (a newer format still syncs).
CREATE TABLE files(repo TEXT NOT NULL, path TEXT NOT NULL, hash BLOB NOT NULL, PRIMARY KEY(repo, path)) WITHOUT ROWID;
CREATE TABLE records(
    id TEXT PRIMARY KEY, repo TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL,
    created INTEGER NOT NULL, node TEXT NOT NULL, by_user INTEGER NOT NULL, run TEXT,
    title TEXT, fields TEXT NOT NULL);
CREATE INDEX records_kind ON records(kind, created);
CREATE INDEX records_path ON records(repo, path);
-- The graph. `derived` edges (mentions) were not written as refs.
CREATE TABLE edges(
    src TEXT NOT NULL, rel TEXT NOT NULL, dst TEXT NOT NULL, dst_sub TEXT NOT NULL,
    dst_repo TEXT, created INTEGER NOT NULL, derived INTEGER NOT NULL,
    PRIMARY KEY(src, rel, dst, dst_sub)) WITHOUT ROWID;
CREATE INDEX edges_dst ON edges(dst, rel);
CREATE INDEX edges_rel ON edges(rel, dst_sub);
CREATE TABLE attachments(record TEXT NOT NULL, hash TEXT NOT NULL, type TEXT NOT NULL, name TEXT, bytes INTEGER NOT NULL);
CREATE INDEX attachments_record ON attachments(record);
CREATE TABLE segments(
    id TEXT PRIMARY KEY, repo TEXT NOT NULL, path TEXT NOT NULL, thread TEXT NOT NULL,
    prev TEXT, created INTEGER NOT NULL, node TEXT NOT NULL, rewind TEXT);
CREATE INDEX segments_thread ON segments(thread);
CREATE TABLE usage(
    segment TEXT NOT NULL, seq INTEGER NOT NULL, thread TEXT NOT NULL, at INTEGER NOT NULL,
    model TEXT NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, usd REAL,
    PRIMARY KEY(segment, seq));
CREATE INDEX usage_thread ON usage(thread);
-- What the tool calls of a sealed segment touched (artefacts.rs), once per
-- (kind, value, verb) in a segment; messages are local IDs (`m12`).
CREATE TABLE artefacts(
    segment TEXT NOT NULL, thread TEXT NOT NULL, kind TEXT NOT NULL, value TEXT NOT NULL,
    verb TEXT NOT NULL, first_at INTEGER NOT NULL, last_at INTEGER NOT NULL, n INTEGER NOT NULL,
    first_msg TEXT, last_msg TEXT, subject TEXT,
    PRIMARY KEY(segment, kind, value, verb)) WITHOUT ROWID;
CREATE INDEX artefacts_thread ON artefacts(thread);
CREATE INDEX artefacts_value ON artefacts(kind, value);
-- The latest digest of each thread (design §2.25, step 2), filled from its
-- `digest` records as they are indexed: what a list shows without reading
-- the record (headline, counts of open questions and asks).
CREATE TABLE digests(
    thread TEXT PRIMARY KEY, record TEXT NOT NULL, created INTEGER NOT NULL, upto TEXT,
    step INTEGER, headline TEXT NOT NULL, asks INTEGER NOT NULL, questions INTEGER NOT NULL) WITHOUT ROWID;
CREATE INDEX digests_record ON digests(record);
-- Every blob a record or segment references, for pulling the ones missing.
CREATE TABLE blob_refs(owner TEXT NOT NULL, repo TEXT NOT NULL, hash TEXT NOT NULL, PRIMARY KEY(owner, hash)) WITHOUT ROWID;
CREATE INDEX blob_refs_repo ON blob_refs(repo);
";

const ALL_TABLES: [&str; 11] = [
  "blob_refs",
  "meta",
  "roots",
  "files",
  "records",
  "edges",
  "attachments",
  "segments",
  "usage",
  "artefacts",
  "digests",
];

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedRecord {
  pub id: RecordId,
  pub repo: RepoId,
  pub path: String,
  pub kind: String,
  pub created: Timestamp,
  pub node: String,
  pub by_user: bool,
  pub run: Option<RecordId>,
  /// From the body; a `set` of `title` overrides it (see `Index::title`).
  pub title: Option<String>,
  /// The record's fields as JSON (date-times become RFC 3339 strings).
  pub fields: Json,
}

/// What a thread did to one artefact (`Index::artefacts`): `kind` and `verb`
/// as `artefacts::Kind::as_str` and `Verb::as_str` say them; the messages
/// are local IDs (`m12`) of that thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
  pub thread: RecordId,
  pub kind: String,
  pub value: String,
  pub verb: String,
  pub first_at: Timestamp,
  pub last_at: Timestamp,
  pub n: u32,
  pub first_msg: Option<String>,
  pub last_msg: Option<String>,
  pub subject: Option<String>,
}

/// A thread's latest digest as the index keeps it (`Index::digests`): what
/// a list shows; the whole digest is the record `record`.
#[derive(Clone, Debug, PartialEq)]
pub struct DigestRow {
  pub thread: RecordId,
  pub record: RecordId,
  pub created: Timestamp,
  /// The last message it covers (`<thread>#<id>`).
  pub upto: Option<String>,
  /// The thread's latest segment when it was read: later activity makes
  /// the digest stale.
  pub step: Option<Timestamp>,
  pub headline: String,
  pub asks: u32,
  pub questions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
  pub src: RecordId,
  pub rel: String,
  pub dst: Ref,
  pub created: Timestamp,
  pub derived: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RefreshReport {
  pub records: usize,
  pub segments: usize,
  /// Files in a format newer than this node reads: kept and synced, not indexed.
  pub newer: Vec<String>,
  /// Files that failed verification, moved out of the layout.
  pub quarantined: Vec<(String, String)>,
  /// Files gone from the root since the last refresh.
  pub removed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
  Open,
  Running,
  Waiting,
  OverBudget,
  ToReview,
  Done,
  Dropped,
  Failed,
}

impl TaskStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      TaskStatus::Open => "open",
      TaskStatus::Running => "running",
      TaskStatus::Waiting => "waiting",
      TaskStatus::OverBudget => "over-budget",
      TaskStatus::ToReview => "to-review",
      TaskStatus::Done => "done",
      TaskStatus::Dropped => "dropped",
      TaskStatus::Failed => "failed",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingStatus {
  Open,
  Addressed,
  Verified,
  StillFailing,
}

impl FindingStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      FindingStatus::Open => "open",
      FindingStatus::Addressed => "addressed",
      FindingStatus::Verified => "verified",
      FindingStatus::StillFailing => "still-failing",
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignmentStatus {
  Unclaimed,
  Claimed { node: String, until: Timestamp },
  Expired { node: String, until: Timestamp },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetValue {
  pub value: Json,
  pub record: RecordId,
  /// Two `set` records superseded the same value (concurrent edits); the
  /// later one won.
  pub conflict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttentionItem {
  Question { id: RecordId, title: Option<String> },
  Finding { finding: Ref },
  OverBudget { task: RecordId },
}

/// The index connection's page cache, in KiB: less on a phone or headset,
/// whose memory the system takes back from an app that holds much.
#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
const CACHE_KIB: u32 = 64 * 1024;
#[cfg(any(target_os = "ios", target_os = "visionos"))]
const CACHE_KIB: u32 = 16 * 1024;

pub struct Index {
  pub(crate) conn: Connection,
  /// Who is told of what comes in (`changes`).
  feed: ChangeFeed,
  /// Each repository's whole fingerprint, kept once asked for and brought
  /// up to date as files come in.
  fingerprints: HashMap<String, Fingerprint>,
  /// `PRAGMA data_version` at the last `poll_changes`.
  polled: Option<i64>,
  /// What `records_shared` read, while the index has not changed.
  kept: std::cell::RefCell<Kept>,
}

#[derive(Default)]
struct Kept {
  stamp: Option<(i64, u64)>,
  kinds: HashMap<String, Arc<Vec<IndexedRecord>>>,
}

fn ms(t: Timestamp) -> i64 {
  t.as_millisecond()
}

fn from_ms(v: i64) -> Timestamp {
  Timestamp::from_millisecond(v).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn toml_to_json(v: &toml::Value) -> Json {
  match v {
    toml::Value::String(s) => Json::String(s.clone()),
    toml::Value::Integer(i) => Json::from(*i),
    toml::Value::Float(f) => serde_json::Number::from_f64(*f).map_or(Json::Null, Json::Number),
    toml::Value::Boolean(b) => Json::Bool(*b),
    toml::Value::Datetime(d) => Json::String(d.to_string()),
    toml::Value::Array(a) => Json::Array(a.iter().map(toml_to_json).collect()),
    toml::Value::Table(t) => Json::Object(
      t.iter()
        .map(|(k, v)| (k.clone(), toml_to_json(v)))
        .collect(),
    ),
  }
}

/// `stitch:<ref>` links in a body.
fn mentions(body: &str) -> Vec<Ref> {
  let mut out = Vec::new();
  for (start, _) in body.match_indices("stitch:") {
    let rest = &body[start + "stitch:".len()..];
    let end = rest
      .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '#' | '_')))
      .unwrap_or(rest.len());
    if let Ok(r) = rest[..end].parse() {
      out.push(r);
    }
  }
  out
}

fn record_id(s: String) -> rusqlite::Result<RecordId> {
  s.parse().map_err(|e: FormatError| {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
  })
}

impl Index {
  /// A value this node keeps for itself (never synced, lost with the
  /// index): a loop's step not written yet, say. `None` removes it.
  pub fn set_local(&self, key: &str, value: Option<&str>) -> Result<()> {
    let key = format!("local:{key}");
    match value {
      Some(v) => self.conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, v],
      )?,
      None => self
        .conn
        .execute("DELETE FROM meta WHERE key = ?1", [key])?,
    };
    Ok(())
  }

  /// What `set_local` kept under `key`.
  pub fn local(&self, key: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(
      self
        .conn
        .query_row(
          "SELECT value FROM meta WHERE key = ?1",
          [format!("local:{key}")],
          |r| r.get(0),
        )
        .optional()?,
    )
  }

  pub fn open(path: &Path) -> Result<Self> {
    Index::with(Connection::open(path)?)
  }

  pub fn open_in_memory() -> Result<Self> {
    Index::with(Connection::open_in_memory()?)
  }

  fn with(conn: Connection) -> Result<Self> {
    // The node and short-lived commands of the same binary share the file.
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    // SQLite's default page cache is 2 MB: smaller than the records
    // table alone, so every pass over a kind's records (a list's, each
    // second while a client follows the node) read its pages again from
    // the OS, hundreds of MB a second on a large index. Pages are only
    // held once read, so a small index costs no more.
    conn.execute_batch(&format!("PRAGMA cache_size = -{CACHE_KIB};"))?;
    let version: Option<i64> = conn
      .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get::<_, String>(0)
      })
      .optional()
      .unwrap_or(None)
      .and_then(|v| v.parse().ok());
    let mut index = Index {
      conn,
      feed: ChangeFeed::default(),
      fingerprints: HashMap::new(),
      polled: None,
      kept: Default::default(),
    };
    if version != Some(SCHEMA) {
      index.reset()?;
    }
    // Added to the schema without a new version (no rebuild): `attention`
    // (each second while a client listens) found the `verifies` edges by
    // scanning every edge.
    index
      .conn
      .execute_batch("CREATE INDEX IF NOT EXISTS edges_rel ON edges(rel, dst_sub);")?;
    Ok(index)
  }

  /// Empties the index (its tables dropped and made again, in one
  /// transaction), to be filled again by `refresh`. Done through SQLite,
  /// never by deleting the files: other processes (the node, a TUI) keep
  /// the database open, and a WAL file or shared memory deleted under them
  /// corrupts what they read.
  pub fn reset(&mut self) -> Result<()> {
    let tx = self.conn.transaction()?;
    for t in ALL_TABLES {
      tx.execute_batch(&format!("DROP TABLE IF EXISTS {t};"))?;
    }
    tx.execute_batch(TABLES)?;
    tx.execute(
      "INSERT INTO meta(key, value) VALUES ('schema', ?1)",
      [SCHEMA.to_string()],
    )?;
    tx.commit()?;
    self.fingerprints.clear();
    Ok(())
  }

  /// The feed that tells of files coming in (`changes`).
  pub fn changes(&self) -> ChangeFeed {
    self.feed.clone()
  }

  /// A repository's whole fingerprint, from what the index holds.
  pub fn fingerprint(&mut self, repo: &RepoId) -> Result<Fingerprint> {
    if let Some(f) = self.fingerprints.get(repo.as_str()) {
      return Ok(*f);
    }
    let f = self.items(repo)?.summary();
    self.fingerprints.insert(repo.to_string(), f);
    Ok(f)
  }

  /// What another process indexed since the last call (the index is
  /// shared: a command's `refresh` makes files known here without this
  /// one seeing them come): each repository whose fingerprint was kept and
  /// is no longer the same is told to the feed. The first call only notes
  /// where things are.
  pub fn poll_changes(&mut self) -> Result<Vec<RepoId>> {
    let version = self.data_version()?;
    let first = self.polled.is_none();
    if self.polled == Some(version) {
      return Ok(Vec::new());
    }
    self.polled = Some(version);
    if first {
      return Ok(Vec::new());
    }
    let mut changed = Vec::new();
    let repos: Vec<String> = self.fingerprints.keys().cloned().collect();
    for repo in repos {
      let Ok(id) = repo.parse::<RepoId>() else {
        continue;
      };
      let now = self.items(&id)?.summary();
      if self.fingerprints.insert(repo, now) != Some(now) {
        self.feed.emit(&Change {
          repo: id.clone(),
          added: Vec::new(),
          removed: 0,
          origin: None,
        });
        changed.push(id);
      }
    }
    Ok(changed)
  }

  /// Brings the index up to date with a root: new files are hashed, parsed
  /// and indexed (files are immutable, so known ones are not reread), and
  /// files gone from disk are dropped.
  pub fn refresh(&mut self, root: &RecordRoot) -> Result<RefreshReport> {
    self.refresh_from(root, None)
  }

  /// `refresh`, for files a sync with `origin` brought: the change it
  /// reports says where they came from.
  pub fn refresh_from(&mut self, root: &RecordRoot, origin: Option<&str>) -> Result<RefreshReport> {
    let repo = root.repo().to_string();
    let mut report = RefreshReport::default();
    let known: HashSet<String> = {
      let mut st = self
        .conn
        .prepare("SELECT path FROM files WHERE repo = ?1")?;
      st.query_map([&repo], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?
    };
    let on_disk = root.scan()?;
    let present: HashSet<String> = on_disk.iter().map(ToString::to_string).collect();
    // Nothing new, nothing gone, the root where it was: no write, so
    // other connections' `data_version` (what a TUI watches) stays.
    let dir = root.dir().display().to_string();
    if present == known {
      let stored: Option<String> = self
        .conn
        .query_row("SELECT dir FROM roots WHERE repo = ?1", [&repo], |r| {
          r.get(0)
        })
        .optional()?;
      if stored.as_deref() == Some(dir.as_str()) {
        return Ok(report);
      }
    }
    let tx = self.conn.transaction()?;
    tx.execute(
      "INSERT INTO roots(repo, dir) VALUES (?1, ?2) ON CONFLICT(repo) DO UPDATE SET dir = ?2",
      params![repo, dir],
    )?;
    for gone in known.difference(&present) {
      remove_path(&tx, &repo, gone)?;
      report.removed += 1;
    }
    let mut added = Vec::new();
    for item in &on_disk {
      let path = item.to_string();
      if known.contains(&path) {
        continue;
      }
      let bytes = root.read(item)?;
      let parsed = std::str::from_utf8(&bytes)
        .map_err(|_| "not UTF-8".to_owned())
        .and_then(|text| match item {
          ItemPath::Record(id) => match Record::parse(text) {
            Ok(r) if &r.id == id => Ok(Some(Parsed::Record(Box::new(r)))),
            Ok(r) => Err(format!("holds record {}", r.id)),
            Err(FormatError::Version(_)) => Ok(None),
            Err(e) => Err(e.to_string()),
          },
          ItemPath::Segment { thread, segment } => match Segment::parse(text) {
            Ok(s) if &s.header.segment == segment && &s.header.thread == thread => {
              Ok(Some(Parsed::Segment(Box::new(s))))
            }
            Ok(_) => Err("IDs do not match the path".to_owned()),
            Err(FormatError::Version(_)) => Ok(None),
            Err(e) => Err(e.to_string()),
          },
        });
      match parsed {
        Err(why) => {
          root.quarantine(item)?;
          report.quarantined.push((path, why));
          continue;
        }
        Ok(None) => report.newer.push(path.clone()),
        Ok(Some(Parsed::Record(r))) => {
          insert_record(&tx, &repo, &path, &r)?;
          report.records += 1;
        }
        Ok(Some(Parsed::Segment(s))) => {
          insert_segment(&tx, &repo, &path, &s)?;
          report.segments += 1;
        }
      }
      let hash = *BlobHash::of(&bytes).0.as_bytes();
      tx.execute(
        "INSERT OR REPLACE INTO files(repo, path, hash) VALUES (?1, ?2, ?3)",
        params![repo, path, hash.as_slice()],
      )?;
      added.push(Item { key: path, hash });
    }
    tx.commit()?;
    if report.removed > 0 {
      self.fingerprints.remove(&repo);
    } else if let Some(f) = self.fingerprints.get_mut(&repo) {
      for item in &added {
        f.add(item);
      }
    }
    if !added.is_empty() || report.removed > 0 {
      self.feed.emit(&Change {
        repo: root.repo().clone(),
        added: added.into_iter().map(|i| i.key).collect(),
        removed: report.removed,
        origin: origin.map(str::to_owned),
      });
    }
    Ok(report)
  }

  /// The sync item set of a repository, from the hashes already known.
  pub fn items(&self, repo: &RepoId) -> Result<ItemSet> {
    let mut st = self
      .conn
      .prepare("SELECT path, hash FROM files WHERE repo = ?1")?;
    let items = st
      .query_map([repo.as_str()], |r| {
        let hash: Vec<u8> = r.get(1)?;
        Ok(Item {
          key: r.get(0)?,
          hash: hash.try_into().unwrap_or([0; 32]),
        })
      })?
      .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ItemSet::new(items))
  }

  pub fn record(&self, id: &RecordId) -> Result<Option<IndexedRecord>> {
    Ok(
      self
        .conn
        .query_row(
          &format!("{SELECT_RECORD} WHERE id = ?1"),
          [id.as_str()],
          row_record,
        )
        .optional()?,
    )
  }

  /// Records of a kind (all when `None`), oldest first.
  pub fn records(&self, kind: Option<&str>) -> Result<Vec<IndexedRecord>> {
    // Two statements: `?1 IS NULL OR kind = ?1` cannot use `records_kind`
    // (the plan is made before the value is known), so every kind's read
    // scanned the whole table: the 8,000 records of a year, for the few
    // projects a live view asks for on each of its rows.
    let rows = match kind {
      Some(k) => self
        .conn
        .prepare_cached(RECORDS_OF_KIND)?
        .query_map([k], row_record)?
        .collect::<rusqlite::Result<_>>()?,
      None => self
        .conn
        .prepare_cached(&format!("{SELECT_RECORD} ORDER BY created, id"))?
        .query_map([], row_record)?
        .collect::<rusqlite::Result<_>>()?,
    };
    Ok(rows)
  }

  /// `records(Some(kind))`, shared: read once and kept until the index
  /// changes (a write here, or another connection's), for the readers that
  /// ask for the same kind again and again (a list asks for the `place`
  /// records once per place it counts, and each holds its skills whole).
  /// SQLite reads a large value's overflow pages from the file every time,
  /// past its page cache, so `records` of such a kind reads megabytes each
  /// call however large the cache.
  pub fn records_shared(&self, kind: &str) -> Result<Arc<Vec<IndexedRecord>>> {
    let stamp = self.stamp()?;
    let mut kept = self.kept.borrow_mut();
    if kept.stamp != Some(stamp) {
      kept.stamp = Some(stamp);
      kept.kinds.clear();
    }
    if let Some(v) = kept.kinds.get(kind) {
      return Ok(v.clone());
    }
    let v = Arc::new(self.records(Some(kind))?);
    kept.kinds.insert(kind.to_owned(), v.clone());
    Ok(v)
  }

  /// What moves whenever the index changes: `PRAGMA data_version` (other
  /// connections' commits) and this connection's own changes.
  fn stamp(&self) -> Result<(i64, u64)> {
    Ok((self.data_version()?, self.conn.total_changes()))
  }

  /// Where the records and segments stand: their counts and largest
  /// rowids (`[records, max, segments, max]`), for a reader that follows
  /// what came in since (`records_since`, `segment_threads_since`): rows
  /// are only added, so a count that is not the last one plus what came
  /// since says rows were removed or replaced.
  pub fn cursor(&self) -> Result<[i64; 4]> {
    let (a, b) = self.conn.query_row(
      "SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM records",
      [],
      |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (c, d) = self.conn.query_row(
      "SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM segments",
      [],
      |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok([a, b, c, d])
  }

  /// The records indexed after rowid `after`, with their kind, oldest first.
  pub fn records_since(&self, after: i64) -> Result<Vec<(RecordId, String)>> {
    let mut st = self
      .conn
      .prepare_cached("SELECT id, kind FROM records WHERE rowid > ?1 ORDER BY rowid")?;
    let rows = st.query_map([after], |r| Ok((record_id(r.get(0)?)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// The threads of the segments indexed after rowid `after`.
  pub fn segment_threads_since(&self, after: i64) -> Result<Vec<RecordId>> {
    let mut st = self
      .conn
      .prepare_cached("SELECT DISTINCT thread FROM segments WHERE rowid > ?1")?;
    let rows = st.query_map([after], |r| record_id(r.get(0)?))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Every edge, for the graph.
  pub fn edges(&self) -> Result<Vec<Edge>> {
    self.query_edges(
      "SELECT src, rel, dst, dst_sub, dst_repo, created, derived FROM edges ORDER BY created",
      [],
    )
  }

  pub fn edges_from(&self, id: &RecordId) -> Result<Vec<Edge>> {
    self.query_edges(
            "SELECT src, rel, dst, dst_sub, dst_repo, created, derived FROM edges WHERE src = ?1 ORDER BY rel",
            [id.as_str()],
        )
  }

  pub fn edges_to(&self, id: &RecordId) -> Result<Vec<Edge>> {
    self.query_edges(
            "SELECT src, rel, dst, dst_sub, dst_repo, created, derived FROM edges WHERE dst = ?1 ORDER BY created",
            [id.as_str()],
        )
  }

  fn query_edges<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<Edge>> {
    let mut st = self.conn.prepare(sql)?;
    let rows = st.query_map(p, |r| {
      let sub: String = r.get(3)?;
      let repo: Option<String> = r.get(4)?;
      Ok(Edge {
        src: record_id(r.get(0)?)?,
        rel: r.get(1)?,
        dst: Ref {
          repo: repo.and_then(|s| s.parse().ok()),
          id: record_id(r.get(2)?)?,
          sub: (!sub.is_empty()).then_some(sub),
        },
        created: from_ms(r.get(5)?),
        derived: r.get(6)?,
      })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// Records of `kind` with an edge `rel` to `dst` (and `sub`, if given),
  /// oldest first.
  pub fn sources(
    &self,
    kind: &str,
    rel: &str,
    dst: &RecordId,
    sub: Option<&str>,
  ) -> Result<Vec<IndexedRecord>> {
    // From the edges to the records (`CROSS JOIN` keeps that order): with
    // `kind = ?1 AND id IN (…)` SQLite walked every record of the kind
    // through `records_kind` and tested each against the list, so a
    // field's current value read all 1,600 `set` records, and a live view
    // did that for each field of each of its rows.
    let mut st = self.conn.prepare_cached(SOURCES)?;
    let mut out: Vec<IndexedRecord> = st
      .query_map(params![kind, rel, dst.as_str(), sub], row_record)?
      .collect::<rusqlite::Result<_>>()?;
    // A record with edges to several of `dst`'s items comes once per edge
    // (side by side, by the order), where the `IN` gave it once.
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
  }

  /// The current value of a mutable field (docs/records.md §6): follow the
  /// supersedes chain of its `set` records; concurrent heads resolve to the
  /// latest, flagged as a conflict.
  pub fn current_value(&self, target: &RecordId, field: &str) -> Result<Option<SetValue>> {
    self.value_of(target, field, false)
  }

  /// The current value of a field only the user sets (a thread's
  /// read-only mode): `set` records written by anything else (an agent's
  /// run, a node on its own) do not count.
  pub fn current_user_value(&self, target: &RecordId, field: &str) -> Result<Option<SetValue>> {
    self.value_of(target, field, true)
  }

  fn value_of(&self, target: &RecordId, field: &str, by_user: bool) -> Result<Option<SetValue>> {
    let all: Vec<IndexedRecord> = self
      .sources(kinds::SET, "target", target, None)?
      .into_iter()
      .filter(|r| r.fields.get("field").and_then(Json::as_str) == Some(field))
      .collect();
    let counts = |r: &IndexedRecord| !by_user || (r.by_user && r.run.is_none());
    let sets: Vec<&IndexedRecord> = all.iter().filter(|r| counts(r)).collect();
    // What each `set` supersedes. A counted `set` also supersedes what the
    // ones it supersedes that do not count (an agent's, when only the
    // user's count) superseded in turn: the user replacing an agent's
    // ignored switch replaces the user's switch under it, whatever the
    // two timestamps say.
    let mut supersedes: std::collections::HashMap<&str, Vec<String>> =
      std::collections::HashMap::new();
    {
      let mut st = self
        .conn
        .prepare("SELECT dst FROM edges WHERE src = ?1 AND rel = 'supersedes'")?;
      for s in &all {
        let mut dsts = Vec::new();
        for d in st.query_map([s.id.as_str()], |r| r.get::<_, String>(0))? {
          dsts.push(d?);
        }
        supersedes.insert(s.id.as_str(), dsts);
      }
    }
    let counted: HashSet<&str> = sets.iter().map(|s| s.id.as_str()).collect();
    let superseded: HashSet<String> = {
      let mut out = HashSet::new();
      for s in &sets {
        let mut todo: Vec<String> = supersedes[s.id.as_str()].clone();
        while let Some(d) = todo.pop() {
          if !out.insert(d.clone()) {
            continue;
          }
          if !counted.contains(d.as_str())
            && let Some(more) = supersedes.get(d.as_str())
          {
            todo.extend(more.iter().cloned());
          }
        }
      }
      out
    };
    let heads: Vec<&IndexedRecord> = sets
      .iter()
      .copied()
      .filter(|s| !superseded.contains(s.id.as_str()))
      .collect();
    let Some(winner) = heads
      .iter()
      .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)))
    else {
      return Ok(None);
    };
    Ok(Some(SetValue {
      value: winner.fields.get("value").cloned().unwrap_or(Json::Null),
      record: winner.id.clone(),
      conflict: heads.len() > 1,
    }))
  }

  /// A record's title: the current `set` of `title`, else its heading.
  pub fn title(&self, id: &RecordId) -> Result<Option<String>> {
    if let Some(v) = self.current_value(id, "title")?
      && let Some(s) = v.value.as_str()
    {
      return Ok(Some(s.to_owned()));
    }
    Ok(self.record(id)?.and_then(|r| r.title))
  }

  pub fn question_answered(&self, question: &Ref) -> Result<bool> {
    Ok(
      !self
        .sources(
          kinds::ANSWER,
          "answers",
          &question.id,
          Some(question.sub.as_deref().unwrap_or("")),
        )?
        .is_empty(),
    )
  }

  /// A run has ended when a report on it says `final = true`.
  pub fn run_ended(&self, run: &RecordId) -> Result<bool> {
    Ok(
      self
        .sources(kinds::REPORT, "run", run, None)?
        .iter()
        .any(|r| r.fields.get("final").and_then(Json::as_bool) == Some(true)),
    )
  }

  pub fn finding_status(&self, finding: &Ref) -> Result<FindingStatus> {
    let sub = finding.sub.as_deref().unwrap_or("");
    let verifications = self.sources(kinds::VERIFICATION, "verifies", &finding.id, Some(sub))?;
    let latest = verifications
      .iter()
      .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    match latest
      .and_then(|v| v.fields.get("result"))
      .and_then(Json::as_str)
    {
      Some("verified") => return Ok(FindingStatus::Verified),
      Some("still-failing") => return Ok(FindingStatus::StillFailing),
      _ => {}
    }
    if !self
      .sources(kinds::RESOLUTION, "addresses", &finding.id, Some(sub))?
      .is_empty()
    {
      return Ok(FindingStatus::Addressed);
    }
    Ok(FindingStatus::Open)
  }

  pub fn assignment_status(
    &self,
    assignment: &RecordId,
    now: Timestamp,
  ) -> Result<AssignmentStatus> {
    let mut claims = self.sources(kinds::CLAIM, "claims", assignment, None)?;
    let mut i = 0;
    while i < claims.len() {
      let renewals = self.sources(kinds::CLAIM, "renews", &claims[i].id.clone(), None)?;
      claims.extend(renewals);
      i += 1;
    }
    let best = claims
      .iter()
      .filter_map(|c| {
        let until = c
          .fields
          .get("lease_until")?
          .as_str()?
          .parse::<Timestamp>()
          .ok()?;
        Some((until, c.node.clone()))
      })
      .max();
    Ok(match best {
      None => AssignmentStatus::Unclaimed,
      Some((until, node)) if until > now => AssignmentStatus::Claimed { node, until },
      Some((until, node)) => AssignmentStatus::Expired { node, until },
    })
  }

  /// Spending of a task's runs: (USD, tokens).
  pub fn task_usage(&self, task: &RecordId) -> Result<(f64, u64)> {
    let runs = self.sources(kinds::RUN, "task", task, None)?;
    let mut st = self.conn.prepare(
            "SELECT COALESCE(SUM(usd), 0), COALESCE(SUM(input_tokens + output_tokens), 0) FROM usage WHERE thread = ?1",
        )?;
    let (mut usd, mut tokens) = (0.0, 0u64);
    for run in &runs {
      let (u, t): (f64, i64) = st.query_row([run.id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?;
      usd += u;
      tokens += t as u64;
    }
    Ok((usd, tokens))
  }

  /// docs/records.md §8: closed, then waiting, over budget, running, to
  /// review, open.
  pub fn task_status(&self, task: &RecordId) -> Result<TaskStatus> {
    let closes = self.sources(kinds::CLOSE, "closes", task, None)?;
    if let Some(c) = closes
      .iter()
      .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)))
    {
      return Ok(match c.fields.get("result").and_then(Json::as_str) {
        Some("dropped") => TaskStatus::Dropped,
        Some("failed") => TaskStatus::Failed,
        _ => TaskStatus::Done,
      });
    }
    let runs = self.sources(kinds::RUN, "task", task, None)?;
    let mut questions = self.sources(kinds::QUESTION, "task", task, None)?;
    for run in &runs {
      questions.extend(self.sources(kinds::QUESTION, "run", &run.id, None)?);
    }
    for q in &questions {
      if !self.question_answered(&Ref::to(q.id.clone()))? {
        return Ok(TaskStatus::Waiting);
      }
    }
    if let Some(task_record) = self.record(task)?
      && let Some(budget) = task_record.fields.get("budget")
    {
      let (usd, tokens) = self.task_usage(task)?;
      let over_usd = budget
        .get("usd")
        .and_then(Json::as_f64)
        .is_some_and(|b| usd > b);
      let over_tokens = budget
        .get("tokens")
        .and_then(Json::as_u64)
        .is_some_and(|b| tokens > b);
      if over_usd || over_tokens {
        return Ok(TaskStatus::OverBudget);
      }
    }
    if runs.is_empty() {
      return Ok(TaskStatus::Open);
    }
    for run in &runs {
      if !self.run_ended(&run.id)? {
        return Ok(TaskStatus::Running);
      }
    }
    Ok(TaskStatus::ToReview)
  }

  /// What is blocked on the user (design §2.3): open questions, findings
  /// still failing, tasks over budget.
  pub fn attention(&self) -> Result<Vec<AttentionItem>> {
    let mut out: Vec<AttentionItem> = self
      .open_questions()?
      .into_iter()
      .map(|(id, title)| AttentionItem::Question { id, title })
      .collect();
    let findings: Vec<(String, String)> = {
      let mut st = self.conn.prepare(
        "SELECT DISTINCT dst, dst_sub FROM edges WHERE rel = 'verifies' AND dst_sub != ''",
      )?;
      st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?
    };
    for (dst, sub) in findings {
      let Ok(id) = dst.parse() else { continue };
      let finding = Ref::item(id, &sub)?;
      if self.finding_status(&finding)? == FindingStatus::StillFailing {
        out.push(AttentionItem::Finding { finding });
      }
    }
    for t in self.records(Some(kinds::TASK))? {
      if self.task_status(&t.id)? == TaskStatus::OverBudget {
        out.push(AttentionItem::OverBudget { task: t.id });
      }
    }
    Ok(out)
  }

  /// The questions no answer answers, the oldest first, with their
  /// titles: `attention`'s first part alone, for a reader that shows only
  /// those (the phone's approvals, read every few seconds), without the
  /// findings and tasks.
  pub fn open_questions(&self) -> Result<Vec<(RecordId, Option<String>)>> {
    // In one query (one per question took 37 ms of a TUI's every refresh
    // with 465 of them).
    let open: Vec<String> = {
      let mut st = self.conn.prepare(
        "SELECT q.id FROM records q WHERE q.kind = ?1 AND NOT EXISTS (
           SELECT 1 FROM edges e JOIN records a ON a.id = e.src
           WHERE e.dst = q.id AND e.rel = 'answers' AND e.dst_sub = '' AND a.kind = ?2)
         ORDER BY q.created, q.id",
      )?;
      st.query_map([kinds::QUESTION, kinds::ANSWER], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut out = Vec::new();
    for id in open {
      let Ok(id) = id.parse::<RecordId>() else {
        continue;
      };
      out.push((id.clone(), self.title(&id)?));
    }
    Ok(out)
  }

  /// Attachments of a record: what a phone fetches lazily.
  pub fn attachments(&self, id: &RecordId) -> Result<Vec<Attachment>> {
    let mut st = self
      .conn
      .prepare("SELECT hash, type, name, bytes FROM attachments WHERE record = ?1")?;
    let rows = st.query_map([id.as_str()], |r| {
      Ok((
        r.get::<_, String>(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get::<_, i64>(3)?,
      ))
    })?;
    let mut out = Vec::new();
    for row in rows {
      let (hash, media_type, name, bytes) = row?;
      out.push(Attachment {
        hash: hash.parse()?,
        media_type,
        name,
        bytes: bytes as u64,
      });
    }
    Ok(out)
  }

  /// The user's nodes admitted by `peer` records and not revoked since
  /// (docs/records.md §7): the shared part of every node's allowlist.
  pub fn peer_records(&self) -> Result<Vec<IndexedRecord>> {
    let mut out = Vec::new();
    for r in self.records(Some(kinds::PEER))? {
      if self
        .sources(kinds::REVOKE, "revokes", &r.id, None)?
        .is_empty()
      {
        out.push(r);
      }
    }
    Ok(out)
  }

  /// Threads by last activity (their newest segment, else their creation),
  /// newest first.
  pub fn threads(&self) -> Result<Vec<(IndexedRecord, Timestamp)>> {
    let out = self.threads_active()?;
    // Pinned first (a `set` of `pinned`), then the latest.
    Ok(self.pinned_first(out, |r| self.pinned(&r.id)))
  }

  /// Every thread with its last activity (its newest segment, else its
  /// creation), in no order.
  fn threads_active(&self) -> Result<Vec<(IndexedRecord, Timestamp)>> {
    let mut out = Vec::new();
    let mut last_at = self
      .conn
      .prepare_cached("SELECT MAX(created) FROM segments WHERE thread = ?1")?;
    for r in self.records_shared(kinds::THREAD)?.iter() {
      let last: Option<i64> = last_at.query_row([r.id.as_str()], |row| row.get(0))?;
      let at = last.map(from_ms).unwrap_or(r.created);
      out.push((r.clone(), at));
    }
    Ok(out)
  }

  /// Whether a thread's own `pinned` is set.
  fn pinned(&self, thread: &RecordId) -> bool {
    self
      .current_value(thread, "pinned")
      .ok()
      .flatten()
      .is_some_and(|v| v.value.as_bool() == Some(true))
  }

  fn pinned_first(
    &self,
    threads: Vec<(IndexedRecord, Timestamp)>,
    pinned: impl Fn(&IndexedRecord) -> bool,
  ) -> Vec<(IndexedRecord, Timestamp)> {
    let mut out: Vec<(IndexedRecord, Timestamp, bool)> = threads
      .into_iter()
      .map(|(r, at)| {
        let p = pinned(&r);
        (r, at, p)
      })
      .collect();
    out.sort_by(|a, b| (b.2, b.1, &b.0.id).cmp(&(a.2, a.1, &a.0.id)));
    out.into_iter().map(|(r, at, _)| (r, at)).collect()
  }

  /// The threads as lists show them (design §2.15, "One conversation"):
  /// `threads`, with each compaction chain as one row, its newest thread;
  /// the threads compacted into another are left out. Pinned first by
  /// `lineage_pinned`, then the latest.
  pub fn listed_threads(&self) -> Result<Vec<(IndexedRecord, Timestamp)>> {
    let all = self.threads_active()?;
    let mut compacted_away: HashSet<RecordId> = HashSet::new();
    let mut compactions: HashSet<RecordId> = HashSet::new();
    for (r, _) in &all {
      if let Some(prev) = self.compacted_from_record(r)? {
        compacted_away.insert(prev);
        compactions.insert(r.id.clone());
      }
    }
    let shown: Vec<_> = all
      .into_iter()
      .filter(|(r, _)| !compacted_away.contains(&r.id))
      .collect();
    // A thread never compacted is its own chain: its own pin.
    Ok(self.pinned_first(shown, |r| {
      if compactions.contains(&r.id) {
        self.lineage_pinned(&r.id)
      } else {
        self.pinned(&r.id)
      }
    }))
  }

  /// The thread a compaction continues: what `r` continues, when it is
  /// marked `compacted = true` (design §2.15).
  fn compacted_from_record(&self, r: &IndexedRecord) -> Result<Option<RecordId>> {
    if r.fields.get("compacted").and_then(Json::as_bool) != Some(true) {
      return Ok(None);
    }
    Ok(
      self
        .edges_from(&r.id)?
        .into_iter()
        .find(|e| e.rel == "continues")
        .map(|e| e.dst.id)
        .filter(|p| p != &r.id),
    )
  }

  /// The thread `thread` was compacted from, if it is a compaction.
  pub fn compacted_from(&self, thread: &RecordId) -> Option<RecordId> {
    let r = self.record(thread).ok().flatten()?;
    self.compacted_from_record(&r).ok().flatten()
  }

  /// The latest thread compacted from `thread` (the newest `compacted`
  /// record continuing it), if any: one step of `compaction_newest`.
  pub fn compacted_into(&self, thread: &RecordId) -> Option<RecordId> {
    self
      .sources(kinds::THREAD, "continues", thread, None)
      .unwrap_or_default()
      .into_iter()
      .filter(|t| t.fields.get("compacted").and_then(Json::as_bool) == Some(true))
      .max_by_key(|t| t.created)
      .map(|t| t.id)
      .filter(|t| t != thread)
  }

  /// What `thread` continues in now: its compactions followed to the
  /// newest (itself when it was never compacted).
  pub fn compaction_newest(&self, thread: &RecordId) -> RecordId {
    let mut id = thread.clone();
    let mut seen = vec![id.clone()];
    for _ in 0..64 {
      match self.compacted_into(&id) {
        Some(n) if !seen.contains(&n) => {
          seen.push(n.clone());
          id = n;
        }
        _ => break,
      }
    }
    id
  }

  /// The compaction chain `thread` is in, oldest first: back to the
  /// thread the first compaction continued, then forward to the newest.
  pub fn compaction_lineage(&self, thread: &RecordId) -> Vec<RecordId> {
    let mut first = thread.clone();
    let mut back = vec![first.clone()];
    for _ in 0..64 {
      match self.compacted_from(&first) {
        Some(p) if !back.contains(&p) => {
          back.push(p.clone());
          first = p;
        }
        _ => break,
      }
    }
    let mut out = vec![first];
    for _ in 0..64 {
      let last = out.last().expect("one").clone();
      match self.compacted_into(&last) {
        Some(n) if !out.contains(&n) => out.push(n),
        _ => break,
      }
    }
    out
  }

  /// A field's current value over a compaction chain: the latest `set`
  /// of it on any of its threads (by when that `set` was written), so a
  /// pin or a title given before a compaction carries over, and one
  /// given after it, on whichever thread, wins.
  pub fn lineage_value(&self, thread: &RecordId, field: &str) -> Option<SetValue> {
    self
      .compaction_lineage(thread)
      .iter()
      .filter_map(|t| self.current_value(t, field).ok().flatten())
      .max_by_key(|v| {
        let at = self.record(&v.record).ok().flatten().map(|r| r.created);
        (at, v.record.clone())
      })
  }

  /// Whether a compaction chain is pinned (`lineage_value` of `pinned`).
  pub fn lineage_pinned(&self, thread: &RecordId) -> bool {
    self
      .lineage_value(thread, "pinned")
      .is_some_and(|v| v.value.as_bool() == Some(true))
  }

  /// A compaction chain's title: the latest `title` set on any of its
  /// threads (`rename`, or the one generated for its first exchange),
  /// else its newest thread's own, without the ✂ a compaction's carries
  /// (it is the same conversation; a fork the user made keeps its prefix).
  pub fn lineage_title(&self, thread: &RecordId) -> Option<String> {
    if let Some(v) = self.lineage_value(thread, "title")
      && let Some(s) = v.value.as_str()
    {
      return Some(s.to_owned());
    }
    let newest = self.compaction_newest(thread);
    let title = self.record(&newest).ok().flatten()?.title?;
    if self.compacted_from(&newest).is_some() {
      Some(title.trim_start_matches(['✂', ' ']).to_owned())
    } else {
      Some(title)
    }
  }

  /// What the given threads touched (the tool calls of their sealed
  /// segments, `artefacts.rs`), one row per `(kind, value, verb, thread)`,
  /// the latest first. `kind` and `value` narrow it to one artefact.
  pub fn artefacts(
    &self,
    threads: &[RecordId],
    what: Option<(artefacts::Kind, &str)>,
  ) -> Result<Vec<Touch>> {
    let mut rows: BTreeMap<(String, String, String, String), Touch> = BTreeMap::new();
    let mut fold = |r: &rusqlite::Row<'_>| -> rusqlite::Result<()> {
      let t = Touch {
        thread: record_id(r.get(0)?)?,
        kind: r.get(1)?,
        value: r.get(2)?,
        verb: r.get(3)?,
        first_at: from_ms(r.get(4)?),
        last_at: from_ms(r.get(5)?),
        n: r.get(6)?,
        first_msg: r.get(7)?,
        last_msg: r.get(8)?,
        subject: r.get(9)?,
      };
      let key = (
        t.thread.to_string(),
        t.kind.clone(),
        t.value.clone(),
        t.verb.clone(),
      );
      match rows.get_mut(&key) {
        None => {
          rows.insert(key, t);
        }
        Some(a) => {
          a.n += t.n;
          if t.first_at < a.first_at {
            a.first_at = t.first_at;
            a.first_msg = t.first_msg;
          }
          if t.last_at >= a.last_at {
            a.last_at = t.last_at;
            a.last_msg = t.last_msg;
            if t.subject.is_some() {
              a.subject = t.subject;
            }
          }
        }
      }
      Ok(())
    };
    const COLS: &str =
      "thread, kind, value, verb, first_at, last_at, n, first_msg, last_msg, subject";
    match what {
      Some((kind, value)) => {
        let mut st = self.conn.prepare_cached(&format!(
          "SELECT {COLS} FROM artefacts WHERE kind = ?1 AND value = ?2"
        ))?;
        let mut q = st.query(params![kind.as_str(), value])?;
        while let Some(r) = q.next()? {
          fold(r)?;
        }
      }
      None => {
        let mut st = self
          .conn
          .prepare_cached(&format!("SELECT {COLS} FROM artefacts WHERE thread = ?1"))?;
        for t in threads {
          let mut q = st.query([t.as_str()])?;
          while let Some(r) = q.next()? {
            fold(r)?;
          }
        }
      }
    }
    let mut out: Vec<Touch> = rows.into_values().collect();
    out.sort_by(|a, b| {
      b.last_at
        .cmp(&a.last_at)
        .then_with(|| a.value.cmp(&b.value))
    });
    Ok(out)
  }

  /// Every thread's latest digest, in one read (a list of threads takes
  /// its rows' digests from it, not one query per row).
  pub fn digests(&self) -> Result<Vec<DigestRow>> {
    let mut st = self.conn.prepare_cached(
      "SELECT thread, record, created, upto, step, headline, asks, questions FROM digests",
    )?;
    let rows = st.query_map([], digest_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// One thread's latest digest.
  pub fn digest(&self, thread: &RecordId) -> Result<Option<DigestRow>> {
    let mut st = self.conn.prepare_cached(
      "SELECT thread, record, created, upto, step, headline, asks, questions FROM digests WHERE thread = ?1",
    )?;
    let mut rows = st.query_map([thread.as_str()], digest_row)?;
    Ok(rows.next().transpose()?)
  }

  /// Every blob the records and segments of a repository reference.
  pub fn blob_refs(&self, repo: &RepoId) -> Result<Vec<BlobHash>> {
    let mut st = self
      .conn
      .prepare("SELECT DISTINCT hash FROM blob_refs WHERE repo = ?1 ORDER BY hash")?;
    let hashes = st
      .query_map([repo.as_str()], |r| r.get::<_, String>(0))?
      .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hashes.into_iter().filter_map(|h| h.parse().ok()).collect())
  }

  /// Segments of a thread in chain order; forks come out in creation order.
  pub fn thread_segments(
    &self,
    thread: &RecordId,
  ) -> Result<Vec<(RecordId, Option<RecordId>, String)>> {
    let mut st = self
      .conn
      .prepare("SELECT id, prev, path FROM segments WHERE thread = ?1 ORDER BY created, id")?;
    let rows = st.query_map([thread.as_str()], |r| {
      let prev: Option<String> = r.get(1)?;
      Ok((
        record_id(r.get(0)?)?,
        prev.map(record_id).transpose()?,
        r.get(2)?,
      ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// The segments of a thread that start a branch from a rewind: (segment,
  /// the rewind).
  pub fn segment_rewinds(&self, thread: &RecordId) -> Result<Vec<(RecordId, RecordId)>> {
    let mut st = self
      .conn
      .prepare("SELECT id, rewind FROM segments WHERE thread = ?1 AND rewind IS NOT NULL")?;
    let rows = st.query_map([thread.as_str()], |r| {
      Ok((record_id(r.get(0)?)?, record_id(r.get(1)?)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
  }

  /// The user's rewinds of a thread, oldest first (one a node or an
  /// agent's run wrote counts for nothing).
  pub fn rewinds(&self, thread: &RecordId) -> Result<Vec<IndexedRecord>> {
    Ok(
      self
        .sources(kinds::REWIND, "thread", thread, None)?
        .into_iter()
        .filter(|r| r.by_user)
        .collect(),
    )
  }

  /// Forgets files of a repository, so the next refresh reads them again
  /// (their content changed: a migration rewrote them).
  pub fn forget(&mut self, repo: &RepoId, paths: &[String]) -> Result<()> {
    let tx = self.conn.transaction()?;
    for p in paths {
      remove_path(&tx, repo.as_str(), p)?;
    }
    tx.commit()?;
    self.fingerprints.remove(repo.as_str());
    Ok(())
  }

  /// When a thread's latest segment was written; `None` before its first.
  pub fn last_segment_at(&self, thread: &RecordId) -> Result<Option<Timestamp>> {
    let at: Option<i64> = self.conn.query_row(
      "SELECT MAX(created) FROM segments WHERE thread = ?1",
      [thread.as_str()],
      |r| r.get(0),
    )?;
    Ok(at.map(from_ms))
  }

  /// A cheap mark of what the index holds of `kinds` and of transcripts:
  /// the count and latest row of those records, and of segments. It
  /// changes when one is added or removed (apps' live data, design §2.10).
  pub fn mark(&self, kinds: &[&str]) -> Result<[i64; 4]> {
    let marks = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let (n, last): (i64, i64) = self.conn.query_row(
      &format!("SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM records WHERE kind IN ({marks})"),
      rusqlite::params_from_iter(kinds.iter()),
      |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (segs, seg_last): (i64, i64) = self.conn.query_row(
      "SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM segments",
      [],
      |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok([n, last, segs, seg_last])
  }

  /// Like `mark`, for the records of `kinds` alone: transcript segments do
  /// not move it (every model step writes one), so a list that does not
  /// show them is not told of each.
  pub fn mark_records(&self, kinds: &[&str]) -> Result<[i64; 2]> {
    let marks = kinds.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let (n, last): (i64, i64) = self.conn.query_row(
      &format!("SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM records WHERE kind IN ({marks})"),
      rusqlite::params_from_iter(kinds.iter()),
      |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok([n, last])
  }

  /// SQLite's `data_version` of this connection: it changes when another
  /// connection (the node, another command) commits to the index, so a
  /// reader polling it re-reads only when something was written.
  pub fn data_version(&self) -> Result<i64> {
    Ok(
      self
        .conn
        .query_row("PRAGMA data_version", [], |r| r.get(0))?,
    )
  }

  /// Repositories the index knows, with their record root.
  pub fn roots(&self) -> Result<BTreeMap<String, String>> {
    let mut st = self.conn.prepare("SELECT repo, dir FROM roots")?;
    Ok(
      st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?,
    )
  }
}

enum Parsed {
  Record(Box<Record>),
  Segment(Box<Segment>),
}

const SELECT_RECORD: &str =
  "SELECT id, repo, path, kind, created, node, by_user, run, title, fields FROM records";
const RECORDS_OF_KIND: &str =
  "SELECT id, repo, path, kind, created, node, by_user, run, title, fields
   FROM records WHERE kind = ?1 ORDER BY created, id";
const SOURCES: &str =
  "SELECT r.id, r.repo, r.path, r.kind, r.created, r.node, r.by_user, r.run, r.title, r.fields
   FROM edges e CROSS JOIN records r ON r.id = e.src
   WHERE e.dst = ?3 AND e.rel = ?2 AND (?4 IS NULL OR e.dst_sub = ?4) AND r.kind = ?1
   ORDER BY r.created, r.id";

fn row_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedRecord> {
  let repo: String = r.get(1)?;
  let run: Option<String> = r.get(7)?;
  let fields: String = r.get(9)?;
  Ok(IndexedRecord {
    id: record_id(r.get(0)?)?,
    repo: repo.parse().map_err(|e: FormatError| {
      rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?,
    path: r.get(2)?,
    kind: r.get(3)?,
    created: from_ms(r.get(4)?),
    node: r.get(5)?,
    by_user: r.get(6)?,
    run: run.map(record_id).transpose()?,
    title: r.get(8)?,
    fields: serde_json::from_str(&fields).unwrap_or(Json::Null),
  })
}

/// The kind of a thread's digest record (`stitch-node`'s `digests.rs`).
pub const DIGEST_KIND: &str = "digest";

/// A `digest` record's row: the thread's latest by when it was written (the
/// greater ID on a tie); an older one indexed later leaves it alone.
fn insert_digest(tx: &Transaction<'_>, r: &Record) -> Result<()> {
  let Some(thread) = r.fields.get("thread").and_then(toml::Value::as_str) else {
    return Ok(());
  };
  let count = |kind: &str| {
    r.fields
      .get("items")
      .and_then(toml::Value::as_array)
      .map_or(0, |a| {
        a.iter()
          .filter(|i| i.get("kind").and_then(toml::Value::as_str) == Some(kind))
          .count()
      }) as i64
  };
  let step = r
    .fields
    .get("step")
    .and_then(toml::Value::as_str)
    .and_then(|v| v.parse::<Timestamp>().ok())
    .map(ms);
  tx.execute(
    "INSERT INTO digests(thread, record, created, upto, step, headline, asks, questions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(thread) DO UPDATE SET record = excluded.record, created = excluded.created,
           upto = excluded.upto, step = excluded.step, headline = excluded.headline,
           asks = excluded.asks, questions = excluded.questions
         WHERE excluded.created > created OR (excluded.created = created AND excluded.record > record)",
    params![
      thread,
      r.id.as_str(),
      ms(r.created),
      r.fields.get("upto").and_then(toml::Value::as_str),
      step,
      r.fields
        .get("headline")
        .and_then(toml::Value::as_str)
        .unwrap_or_default(),
      count("ask"),
      count("question"),
    ],
  )?;
  Ok(())
}

fn digest_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DigestRow> {
  Ok(DigestRow {
    thread: record_id(r.get(0)?)?,
    record: record_id(r.get(1)?)?,
    created: from_ms(r.get(2)?),
    upto: r.get(3)?,
    step: r.get::<_, Option<i64>>(4)?.map(from_ms),
    headline: r.get(5)?,
    asks: r.get(6)?,
    questions: r.get(7)?,
  })
}

fn insert_record(tx: &Transaction<'_>, repo: &str, path: &str, r: &Record) -> Result<()> {
  let fields = Json::Object(
    r.fields
      .iter()
      .map(|(k, v)| (k.clone(), toml_to_json(v)))
      .collect(),
  );
  tx.execute(
        "INSERT OR IGNORE INTO records(id, repo, path, kind, created, node, by_user, run, title, fields)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            r.id.as_str(),
            repo,
            path,
            r.kind.as_str(),
            ms(r.created),
            r.author.node.as_str(),
            r.author.user,
            r.author.run.as_ref().map(RecordId::as_str),
            r.title(),
            fields.to_string(),
        ],
    )?;
  if r.kind == DIGEST_KIND {
    insert_digest(tx, r)?;
  }
  let mut edge = tx.prepare_cached(
    "INSERT OR IGNORE INTO edges(src, rel, dst, dst_sub, dst_repo, created, derived)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
  )?;
  for (rel, target) in r.refs.iter() {
    edge.execute(params![
      r.id.as_str(),
      rel,
      target.id.as_str(),
      target.sub.as_deref().unwrap_or(""),
      target.repo.as_ref().map(RepoId::as_str),
      ms(r.created),
      false
    ])?;
  }
  for target in mentions(&r.body) {
    edge.execute(params![
      r.id.as_str(),
      "mentions",
      target.id.as_str(),
      target.sub.as_deref().unwrap_or(""),
      target.repo.as_ref().map(RepoId::as_str),
      ms(r.created),
      true
    ])?;
  }
  let mut att = tx.prepare_cached(
    "INSERT INTO attachments(record, hash, type, name, bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
  )?;
  let mut blob =
    tx.prepare_cached("INSERT OR IGNORE INTO blob_refs(owner, repo, hash) VALUES (?1, ?2, ?3)")?;
  for a in &r.attachments {
    att.execute(params![
      r.id.as_str(),
      a.hash.to_string(),
      a.media_type,
      a.name,
      a.bytes as i64
    ])?;
    blob.execute(params![r.id.as_str(), repo, a.hash.to_string()])?;
  }
  Ok(())
}

fn insert_segment(tx: &Transaction<'_>, repo: &str, path: &str, s: &Segment) -> Result<()> {
  let h = &s.header;
  tx.execute(
        "INSERT OR IGNORE INTO segments(id, repo, path, thread, prev, created, node, rewind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            h.segment.as_str(),
            repo,
            path,
            h.thread.as_str(),
            h.prev.as_ref().map(RecordId::as_str),
            ms(h.created),
            h.node.as_str(),
            h.rewind.as_ref().map(RecordId::as_str)
        ],
    )?;
  let mut st = tx.prepare_cached(
    "INSERT OR IGNORE INTO usage(segment, seq, thread, at, model, input_tokens, output_tokens, usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
  )?;
  for (seq, u) in s.usage().enumerate() {
    st.execute(params![
      h.segment.as_str(),
      seq as i64,
      h.thread.as_str(),
      ms(u.at),
      u.model,
      u.input_tokens as i64,
      u.output_tokens as i64,
      u.usd
    ])?;
  }
  let mut art = tx.prepare_cached(
    "INSERT OR IGNORE INTO artefacts(segment, thread, kind, value, verb, first_at, last_at, n, first_msg, last_msg, subject)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
  )?;
  for r in artefacts::segment_rows(s) {
    art.execute(params![
      h.segment.as_str(),
      h.thread.as_str(),
      r.kind.as_str(),
      r.value,
      r.verb.as_str(),
      ms(r.first_at),
      ms(r.last_at),
      r.n,
      r.first_msg,
      r.last_msg,
      r.subject
    ])?;
  }
  let mut blob =
    tx.prepare_cached("INSERT OR IGNORE INTO blob_refs(owner, repo, hash) VALUES (?1, ?2, ?3)")?;
  for hash in s.blob_refs() {
    blob.execute(params![h.segment.as_str(), repo, hash.to_string()])?;
  }
  Ok(())
}

fn remove_path(tx: &Transaction<'_>, repo: &str, path: &str) -> Result<()> {
  tx.execute(
    "DELETE FROM files WHERE repo = ?1 AND path = ?2",
    params![repo, path],
  )?;
  match ItemPath::parse(path) {
    Some(ItemPath::Record(id)) => {
      tx.execute(
        "DELETE FROM records WHERE id = ?1 AND repo = ?2",
        params![id.as_str(), repo],
      )?;
      tx.execute("DELETE FROM edges WHERE src = ?1", [id.as_str()])?;
      tx.execute("DELETE FROM digests WHERE record = ?1", [id.as_str()])?;
      tx.execute("DELETE FROM attachments WHERE record = ?1", [id.as_str()])?;
      tx.execute("DELETE FROM blob_refs WHERE owner = ?1", [id.as_str()])?;
    }
    Some(ItemPath::Segment { segment, .. }) => {
      tx.execute("DELETE FROM segments WHERE id = ?1", [segment.as_str()])?;
      tx.execute("DELETE FROM usage WHERE segment = ?1", [segment.as_str()])?;
      tx.execute(
        "DELETE FROM artefacts WHERE segment = ?1",
        [segment.as_str()],
      )?;
      tx.execute("DELETE FROM blob_refs WHERE owner = ?1", [segment.as_str()])?;
    }
    None => return Err(Error::InvalidPath(path.to_owned())),
  }
  Ok(())
}

#[cfg(test)]
mod plans {
  use super::*;

  /// SQLite's plan for `sql`, one step a line.
  fn plan(index: &Index, sql: &str) -> String {
    let mut st = index
      .conn
      .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
      .unwrap();
    let n = st.parameter_count();
    let args: Vec<Option<String>> = (0..n).map(|_| Some("x".into())).collect();
    st.query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
      .unwrap()
      .collect::<rusqlite::Result<Vec<_>>>()
      .unwrap()
      .join("\n")
  }

  /// The reads live views make for each of their rows find their records
  /// through an index, never by walking a table (a node spent a core,
  /// 6 GB/s of reads from the page cache, on those walks).
  #[test]
  fn artefact_reads_use_an_index() {
    let index = Index::open_in_memory().unwrap();
    for sql in [
      "SELECT thread FROM artefacts WHERE thread = ?1",
      "SELECT thread FROM artefacts WHERE kind = ?1 AND value = ?2",
    ] {
      let p = plan(&index, sql);
      assert!(p.contains("USING"), "{sql}: {p}");
    }
  }

  /// The time of each index read a thread row makes, over every thread of a
  /// copy of an index file: `STITCH_INDEX_COPY=<index.sqlite copy> cargo test
  /// -p stitch-store --release row_reads -- --ignored --nocapture`.
  #[test]
  #[ignore]
  fn row_reads() {
    let path = std::env::var("STITCH_INDEX_COPY").expect("STITCH_INDEX_COPY");
    let index = Index::open(Path::new(&path)).unwrap();
    let time = |what: &str, f: &dyn Fn()| {
      let t = std::time::Instant::now();
      f();
      println!("{what}: {:?}", t.elapsed());
    };
    let listed = index.listed_threads().unwrap();
    println!("listed: {}", listed.len());
    time("listed_threads", &|| {
      index.listed_threads().unwrap();
    });
    let ids: Vec<RecordId> = listed.iter().map(|(r, _)| r.id.clone()).collect();
    time("is_deleted", &|| {
      ids.iter().for_each(|i| {
        index.is_deleted(i).unwrap();
      })
    });
    time("record", &|| {
      ids.iter().for_each(|i| {
        index.record(i).unwrap();
      })
    });
    time("edges_from", &|| {
      ids.iter().for_each(|i| {
        index.edges_from(i).unwrap();
      })
    });
    time("compaction_newest", &|| {
      ids.iter().for_each(|i| {
        index.compaction_newest(i);
      })
    });
    time("compaction_lineage", &|| {
      ids.iter().for_each(|i| {
        index.compaction_lineage(i);
      })
    });
    time("lineage_title", &|| {
      ids.iter().for_each(|i| {
        index.lineage_title(i);
      })
    });
    time("lineage_pinned", &|| {
      ids.iter().for_each(|i| {
        index.lineage_pinned(i);
      })
    });
    time("last_segment_at", &|| {
      ids.iter().for_each(|i| {
        index.last_segment_at(i).unwrap();
      })
    });
    time("digests", &|| {
      index.digests().unwrap();
    });
  }

  /// `STITCH_HOME_COPY=<record roots, colon-separated> cargo test -p
  /// stitch-store --release artefact_sizes -- --ignored --nocapture`:
  /// the time of a full index build and the size of the artefacts table.
  #[test]
  #[ignore]
  fn artefact_sizes() {
    let dir = std::env::var("STITCH_HOME_COPY").expect("STITCH_HOME_COPY");
    let roots: Vec<_> = dir
      .split(':')
      .map(|d| crate::RecordRoot::open(Path::new(d)).unwrap())
      .collect();
    let mut index = Index::open_in_memory().unwrap();
    let t = std::time::Instant::now();
    for root in &roots {
      index.refresh(root).unwrap();
    }
    println!("full refresh: {:?}", t.elapsed());
    let q = |sql: &str| -> i64 { index.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    println!("segments: {}", q("SELECT COUNT(*) FROM segments"));
    println!("usage rows: {}", q("SELECT COUNT(*) FROM usage"));
    println!("artefact rows: {}", q("SELECT COUNT(*) FROM artefacts"));
    for k in ["path", "url", "commit", "thread"] {
      println!(
        "  {k}: {} rows, {} distinct values, {} bytes of values",
        q(&format!(
          "SELECT COUNT(*) FROM artefacts WHERE kind = '{k}'"
        )),
        q(&format!(
          "SELECT COUNT(DISTINCT value) FROM artefacts WHERE kind = '{k}'"
        )),
        q(&format!(
          "SELECT COALESCE(SUM(LENGTH(value)),0) FROM artefacts WHERE kind = '{k}'"
        )),
      );
    }
    println!(
      "payload bytes (all columns): {}",
      q(
        "SELECT SUM(LENGTH(segment)+LENGTH(thread)+LENGTH(kind)+LENGTH(value)+LENGTH(verb)+16+COALESCE(LENGTH(first_msg),0)+COALESCE(LENGTH(last_msg),0)+COALESCE(LENGTH(subject),0)) FROM artefacts"
      )
    );
    println!(
      "threads with rows: {}",
      q("SELECT COUNT(DISTINCT thread) FROM artefacts")
    );
    println!(
      "max rows in one thread: {}",
      q("SELECT MAX(c) FROM (SELECT COUNT(*) c FROM artefacts GROUP BY thread)")
    );
    println!(
      "rows if deduped per thread: {}",
      q("SELECT COUNT(*) FROM (SELECT 1 FROM artefacts GROUP BY thread, kind, value, verb)")
    );
    if let Ok(pages) = index.conn.query_row(
      "SELECT SUM(pgsize) FROM dbstat WHERE name LIKE 'artefacts%'",
      [],
      |r| r.get::<_, i64>(0),
    ) {
      println!("artefacts pages bytes (table+indexes): {pages}");
      let all: i64 = index
        .conn
        .query_row("SELECT SUM(pgsize) FROM dbstat", [], |r| r.get(0))
        .unwrap();
      println!("whole index bytes: {all}");
    }
  }

  #[test]
  fn per_row_reads_use_an_index() {
    let index = Index::open_in_memory().unwrap();
    for sql in [RECORDS_OF_KIND, SOURCES] {
      let p = plan(&index, sql);
      assert!(!p.contains("SCAN"), "{sql}\n{p}");
    }
    // From the edges: not every record of the kind, tested one by one.
    let p = plan(&index, SOURCES);
    assert!(p.lines().next().unwrap().contains("edges_dst"), "{p}");
    assert!(!p.contains("records_kind"), "{p}");
  }
}
