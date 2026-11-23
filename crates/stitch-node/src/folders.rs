//! Folder sync (design §2.16): a folder place's working files, the same on
//! every node that has the folder, synced both ways the way iCloud Drive
//! syncs them: no CRDT, nothing to resolve by hand.
//!
//! A folder's state is a tree of directories, each a blob (JSON, so an
//! unchanged directory costs nothing again), its files' contents blobs too;
//! each change is a `snapshot` record in the folder's own store naming the
//! root tree and the snapshots it follows, the node and, when an agent made
//! it, the thread. Every node finds the store from the folder's identity,
//! as a project's from its first commit. A snapshot attaches the blobs it
//! adds, so always-on nodes pull them with it.
//!
//! A node with the folder scans it (size and modification time, hashes for
//! what changed) before and after each turn on the place, after each sync,
//! and when the app asks; publishes what changed since the snapshot it last
//! had; and merges what other nodes published: text edited on both sides
//! three ways when the edits do not overlap, else the later edit wins and
//! the other is kept beside it as a conflict copy; an edit wins over a
//! deletion. Changes are applied only while no turn runs on the place.
//!
//! Files up to a size cap travel, binaries too (only text merges three
//! ways; a binary changed on both sides keeps a conflict copy); larger
//! files and links are listed as there but not carried, so no node takes
//! their absence for a deletion. A git work
//! tree never syncs: a folder that is one (or becomes one) is a project,
//! and a checkout inside a folder is listed as that project, its files left
//! alone.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Attachment, Author, BlobHash, Draft, NodeName, RecordId, RepoId};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

use crate::node::Shared;
use crate::place::{self, Place};
use crate::{Error, Result};

/// The record kind of a change.
pub const SNAPSHOT: &str = "snapshot";

/// Files larger than this stay on their node, by default (`[folders]
/// max_file_mb`).
pub const MAX_FILE_MB: u64 = 25;

/// Left out wherever they are: caches, builds, editors' leftovers.
const IGNORED: &[&str] = &[
  ".DS_Store",
  ".Trash",
  ".git",
  "node_modules",
  "target",
  ".build",
  "build",
  "dist",
  "__pycache__",
  ".venv",
  "*.swp",
  "*~",
  ".#*",
];

/// A path's state in a folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Leaf {
  /// A file, carried (text or binary).
  File { hash: BlobHash, size: u64 },
  /// There but not carried (too large, a link): other nodes leave
  /// theirs alone.
  Skip,
  /// A git checkout inside the folder, by its project's store.
  Project { repo: String },
}

/// A folder's paths (`/`-separated, relative), each with its state.
pub type Flat = BTreeMap<String, Leaf>;

/// Blobs written that were not there: hash, size, and the path they are
/// for (a directory's ends with `/`), for a snapshot to attach.
pub type NewBlobs = Vec<(BlobHash, u64, String)>;

/// A directory as its blob holds it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Tree {
  entries: BTreeMap<String, Node>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Node {
  File { hash: BlobHash, size: u64 },
  Skip,
  Project { repo: String },
  Dir { tree: BlobHash },
}

/// The store of a folder, from its identity (`Folders::key`), so every
/// node finds the same one.
pub fn store_id(key: &str) -> RepoId {
  let hex = blake3::hash(key.as_bytes()).to_hex();
  format!("f-{}", &hex[..16])
    .parse()
    .expect("hex digits and a dash make a repository ID")
}

/// The identity of a global thread's working folder (design §2.3): its
/// conversation's first thread (`fork::origin`), so its forks and
/// compactions share it and every node finds it from the thread.
pub fn workdir_key(origin: &RecordId) -> String {
  format!("{WORKDIR}{origin}")
}

const WORKDIR: &str = "workdir:";

/// The thread a working folder's identity names (`workdir_key`); `None`
/// for any other folder.
pub fn workdir_of(key: &str) -> Option<&str> {
  key
    .strip_prefix(WORKDIR)
    .filter(|o| !o.is_empty() && *o != "." && *o != ".." && !o.contains(['/', '\\']))
}

/// A name another node wrote that may not be written here.
fn bad_name(name: &str) -> bool {
  name.is_empty()
    || name == "."
    || name == ".."
    || name == ".git"
    || name.contains('/')
    || name.contains('\\')
    || name.contains('\0')
}

/// Stores `flat` as trees; the root's hash, and the blobs written that
/// were not there (hash, size, path), for the snapshot to attach.
pub fn store_tree(blobs: &BlobStore, flat: &Flat) -> Result<(BlobHash, NewBlobs)> {
  #[derive(Default)]
  struct Dir {
    leaves: BTreeMap<String, Leaf>,
    dirs: BTreeMap<String, Dir>,
  }
  let mut top = Dir::default();
  for (path, leaf) in flat {
    let mut d = &mut top;
    let mut parts: Vec<&str> = path.split('/').collect();
    let name = parts.pop().expect("a path has a name");
    for p in parts {
      d = d.dirs.entry(p.to_owned()).or_default();
    }
    d.leaves.insert(name.to_owned(), leaf.clone());
  }
  fn put(blobs: &BlobStore, d: &Dir, at: &str, new: &mut NewBlobs) -> Result<BlobHash> {
    let mut tree = Tree::default();
    for (name, sub) in &d.dirs {
      let path = if at.is_empty() {
        name.clone()
      } else {
        format!("{at}/{name}")
      };
      let tree_hash = put(blobs, sub, &path, new)?;
      tree
        .entries
        .insert(name.clone(), Node::Dir { tree: tree_hash });
    }
    for (name, leaf) in &d.leaves {
      tree.entries.insert(
        name.clone(),
        match leaf {
          Leaf::File { hash, size } => Node::File {
            hash: *hash,
            size: *size,
          },
          Leaf::Skip => Node::Skip,
          Leaf::Project { repo } => Node::Project { repo: repo.clone() },
        },
      );
    }
    let bytes = serde_json::to_vec(&tree).map_err(|e| Error::Config(e.to_string()))?;
    let hash = BlobHash::of(&bytes);
    if !blobs.contains(&hash) {
      blobs.put_bytes(&bytes)?;
      new.push((hash, bytes.len() as u64, format!("{at}/")));
    }
    Ok(hash)
  }
  let mut new = Vec::new();
  let root = put(blobs, &top, "", &mut new)?;
  Ok((root, new))
}

/// Every blob a root tree holds: its trees (no path) and its files (with
/// their paths), for a copy of it elsewhere (a shared app, design §2.31);
/// the blobs missing here, when some are.
pub(crate) fn tree_blobs(
  blobs: &BlobStore,
  root: &BlobHash,
) -> std::result::Result<Vec<(Option<String>, BlobHash)>, Vec<BlobHash>> {
  let mut out = vec![(None, *root)];
  let mut missing = Vec::new();
  let mut todo = vec![(String::new(), *root)];
  while let Some((at, hash)) = todo.pop() {
    let Ok(bytes) = std::fs::read(blobs.path(&hash)) else {
      missing.push(hash);
      continue;
    };
    let Ok(tree) = serde_json::from_slice::<Tree>(&bytes) else {
      continue;
    };
    for (name, node) in tree.entries {
      if bad_name(&name) {
        continue;
      }
      let path = if at.is_empty() {
        name
      } else {
        format!("{at}/{name}")
      };
      match node {
        Node::Dir { tree } => {
          out.push((None, tree));
          todo.push((path, tree));
        }
        Node::File { hash, .. } => {
          if blobs.contains(&hash) {
            out.push((Some(path), hash));
          } else {
            missing.push(hash);
          }
        }
        Node::Skip | Node::Project { .. } => {}
      }
    }
  }
  if missing.is_empty() {
    Ok(out)
  } else {
    Err(missing)
  }
}

/// The paths a root tree holds; the trees missing here, when some are.
pub fn load_tree(blobs: &BlobStore, root: &BlobHash) -> std::result::Result<Flat, Vec<BlobHash>> {
  let mut flat = Flat::new();
  let mut missing = Vec::new();
  let mut todo = vec![(String::new(), *root)];
  while let Some((at, hash)) = todo.pop() {
    let Ok(bytes) = std::fs::read(blobs.path(&hash)) else {
      missing.push(hash);
      continue;
    };
    let Ok(tree) = serde_json::from_slice::<Tree>(&bytes) else {
      continue;
    };
    for (name, node) in tree.entries {
      if bad_name(&name) {
        continue;
      }
      let path = if at.is_empty() {
        name
      } else {
        format!("{at}/{name}")
      };
      match node {
        Node::Dir { tree } => todo.push((path, tree)),
        Node::File { hash, size } => {
          flat.insert(path, Leaf::File { hash, size });
        }
        Node::Skip => {
          flat.insert(path, Leaf::Skip);
        }
        Node::Project { repo } => {
          flat.insert(path, Leaf::Project { repo });
        }
      }
    }
  }
  if missing.is_empty() {
    Ok(flat)
  } else {
    Err(missing)
  }
}

// ---------------------------------------------------------------- snapshots

/// A change as its record says it.
#[derive(Clone, Debug)]
pub struct Snap {
  pub id: RecordId,
  pub at: Timestamp,
  pub node: String,
  pub tree: BlobHash,
  pub prev: Vec<RecordId>,
  /// The thread whose turn made it.
  pub thread: Option<String>,
  /// The paths it changed from the first snapshot it follows.
  pub changed: Vec<String>,
}

