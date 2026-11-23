//! A record root: `repo.toml`, `records/<YYYY-MM>/<id>.md` and
//! `threads/<thread>/<segment>.jsonl` (docs/records.md §3). Files are created
//! exclusively and never modified; a thread's segment is written in
//! `journal/` while its turn runs (`crate::journal`), never synced.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use jiff::Timestamp;
use stitch_records::{
  BlobHash, Draft, Event, FORMAT_VERSION, NodeName, RawStream, Record, RecordId, RepoId, Segment,
};
use toml::Value;

use crate::error::IoContext;
use crate::{Error, Result};

const REPO_FILE: &str = "repo.toml";
/// Present in a root whose segments are one per turn (2026-10-13): a root
/// made since, or one migrated (`stitch-node`'s `migrate`). Local, never
/// synced.
const TURNS_MARKER: &str = ".segments-by-turn";
const TMP_DIR: &str = ".tmp";
const MAX_ATTEMPTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
  Home,
  Project,
  /// A folder place's synced files (design §2.16).
  Folder,
}

impl Role {
  pub fn as_str(self) -> &'static str {
    match self {
      Role::Home => "home",
      Role::Project => "project",
      Role::Folder => "folder",
    }
  }
}

impl FromStr for Role {
  type Err = String;
  fn from_str(s: &str) -> std::result::Result<Self, String> {
    match s {
      "home" => Ok(Role::Home),
      "project" => Ok(Role::Project),
      "folder" => Ok(Role::Folder),
      _ => Err(format!("unknown role `{s}` (home, project or folder)")),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoConfig {
  pub id: RepoId,
  pub role: Role,
  pub created: Timestamp,
}

impl RepoConfig {
  fn parse(text: &str, path: &Path) -> Result<Self> {
    let bad = |message: &str| Error::Mismatch {
      path: path.display().to_string(),
      message: message.to_owned(),
    };
    let t: toml::Table = toml::from_str(text).map_err(|e| Error::Record {
      path: path.to_owned(),
      source: e.into(),
    })?;
    if t.get("v").and_then(Value::as_integer) != Some(FORMAT_VERSION) {
      return Err(bad("unsupported or missing `v`"));
    }
    let id = t
      .get("id")
      .and_then(Value::as_str)
      .ok_or_else(|| bad("missing `id`"))?;
    let role = t
      .get("role")
      .and_then(Value::as_str)
      .ok_or_else(|| bad("missing `role`"))?;
    let created = match t.get("created") {
      Some(Value::Datetime(d)) => d.to_string().parse().map_err(|_| bad("bad `created`"))?,
      _ => return Err(bad("missing `created`")),
    };
    Ok(RepoConfig {
      id: id.parse()?,
      role: role.parse().map_err(|e: String| bad(&e))?,
      created,
    })
  }

  fn to_text(&self) -> String {
    format!(
      "v = {FORMAT_VERSION}\nid = \"{}\"\nrole = \"{}\"\ncreated = {}\n",
      self.id,
      self.role.as_str(),
      self.created
    )
  }
}

/// Where a file lives in a record root, which is also its key in sync.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemPath {
  Record(RecordId),
  Segment { thread: RecordId, segment: RecordId },
}

impl ItemPath {
  /// Parses a root-relative path. Only the two shapes of the layout are
  /// accepted, so a path from a peer cannot name anything else.
  pub fn parse(s: &str) -> Option<ItemPath> {
    let mut parts = s.split('/');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
      return None;
    }
    match a {
      "records" => {
        let id: RecordId = c.strip_suffix(".md")?.parse().ok()?;
        (id.month() == b).then_some(ItemPath::Record(id))
      }
      "threads" => Some(ItemPath::Segment {
        thread: b.parse().ok()?,
        segment: c.strip_suffix(".jsonl")?.parse().ok()?,
      }),
      _ => None,
    }
  }
}

impl fmt::Display for ItemPath {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      ItemPath::Record(id) => write!(f, "records/{}/{id}.md", id.month()),
      ItemPath::Segment { thread, segment } => write!(f, "threads/{thread}/{segment}.jsonl"),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inserted {
  New,
  /// The same file was already there.
  Present,
  /// Not written: it is a deleted thread's, or goes with one
  /// (`crate::purge`).
  Purged,
}

#[derive(Clone, Debug)]
pub struct RecordRoot {
  dir: PathBuf,
  config: RepoConfig,
}

impl RecordRoot {
  /// Creates a record root in `dir` (created if needed). Fails if `dir`
  /// already holds a `repo.toml`.
  pub fn init(dir: &Path, id: RepoId, role: Role) -> Result<Self> {
    fs::create_dir_all(dir).at(dir)?;
    let config = RepoConfig {
      id,
      role,
      created: stitch_records::millis(Timestamp::now()),
    };
    let root = RecordRoot {
      dir: dir.to_owned(),
      config,
    };
    for sub in ["records", "threads", TMP_DIR] {
      let p = dir.join(sub);
      fs::create_dir_all(&p).at(&p)?;
    }
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
      fs::write(&ignore, format!("/{TMP_DIR}/\n/journal/\n")).at(&ignore)?;
    }
    root.mark_turns()?;
    if !root.write_new(
      REPO_FILE,
      root.config.to_text().as_bytes(),
      Durability::Synced,
    )? {
      return Err(Error::Mismatch {
        path: dir.join(REPO_FILE).display().to_string(),
        message: "already a record root".into(),
      });
    }
    Ok(root)
  }