/// Every snapshot of a folder's store, oldest first.
pub fn snapshots(index: &Index, repo: &RepoId) -> Result<Vec<Snap>> {
  let s = |v: &Value| v.as_str().map(str::to_owned);
  Ok(
    index
      .records(Some(SNAPSHOT))?
      .into_iter()
      .filter(|r| &r.repo == repo)
      .filter_map(|r| {
        let f = &r.fields;
        let tree = f.get("tree").and_then(s)?.parse().ok()?;
        let list = |k: &str| -> Vec<String> {
          f.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(s).collect())
            .unwrap_or_default()
        };
        Some(Snap {
          at: r.created,
          node: r.node.clone(),
          tree,
          prev: list("prev").iter().filter_map(|p| p.parse().ok()).collect(),
          thread: f.get("thread").and_then(s),
          changed: list("changed"),
          id: r.id,
        })
      })
      .collect(),
  )
}

/// The snapshots none follows.
pub fn heads(snaps: &[Snap]) -> Vec<&Snap> {
  let followed: BTreeSet<&RecordId> = snaps.iter().flat_map(|s| &s.prev).collect();
  snaps.iter().filter(|s| !followed.contains(&s.id)).collect()
}

fn ancestors(by: &HashMap<&RecordId, &Snap>, id: &RecordId) -> BTreeSet<RecordId> {
  let mut seen = BTreeSet::new();
  let mut todo = vec![id.clone()];
  while let Some(i) = todo.pop() {
    if !seen.insert(i.clone()) {
      continue;
    }
    if let Some(s) = by.get(&i) {
      todo.extend(s.prev.iter().cloned());
    }
  }
  seen
}

/// The latest snapshot both follow (or are): their common ancestor.
fn common<'a>(by: &HashMap<&RecordId, &'a Snap>, a: &RecordId, b: &RecordId) -> Option<&'a Snap> {
  let (x, y) = (ancestors(by, a), ancestors(by, b));
  x.intersection(&y)
    .filter_map(|i| by.get(i).copied())
    .max_by(|p, q| (p.at, &p.id).cmp(&(q.at, &q.id)))
}

/// The paths whose state differs between two.
pub fn changed(a: &Flat, b: &Flat) -> Vec<String> {
  let paths: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
  paths
    .into_iter()
    .filter(|p| a.get(*p) != b.get(*p))
    .cloned()
    .collect()
}

/// Publishes a change: its tree, what it follows, the blobs it adds.
#[allow(clippy::too_many_arguments)]
fn publish(
  root: &RecordRoot,
  index: &Mutex<Index>,
  node: &NodeName,
  key: &str,
  tree: BlobHash,
  prev: &[RecordId],
  thread: Option<&RecordId>,
  changed: &[String],
  new: &[(BlobHash, u64, String)],
) -> Result<(RecordId, Timestamp)> {
  let prev = toml::Value::Array(prev.iter().map(|p| p.to_string().into()).collect());
  // A long list says its start: `log` reads the rest from the trees.
  let shown = toml::Value::Array(
    changed
      .iter()
      .take(200)
      .map(|p| p.as_str().into())
      .collect(),
  );
  let mut draft = Draft::new(SNAPSHOT, Author::node(node.clone()))?
    .field("folder", key)?
    .field("tree", tree.to_string())?
    .field("prev", prev)?
    .field("changed", shown)?;
  if let Some(t) = thread {
    draft = draft.field("thread", t.to_string())?;
  }
  for (hash, bytes, path) in new {
    draft = draft.attach(Attachment {
      hash: *hash,
      media_type: if path.ends_with('/') {
        "application/json"
      } else {
        "text/plain"
      }
      .into(),
      name: Some(path.trim_end_matches('/').to_owned()).filter(|p| !p.is_empty()),
      bytes: *bytes,
    });
  }
  let record = root.publish(node, draft)?;
  index.lock().expect("index lock").refresh(root)?;
  Ok((record.id, record.created))
}

// ------------------------------------------------------------------- local

/// What a node keeps of a folder, never synced: the snapshot its files are
/// at, and what it knows of each file so a scan hashes only what changed.
#[derive(Debug, Default, Serialize, Deserialize)]
struct LocalState {
  base: Option<RecordId>,
  #[serde(default)]
  files: BTreeMap<String, Seen>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Seen {
  size: u64,
  /// Modification time, in nanoseconds since the epoch.
  mtime: i128,
  hash: BlobHash,
  /// When it was seen, in nanoseconds since the epoch (0: unknown).
  #[serde(default)]
  at: i128,
}

/// How close to being seen a file's modification time must not be for a
/// scan to trust it: file systems keep times coarsely (Linux by the
/// kernel's tick, FAT by two seconds), so an edit that soon after may have
/// the same time and size (git's "racy" files). Those are read again.
const RACY_NS: i128 = 2_000_000_000;

fn now_ns() -> i128 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_nanos() as i128)
}

impl Seen {
  /// Whether a file of this size and time is surely as seen, unread.
  fn still(&self, size: u64, mtime: i128) -> bool {
    self.size == size && self.mtime == mtime && self.mtime + RACY_NS < self.at
  }
}

fn dir_of(data: &Path, repo: &RepoId) -> PathBuf {
  data.join("folders").join(repo.as_str())
}

impl LocalState {
  fn load(data: &Path, repo: &RepoId) -> LocalState {
    std::fs::read(dir_of(data, repo).join("state.json"))
      .ok()
      .and_then(|b| serde_json::from_slice(&b).ok())
      .unwrap_or_default()
  }

  fn save(&self, data: &Path, repo: &RepoId) -> Result<()> {
    let dir = dir_of(data, repo);
    std::fs::create_dir_all(&dir).map_err(Error::io(dir.display()))?;
    let path = dir.join("state.json");
    let tmp = dir.join("state.json.tmp");
    let bytes = serde_json::to_vec(self).map_err(|e| Error::Config(e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(Error::io(tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))
  }
}

fn mtime(meta: &std::fs::Metadata) -> i128 {
  meta
    .modified()
    .ok()
    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map_or(0, |d| d.as_nanos() as i128)
}

/// Whether a file is as a scan saw it, by its content: its time can be
/// the same after an edit (`RACY_NS`).
fn unchanged(path: &Path, seen: Option<&Seen>) -> bool {
  match (std::fs::symlink_metadata(path), seen) {
    (Err(e), None) => e.kind() == std::io::ErrorKind::NotFound,
    (Ok(m), Some(s)) if m.is_file() && m.len() == s.size => {
      std::fs::read(path).is_ok_and(|b| BlobHash::of(&b) == s.hash)
    }
    _ => false,
  }
}

/// What a folder leaves out: the defaults and its `.stitch/ignore` (a
/// pattern a line, `*` and `?`, against a name, or against the path when
/// it has a `/`; `#` comments).
pub struct Ignore {
  patterns: Vec<String>,
}

impl Ignore {
  pub fn of(dir: &Path) -> Ignore {
    let mut patterns: Vec<String> = IGNORED.iter().map(|p| (*p).to_owned()).collect();
    if let Ok(text) = std::fs::read_to_string(dir.join(".stitch").join("ignore")) {
      patterns.extend(
        text
          .lines()
          .map(str::trim)
          .filter(|l| !l.is_empty() && !l.starts_with('#'))
          .map(|l| l.trim_matches('/').to_owned()),
      );
    }
    Ignore { patterns }
  }

  pub fn ignores(&self, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    self.patterns.iter().any(|p| {
      if p.contains('/') {
        glob(p, path)
      } else {
        glob(p, name)
      }
    })
  }
}

/// `*` (any run, not `/`) and `?` (one character).
fn glob(pattern: &str, text: &str) -> bool {
  fn go(p: &[char], t: &[char]) -> bool {
    match (p.first(), t.first()) {
      (None, None) => true,
      (Some('*'), _) => go(&p[1..], t) || (t.first().is_some_and(|c| *c != '/') && go(p, &t[1..])),
      (Some('?'), Some(c)) if *c != '/' => go(&p[1..], &t[1..]),
      (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
      _ => false,
    }
  }
  let p: Vec<char> = pattern.chars().collect();
  let t: Vec<char> = text.chars().collect();
  go(&p, &t)
}

/// Whether a directory is a git work tree (`.git`, a directory, or a file
/// as worktrees and submodules have).
pub fn is_checkout(dir: &Path) -> bool {
  std::fs::symlink_metadata(dir.join(".git")).is_ok()
}

/// A folder's files as they are: files up to `max_file` bytes, binaries
/// too, stored as blobs (only those
/// that changed since the last scan are read); `files` is updated. Blobs
/// written that were not there are returned for the snapshot to attach.
fn scan(
  dir: &Path,
  files: &mut BTreeMap<String, Seen>,
  blobs: &BlobStore,
  project: &dyn Fn(&Path) -> Option<String>,
  max_file: u64,
) -> Result<(Flat, NewBlobs)> {
  let ignore = Ignore::of(dir);
  let mut flat = Flat::new();
  let mut new = Vec::new();
  let mut seen = BTreeMap::new();
  let mut walk = walkdir::WalkDir::new(dir)
    .follow_links(false)
    .min_depth(1)
    .into_iter();
  while let Some(entry) = walk.next() {
    let Ok(entry) = entry else { continue };
    let Ok(rel) = entry.path().strip_prefix(dir) else {
      continue;
    };
    let Some(rel) = rel.to_str().map(|r| r.replace('\\', "/")) else {
      continue;
    };
    if ignore.ignores(&rel) {
      if entry.file_type().is_dir() {
        walk.skip_current_dir();
      }
      continue;
    }
    let kind = entry.file_type();
    if kind.is_dir() {
      if is_checkout(entry.path()) {
        walk.skip_current_dir();
        flat.insert(
          rel,
          match project(entry.path()) {
            Some(repo) => Leaf::Project { repo },
            None => Leaf::Skip,
          },
        );
      }
      continue;
    }
    if !kind.is_file() {
      flat.insert(rel, Leaf::Skip);
      continue;
    }
    let Ok(meta) = entry.metadata() else { continue };
    if meta.len() > max_file {
      flat.insert(rel, Leaf::Skip);
      continue;
    }
    let now = Seen {
      size: meta.len(),
      mtime: mtime(&meta),
      hash: BlobHash::of(b""),
      at: now_ns(),
    };
    let known = files
      .get(&rel)
      .filter(|s| s.still(now.size, now.mtime) && blobs.contains(&s.hash));
    let hash = match known {
      Some(s) => s.hash,
      None => {
        let Ok(bytes) = std::fs::read(entry.path()) else {
          continue;
        };
        let hash = BlobHash::of(&bytes);
        if !blobs.contains(&hash) {
          blobs.put_bytes(&bytes)?;
          new.push((hash, bytes.len() as u64, rel.clone()));
        }
        hash
      }
    };
    seen.insert(rel.clone(), Seen { hash, ..now });
    flat.insert(
      rel,
      Leaf::File {
        hash,
        size: meta.len(),
      },
    );
  }
  *files = seen;
  Ok((flat, new))
}

/// How much of a folder a node syncs, at most, by default (`[folders]`):
/// a folder over either limit syncs nothing.
pub const MAX_FILES: usize = 2000;
pub const MAX_MB: u64 = 250;

/// A node's limits for its folders (`[folders]` in config.toml).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
  /// Files a folder carries, at most.
  pub files: usize,
  /// Their bytes together, at most.
  pub bytes: u64,
  /// One file's bytes, at most: a larger one stays on its node.
  pub file: u64,
}

impl Default for Limits {
  fn default() -> Limits {
    Limits {
      files: MAX_FILES,
      bytes: MAX_MB * 1_000_000,
      file: MAX_FILE_MB * 1_000_000,
    }
  }
}

/// What a folder would carry, as far as sizes tell (no file read).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
  pub files: usize,
  pub bytes: u64,
  /// Over a limit: counting stopped there.
  pub over: bool,
}

impl std::fmt::Display for Size {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    // Counting stopped at the limit: at least this much.
    let more = if self.over { "at least " } else { "" };
    let mb = self.bytes as f64 / 1e6;
    let size = if mb < 1.0 {
      format!("{} KB", self.bytes.div_ceil(1000))
    } else {
      format!("{mb:.0} MB")
    };
    write!(f, "{more}{} file(s), {size}", self.files)
  }
}

/// Counts what a scan of `dir` would carry, with the same rules, stopping
/// once over the limits' files or bytes.
pub fn measure(dir: &Path, limits: Limits) -> Size {
  let ignore = Ignore::of(dir);
  let mut size = Size::default();
  let mut walk = walkdir::WalkDir::new(dir)
    .follow_links(false)
    .min_depth(1)
    .into_iter();
  while let Some(entry) = walk.next() {
    let Ok(entry) = entry else { continue };
    let Some(rel) = entry
      .path()
      .strip_prefix(dir)
      .ok()
      .and_then(|r| r.to_str())
      .map(|r| r.replace('\\', "/"))
    else {
      continue;
    };
    let kind = entry.file_type();
    if ignore.ignores(&rel) || (kind.is_dir() && is_checkout(entry.path())) {
      if kind.is_dir() {
        walk.skip_current_dir();
      }
      continue;
    }
    if !kind.is_file() {
      continue;
    }
    let Ok(meta) = entry.metadata() else { continue };
    if meta.len() > limits.file {
      continue;
    }
    size.files += 1;
    size.bytes += meta.len();
    if size.files > limits.files || size.bytes > limits.bytes {
      size.over = true;
      break;
    }
  }
  size
}

// ------------------------------------------------------------------- merge

/// One side of a merge: its paths, and when and where it was written.
pub struct Side<'a> {
  pub flat: &'a Flat,
  pub at: Timestamp,
  pub node: &'a str,
}

/// Whether a path is one this node has but does not carry: a file it
/// leaves out, or one inside a checkout or a directory it leaves out.
pub fn covered(here: &Flat, path: &str) -> bool {
  let local = |p: &str| matches!(here.get(p), Some(Leaf::Skip | Leaf::Project { .. }));
  local(path) || path.match_indices('/').any(|(i, _)| local(&path[..i]))
}

/// What a node publishes: the files it carries, and for what it has but
/// does not carry, the state the others share (`shared`), so its tree is
/// the others' wherever the content is the same.
pub fn carried(here: &Flat, shared: &Flat) -> Flat {
  let mut out: Flat = here
    .iter()
    .filter(|(p, l)| matches!(l, Leaf::File { .. }) && !covered(here, p))
    .map(|(p, l)| (p.clone(), l.clone()))
    .collect();
  for (p, l) in shared {
    if covered(here, p) {
      out.insert(p.clone(), l.clone());
    }
  }
  out
}

/// Merges `theirs` into `ours` (both as published) from their common
/// ancestor `base`; the result and the conflict copies it made. Text
/// edited on both sides is merged three ways when the edits do not
/// overlap; otherwise the later side wins and the other is kept as a
/// conflict copy. An edit wins over a deletion. What this node has but
/// does not carry (`here`) has no say: theirs stands there.
pub fn merge(
  blobs: &BlobStore,
  base: &Flat,
  ours: &Side,
  theirs: &Side,
  here: &Flat,
) -> Result<(Flat, Vec<String>)> {
  let file = |l: Option<&Leaf>| l.filter(|l| matches!(l, Leaf::File { .. })).cloned();
  let mut out = Flat::new();
  let mut copies = Vec::new();
  let paths: BTreeSet<&String> = base
    .keys()
    .chain(ours.flat.keys())
    .chain(theirs.flat.keys())
    .collect();
  for p in paths {
    let (a, o, t) = (base.get(p), ours.flat.get(p), theirs.flat.get(p));
    if covered(here, p) {
      if let Some(t) = file(t) {
        out.insert(p.clone(), t);
      }
      continue;
    }
    let (a, o, t) = (file(a), file(o), file(t));
    let take = if o == t || t == a {
      o
    } else if o == a {
      t
    } else {
      match (o, t) {
        (Some(o), None) => Some(o),
        (None, Some(t)) => Some(t),
        (Some(o), Some(t)) => {
          match three_way(blobs, a.as_ref(), &o, &t)? {
            Some(merged) => Some(merged),
            None => {
              // The later edit stands; the other is kept beside it.
              // The later edit stands; at the same instant, the node whose
              // name sorts last, so every node decides alike.
              let later = (theirs.at, theirs.node) > (ours.at, ours.node);
              let (win, lose, loser) = if later { (t, o, ours) } else { (o, t, theirs) };
              let copy = conflict_name(p, loser.node, loser.at, |c| {
                base.contains_key(c)
                  || ours.flat.contains_key(c)
                  || theirs.flat.contains_key(c)
                  || out.contains_key(c)
              });
              out.insert(copy.clone(), lose);
              copies.push(copy);
              Some(win)
            }
          }
        }
        (None, None) => None,
      }
    };
    if let Some(l) = take {
      out.insert(p.clone(), l);
    }
  }
  Ok((out, copies))
}

/// Both sides' text merged against the base; `None` when the edits
/// overlap (or there is no base: both added it).
fn three_way(
  blobs: &BlobStore,
  base: Option<&Leaf>,
  ours: &Leaf,
  theirs: &Leaf,
) -> Result<Option<Leaf>> {
  let text = |l: &Leaf| -> Option<String> {
    let Leaf::File { hash, .. } = l else {
      return None;
    };
    // A binary (not UTF-8, or with a NUL) is never merged.
    std::fs::read(blobs.path(hash))
      .ok()
      .filter(|b| !b.contains(&0))
      .and_then(|b| String::from_utf8(b).ok())
  };
  let (Some(base), Some(o), Some(t)) = (base.and_then(text), text(ours), text(theirs)) else {
    return Ok(None);
  };
  match diffy::merge(&base, &o, &t) {
    Ok(merged) => {
      let hash = blobs.put_bytes(merged.as_bytes())?;
      Ok(Some(Leaf::File {
        hash,
        size: merged.len() as u64,
      }))
    }
    Err(_) => Ok(None),
  }
}

/// `notes/AGENTS (mac, 2026-10-14).md`, numbered when taken.
fn conflict_name(path: &str, node: &str, at: Timestamp, taken: impl Fn(&str) -> bool) -> String {
  let (dir, name) = match path.rsplit_once('/') {
    Some((d, n)) => (format!("{d}/"), n),
    None => (String::new(), path),
  };
  let (stem, ext) = match name.rsplit_once('.') {
    Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
    _ => (name, String::new()),
  };
  let day = at.strftime("%Y-%m-%d");
  let mut n = 1;
  loop {
    let suffix = if n == 1 {
      String::new()
    } else {
      format!(" {n}")
    };
    let candidate = format!("{dir}{stem} ({node}, {day}){suffix}{ext}");
    if !taken(&candidate) {
      return candidate;
    }
    n += 1;
  }
}