  pub fn open(dir: &Path) -> Result<Self> {
    let path = dir.join(REPO_FILE);
    let text = match fs::read_to_string(&path) {
      Ok(t) => t,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        return Err(Error::NotARoot(dir.to_owned()));
      }
      Err(e) => return Err(e).at(&path),
    };
    Ok(RecordRoot {
      dir: dir.to_owned(),
      config: RepoConfig::parse(&text, &path)?,
    })
  }

  /// Whether its segments are one per turn (see `TURNS_MARKER`).
  pub fn turns_marked(&self) -> bool {
    self.dir.join(TURNS_MARKER).exists()
  }

  pub fn mark_turns(&self) -> Result<()> {
    let p = self.dir.join(TURNS_MARKER);
    fs::write(&p, "segments are one per turn\n").at(&p)
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  pub fn config(&self) -> &RepoConfig {
    &self.config
  }

  pub fn repo(&self) -> &RepoId {
    &self.config.id
  }

  pub fn path_of(&self, item: &ItemPath) -> PathBuf {
    self.dir.join(item.to_string())
  }

  /// Writes a new record with a fresh ID, created now.
  pub fn publish(&self, node: &NodeName, draft: Draft) -> Result<Record> {
    self.publish_at(node, draft, Timestamp::now())
  }

  /// Writes a record whose ID was chosen beforehand (a thread whose ID its
  /// shell sessions use before the thread is written). Fails if a file has
  /// that name.
  pub fn publish_record(&self, record: &Record) -> Result<()> {
    let item = ItemPath::Record(record.id.clone());
    if self.refuses(&item, Some(record)) {
      return Err(deleted(&item));
    }
    if self.write_new(
      &item.to_string(),
      record.to_text().as_bytes(),
      Durability::Synced,
    )? {
      Ok(())
    } else {
      Err(Error::Collisions(1))
    }
  }

  pub fn publish_at(&self, node: &NodeName, draft: Draft, at: Timestamp) -> Result<Record> {
    self.publish_with(node, draft, at, &mut rand::rng())
  }

  fn publish_with<R: rand::Rng + ?Sized>(
    &self,
    node: &NodeName,
    draft: Draft,
    at: Timestamp,
    rng: &mut R,
  ) -> Result<Record> {
    for _ in 0..MAX_ATTEMPTS {
      let id = RecordId::generate(at, node, rng);
      let record = draft.clone().into_record(id.clone(), at);
      let item = ItemPath::Record(id);
      if self.refuses(&item, Some(&record)) {
        return Err(deleted(&item));
      }
      if self.write_new(
        &item.to_string(),
        record.to_text().as_bytes(),
        Durability::Synced,
      )? {
        return Ok(record);
      }
    }
    Err(Error::Collisions(MAX_ATTEMPTS))
  }

  /// Writes a new transcript segment of `thread`.
  pub fn publish_segment(
    &self,
    node: &NodeName,
    thread: &RecordId,
    prev: Option<RecordId>,
    raw: Option<RawStream>,
    events: Vec<Event>,
  ) -> Result<Segment> {
    if self.purged().contains(thread.as_str()) {
      return Err(Error::Mismatch {
        path: format!("threads/{thread}"),
        message: "the thread was deleted".into(),
      });
    }
    let at = Timestamp::now();
    for _ in 0..MAX_ATTEMPTS {
      let id = RecordId::generate(at, node, &mut rand::rng());
      let mut segment = Segment::new(id.clone(), thread.clone(), at, node.clone(), prev.clone());
      segment.header.raw = raw.clone();
      segment.events = events.clone();
      let item = ItemPath::Segment {
        thread: thread.clone(),
        segment: id,
      };
      if self.write_new(
        &item.to_string(),
        segment.to_text().as_bytes(),
        Durability::Synced,
      )? {
        return Ok(segment);
      }
    }
    Err(Error::Collisions(MAX_ATTEMPTS))
  }

  pub fn read(&self, item: &ItemPath) -> Result<Vec<u8>> {
    let p = self.path_of(item);
    fs::read(&p).at(&p)
  }

  pub fn read_record(&self, id: &RecordId) -> Result<Record> {
    let p = self.path_of(&ItemPath::Record(id.clone()));
    let text = fs::read_to_string(&p).at(&p)?;
    Record::parse(&text).map_err(|source| Error::Record { path: p, source })
  }

  /// Adds a file received from another node. Its content must be a valid
  /// record or segment whose IDs match its path. It is not synced to disk
  /// on its own: the sender still holds it, and a file a crash leaves
  /// incomplete fails verification, is quarantined and is fetched again.
  pub fn insert(&self, path: &str, bytes: &[u8]) -> Result<Inserted> {
    let item = ItemPath::parse(path).ok_or_else(|| Error::InvalidPath(path.to_owned()))?;
    let record = verify(&item, bytes)?;
    if self.refuses(&item, record.as_ref()) {
      return Ok(Inserted::Purged);
    }
    if self.write_new(path, bytes, Durability::Unsynced)? {
      return Ok(Inserted::New);
    }
    let existing = self.read(&item)?;
    if BlobHash::of(&existing) == BlobHash::of(bytes) {
      Ok(Inserted::Present)
    } else {
      Err(Error::Conflict(path.to_owned()))
    }
  }

  /// Every record and segment in the root, sorted by path. Files that do
  /// not fit the layout are skipped.
  pub fn scan(&self) -> Result<Vec<ItemPath>> {
    let mut out = Vec::new();
    for top in ["records", "threads"] {
      let dir = self.dir.join(top);
      let Ok(entries) = fs::read_dir(&dir) else {
        continue;
      };
      for sub in entries {
        let sub = sub.at(&dir)?;
        if !sub.file_type().at(&sub.path())?.is_dir() {
          continue;
        }
        let sub_name = sub.file_name();
        let Some(sub_name) = sub_name.to_str() else {
          continue;
        };
        for file in fs::read_dir(sub.path()).at(&sub.path())? {
          let file = file.at(&sub.path())?;
          let name = file.file_name();
          let Some(name) = name.to_str() else { continue };
          if let Some(item) = ItemPath::parse(&format!("{top}/{sub_name}/{name}")) {
            out.push(item);
          }
        }
      }
    }
    out.sort_by_key(ToString::to_string);
    Ok(out)
  }

  /// Moves a file that fails verification out of the layout, to
  /// `.tmp/corrupt/`, so that sync fetches it again. Returns where it went.
  pub fn quarantine(&self, item: &ItemPath) -> Result<PathBuf> {
    let from = self.path_of(item);
    let dir = self.dir.join(TMP_DIR).join("corrupt");
    fs::create_dir_all(&dir).at(&dir)?;
    let to = dir.join(item.to_string().replace('/', "_"));
    fs::rename(&from, &to).at(&from)?;
    Ok(to)
  }

  /// Creates `rel` with `bytes` atomically and exclusively: the content is
  /// written under a temporary name, then hard-linked into place, which
  /// fails if the name exists. Returns false if it existed.
  /// Writes an item exclusively and durably; `false` when it was there.
  pub(crate) fn write_new_item(&self, item: &ItemPath, bytes: &[u8]) -> Result<bool> {
    self.write_new(&item.to_string(), bytes, Durability::Synced)
  }

  fn write_new(&self, rel: &str, bytes: &[u8], durability: Durability) -> Result<bool> {
    let target = self.dir.join(rel);
    let parent = target.parent().expect("rel paths have a parent");
    fs::create_dir_all(parent).at(parent)?;
    let tmp_dir = self.dir.join(TMP_DIR);
    fs::create_dir_all(&tmp_dir).at(&tmp_dir)?;
    let tmp = tmp_dir.join(format!("{:016x}", rand::random::<u64>()));
    {
      let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .at(&tmp)?;
      f.write_all(bytes).at(&tmp)?;
      if durability == Durability::Synced {
        f.sync_all().at(&tmp)?;
      }
    }
    let linked = fs::hard_link(&tmp, &target);
    let _ = fs::remove_file(&tmp);
    match linked {
      Ok(()) => {
        if durability == Durability::Synced
          && let Ok(d) = fs::File::open(parent)
        {
          let _ = d.sync_all();
        }
        Ok(true)
      }
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
      Err(e) => Err(e).at(&target),
    }
  }
}

/// What writing a deleted thread's file, or one going with it, fails with.
fn deleted(item: &ItemPath) -> Error {
  Error::Mismatch {
    path: item.to_string(),
    message: "it belongs to a deleted thread".into(),
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Durability {
  /// The file and its directory are synced before the call returns.
  Synced,
  /// Left to the OS: for files another node also holds.
  Unsynced,
}

/// Checks that `bytes` is a valid file for `item`; the record, when it
/// is one.
pub(crate) fn verify(item: &ItemPath, bytes: &[u8]) -> Result<Option<Record>> {
  let path = item.to_string();
  let mismatch = |message: String| Error::Mismatch {
    path: path.clone(),
    message,
  };
  let text = std::str::from_utf8(bytes).map_err(|_| mismatch("not UTF-8".into()))?;
  match item {
    ItemPath::Record(id) => {
      let r = Record::parse(text).map_err(|e| mismatch(e.to_string()))?;
      if &r.id != id {
        return Err(mismatch(format!("holds record {}", r.id)));
      }
      return Ok(Some(r));
    }
    ItemPath::Segment { thread, segment } => {
      let s = Segment::parse(text).map_err(|e| mismatch(e.to_string()))?;
      if &s.header.segment != segment || &s.header.thread != thread {
        return Err(mismatch(format!(
          "holds segment {} of thread {}",
          s.header.segment, s.header.thread
        )));
      }
    }
  }
  Ok(None)
}

#[cfg(test)]
mod tests {
  use super::*;
  use stitch_records::{Author, Ref};

  fn node(s: &str) -> NodeName {
    s.parse().unwrap()
  }

  #[test]
  fn item_paths_accept_only_the_layout() {
    for ok in [
      "records/2026-10/2026-10-11T143207Z-mbp-k3f9.md",
      "threads/2026-10-11T140112Z-mbp-q7cd/2026-10-11T141530Z-mbp-77ha.jsonl",
    ] {
      assert_eq!(ItemPath::parse(ok).unwrap().to_string(), ok);
    }
    for bad in [
      "records/2026-11/2026-10-11T143207Z-mbp-k3f9.md",
      "records/2026-10/2026-10-11T143207Z-mbp-k3f9.toml",
      "records/../2026-10-11T143207Z-mbp-k3f9.md",
      "../records/2026-10/2026-10-11T143207Z-mbp-k3f9.md",
      "/records/2026-10/2026-10-11T143207Z-mbp-k3f9.md",
      "records/2026-10/x/2026-10-11T143207Z-mbp-k3f9.md",
      "threads/x/2026-10-11T141530Z-mbp-77ha.jsonl",
      "repo.toml",
    ] {
      assert!(ItemPath::parse(bad).is_none(), "{bad}");
    }
  }

  #[test]
  fn published_records_are_read_back_and_scanned() {
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(dir.path(), "stitch".parse().unwrap(), Role::Project).unwrap();
    let task = root
      .publish(
        &node("phone"),
        Draft::new("task", Author::user(node("phone")))
          .unwrap()
          .body("# Do it\n"),
      )
      .unwrap();
    let q = root
      .publish(
        &node("mbp"),
        Draft::new("question", Author::node(node("mbp")))
          .unwrap()
          .reference("task", Ref::to(task.id.clone()))
          .unwrap(),
      )
      .unwrap();
    assert_eq!(root.read_record(&task.id).unwrap(), task);
    let reopened = RecordRoot::open(dir.path()).unwrap();
    assert_eq!(reopened.config(), root.config());
    let items = reopened.scan().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.contains(&ItemPath::Record(q.id)));
    assert!(RecordRoot::init(dir.path(), "stitch".parse().unwrap(), Role::Project).is_err());
  }

  #[test]
  fn inserting_checks_content_against_the_path() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let ra = RecordRoot::init(a.path(), "r".parse().unwrap(), Role::Home).unwrap();
    let rb = RecordRoot::init(b.path(), "r".parse().unwrap(), Role::Home).unwrap();
    let rec = ra
      .publish(
        &node("a"),
        Draft::new("task", Author::user(node("a"))).unwrap(),
      )
      .unwrap();
    let item = ItemPath::Record(rec.id.clone());
    let bytes = ra.read(&item).unwrap();
    let path = item.to_string();
    assert_eq!(rb.insert(&path, &bytes).unwrap(), Inserted::New);
    assert_eq!(rb.insert(&path, &bytes).unwrap(), Inserted::Present);
    let mut other = bytes.clone();
    other.extend_from_slice(b"\nchanged\n");
    assert!(matches!(rb.insert(&path, &other), Err(Error::Conflict(_))));
    let other_id = rec.id.as_str().replace("-a-", "-b-");
    let wrong = format!("records/{}/{other_id}.md", rec.id.month());
    assert!(matches!(
      rb.insert(&wrong, &bytes),
      Err(Error::Mismatch { .. })
    ));
    assert!(matches!(
      rb.insert("../x.md", &bytes),
      Err(Error::InvalidPath(_))
    ));
  }

  #[test]
  fn colliding_ids_retry_with_a_new_suffix() {
    use rand::SeedableRng;
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(dir.path(), "r".parse().unwrap(), Role::Home).unwrap();
    let at: Timestamp = "2026-10-11T14:32:07Z".parse().unwrap();
    let n = node("mbp");
    let draft = || Draft::new("note", Author::node(n.clone())).unwrap();
    // The same seed draws the same suffixes: each publish after the first
    // collides with every earlier one before it finds a free name.
    let ids: Vec<_> = (0..5)
      .map(|_| {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        root.publish_with(&n, draft(), at, &mut rng).unwrap().id
      })
      .collect();
    assert_eq!(
      ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
      5
    );
    assert_eq!(root.scan().unwrap().len(), 5);
  }
}