/// What `apply` did: how many files it wrote or removed, and those it left
/// because they were edited since the scan.
#[derive(Debug, Default, PartialEq, Eq)]
struct Applied {
  changed: usize,
  spared: Vec<String>,
}

/// Writes `to` over `from` in `dir`, only where every file it would write
/// or remove is still as the scan saw it: a file edited since spares them
/// all (that edit is taken up by the next pass, against what this node had,
/// so it merges with the others' instead of overwriting them). What a node
/// does not carry is never touched.
fn apply(
  dir: &Path,
  from: &Flat,
  to: &Flat,
  files: &mut BTreeMap<String, Seen>,
  blobs: &BlobStore,
) -> Result<Applied> {
  let paths: Vec<&String> = from
    .keys()
    .chain(to.keys())
    .collect::<BTreeSet<_>>()
    .into_iter()
    .filter(|p| from.get(*p) != to.get(*p) && !covered(from, p))
    .collect();
  let spared: Vec<String> = paths
    .iter()
    .filter(|p| !unchanged(&dir.join(p), files.get(**p)))
    .map(|p| (*p).clone())
    .collect();
  if !spared.is_empty() {
    return Ok(Applied { changed: 0, spared });
  }
  let mut out = Applied::default();
  for p in paths {
    let (old, new) = (from.get(p), to.get(p));
    let path = dir.join(p);
    // Edited between the check and here: spared, as above.
    if !unchanged(&path, files.get(p)) {
      out.spared.push(p.clone());
      continue;
    }
    match new {
      Some(Leaf::File { hash, size }) => {
        let bytes =
          std::fs::read(blobs.path(hash)).map_err(Error::io(blobs.path(hash).display()))?;
        let parent = path.parent().expect("a file in the folder");
        std::fs::create_dir_all(parent).map_err(Error::io(parent.display()))?;
        let tmp = parent.join(format!(
          ".{}.stitch-tmp",
          path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
        ));
        std::fs::write(&tmp, &bytes).map_err(Error::io(tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))?;
        let meta = std::fs::metadata(&path).map_err(Error::io(path.display()))?;
        files.insert(
          p.clone(),
          Seen {
            size: *size,
            mtime: mtime(&meta),
            hash: *hash,
            at: now_ns(),
          },
        );
        out.changed += 1;
      }
      None if matches!(old, Some(Leaf::File { .. })) => {
        match std::fs::remove_file(&path) {
          Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(Error::io(path.display())(e));
          }
          _ => {}
        }
        files.remove(p);
        // Directories left empty go too, up to the folder.
        let mut d = path.parent();
        while let Some(x) = d.filter(|x| *x != dir) {
          if std::fs::remove_dir(x).is_err() {
            break;
          }
          d = x.parent();
        }
        out.changed += 1;
      }
      _ => {}
    }
  }
  Ok(out)
}

// ------------------------------------------------------------------- nodes

/// What the node keeps of folder sync while it runs.
#[derive(Default)]
pub(crate) struct FolderSync {
  /// One pass at a time per folder.
  locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
  /// Turns running per folder: changes wait for them.
  turns: Mutex<HashMap<String, usize>>,
  /// New folders whose sync the user is being asked about.
  asking: Mutex<std::collections::HashSet<String>>,
}

impl FolderSync {
  fn lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
    self
      .locks
      .lock()
      .expect("folder locks")
      .entry(key.to_owned())
      .or_default()
      .clone()
  }

  fn running(&self, key: &str) -> bool {
    self
      .turns
      .lock()
      .expect("folder turns")
      .get(key)
      .is_some_and(|n| *n > 0)
  }
}

/// Whether this node keeps folders' files of its own: a phone, which has
/// no folders of the user's, holds a copy in its data directory.
pub(crate) fn keeps_copies() -> bool {
  cfg!(any(target_os = "ios", target_os = "visionos"))
}

/// The copy of a folder a phone keeps.
pub fn copy_dir(data: &Path, key: &str) -> PathBuf {
  dir_of(data, &store_id(key)).join("files")
}

/// Where a folder's files are on this node: the folder, or a phone's copy;
/// `None` where it is not, or no longer a folder (folded into a project).
pub(crate) fn location(s: &Shared, key: &str) -> Option<PathBuf> {
  // A working folder is in the data directory, phones' too; only where
  // a turn on its thread made it (`turn`), so a node holds what it does
  // not use as records alone.
  if let Some(origin) = workdir_of(key) {
    return Some(s.data.join(crate::node::THREAD_FOLDERS).join(origin)).filter(|d| d.is_dir());
  }
  let places = s.places();
  if places.folders.project(key).is_some() {
    return None;
  }
  if let Some(d) = places.folders.dir(key, s.state().node.as_str()) {
    return Some(d);
  }
  keeps_copies().then(|| copy_dir(&s.data, key))
}

/// The folders that are places: those with threads; and the working
/// folders of global threads that synced (a node published one).
pub(crate) fn keys(s: &Shared) -> Vec<String> {
  let places = s.places();
  let state = s.state();
  let home = state.home().map(|h| h.repo().clone());
  let index = state.index.lock().expect("index lock");
  let mut out: BTreeSet<String> = places
    .folders
    .allowed()
    .filter(|k| workdir_of(k).is_some())
    .map(str::to_owned)
    .collect();
  for (r, _) in index.threads().unwrap_or_default() {
    if let Place::Folder(key) = Place::of_thread(&r, home.as_ref(), &places) {
      out.insert(key);
    }
  }
  out.into_iter().collect()
}

/// This node's limits (`[folders]`).
pub(crate) fn limits(s: &Shared) -> Limits {
  let f = s.config.folders.as_ref();
  Limits {
    files: f.and_then(|f| f.max_files).unwrap_or(MAX_FILES),
    bytes: f.and_then(|f| f.max_mb).unwrap_or(MAX_MB) * 1_000_000,
    file: f.and_then(|f| f.max_file_mb).unwrap_or(MAX_FILE_MB) * 1_000_000,
  }
}

/// Whether the folder synced before the user was asked about new ones
/// (design §2.16): a snapshot of it from any node, or one this node's
/// files are at. Such a folder goes on syncing, unasked.
fn syncing(s: &Shared, key: &str) -> bool {
  let repo = store_id(key);
  if LocalState::load(&s.data, &repo).base.is_some() {
    return true;
  }
  let state = s.state();
  let index = state.index.lock().expect("index lock");
  snapshots(&index, &repo).is_ok_and(|v| !v.is_empty())
}

/// Asks the user, as an approval of `thread`, whether the new folder `key`
/// syncs; the answer is a `folder` record (`sync`, `allow`), so every node
/// knows it. Once per folder at a time, without holding up the turn: until
/// it is answered, the folder's files stay where they are.
fn ask_to_sync(s: &Arc<Shared>, key: &str, thread: &RecordId, size: Size) {
  if !s
    .folder_sync
    .asking
    .lock()
    .expect("folder asking")
    .insert(key.to_owned())
  {
    return;
  }
  let (s, key, thread) = (s.clone(), key.to_owned(), thread.clone());
  tokio::spawn(async move {
    let asked = async {
      let (root, _) = s.thread_place(&thread)?;
      let l = limits(&s);
      let over = if size.over {
        format!(
          " (over this node's limits, {} files and {} MB: it would sync nothing until \
           `.stitch/ignore` leaves enough out)",
          l.files,
          l.bytes / 1_000_000
        )
      } else {
        String::new()
      };
      let mut req = crate::tools::ApprovalRequest::new(
        thread.clone(),
        SYNC_SESSION.into(),
        format!("Sync the folder {key} with your other machines: {size}{over}"),
      );
      // Answered elsewhere meanwhile (`stitch folder sync --allow`, another
      // node's question): this one goes. Yolo does not answer it.
      let (s2, k2) = (s.clone(), key.clone());
      req.allowed_now = Some(Arc::new(move || s2.places().folders.sync(&k2).is_some()));
      let d = crate::detached::ask(&s, &root, req, None, None).await;
      let withdrawn = d
        .reason
        .as_deref()
        .is_some_and(|r| r.starts_with("withdrawn") || r.starts_with("the approval could not"));
      if s.places().folders.sync(&key).is_none() && (d.allow || !withdrawn) {
        let Some(home) = s.state().home() else {
          return Ok(());
        };
        let node = s.state().node.clone();
        home.publish(&node, place::sync_draft(&node, &key, d.allow)?)?;
        s.state().index.lock().expect("index lock").refresh(&home)?;
        if d.allow {
          pass_and_push(&s, &key, None).await?;
        }
        crate::node::sync_all(&s, None).await;
      }
      Ok::<_, Error>(())
    };
    if let Err(e) = asked.await {
      eprintln!("folder {key}: asking to sync it: {e}");
    }
    s.folder_sync
      .asking
      .lock()
      .expect("folder asking")
      .remove(&key);
  });
}

/// The session an approval to sync a folder names: no shell's.
pub const SYNC_SESSION: &str = "folder sync";

/// The folder's store, held from now on (made when there is none).
fn hold(s: &Shared, key: &str) -> Result<RecordRoot> {
  Ok(hold_new(s, key)?.0)
}

/// `hold`, and whether the store was not held before.
fn hold_new(s: &Shared, key: &str) -> Result<(RecordRoot, bool)> {
  let repo = store_id(key);
  let state = s.state();
  if let Some(r) = state.root(&repo) {
    return Ok((r, false));
  }
  let dir = dir_of(&s.data, &repo).join("records");
  let root = match RecordRoot::open(&dir) {
    Ok(r) => r,
    Err(_) => RecordRoot::init(&dir, repo, Role::Folder)?,
  };
  state.index.lock().expect("index lock").refresh(&root)?;
  state.add_root(root.clone());
  Ok((root, true))
}

/// Holds the store of every folder with threads (and of every working
/// folder that synced), so syncs carry them; first unfolds this node's
/// folders whose `.git` went away. The folders whose store it began to
/// hold.
pub(crate) fn hold_all(s: &Shared) -> Vec<String> {
  let state = s.state();
  if let Some(home) = state.home() {
    let mut index = state.index.lock().expect("index lock");
    match place::unfold(&mut index, &home, &state.node) {
      Ok(keys) => {
        for key in keys {
          eprintln!("folder {key}: its .git is gone; a folder again, its files synced");
        }
      }
      Err(e) => eprintln!("unfolding folders: {e}"),
    }
  }
  let mut new = Vec::new();
  for key in keys(s) {
    match hold_new(s, &key) {
      Ok((_, true)) => new.push(key),
      Ok(_) => {}
      Err(e) => eprintln!("folder {key}: {e}"),
    }
  }
  new
}

/// What a pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pass {
  /// A snapshot of this node's changes was published.
  pub published: bool,
  /// Files written or removed here.
  pub applied: usize,
  /// Conflict copies made.
  pub conflicts: Vec<String>,
  /// Why nothing was done, when it was not.
  pub skipped: Option<String>,
  /// Files edited here while the others' changes were merged: nothing
  /// was written, and another pass follows.
  pub spared: Vec<String>,
  /// A new folder, waiting for the user to let it sync: what it holds.
  pub ask: Option<Size>,
}

/// A pass merging while files were edited leaves the merge to another;
/// at most this many in a row.
const PASSES: usize = 3;

/// One pass over a folder: what changed here published (by `thread` when
/// a turn made it), what other nodes published merged in and written.
/// Nothing while a turn runs on the folder. A file edited while merging
/// spares the merge (`apply`): another pass takes the edit up.
pub(crate) async fn pass(s: &Arc<Shared>, key: &str, thread: Option<&RecordId>) -> Result<Pass> {
  let mut published = false;
  let mut out = None;
  for _ in 1..PASSES {
    let p = pass_once(s, key, thread).await?;
    published |= p.published;
    if p.spared.is_empty() {
      out = Some(p);
      break;
    }
  }
  let p = match out {
    Some(p) => p,
    None => pass_once(s, key, thread).await?,
  };
  let p = Pass {
    published: published || p.published,
    ..p
  };
  if p.published && workdir_of(key).is_some() && s.places().folders.sync(key).is_none() {
    register(s, key)?;
  }
  Ok(p)
}

/// A working folder's first snapshot: a `folder` record (`sync` = its
/// identity, `allow`) tells every node it syncs, so the always-on ones
/// hold its store and take its changes while the node that made them is
/// still there (a phone, about to sleep), before a turn on it runs there.
fn register(s: &Shared, key: &str) -> Result<()> {
  let state = s.state();
  let Some(home) = state.home() else {
    return Ok(());
  };
  let node = state.node.clone();
  home.publish(&node, place::sync_draft(&node, key, true)?)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok(())
}

async fn pass_once(s: &Arc<Shared>, key: &str, thread: Option<&RecordId>) -> Result<Pass> {
  let lock = s.folder_sync.lock(key);
  let _held = lock.lock().await;
  if s.folder_sync.running(key) {
    return Ok(Pass {
      skipped: Some("a turn runs on it".into()),
      ..Pass::default()
    });
  }
  let Some(dir) = location(s, key) else {
    return Ok(Pass {
      skipped: Some("not on this node".into()),
      ..Pass::default()
    });
  };
  if is_checkout(&dir) {
    return Ok(Pass {
      skipped: Some("a git checkout: a project, synced by git".into()),
      ..Pass::default()
    });
  }
  let lim = limits(s);
  let workdir = workdir_of(key).is_some();
  match s.places().folders.sync(key) {
    Some(true) => {}
    // A working folder syncs unasked: Stitch made it, in its own data
    // directory, for its own thread (design §2.3). Until it holds a file
    // there is nothing to sync, and no store is held for it.
    None if workdir => {
      let d = dir.clone();
      let size = tokio::task::spawn_blocking(move || measure(&d, lim))
        .await
        .map_err(|e| Error::Config(e.to_string()))?;
      if size.files == 0 && !syncing(s, key) {
        return Ok(Pass {
          skipped: Some("an empty working folder: nothing to sync yet".into()),
          ..Pass::default()
        });
      }
    }
    Some(false) => {
      return Ok(Pass {
        skipped: Some(
          "kept on each node: the user declined its sync (`stitch folder sync --allow` in it \
           lets it sync)"
            .into(),
        ),
        ..Pass::default()
      });
    }
    None => {
      if !keeps_copies()
        && let Some(why) = place::never_synced(&dir)
      {
        return Ok(Pass {
          skipped: Some(format!(
            "{why}: never synced unless allowed (`stitch folder sync --allow` in it)"
          )),
          ..Pass::default()
        });
      }
      if !syncing(s, key) {
        let d = dir.clone();
        let size = tokio::task::spawn_blocking(move || measure(&d, lim))
          .await
          .map_err(|e| Error::Config(e.to_string()))?;
        return Ok(Pass {
          skipped: Some(format!(
            "a new folder ({size}): it syncs once the user lets it (`stitch folder sync --allow` \
             in it, or the question its thread asks)"
          )),
          ask: Some(size),
          ..Pass::default()
        });
      }
    }
  }
  {
    let d = dir.clone();
    let size = tokio::task::spawn_blocking(move || measure(&d, lim))
      .await
      .map_err(|e| Error::Config(e.to_string()))?;
    if size.over {
      return Ok(Pass {
        skipped: Some(format!(
          "too large to sync ({size}; at most {} files and {} MB, `[folders] max_files` and \
           `max_mb` in config.toml): add what need not sync to `.stitch/ignore`",
          lim.files,
          lim.bytes / 1_000_000
        )),
        ..Pass::default()
      });
    }
  }
  std::fs::create_dir_all(&dir).map_err(Error::io(dir.display()))?;
  let root = hold(s, key)?;
  let repo = root.repo().clone();
  let mut local = LocalState::load(&s.data, &repo);
  let state = s.state().clone();
  let projects = s.places().projects;

  // This node's changes, published.
  let (scanned, base, tip, mut pass) = {
    let (st, dir, key, thread) = (state.clone(), dir.clone(), key.to_owned(), thread.cloned());
    let (root, repo) = (root.clone(), repo.clone());
    let mut files = std::mem::take(&mut local.files);
    let base_id = local.base.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<_> {
      let project =
        |d: &Path| place::Checkout::of(d).map(|co| place::project_of(&co, &projects).to_string());
      let (flat, new) = scan(&dir, &mut files, &st.blobs, &project, lim.file)?;
      let snaps = snapshots(&st.index.lock().expect("index lock"), &repo)?;
      let base = base_id.and_then(|b| snaps.iter().find(|s| s.id == b).cloned());
      let base_flat = match &base {
        Some(b) => load_tree(&st.blobs, &b.tree).unwrap_or_default(),
        None => Flat::new(),
      };
      let mut pass = Pass::default();
      let mine = carried(&flat, &base_flat);
      let tip = if base.is_none() && mine.is_empty() {
        None
      } else if base.is_some() && mine == base_flat {
        base.clone()
      } else {
        let (tree, mut trees) = store_tree(&st.blobs, &mine)?;
        // Blobs of files it does not carry are not its to attach.
        let mut new: Vec<_> = new
          .into_iter()
          .filter(|(_, _, p)| mine.contains_key(p))
          .collect();
        new.append(&mut trees);
        let prev: Vec<RecordId> = base.iter().map(|b| b.id.clone()).collect();
        let what = changed(&base_flat, &mine);
        let (id, at) = publish(
          &root,
          &st.index,
          &st.node,
          &key,
          tree,
          &prev,
          thread.as_ref(),
          &what,
          &new,
        )?;
        pass.published = true;
        Some(Snap {
          id,
          at,
          node: st.node.to_string(),
          tree,
          prev,
          thread: thread.map(|t| t.to_string()),
          changed: what,
        })
      };
      Ok((flat, files, base, tip, pass))
    })
    .await
    .map_err(|e| Error::Config(e.to_string()))??;
    local.files = r.1;
    (r.0, r.2, r.3, r.4)
  };
  let _ = base;

  // What other nodes published: the trees and files a merge needs.
  let snaps = snapshots(&state.index.lock().expect("index lock"), &repo)?;
  let tip_tree = tip.as_ref().map(|t| t.tree);
  let others: Vec<Snap> = heads(&snaps)
    .into_iter()
    .filter(|h| Some(&h.id) != tip.as_ref().map(|t| &t.id) && Some(h.tree) != tip_tree)
    .cloned()
    .collect();
  if others.is_empty() {
    local.base = tip.map(|t| t.id);
    local.save(&s.data, &repo)?;
    return Ok(pass);
  }
  let by: HashMap<&RecordId, &Snap> = snaps.iter().map(|s| (&s.id, s)).collect();
  let mut wanted: Vec<BlobHash> = Vec::new();
  let mut trees: Vec<BlobHash> = others.iter().map(|o| o.tree).collect();
  for o in &others {
    if let Some(t) = &tip
      && let Some(c) = common(&by, &t.id, &o.id)
    {
      trees.push(c.tree);
    }
  }
  // Trees first, level by level; then the files they name.
  for _ in 0..64 {
    let missing: Vec<BlobHash> = trees
      .iter()
      .filter_map(|t| load_tree(&state.blobs, t).err())
      .flatten()
      .collect();
    if missing.is_empty() {
      break;
    }
    if crate::node::fetch_blobs(s, &missing).await == 0 {
      return Ok(Pass {
        skipped: Some(
          "the other nodes' changes are not here yet (no node that has them could be reached)"
            .into(),
        ),
        ..pass
      });
    }
  }
  for t in &trees {
    if let Ok(flat) = load_tree(&state.blobs, t) {
      for l in flat.values() {
        if let Leaf::File { hash, .. } = l
          && !state.blobs.contains(hash)
        {
          wanted.push(*hash);
        }
      }
    }
  }
  wanted.sort_by_key(|h| h.hex());
  wanted.dedup();
  if !wanted.is_empty() && crate::node::fetch_blobs(s, &wanted).await < wanted.len() {
    return Ok(Pass {
      skipped: Some("some files of the other nodes' changes could not be fetched yet".into()),
      ..pass
    });
  }

  // Merged one head at a time, then written, then published.
  let st = state.clone();
  let key2 = key.to_owned();
  let thread = thread.cloned();
  let r = tokio::task::spawn_blocking(move || -> Result<_> {
    let snaps = snapshots(&st.index.lock().expect("index lock"), &repo)?;
    let by: HashMap<&RecordId, &Snap> = snaps.iter().map(|s| (&s.id, s)).collect();
    let load = |t: &BlobHash| load_tree(&st.blobs, t).map_err(Error::MissingBlobs);
    let tip_flat = match &tip {
      Some(t) => load(&t.tree)?,
      None => Flat::new(),
    };
    let mut merged = carried(&scanned, &tip_flat);
    let mut copies = Vec::new();
    // This side's change dated as its snapshot was, not now: both nodes
    // merging the same two changes at once then pick the same later one.
    let mut at = tip.as_ref().map_or_else(Timestamp::now, |t| t.at);
    for o in &others {
      let base = match tip.as_ref().and_then(|t| common(&by, &t.id, &o.id)) {
        Some(c) => load(&c.tree)?,
        None => Flat::new(),
      };
      let theirs = load(&o.tree)?;
      let (m, mut c) = merge(
        &st.blobs,
        &base,
        &Side {
          flat: &merged,
          at,
          node: st.node.as_str(),
        },
        &Side {
          flat: &theirs,
          at: o.at,
          node: &o.node,
        },
        &scanned,
      )?;
      merged = m;
      copies.append(&mut c);
      at = at.max(o.at);
    }
    let mut files = local.files;
    let applied = apply(&dir, &scanned, &merged, &mut files, &st.blobs)?;
    local.files = files;
    if !applied.spared.is_empty() {
      // A file was edited while merging: the merge is not this node's yet.
      // Its base stays its own tip, so the next pass publishes the edit
      // from there and merges it with the others' (a copy if they meet).
      local.base = tip.as_ref().map(|t| t.id.clone());
      return Ok((local, applied, Vec::new(), false));
    }
    // One head again. Another node's change that follows this node's tip
    // is taken as it is (a fast-forward); anything else is a merge.
    let (tree, new) = store_tree(&st.blobs, &merged)?;
    let follows = |o: &Snap| match &tip {
      None => true,
      Some(t) => ancestors(&by, &o.id).contains(&t.id),
    };
    let forward = match others.as_slice() {
      [o] if o.tree == tree && follows(o) => Some(o.id.clone()),
      _ => None,
    };
    let published = forward.is_none();
    let base = match forward {
      Some(id) => id,
      None => {
        let mut prev: Vec<RecordId> = tip.iter().map(|t| t.id.clone()).collect();
        prev.extend(others.iter().map(|o| o.id.clone()));
        publish(
          &root,
          &st.index,
          &st.node,
          &key2,
          tree,
          &prev,
          thread.as_ref(),
          &changed(&tip_flat, &merged),
          &new,
        )?
        .0
      }
    };
    local.base = Some(base);
    Ok((local, applied, copies, published))
  })
  .await
  .map_err(|e| Error::Config(e.to_string()))??;
  let (local, applied, copies, published) = r;
  local.save(&s.data, &store_id(key))?;
  pass.applied = applied.changed;
  pass.conflicts = copies;
  pass.published |= published;
  pass.spared = applied.spared;
  Ok(pass)
}

/// A pass, and what it published sent to the other nodes at once.
pub(crate) async fn pass_and_push(
  s: &Arc<Shared>,
  key: &str,
  thread: Option<&RecordId>,
) -> Result<Pass> {
  let new = workdir_of(key).is_some() && s.places().folders.sync(key).is_none();
  let p = pass(s, key, thread).await?;
  if p.published {
    // A working folder that just registered: the others learn of it
    // first, so they hold its store when it comes.
    if new
      && s.places().folders.sync(key).is_some()
      && let Some(home) = s.state().home()
    {
      crate::node::sync_one(s, home.repo()).await;
    }
    crate::node::sync_one(s, &store_id(key)).await;
  }
  Ok(p)
}

/// A pass over every folder here; what each did, where it did something.
/// A folder whose store this node just began to hold (a working folder
/// another node registered) is synced first, from the node that has it.
pub(crate) async fn pass_all(s: &Arc<Shared>) -> Vec<(String, Pass)> {
  for key in hold_all(s) {
    crate::node::sync_one(s, &store_id(&key)).await;
  }
  let mut out = Vec::new();
  for key in keys(s) {
    match pass_and_push(s, &key, None).await {
      Ok(p) if p != Pass::default() && p.skipped.as_deref() != Some("not on this node") => {
        out.push((key, p))
      }
      Ok(_) => {}
      Err(e) => eprintln!("folder {key}: {e}"),
    }
  }
  out
}

/// A turn on a folder: a pass before it, none while it runs, and a pass
/// when it ends with what it changed, by its thread.
pub(crate) struct Turn {
  s: Arc<Shared>,
  key: String,
  thread: Option<RecordId>,
}

pub(crate) async fn turn(
  s: &Arc<Shared>,
  place: &Place,
  thread: Option<&RecordId>,
) -> Option<Turn> {
  let workdir;
  let key = match place {
    Place::Folder(key) => key,
    // A global thread's working folder syncs as a folder does, made here
    // when another node synced it, so the turn finds what was written
    // there elsewhere (design §2.3).
    Place::Global => {
      let t = thread?;
      workdir = workdir_key(&crate::fork::origin(s, t));
      if s.places().folders.sync(&workdir) == Some(true) {
        let dir = s
          .data
          .join(crate::node::THREAD_FOLDERS)
          .join(workdir_of(&workdir)?);
        if let Err(e) = std::fs::create_dir_all(&dir) {
          eprintln!("thread {t}: no working folder {}: {e}", dir.display());
        }
        // Held here first (a node that synced since holds it already):
        // what the others published, fetched before the pass.
        if let Ok((_, true)) = hold_new(s, &workdir) {
          crate::node::sync_one(s, &store_id(&workdir)).await;
        }
      }
      &workdir
    }
    Place::Project(_) => {
      // A project's files, on a phone, are its clone: fetched first.
      crate::clones::before_turn(s, place, thread).await;
      return None;
    }
  };
  match pass(s, key, None).await {
    Ok(Pass {
      ask: Some(size), ..
    }) => {
      if let Some(t) = thread {
        ask_to_sync(s, key, t, size);
      }
    }
    Ok(_) => {}
    Err(e) => eprintln!("folder {key}: {e}"),
  }
  *s.folder_sync
    .turns
    .lock()
    .expect("folder turns")
    .entry(key.clone())
    .or_default() += 1;
  Some(Turn {
    s: s.clone(),
    key: key.clone(),
    thread: thread.cloned(),
  })
}

impl Drop for Turn {
  fn drop(&mut self) {
    if let Some(n) = self
      .s
      .folder_sync
      .turns
      .lock()
      .expect("folder turns")
      .get_mut(&self.key)
    {
      *n = n.saturating_sub(1);
    }
    let (s, key, thread) = (self.s.clone(), self.key.clone(), self.thread.clone());
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
      return;
    };
    rt.spawn(async move {
      if let Err(e) = pass_and_push(&s, &key, thread.as_ref()).await {
        eprintln!("folder {key}: {e}");
      }
    });
  }
}

// ------------------------------------------------------------- the history

/// A change, for `stitch folder log` and the app.
#[derive(Clone, Debug)]
pub struct Change {
  pub id: RecordId,
  pub at: Timestamp,
  pub node: String,
  pub thread: Option<String>,
  pub changed: Vec<String>,
  /// It joined several (a merge).
  pub merge: bool,
}

/// A folder's changes, the latest first; only those touching `path` when
/// given.
pub fn log(index: &Index, key: &str, path: Option<&str>) -> Result<Vec<Change>> {
  let mut out: Vec<Change> = snapshots(index, &store_id(key))?
    .into_iter()
    .filter(|s| {
      path.is_none_or(|p| {
        s.changed
          .iter()
          .any(|c| c == p || c.starts_with(&format!("{p}/")))
      })
    })
    .map(|s| Change {
      merge: s.prev.len() > 1,
      id: s.id,
      at: s.at,
      node: s.node,
      thread: s.thread,
      changed: s.changed,
    })
    .collect();
  out.reverse();
  Ok(out)
}

/// A file's text in a snapshot.
pub fn text_at(
  index: &Index,
  blobs: &BlobStore,
  key: &str,
  snapshot: &RecordId,
  path: &str,
) -> Option<String> {
  let snaps = snapshots(index, &store_id(key)).ok()?;
  let s = snaps.iter().find(|s| &s.id == snapshot)?;
  let flat = load_tree(blobs, &s.tree).ok()?;
  let Some(Leaf::File { hash, .. }) = flat.get(path) else {
    return None;
  };
  std::fs::read(blobs.path(hash))
    .ok()
    .and_then(|b| String::from_utf8(b).ok())
}

/// What a change did, as a unified diff: `to` (the latest by default)
/// against `from` (by default what it follows), only `path` when given.
pub fn diff(
  index: &Index,
  blobs: &BlobStore,
  key: &str,
  from: Option<&RecordId>,
  to: Option<&RecordId>,
  path: Option<&str>,
) -> Result<String> {
  let snaps = snapshots(index, &store_id(key))?;
  let to = match to {
    Some(t) => snaps.iter().find(|s| &s.id == t),
    None => snaps
      .iter()
      .max_by(|a, b| (a.at, &a.id).cmp(&(b.at, &b.id))),
  }
  .ok_or_else(|| Error::Config("no such change of the folder".into()))?;
  let from = match from.or(to.prev.first()) {
    Some(f) => Some(
      snaps
        .iter()
        .find(|s| &s.id == f)
        .ok_or_else(|| Error::Config(format!("no change {f} of the folder")))?,
    ),
    None => None,
  };
  let load = |s: Option<&Snap>| -> Result<Flat> {
    match s {
      Some(s) => load_tree(blobs, &s.tree).map_err(Error::MissingBlobs),
      None => Ok(Flat::new()),
    }
  };
  let (a, b) = (load(from)?, load(Some(to))?);
  // A file's text; `None` for a binary (not UTF-8, or with a NUL).
  let text = |l: Option<&Leaf>| -> Option<String> {
    match l {
      Some(Leaf::File { hash, .. }) => std::fs::read(blobs.path(hash))
        .ok()
        .filter(|b| !b.contains(&0))
        .and_then(|b| String::from_utf8(b).ok()),
      _ => Some(String::new()),
    }
  };
  let mut out = String::new();
  for p in changed(&a, &b) {
    if path.is_some_and(|w| p != w && !p.starts_with(&format!("{w}/"))) {
      continue;
    }
    let old = if a.contains_key(&p) {
      format!("a/{p}")
    } else {
      "/dev/null".into()
    };
    let new = if b.contains_key(&p) {
      format!("b/{p}")
    } else {
      "/dev/null".into()
    };
    let (Some(x), Some(y)) = (text(a.get(&p)), text(b.get(&p))) else {
      out.push_str(&format!("Binary files {old} and {new} differ\n"));
      continue;
    };
    out.push_str(
      &similar::TextDiff::from_lines(&x, &y)
        .unified_diff()
        .header(&old, &new)
        .to_string(),
    );
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn text(blobs: &BlobStore, s: &str) -> Leaf {
    Leaf::File {
      hash: blobs.put_bytes(s.as_bytes()).unwrap(),
      size: s.len() as u64,
    }
  }

  fn read(blobs: &BlobStore, l: Option<&Leaf>) -> String {
    let Some(Leaf::File { hash, .. }) = l else {
      panic!("not a file: {l:?}")
    };
    String::from_utf8(std::fs::read(blobs.path(hash)).unwrap()).unwrap()
  }

  #[test]
  fn trees_round_trip_and_share_what_did_not_change() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let mut flat = Flat::new();
    flat.insert("AGENTS.md".into(), text(&blobs, "be kind\n"));
    flat.insert("docs/a.md".into(), text(&blobs, "a\n"));
    flat.insert("docs/deep/b.md".into(), text(&blobs, "b\n"));
    flat.insert("photo.png".into(), Leaf::Skip);
    flat.insert(
      "tool".into(),
      Leaf::Project {
        repo: "p-1234".into(),
      },
    );
    let (root, new) = store_tree(&blobs, &flat).unwrap();
    assert_eq!(new.len(), 3, "three directories");
    assert_eq!(load_tree(&blobs, &root).unwrap(), flat);
    flat.insert("AGENTS.md".into(), text(&blobs, "be very kind\n"));
    let (_, new) = store_tree(&blobs, &flat).unwrap();
    assert_eq!(new.len(), 1, "only the top changed: {new:?}");
  }

  #[test]
  fn edits_on_both_sides_merge_or_keep_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let base_text = "one\ntwo\nthree\nfour\nfive\n";
    let mut base = Flat::new();
    base.insert("notes.md".into(), text(&blobs, base_text));
    base.insert("gone.md".into(), text(&blobs, "x\n"));
    base.insert("kept.md".into(), text(&blobs, "y\n"));
    let mut ours = base.clone();
    ours.insert(
      "notes.md".into(),
      text(&blobs, "ONE\ntwo\nthree\nfour\nfive\n"),
    );
    ours.remove("gone.md");
    ours.insert("kept.md".into(), text(&blobs, "y, edited\n"));
    ours.insert("big.bin".into(), Leaf::Skip);
    let mut theirs = base.clone();
    theirs.insert(
      "notes.md".into(),
      text(&blobs, "one\ntwo\nthree\nfour\nFIVE\n"),
    );
    theirs.remove("kept.md");
    theirs.insert("big.bin".into(), text(&blobs, "theirs\n"));
    let t0: Timestamp = "2026-10-14T10:00:00Z".parse().unwrap();
    let t1: Timestamp = "2026-10-14T11:00:00Z".parse().unwrap();
    let here = ours.clone();
    let (m, copies) = merge(
      &blobs,
      &base,
      &Side {
        flat: &ours,
        at: t0,
        node: "mac",
      },
      &Side {
        flat: &theirs,
        at: t1,
        node: "phone",
      },
      &here,
    )
    .unwrap();
    assert_eq!(
      read(&blobs, m.get("notes.md")),
      "ONE\ntwo\nthree\nfour\nFIVE\n",
      "merged three ways"
    );
    assert!(!m.contains_key("gone.md"), "deleted here");
    assert_eq!(
      read(&blobs, m.get("kept.md")),
      "y, edited\n",
      "an edit wins over a deletion"
    );
    assert_eq!(
      m.get("big.bin"),
      theirs.get("big.bin"),
      "not carried here: theirs stands"
    );
    assert!(copies.is_empty());

    // The same line on both: the later stands, the other is kept.
    let mut ours = base.clone();
    ours.insert(
      "notes.md".into(),
      text(&blobs, "one\nTWO (mac)\nthree\nfour\nfive\n"),
    );
    let mut theirs = base.clone();
    theirs.insert(
      "notes.md".into(),
      text(&blobs, "one\nTWO (phone)\nthree\nfour\nfive\n"),
    );
    let (m, copies) = merge(
      &blobs,
      &base,
      &Side {
        flat: &ours,
        at: t0,
        node: "mac",
      },
      &Side {
        flat: &theirs,
        at: t1,
        node: "phone",
      },
      &ours,
    )
    .unwrap();
    assert_eq!(copies, vec!["notes (mac, 2026-10-14).md".to_owned()]);
    assert!(read(&blobs, m.get("notes.md")).contains("(phone)"));
    assert!(read(&blobs, m.get(&copies[0])).contains("(mac)"));
  }

  #[test]
  fn scans_carry_files_and_leave_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
    let f = dir.path().join("folder");
    std::fs::create_dir_all(f.join("docs")).unwrap();
    std::fs::create_dir_all(f.join("node_modules/x")).unwrap();
    std::fs::create_dir_all(f.join("tool/.git")).unwrap();
    std::fs::create_dir_all(f.join(".stitch")).unwrap();
    std::fs::write(f.join("AGENTS.md"), "hi\n").unwrap();
    std::fs::write(f.join("docs/a.md"), "a\n").unwrap();
    std::fs::write(f.join("image.png"), [0u8, 1, 2]).unwrap();
    std::fs::write(f.join("big.pdf"), [7u8; 40]).unwrap();
    std::fs::write(f.join("node_modules/x/i.js"), "x").unwrap();
    std::fs::write(f.join("tool/main.rs"), "fn main() {}").unwrap();
    std::fs::write(f.join("scratch.log"), "noise").unwrap();
    std::fs::write(f.join(".stitch/ignore"), "# logs\n*.log\n").unwrap();
    let mut files = BTreeMap::new();
    let (flat, new) = scan(&f, &mut files, &blobs, &|_| Some("p-tool".into()), 32).unwrap();
    let paths: Vec<&str> = flat.keys().map(String::as_str).collect();
    assert_eq!(
      paths,
      [
        ".stitch/ignore",
        "AGENTS.md",
        "big.pdf",
        "docs/a.md",
        "image.png",
        "tool"
      ]
    );
    // A binary travels; a file over the cap stays.
    assert!(matches!(flat["image.png"], Leaf::File { size: 3, .. }));
    assert_eq!(flat["big.pdf"], Leaf::Skip);
    assert_eq!(
      flat["tool"],
      Leaf::Project {
        repo: "p-tool".into()
      }
    );
    assert_eq!(new.len(), 4);
    // Nothing read again when nothing changed.
    let (again, new) = scan(&f, &mut files, &blobs, &|_| None, 32).unwrap();
    assert!(new.is_empty());
    assert_eq!(again.len(), flat.len());

    // Measured by the same rules, from sizes alone.
    let lim = |files, bytes| Limits {
      files,
      bytes,
      file: 32,
    };
    let size = measure(&f, lim(100, 1_000_000));
    assert_eq!((size.files, size.over), (4, false));
    assert_eq!(size.bytes, 13 + 3 + 2 + 3);
    assert_eq!(size.to_string(), "4 file(s), 1 KB");
    let over = measure(&f, lim(2, 1_000_000));
    assert_eq!((over.files, over.over), (3, true));
    assert!(measure(&f, lim(100, 5)).over);
  }

  #[test]
  fn applying_writes_what_changed_and_spares_fresh_edits() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
    let f = dir.path().join("folder");
    std::fs::create_dir_all(&f).unwrap();
    std::fs::write(f.join("a.md"), "a\n").unwrap();
    std::fs::write(f.join("b.md"), "b\n").unwrap();
    let mut files = BTreeMap::new();
    let (from, _) = scan(&f, &mut files, &blobs, &|_| None, MAX_FILE_MB << 20).unwrap();
    let mut to = from.clone();
    to.insert("a.md".into(), text(&blobs, "a, theirs\n"));
    to.insert("new/c.md".into(), text(&blobs, "c\n"));
    to.remove("b.md");
    let n = apply(&f, &from, &to, &mut files, &blobs).unwrap();
    assert_eq!(n.changed, 3);
    assert!(n.spared.is_empty());
    assert_eq!(
      std::fs::read_to_string(f.join("a.md")).unwrap(),
      "a, theirs\n"
    );
    assert_eq!(std::fs::read_to_string(f.join("new/c.md")).unwrap(), "c\n");
    assert!(!f.join("b.md").exists());
    // Edited since the scan, with the same size and no pause (a coarse
    // clock gives it the same time): it spares the whole merge, nothing
    // else is written, and the next pass takes the edit up.
    let (from, _) = scan(&f, &mut files, &blobs, &|_| None, MAX_FILE_MB << 20).unwrap();
    std::fs::write(f.join("a.md"), "A, THEIRS\n").unwrap();
    let mut to = from.clone();
    to.insert("a.md".into(), text(&blobs, "a, theirs again\n"));
    to.insert("new/d.md".into(), text(&blobs, "d\n"));
    assert_eq!(
      apply(&f, &from, &to, &mut files, &blobs).unwrap(),
      Applied {
        changed: 0,
        spared: vec!["a.md".into()]
      }
    );
    assert_eq!(
      std::fs::read_to_string(f.join("a.md")).unwrap(),
      "A, THEIRS\n"
    );
    assert!(!f.join("new/d.md").exists(), "nothing of the merge written");
  }

  /// A file whose time is that close to when it was seen is read again:
  /// an edit after it may have the same time and size.
  #[test]
  fn racy_times_are_not_trusted() {
    let hash = BlobHash::of(b"x");
    let at = now_ns();
    let fresh = Seen {
      size: 1,
      mtime: at - 1_000,
      hash,
      at,
    };
    assert!(!fresh.still(1, at - 1_000), "seen as it was written");
    let old = Seen {
      mtime: at - 3 * RACY_NS,
      ..fresh.clone()
    };
    assert!(old.still(1, at - 3 * RACY_NS));
    assert!(!old.still(2, at - 3 * RACY_NS), "another size");
    let unknown = Seen { at: 0, ..old };
    assert!(
      !unknown.still(1, at - 3 * RACY_NS),
      "from before `at` was kept"
    );
  }

  /// Two nodes merging the same two changes at once decide alike: the
  /// later stands, the other is the copy, on both (CI found each keeping
  /// its own when "ours" was dated now).
  #[test]
  fn both_sides_of_a_conflict_decide_alike() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let mut base = Flat::new();
    base.insert("AGENTS.md".into(), text(&blobs, "Be brief.\n"));
    let mut mac = Flat::new();
    mac.insert("AGENTS.md".into(), text(&blobs, "Be brief (mac).\n"));
    let mut linux = Flat::new();
    linux.insert("AGENTS.md".into(), text(&blobs, "Be brief (linux).\n"));
    for (t_mac, t_linux) in [
      ("2026-10-14T10:00:00Z", "2026-10-14T10:00:01Z"),
      ("2026-10-14T10:00:01Z", "2026-10-14T10:00:00Z"),
      ("2026-10-14T10:00:00Z", "2026-10-14T10:00:00Z"),
    ] {
      let (tm, tl): (Timestamp, Timestamp) = (t_mac.parse().unwrap(), t_linux.parse().unwrap());
      let m = Side {
        flat: &mac,
        at: tm,
        node: "mac",
      };
      let l = Side {
        flat: &linux,
        at: tl,
        node: "linux",
      };
      let on_mac = merge(&blobs, &base, &m, &l, &mac).unwrap();
      let on_linux = merge(&blobs, &base, &l, &m, &linux).unwrap();
      assert_eq!(on_mac, on_linux, "{t_mac} / {t_linux}");
      assert_eq!(on_mac.1.len(), 1);
    }
  }

  #[test]
  fn binaries_changed_on_both_sides_keep_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let bin = |b: &[u8]| Leaf::File {
      hash: blobs.put_bytes(b).unwrap(),
      size: b.len() as u64,
    };
    // Lines that would merge as text, but with a NUL: a binary.
    let mut base = Flat::new();
    base.insert("plan.pdf".into(), bin(b"%PDF\0\na\nb\nc\nd\ne\n"));
    let mut ours = Flat::new();
    ours.insert("plan.pdf".into(), bin(b"%PDF\0\nA\nb\nc\nd\ne\n"));
    let mut theirs = Flat::new();
    let t = bin(b"%PDF\0\na\nb\nc\nd\nE\n");
    theirs.insert("plan.pdf".into(), t.clone());
    let o = Side {
      flat: &ours,
      at: "2026-10-22T10:00:00Z".parse().unwrap(),
      node: "mac",
    };
    let th = Side {
      flat: &theirs,
      at: "2026-10-22T10:00:01Z".parse().unwrap(),
      node: "gamer",
    };
    let (out, copies) = merge(&blobs, &base, &o, &th, &ours).unwrap();
    assert_eq!(out["plan.pdf"], t);
    assert_eq!(copies.len(), 1);
    assert_eq!(out[&copies[0]], ours["plan.pdf"]);
  }

  #[test]
  fn what_a_node_does_not_carry_mirrors_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let mut here = Flat::new();
    here.insert("a.md".into(), text(&blobs, "a\n"));
    here.insert("big.pdf".into(), Leaf::Skip);
    here.insert("tool".into(), Leaf::Project { repo: "p-1".into() });
    let mut shared = Flat::new();
    shared.insert("big.pdf".into(), text(&blobs, "small elsewhere\n"));
    shared.insert("tool/main.rs".into(), text(&blobs, "fn main() {}\n"));
    shared.insert("gone.md".into(), text(&blobs, "x\n"));
    let mine = carried(&here, &shared);
    let paths: Vec<&str> = mine.keys().map(String::as_str).collect();
    assert_eq!(paths, ["a.md", "big.pdf", "tool/main.rs"]);
    assert!(covered(&here, "tool/main.rs") && !covered(&here, "toolbox.md"));
  }

  #[test]
  fn names_from_elsewhere_are_checked() {
    for bad in ["", ".", "..", ".git", "a/b", "a\\b"] {
      assert!(bad_name(bad), "{bad:?}");
    }
    assert!(!bad_name("notes.md"));
    assert!(glob("*.log", "x.log"));
    assert!(!glob("*.log", "dir/x.log"));
    assert!(glob("build/*", "build/x"));
    let ignore = Ignore {
      patterns: vec!["*.swp".into(), "out/*".into()],
    };
    assert!(ignore.ignores("a/b/.x.swp"));
    assert!(ignore.ignores("out/x"));
    assert!(!ignore.ignores("src/out"));
  }

  #[test]
  fn conflict_copies_are_numbered_when_taken() {
    let at: Timestamp = "2026-10-14T10:00:00Z".parse().unwrap();
    assert_eq!(
      conflict_name("a/AGENTS.md", "mac", at, |_| false),
      "a/AGENTS (mac, 2026-10-14).md"
    );
    assert_eq!(
      conflict_name("Makefile", "mac", at, |c| c == "Makefile (mac, 2026-10-14)"),
      "Makefile (mac, 2026-10-14) 2"
    );
  }
}
