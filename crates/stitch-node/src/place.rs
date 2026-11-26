//! Where a thread belongs (design §2.3): a project, a folder on one node, or
//! nowhere in particular (global).
//!
//! A git checkout is a project, known by the root commit of its history, so
//! every clone on every node finds it with nothing to set up; a fork shares
//! that commit and so its project, unless it was split off (`stitch project
//! split`), which ties its remote to a project of its own. A project's
//! records live in a record store of their own in the node's data
//! directory, never in the checkout, and every node holds every project's.
//! Outside git, a thread is tied to its folder on its node; in the home
//! directory, or when asked, it is global.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use stitch_records::{RepoId, kinds};
use stitch_store::{Index, IndexedRecord};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
  /// A project's record store.
  Project(RepoId),
  /// A folder, by identity (see `Folders`): its `~` path, or `node:path`.
  Folder(String),
  Global,
}

/// A git checkout as a project sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
  /// The top of the working tree.
  pub top: PathBuf,
  /// The first commit of its history (the oldest, when there are several).
  pub root: String,
  /// Its `origin` remote, normalized (`github.com/org/repo`).
  pub remote: Option<String>,
}

impl Checkout {
  /// The checkout holding `dir`; `None` outside git, or before the first
  /// commit.
  pub fn of(dir: &Path) -> Option<Checkout> {
    work_tree(dir)?;
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let root = git(dir, &["rev-list", "--max-parents=0", "HEAD"])?
      .lines()
      .last()?
      .to_owned();
    let remote = git(dir, &["config", "--get", "remote.origin.url"]).map(|u| normalize_remote(&u));
    Some(Checkout {
      top: std::fs::canonicalize(&top).unwrap_or_else(|_| top.into()),
      root,
      remote,
    })
  }

  /// A short name: the remote's last part, else the folder's.
  pub fn name(&self) -> String {
    self
      .remote
      .as_deref()
      .and_then(|r| r.rsplit('/').next())
      .filter(|n| !n.is_empty())
      .map(str::to_owned)
      .or_else(|| {
        self
          .top
          .file_name()
          .map(|n| n.to_string_lossy().into_owned())
      })
      .unwrap_or_else(|| "project".into())
  }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .ok()?;
  let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
  (out.status.success() && !text.is_empty()).then_some(text)
}

/// `git@github.com:Org/Repo.git`, `https://user@github.com/Org/Repo` and
/// `ssh://git@github.com:22/Org/Repo.git` all become
/// `github.com/Org/Repo`; a local path stays as it is.
pub fn normalize_remote(url: &str) -> String {
  let url = url.trim().trim_end_matches('/');
  let url = url.strip_suffix(".git").unwrap_or(url);
  let (host, path) = if let Some((_, rest)) = url.split_once("://") {
    let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
    match rest.split_once('/') {
      Some((host, path)) => (host.split(':').next().unwrap_or(host), path),
      None => return rest.to_owned(),
    }
  } else if !url.starts_with('/')
    && let Some((user_host, path)) = url.split_once(':')
  {
    (
      user_host.rsplit_once('@').map_or(user_host, |(_, h)| h),
      path,
    )
  } else {
    return url.to_owned();
  };
  format!(
    "{}/{}",
    host.to_ascii_lowercase(),
    path.trim_start_matches('/')
  )
}

/// A project the home workspace lists (a `project` record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
  /// Its record store.
  pub repo: RepoId,
  pub root: String,
  pub remote: Option<String>,
  pub name: String,
  /// Split off from the project of its root commit (a fork of its own).
  pub split: bool,
  /// The first commits its repository was re-created from since (the same
  /// origin, a new history: `reroot` records), oldest first: checkouts of
  /// them are this project's too.
  pub reroots: Vec<String>,
  /// When it was first listed (its earliest `project` record with a
  /// `root`): re-rooting only goes forward, from an older project to a
  /// newer root (`recreates`).
  pub created: jiff::Timestamp,
}

/// Every project the home workspace lists, one per store.
pub fn projects(index: &Index) -> Vec<Project> {
  let mut out: BTreeMap<RepoId, Project> = BTreeMap::new();
  let records = index.records(Some(kinds::PROJECT)).unwrap_or_default();
  // A project re-created from a new first commit (`reroot`, with no
  // `root`: builds before it skip it), once its project is known.
  let mut reroots: Vec<(RepoId, String)> = Vec::new();
  for r in &records {
    let s = |k: &str| r.fields.get(k).and_then(Value::as_str).map(str::to_owned);
    let repo = s("repo").and_then(|r| r.parse::<RepoId>().ok());
    if let (Some(repo), None, Some(new)) = (&repo, s("root"), s("reroot")) {
      reroots.push((repo.clone(), new));
      continue;
    }
    let (Some(repo), Some(root)) = (repo, s("root")) else {
      continue;
    };
    // Two nodes may list the same project at once: the first one counts.
    out.entry(repo).or_insert_with_key(|repo| Project {
      repo: repo.clone(),
      name: s("name").unwrap_or_else(|| repo.to_string()),
      root,
      remote: s("remote"),
      split: r.fields.get("split").and_then(Value::as_bool) == Some(true),
      reroots: Vec::new(),
      created: r.created,
    });
  }
  for (repo, new) in reroots {
    if let Some(p) = out.get_mut(&repo)
      && p.root != new
      && !p.reroots.contains(&new)
    {
      p.reroots.push(new);
    }
  }
  out.into_values().collect()
}

/// The store of a root commit's project, or of a fork split off with
/// `remote`: derived from them, so nodes that list it at once agree.
pub fn project_repo(root: &str, split_remote: Option<&str>) -> RepoId {
  let head: String = root.chars().take(16).collect();
  let id = match split_remote {
    None => format!("p-{head}"),
    Some(remote) => format!("p-{head}-{:08x}", fnv1a(remote.as_bytes()) as u32),
  };
  id.to_ascii_lowercase()
    .parse()
    .expect("hex digits and dashes make a repository ID")
}

fn fnv1a(bytes: &[u8]) -> u64 {
  bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
    (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
  })
}

/// The project of a checkout: the fork split off with its remote, else the
/// project whose repository was re-created from its root commit (the
/// earliest listed), else the project of its root commit.
pub fn project_of(co: &Checkout, projects: &[Project]) -> RepoId {
  co.remote
    .as_deref()
    .and_then(|remote| {
      projects
        .iter()
        .find(|p| p.split && p.root == co.root && p.remote.as_deref() == Some(remote))
    })
    .or_else(|| {
      // A cycle (two nodes re-rooting each way at once) counts for nothing:
      // a reroot of P to R is ignored when R's project re-rooted to P's root.
      let of_root = projects.iter().find(|q| !q.split && q.root == co.root);
      projects.iter().find(|p| {
        !p.split
          && p.reroots.contains(&co.root)
          && !of_root.is_some_and(|q| q.repo != p.repo && q.reroots.contains(&p.root))
      })
    })
    .map_or_else(|| project_repo(&co.root, None), |p| p.repo.clone())
}

/// Whether `co` is a checkout of project `repo`: its project, or a checkout
/// of the project's own first commit and origin (which a re-creation of
/// another project's repository may since have taken for that project).
pub fn holds(co: &Checkout, repo: &RepoId, projects: &[Project]) -> bool {
  &project_of(co, projects) == repo
    || projects
      .iter()
      .any(|p| &p.repo == repo && p.root == co.root && p.remote == co.remote)
}

/// Whether `co` may be project `repo`'s repository re-created from a new
/// first commit: the same origin, a root that is not one of the project's.
/// Only a hint: the project follows it there (`reroot`) when the user says
/// so (`stitch project rebind`), never by itself. Not for a fork split off,
/// and only
/// forward: not when the root is that of another project listed before it
/// (a checkout of the old repository, not yet reset, seen by a thread of
/// the project listed for its new root).
pub fn recreates(co: &Checkout, repo: &RepoId, projects: &[Project]) -> bool {
  projects.iter().any(|p| {
    &p.repo == repo
      && !p.split
      && p.remote.is_some()
      && co.remote == p.remote
      && co.root != p.root
      && !p.reroots.contains(&co.root)
      && !projects
        .iter()
        .any(|q| q.repo != p.repo && q.root == co.root && q.created < p.created)
  })
}

/// What the home workspace says about places: the projects, and how
/// folders on different nodes are the same folder.
#[derive(Clone, Debug, Default)]
pub struct Places {
  pub projects: Vec<Project>,
  pub folders: Folders,
}

impl Places {
  pub fn load(index: &Index) -> Places {
    Places {
      projects: projects(index),
      folders: Folders::load(index),
    }
  }

  /// `place` as it is now: a folder folded into a project since is that
  /// project, a project unfolded into a folder that folder (as
  /// `Place::of_thread` says of their threads).
  pub fn now(&self, place: &Place) -> Place {
    match place {
      Place::Folder(key) => match self.folders.project(key) {
        Some(repo) => Place::Project(repo.clone()),
        None => place.clone(),
      },
      Place::Project(repo) => match self.folders.unfolded(repo) {
        Some(key) => Place::Folder(key.to_owned()),
        None => place.clone(),
      },
      Place::Global => Place::Global,
    }
  }

  /// The projects that are places: not those that went back to being a
  /// folder (`Folders::unfolded`), whose threads are the folder's. Their
  /// records are still held and synced.
  pub fn listed(&self) -> impl Iterator<Item = &Project> {
    self
      .projects
      .iter()
      .filter(|p| self.folders.unfolded(&p.repo).is_none())
  }
}

/// Folders by identity (`folder` records, in the order written). A folder
/// is a location, `node:path`, with the path relative to the home directory
/// where it is in it (`~/notes`). Locations with the same `~` path are one
/// folder on every node, unless one was unlinked; `link` joins two
/// locations; a folder that became a git checkout is folded into its
/// project, and its threads are listed there, until its `.git` goes away
/// (`unfold`).
#[derive(Clone, Debug, Default)]
pub struct Folders {
  /// Joined identities, each to a smaller one.
  parent: BTreeMap<String, String>,
  /// Locations that keep to their own node.
  unlinked: BTreeSet<String>,
  /// Identities folded into a project, with the location that became its
  /// checkout.
  folded: Vec<(String, RepoId, String)>,
  /// Identities unfolded from a project, with the project.
  unfolded: Vec<(String, RepoId)>,
  /// Whether the user let a folder's files sync (`sync`, `allow`): the
  /// latest answer for each identity.
  sync: BTreeMap<String, bool>,
}

impl Folders {
  pub fn load(index: &Index) -> Folders {
    let mut records = index.records(Some(kinds::FOLDER)).unwrap_or_default();
    records.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    let mut links: Vec<(String, String)> = Vec::new();
    let mut unlinked = BTreeSet::new();
    let mut folds = Vec::new();
    let mut unfolds: Vec<(String, RepoId)> = Vec::new();
    let mut sync: Vec<(String, bool)> = Vec::new();
    for r in records {
      let s = |k: &str| r.fields.get(k).and_then(Value::as_str).map(location);
      if let Some(key) = r.fields.get("sync").and_then(Value::as_str) {
        let allow = r.fields.get("allow").and_then(Value::as_bool) == Some(true);
        sync.push((key.to_owned(), allow));
      }
      if let Some(l) = s("unlink") {
        // An unlink ends the links made before it.
        links.retain(|(a, b)| a != &l && b != &l);
        unlinked.insert(l);
      }
      if let Some(pair) = r.fields.get("link").and_then(Value::as_array)
        && let (Some(a), Some(b)) = (
          pair.first().and_then(Value::as_str),
          pair.get(1).and_then(Value::as_str),
        )
      {
        links.push((location(a), location(b)));
      }
      if let (Some(l), Some(repo)) = (
        s("location"),
        r.fields
          .get("project")
          .and_then(Value::as_str)
          .and_then(|p| p.parse().ok()),
      ) {
        folds.push((l, repo));
      }
      // An unfold ends the fold of its location made before it.
      if let (Some(l), Some(repo)) = (
        s("location"),
        r.fields
          .get("unfold")
          .and_then(Value::as_str)
          .and_then(|p| p.parse().ok()),
      ) {
        folds.retain(|(f, _)| f != &l);
        unfolds.push((l, repo));
      }
    }
    let mut f = Folders {
      unlinked,
      ..Folders::default()
    };
    for (a, b) in links {
      let (a, b) = (f.find(&f.identity(&a)), f.find(&f.identity(&b)));
      if a != b {
        let (small, big) = if a < b { (a, b) } else { (b, a) };
        f.parent.insert(big, small);
      }
    }
    f.folded = folds
      .into_iter()
      .map(|(l, repo)| (f.find(&f.identity(&l)), repo, l))
      .collect();
    f.unfolded = unfolds
      .into_iter()
      .map(|(l, repo)| (f.find(&f.identity(&l)), repo))
      .collect();
    // In the order written: the latest answer counts, under the identity
    // the folder has now (a link may have joined it to another).
    for (key, allow) in sync {
      let key = f.find(&key);
      f.sync.insert(key, allow);
    }
    f
  }

  /// Whether the user let the folder's files sync: `None` when never
  /// asked or answered (design §2.16, "A new folder asks first").
  pub fn sync(&self, key: &str) -> Option<bool> {
    self.sync.get(key).copied()
  }

  /// The folders whose latest answer lets them sync.
  pub fn allowed(&self) -> impl Iterator<Item = &str> {
    self
      .sync
      .iter()
      .filter(|(_, a)| **a)
      .map(|(k, _)| k.as_str())
  }

  fn find(&self, key: &str) -> String {
    let mut k = key.to_owned();
    while let Some(p) = self.parent.get(&k) {
      k = p.clone();
    }
    k
  }

  /// A location's own identity: its `~` path, shared by every node, or
  /// itself when it is outside the home directory or unlinked.
  fn identity(&self, location: &str) -> String {
    match location.split_once(':') {
      Some((_, path)) if path.starts_with('~') && !self.unlinked.contains(location) => {
        path.to_owned()
      }
      _ => location.to_owned(),
    }
  }

  /// The folder a location on a node belongs to.
  pub fn key(&self, node: &str, path: &str) -> String {
    self.find(&self.identity(&format!("{node}:{}", home_relative_str(path))))
  }

  /// The project a folder was folded into.
  pub fn project(&self, key: &str) -> Option<&RepoId> {
    self
      .folded
      .iter()
      .find(|(k, ..)| k == key)
      .map(|(_, r, _)| r)
  }

  /// The folder a project was unfolded into, when no fold of it is left:
  /// its `.git` went away (`unfold`). Its threads are the folder's, and it
  /// is no place of its own (`Places::listed`).
  pub fn unfolded(&self, repo: &RepoId) -> Option<&str> {
    if self.folded.iter().any(|(_, r, _)| r == repo) {
      return None;
    }
    self
      .unfolded
      .iter()
      .rev()
      .find(|(_, r)| r == repo)
      .map(|(k, _)| k.as_str())
  }

  /// The folders folded into a project: each folder's identity, and the
  /// location (`node:path`) that became the checkout.
  pub fn folded_into(&self, repo: &RepoId) -> Vec<(&str, &str)> {
    self
      .folded
      .iter()
      .filter(|(_, r, _)| r == repo)
      .map(|(k, _, l)| (k.as_str(), l.as_str()))
      .collect()
  }

  /// The identities a folder joins: `~` paths and `node:path` locations.
  fn members<'a>(&'a self, key: &'a str) -> Vec<&'a str> {
    let mut members: Vec<&str> = vec![key];
    members.extend(
      self
        .parent
        .keys()
        .map(String::as_str)
        .filter(|k| self.find(k) == key),
    );
    members
  }

  /// A folder's name seen from `here`: its `~` path, else its path on
  /// `here`, else a path with its node.
  pub fn label(&self, key: &str, here: &str) -> String {
    let members = self.members(key);
    if let Some(m) = members.iter().find(|m| m.starts_with('~')) {
      return (*m).to_owned();
    }
    if let Some(path) = members.iter().find_map(|m| {
      m.split_once(':')
        .filter(|(n, _)| *n == here)
        .map(|(_, p)| p)
    }) {
      return path.to_owned();
    }
    match key.split_once(':') {
      Some((node, path)) => format!("{path} on {node}"),
      None => key.to_owned(),
    }
  }

  /// The paths a folder has on every node, home-relative (`~/notes`,
  /// `/srv/data`): what earlier turns elsewhere called it.
  pub fn paths(&self, key: &str) -> Vec<String> {
    let mut out: Vec<String> = self
      .members(key)
      .into_iter()
      .map(|m| m.split_once(':').map_or(m, |(_, p)| p))
      .map(home_relative_str)
      .collect();
    out.sort();
    out.dedup();
    out
  }

  /// The folder's directory on `node`: one of its locations there.
  pub fn dir(&self, key: &str, node: &str) -> Option<PathBuf> {
    self.members(key).into_iter().find_map(|m| {
      let path = match m.split_once(':') {
        Some((n, path)) if n == node => path,
        Some(_) => return None,
        None => m,
      };
      Some(expand_home(path)).filter(|p| p.is_dir())
    })
  }
}

/// Folds the folder of a checkout's top into the checkout's project when
/// the folder has threads (it was a folder before `git init`): writes the
/// `folder` record into the home workspace; whether it did. In a folder,
/// unfolds this node's folders whose `.git` went away (`unfold`) instead.
pub fn fold(
  index: &mut Index,
  home: &stitch_store::RecordRoot,
  node: &stitch_records::NodeName,
  place: &Place,
  co: Option<&Checkout>,
) -> Result<bool, stitch_store::Error> {
  if matches!(place, Place::Folder(_)) {
    return Ok(!unfold(index, home, node)?.is_empty());
  }
  let (Place::Project(repo), Some(co)) = (place, co) else {
    return Ok(false);
  };
  let places = Places::load(index);
  let top = co.top.display().to_string();
  let key = places.folders.key(node.as_str(), &top);
  if places.folders.project(&key).is_some() {
    return Ok(false);
  }
  let folder = Place::Folder(key);
  let has_threads = index
    .threads()?
    .iter()
    .any(|(r, _)| Place::of_thread(r, Some(home.repo()), &places) == folder);
  if !has_threads {
    return Ok(false);
  }
  let draft = stitch_records::Draft::new(kinds::FOLDER, stitch_records::Author::node(node.clone()))
    .and_then(|d| d.field("location", location_of(node.as_str(), &co.top)))
    .and_then(|d| d.field("project", repo.as_str()))
    .map_err(stitch_store::Error::from)?;
  home.publish(node, draft)?;
  index.refresh(home)?;
  Ok(true)
}

/// Ends the folds of this node's locations that are no longer checkouts:
/// still a directory, its `.git` gone (design §2.16). Each gets a `folder`
/// record (`location`, `unfold` = the project) in the home workspace, so
/// every node takes it for a folder again, its threads with it (those
/// started in the project too, their records staying in its store: see
/// `Folders::unfolded`), and syncs its files (merged with its last
/// snapshot as any change is). A location
/// that is not a directory here says nothing about its `.git`, and stays
/// folded. The folders unfolded.
pub fn unfold(
  index: &mut Index,
  home: &stitch_store::RecordRoot,
  node: &stitch_records::NodeName,
) -> Result<Vec<String>, stitch_store::Error> {
  let places = Places::load(index);
  let mut gone: Vec<(&str, &RepoId, &str)> = Vec::new();
  for (key, repo, l) in &places.folders.folded {
    let Some((n, path)) = l.split_once(':') else {
      continue;
    };
    let dir = expand_home(path);
    if n == node.as_str()
      && dir.is_dir()
      && !crate::folders::is_checkout(&dir)
      && !gone.iter().any(|(_, _, g)| g == l)
    {
      gone.push((key, repo, l));
    }
  }
  if gone.is_empty() {
    return Ok(Vec::new());
  }
  for (_, repo, l) in &gone {
    let draft =
      stitch_records::Draft::new(kinds::FOLDER, stitch_records::Author::node(node.clone()))
        .and_then(|d| d.field("location", *l))
        .and_then(|d| d.field("unfold", repo.as_str()))
        .map_err(stitch_store::Error::from)?;
    home.publish(node, draft)?;
  }
  index.refresh(home)?;
  Ok(gone.into_iter().map(|(k, ..)| k.to_owned()).collect())
}

/// A place's paths on other nodes, each with its directory here: on a
/// phone, a folder's paths elsewhere (`~/notes`, `/Users/<name>/notes`)
/// name its copy, so paths from turns on other nodes still work.
#[derive(Clone, Debug, Default)]
pub struct Aliases(Vec<(String, PathBuf)>);

impl Aliases {
  /// `paths` (home-relative, as `Folders::paths` gives them) naming `dir`.
  pub fn new(paths: Vec<String>, dir: &Path) -> Aliases {
    Aliases(
      paths
        .into_iter()
        .filter(|p| p != "~" && p != "/")
        .map(|p| (p, dir.to_owned()))
        .collect(),
    )
  }

  /// The directory here a path names, when it is one of these places' or
  /// inside one.
  pub fn map(&self, path: &str) -> Option<PathBuf> {
    let path = home_relative_str(path.trim());
    self.0.iter().find_map(|(at, dir)| {
      if path == *at {
        return Some(dir.clone());
      }
      let rest = path.strip_prefix(at.as_str())?.strip_prefix('/')?;
      Some(dir.join(rest.trim_start_matches('/')))
    })
  }
}

/// The location of the directory `cwd` on `node`: `node:~/notes`.
pub fn location_of(node: &str, cwd: &Path) -> String {
  let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
  format!("{node}:{}", home_relative_str(&cwd.display().to_string()))
}

/// A `folder` record that makes the folder at `here` (a location) the same
/// as the one at `other` (`node:path`, with a relative path in that node's
/// home directory, or a path on `here`'s node, a relative one from the
/// current directory).
pub fn link_draft(
  node: &stitch_records::NodeName,
  here: &str,
  other: &str,
) -> Result<stitch_records::Draft, stitch_records::Error> {
  let other = link_target(node.as_str(), here, other);
  let pair = toml::Value::Array(vec![here.into(), other.into()]);
  stitch_records::Draft::new(kinds::FOLDER, stitch_records::Author::user(node.clone()))?
    .field("link", pair)
}

/// The location `other` names, as `link` takes it: `node:path` (a relative
/// path in that node's home directory), or a path on `here`'s node (a
/// relative one from the current directory).
pub fn link_target(node: &str, here: &str, other: &str) -> String {
  if other.contains(':') {
    return location(other);
  }
  let (n, _) = here.split_once(':').unwrap_or((node, ""));
  let path = Path::new(other);
  let path = match std::env::current_dir() {
    Ok(cwd) if path.is_relative() && !other.starts_with('~') => cwd.join(path),
    _ => path.to_owned(),
  };
  location(&format!("{n}:{}", path.display()))
}

/// What `link` decided.
#[derive(Debug)]
pub enum Link {
  /// The record to write.
  Draft(Box<stitch_records::Draft>),
  /// Both sides are already one folder (named as `here` sees it): nothing
  /// to write.
  Same(String),
}

/// `stitch folder link` (and `/link`), checked: the folder at `here` (a
/// location on `node`) made the same as `other`, unless they already are
/// one folder, or a side is not a folder Stitch knows (`here` must be a
/// directory; `other` must be one on this node, or named by a thread's
/// folder, a `folder` record or a folder sync snapshot from its node), when
/// it refuses, unless `force`: a typo would make up a folder, and the
/// folder's name could change everywhere (design §2.3, "Outside git").
pub fn link(
  index: &Index,
  node: &stitch_records::NodeName,
  here: &str,
  other: &str,
  force: bool,
) -> Result<Link, String> {
  let here = location(here);
  let other = link_target(node.as_str(), &here, other);
  let folders = Folders::load(index);
  let split = |l: &str| -> (String, String) {
    l.split_once(':')
      .map(|(n, p)| (n.to_owned(), p.to_owned()))
      .unwrap_or_else(|| (node.to_string(), l.to_owned()))
  };
  let ((hn, hp), (on, op)) = (split(&here), split(&other));
  let key = folders.key(&hn, &hp);
  if key == folders.key(&on, &op) {
    return Ok(Link::Same(folders.label(&key, node.as_str())));
  }
  if !force {
    if hn != node.as_str() || !expand_home(&hp).is_dir() {
      return Err(format!(
        "{here} is not a directory on this node (--force links it anyway)"
      ));
    }
    if !known_location(index, &folders, node.as_str(), &other) {
      return Err(format!(
        "{other} is not a folder Stitch knows: no thread, folder record or folder sync \
         shows it on {on}. It may be a typo, or no thread has run there yet \
         (--force links it anyway)"
      ));
    }
  }
  link_draft(node, &here, &other)
    .map(|d| Link::Draft(Box::new(d)))
    .map_err(|e| e.to_string())
}

/// Whether a location (`node:path`, home-relative) is a folder Stitch
/// knows: a directory when on `this` node, else one a thread's `folder`, a
/// `folder` record (location, link, unlink, sync) or a folder sync snapshot
/// from that node names.
fn known_location(index: &Index, folders: &Folders, this: &str, loc: &str) -> bool {
  let Some((node, path)) = loc.split_once(':') else {
    return false;
  };
  if node == this && expand_home(path).is_dir() {
    return true;
  }
  let key = folders.key(node, path);
  let str_of = |v: &Value| v.as_str().map(location);
  let threads = index.records(Some(kinds::THREAD)).unwrap_or_default();
  let in_thread = threads.iter().any(|r| {
    r.fields.get("folder").is_some_and(|f| {
      match (
        f.get("node").and_then(Value::as_str),
        f.get("path").and_then(Value::as_str),
      ) {
        (Some(n), Some(p)) => location(&format!("{n}:{p}")) == loc,
        _ => false,
      }
    })
  });
  if in_thread {
    return true;
  }
  let records = index.records(Some(kinds::FOLDER)).unwrap_or_default();
  let in_folder = records.iter().any(|r| {
    let f = &r.fields;
    let named = ["location", "unlink"]
      .iter()
      .any(|k| f.get(*k).and_then(str_of).as_deref() == Some(loc));
    let linked = f
      .get("link")
      .and_then(Value::as_array)
      .is_some_and(|pair| pair.iter().filter_map(str_of).any(|l| l == loc));
    let synced = r.node == node
      && f
        .get("sync")
        .and_then(Value::as_str)
        .is_some_and(|k| folders.find(k) == key);
    named || linked || synced
  });
  if in_folder {
    return true;
  }
  let snaps = index
    .records(Some(crate::folders::SNAPSHOT))
    .unwrap_or_default();
  snaps.iter().any(|r| {
    r.node == node
      && r
        .fields
        .get("folder")
        .and_then(Value::as_str)
        .is_some_and(|k| folders.find(k) == key)
  })
}

/// A `folder` record that keeps the folder at `here` to its own node,
/// apart from same-named folders elsewhere and from its links.
pub fn unlink_draft(
  node: &stitch_records::NodeName,
  here: &str,
) -> Result<stitch_records::Draft, stitch_records::Error> {
  stitch_records::Draft::new(kinds::FOLDER, stitch_records::Author::user(node.clone()))?
    .field("unlink", here)
}

/// A `folder` record that lets the folder `key` sync its files with the
/// user's other machines, or keeps them on each node (`allow` false).
pub fn sync_draft(
  node: &stitch_records::NodeName,
  key: &str,
  allow: bool,
) -> Result<stitch_records::Draft, stitch_records::Error> {
  stitch_records::Draft::new(kinds::FOLDER, stitch_records::Author::user(node.clone()))?
    .field("sync", key)?
    .field("allow", allow)
}

/// Why a folder place in `dir` never syncs its files unless the user lets
/// it (design §2.16), not even asking: a hidden directory in the home
/// directory (`~/.cargo`, `~/.cache`, `~/.config`), a temporary directory,
/// the system's. `None` for any other.
pub fn never_synced(dir: &Path) -> Option<&'static str> {
  let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
  let canon = |p: PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
  let home = std::env::var_os("HOME").map(|h| canon(PathBuf::from(h)));
  if let Some(home) = &home
    && let Ok(rel) = dir.strip_prefix(home)
  {
    let hidden = rel
      .components()
      .any(|c| c.as_os_str().to_str().is_some_and(|s| s.starts_with('.')));
    return hidden.then_some("a hidden directory (tools' caches and settings)");
  }
  let temp = [
    canon(std::env::temp_dir()),
    canon("/tmp".into()),
    canon("/var/tmp".into()),
  ];
  if temp.iter().any(|t| dir.starts_with(t)) {
    return Some("a temporary directory");
  }
  const SYSTEM: &[&str] = &[
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/lib",
    "/lib64",
    "/nix",
    "/opt",
    "/proc",
    "/run",
    "/sbin",
    "/sys",
    "/usr",
    "/var",
    "/Applications",
    "/Library",
    "/System",
    "/private",
  ];
  SYSTEM
    .iter()
    .any(|s| dir.starts_with(s))
    .then_some("a system directory")
}

/// `node:path` with the path made home-relative. A relative path is in the
/// node's home directory, as with `scp`: `mbp:notes` is `mbp:~/notes`.
pub fn location(s: &str) -> String {
  match s.split_once(':') {
    Some((node, path)) if !path.starts_with(['/', '~']) => {
      format!("{node}:{}", tilde(Path::new(path.trim_start_matches("./"))))
    }
    Some((node, path)) => format!("{node}:{}", home_relative_str(path)),
    None => s.to_owned(),
  }
}

/// A path relative to the home directory where it is in it: `~/notes`.
/// Paths another node wrote as `/Users/<name>/…` or `/home/<name>/…` are
/// taken as in that node's home directory.
pub fn home_relative_str(path: &str) -> String {
  if path.starts_with('~') {
    return path.to_owned();
  }
  if let Some(h) = std::env::var_os("HOME") {
    let h = PathBuf::from(h);
    for home in [std::fs::canonicalize(&h).unwrap_or_else(|_| h.clone()), h] {
      if let Ok(rest) = Path::new(path).strip_prefix(&home) {
        return tilde(rest);
      }
    }
  }
  for prefix in ["/Users/", "/home/"] {
    if let Some(rest) = path.strip_prefix(prefix) {
      let rest = rest.split_once('/').map_or("", |(_, r)| r);
      return tilde(Path::new(rest));
    }
  }
  path.to_owned()
}

fn tilde(rest: &Path) -> String {
  if rest.as_os_str().is_empty() {
    "~".into()
  } else {
    format!("~/{}", rest.display())
  }
}

/// `~/notes` on this node.
pub fn expand_home(path: &str) -> PathBuf {
  match (path.strip_prefix('~'), std::env::var_os("HOME")) {
    (Some(rest), Some(h)) => PathBuf::from(format!("{}{rest}", Path::new(&h).display())),
    _ => PathBuf::from(path),
  }
}

impl Place {
  /// The place of the directory `cwd` on `node`, with the checkout it is
  /// in; `home_workspace` is where the home workspace's records are.
  pub fn of_dir(
    cwd: &Path,
    node: &str,
    home_workspace: Option<&Path>,
    places: &Places,
  ) -> (Place, Option<Checkout>) {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
    let home = std::env::var_os("HOME").map(|h| {
      let h = PathBuf::from(h);
      std::fs::canonicalize(&h).unwrap_or(h)
    });
    let is_home = Some(&cwd) == home.as_ref();
    if is_home || cwd.parent().is_none() || home_workspace.is_some_and(|w| cwd.starts_with(w)) {
      return (Place::Global, None);
    }
    // A home directory kept in git (dotfiles) is not a project.
    if let Some(co) = Checkout::of(&cwd).filter(|c| Some(&c.top) != home.as_ref()) {
      return (Place::Project(project_of(&co, &places.projects)), Some(co));
    }
    let key = places.folders.key(node, &cwd.display().to_string());
    (Place::Folder(key), None)
  }

  /// The place of a thread record; `home` is the home workspace's ID.
  pub fn of_thread(r: &IndexedRecord, home: Option<&RepoId>, places: &Places) -> Place {
    if home.is_some_and(|h| h != &r.repo) {
      // A project that went back to being a folder: its threads with it.
      return match places.folders.unfolded(&r.repo) {
        Some(key) => Place::Folder(key.to_owned()),
        None => Place::Project(r.repo.clone()),
      };
    }
    if let Some(f) = r.fields.get("folder")
      && let (Some(node), Some(path)) = (
        f.get("node").and_then(Value::as_str),
        f.get("path").and_then(Value::as_str),
      )
    {
      let key = places.folders.key(node, path);
      return match places.folders.project(&key) {
        Some(repo) => Place::Project(repo.clone()),
        None => Place::Folder(key),
      };
    }
    Place::Global
  }

  /// The directory this place is on `node`, when it is there.
  pub fn dir(&self, node: &str, checkouts: &mut Checkouts, places: &Places) -> Option<PathBuf> {
    match self {
      Place::Project(repo) => checkouts.of(repo, &places.projects),
      Place::Folder(key) => places.folders.dir(key, node),
      Place::Global => None,
    }
  }

  /// The place a key names (`key`).
  pub fn of_key(key: &str) -> Option<Place> {
    match key.split_once(':') {
      _ if key == "global" => Some(Place::Global),
      Some(("project", repo)) => repo.parse().ok().map(Place::Project),
      Some(("folder", folder)) if !folder.is_empty() => Some(Place::Folder(folder.to_owned())),
      _ => None,
    }
  }

  /// A key for remembering things per place.
  pub fn key(&self) -> String {
    match self {
      Place::Project(repo) => format!("project:{repo}"),
      Place::Folder(key) => format!("folder:{key}"),
      Place::Global => "global".into(),
    }
  }

  /// A short name: the project's, the folder (with its node when it is
  /// one node's and not `here`'s), or `global`.
  pub fn label(&self, here: &str, places: &Places) -> String {
    match self {
      Place::Project(repo) => places
        .projects
        .iter()
        .find(|p| &p.repo == repo)
        .map_or_else(|| repo.to_string(), |p| p.name.clone()),
      Place::Folder(key) => places.folders.label(key, here),
      Place::Global => "global".into(),
    }
  }
}

/// Where this node has seen each project checked out (`checkouts.toml` in
/// the data directory; local, never synced): where a project's shells
/// start when the user is not in it. The file is shared by the node and
/// the CLI, each with its own copy: every change re-reads it first, then
/// writes it back, so neither writes over the other's.
#[derive(Debug, Default)]
pub struct Checkouts {
  path: Option<PathBuf>,
  seen: BTreeMap<String, Vec<PathBuf>>,
  /// The checkouts found still their project (by project and directory),
  /// with their `.git` as it was then: one whose `.git` is the same is not
  /// read with git again. Not written: read once per process.
  checked: std::collections::HashMap<(String, PathBuf), GitStamp>,
}

/// What tells a checkout's `.git` from another one made in its place (the
/// directory replaced, re-cloned, `git init` again): its device and inode,
/// and when it was made, against an inode reused.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GitStamp {
  dev: u64,
  ino: u64,
  born: Option<std::time::SystemTime>,
}

impl GitStamp {
  /// The stamp of `top`'s `.git` (a directory, or a worktree's file);
  /// `None` when it has none.
  fn of(top: &Path) -> Option<GitStamp> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(top.join(".git")).ok()?;
    Some(GitStamp {
      dev: m.dev(),
      ino: m.ino(),
      born: m.created().ok(),
    })
  }
}

/// Whether `git` runs here, asked once per process.
fn git_runs_here() -> bool {
  static RUNS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
  *RUNS.get_or_init(git_runs)
}

impl Checkouts {
  pub fn load(data: &Path) -> Checkouts {
    let path = data.join("checkouts.toml");
    let seen = std::fs::read_to_string(&path)
      .ok()
      .and_then(|t| toml::from_str(&t).ok())
      .unwrap_or_default();
    Checkouts {
      path: Some(path),
      seen,
      checked: Default::default(),
    }
  }

  /// A checkout of `repo` that is still there, and still it: one whose
  /// directory now holds another repository, or none, is forgotten (the
  /// search of `checkout::ensure` takes over). Read with git once, then
  /// again only when its `.git` changed (`GitStamp`): a stat per turn.
  pub fn of(&mut self, repo: &RepoId, projects: &[Project]) -> Option<PathBuf> {
    let noted = self.seen.get(repo.as_str())?.clone();
    let mut stale = Vec::new();
    let mut found = None;
    for p in noted.iter().filter(|p| p.is_dir()) {
      match self.still(repo, p, projects) {
        Ok(()) => {
          found = Some(p.clone());
          break;
        }
        Err(why) => {
          eprintln!(
            "project {repo}: the checkout noted at {} {why}: forgotten",
            p.display()
          );
          stale.push(p.clone());
        }
      }
    }
    if !stale.is_empty() {
      self.reload();
      if let Some(list) = self.seen.get_mut(repo.as_str()) {
        list.retain(|p| !stale.contains(p));
        if list.is_empty() {
          self.seen.remove(repo.as_str());
        }
      }
      for p in stale {
        self.checked.remove(&(repo.to_string(), p));
      }
      self.write();
    }
    found
  }

  /// Whether the checkout noted at `top` is still `repo`'s; why not.
  fn still(
    &mut self,
    repo: &RepoId,
    top: &Path,
    projects: &[Project],
  ) -> std::result::Result<(), String> {
    // A phone's clones are its own, in its data directory, by project.
    if crate::clones::keeps_clones() {
      return Ok(());
    }
    let Some(stamp) = GitStamp::of(top) else {
      return Err("has no `.git` any more".into());
    };
    let key = (repo.to_string(), top.to_owned());
    if self.checked.get(&key) == Some(&stamp) {
      return Ok(());
    }
    // Where git cannot run, a checkout cannot be read: trusted.
    if !git_runs_here() {
      return Ok(());
    }
    let co = Checkout::of(top).ok_or("is not a git checkout with commits any more")?;
    let here = std::fs::canonicalize(top).unwrap_or_else(|_| top.to_owned());
    if co.top != here {
      return Err(format!("is inside {} now", co.top.display()));
    }
    if !holds(&co, repo, projects) {
      let now = project_of(&co, projects);
      return Err(format!("holds another project now ({now})"));
    }
    self.checked.insert(key, stamp);
    Ok(())
  }

  /// Replaces the copy with the file as it is now (as `load` reads it).
  fn reload(&mut self) {
    if let Some(path) = &self.path {
      self.seen = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    }
  }

  fn write(&self) {
    if let Some(path) = &self.path
      && let Ok(text) = toml::to_string(&self.seen)
    {
      let _ = std::fs::write(path, text);
    }
  }

  /// Every checkout noted here, of any project, that is still there.
  pub fn all(&self) -> Vec<PathBuf> {
    self
      .seen
      .values()
      .flatten()
      .filter(|p| p.is_dir())
      .cloned()
      .collect()
  }

  /// Notes a checkout of `repo`, most recent first.
  pub fn saw(&mut self, repo: &RepoId, top: &Path) {
    self.reload();
    let list = self.seen.entry(repo.to_string()).or_default();
    if list.first().map(PathBuf::as_path) == Some(top) {
      return;
    }
    list.retain(|p| p != top);
    list.insert(0, top.to_owned());
    self.write();
  }
}

/// The top of the git working tree holding `dir`.
pub fn work_tree(dir: &Path) -> Option<PathBuf> {
  dir
    .ancestors()
    .find(|a| a.join(".git").exists())
    .map(Path::to_owned)
}

/// Whether the `git` command runs on this node.
pub fn git_runs() -> bool {
  Command::new("git")
    .arg("--version")
    .output()
    .is_ok_and(|o| o.status.success())
}

/// The top of the git checkout holding `dir` when `git` cannot be run
/// here to read it: such a checkout is taken for a folder until git is
/// installed (then it becomes its project, its threads with it).
pub fn unread_checkout(dir: &Path) -> Option<PathBuf> {
  let top = work_tree(dir)?;
  (!git_runs()).then_some(top)
}

/// What is said of a checkout `git` cannot read here.
pub fn unread_checkout_note(top: &Path) -> String {
  format!(
    "{} is a git checkout, but the `git` command cannot be run on this node, so it is taken \
     for a folder, not its project, until git is installed (`brew install git`, `apt install \
     git`); then it becomes the project, its threads with it.",
    top.display()
  )
}

/// Where a thread's shell sessions start: where the user is when that is
/// inside the thread's place, else the place, else where the user is.
pub fn start_dir(place_dir: Option<PathBuf>, cwd: Option<&Path>) -> Option<PathBuf> {
  let cwd = cwd.map(|c| std::fs::canonicalize(c).unwrap_or_else(|_| c.to_owned()));
  match (place_dir, cwd) {
    (Some(d), Some(c)) if c.starts_with(&d) => Some(c),
    (Some(d), _) => Some(d),
    (None, c) => c,
  }
}

/// Threads as a place lists them (the TUI, `stitch threads`, the app):
/// its own and global ones, with `all` every other too, most recent first
/// whatever place each belongs to (decided 2026-10-13: a thread started on
/// the phone a minute ago, global, comes before a project's of yesterday).
pub fn listed<T>(
  threads: Vec<(IndexedRecord, T)>,
  home: Option<&RepoId>,
  here: &Place,
  all: bool,
  places: &Places,
) -> Vec<(IndexedRecord, T, Place)> {
  threads
    .into_iter()
    .filter_map(|(r, t)| {
      let p = Place::of_thread(&r, home, places);
      (all || &p == here || p == Place::Global).then_some((r, t, p))
    })
    .collect()
}

/// Threads in the order a place resumes them (`last`): its own, then
/// global ones, then (with `all`) every other; most recent first within
/// each.
pub fn ordered<T>(
  threads: Vec<(IndexedRecord, T)>,
  home: Option<&RepoId>,
  here: &Place,
  all: bool,
  places: &Places,
) -> Vec<(IndexedRecord, T, Place)> {
  let mut out: Vec<(u8, usize, IndexedRecord, T, Place)> = threads
    .into_iter()
    .enumerate()
    .filter_map(|(i, (r, t))| {
      let p = Place::of_thread(&r, home, places);
      let rank = if &p == here {
        0
      } else if p == Place::Global {
        1
      } else if all {
        2
      } else {
        return None;
      };
      Some((rank, i, r, t, p))
    })
    .collect();
  out.sort_by_key(|(rank, i, ..)| (*rank, *i));
  out.into_iter().map(|(_, _, r, t, p)| (r, t, p)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn run(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
      .arg("-C")
      .arg(dir)
      .args(args)
      .output()
      .unwrap()
      .status
      .success();
    assert!(ok, "git {args:?}");
  }

  /// A repository with one commit and, maybe, an origin.
  fn repo(dir: &Path, origin: Option<&str>) {
    std::fs::create_dir_all(dir).unwrap();
    run(dir, &["init", "--quiet"]);
    run(
      dir,
      &[
        "-c",
        "user.name=T",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "first",
      ],
    );
    if let Some(o) = origin {
      run(dir, &["remote", "add", "origin", o]);
    }
  }

  #[test]
  fn remotes_are_normalized() {
    for url in [
      "git@github.com:Org/Repo.git",
      "https://user@GitHub.com/Org/Repo",
      "ssh://git@github.com:22/Org/Repo.git",
      "https://github.com/Org/Repo/",
    ] {
      assert_eq!(normalize_remote(url), "github.com/Org/Repo", "{url}");
    }
    assert_eq!(normalize_remote("/srv/git/app.git"), "/srv/git/app");
  }

  #[test]
  fn a_directory_finds_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let workspace = d.join("home");
    std::fs::create_dir_all(&workspace).unwrap();
    let app = d.join("app");
    repo(&app, Some("git@github.com:me/app.git"));
    std::fs::create_dir_all(app.join("src")).unwrap();
    // A clone elsewhere shares the root commit: the same project.
    let clone = d.join("elsewhere/app-clone");
    std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
    run(
      &d,
      &[
        "clone",
        "--quiet",
        app.to_str().unwrap(),
        clone.to_str().unwrap(),
      ],
    );
    let plain = d.join("plain/deep");
    std::fs::create_dir_all(&plain).unwrap();

    let none = Places::default();
    let of = |p: &Path, places: &Places| Place::of_dir(p, "mbp", Some(&workspace), places);
    let (here, co) = of(&app.join("src"), &none);
    let co = co.unwrap();
    assert_eq!(co.top, app);
    assert_eq!(co.remote.as_deref(), Some("github.com/me/app"));
    assert_eq!(co.name(), "app");
    let project = project_repo(&co.root, None);
    assert_eq!(here, Place::Project(project.clone()));
    assert_eq!(of(&clone, &none).0, Place::Project(project.clone()));

    // The clone, split off as a fork, has a project of its own; the
    // original keeps its.
    let fork = Checkout::of(&clone).unwrap();
    let split = Project {
      repo: project_repo(&fork.root, fork.remote.as_deref()),
      root: fork.root.clone(),
      remote: fork.remote.clone(),
      name: "fork".into(),
      split: true,
      reroots: Vec::new(),
      created: jiff::Timestamp::UNIX_EPOCH,
    };
    assert_ne!(split.repo, project);
    let places = Places {
      projects: vec![split.clone()],
      ..Places::default()
    };
    assert_eq!(of(&clone, &places).0, Place::Project(split.repo.clone()));
    assert_eq!(of(&app, &places).0, Place::Project(project.clone()));
    assert_eq!(Place::Project(split.repo).label("mbp", &places), "fork");

    // Outside the home directory, a folder is its node's.
    assert_eq!(
      of(&plain, &none).0,
      Place::Folder(format!("mbp:{}", plain.display()))
    );
    assert_eq!(of(&workspace, &none).0, Place::Global);
    assert_eq!(of(Path::new("/"), &none).0, Place::Global);

    let mut seen = Checkouts::default();
    assert_eq!(
      Place::Project(project.clone()).dir("mbp", &mut seen, &none),
      None
    );
    seen.saw(&project, &app);
    assert_eq!(
      Place::Project(project).dir("mbp", &mut seen, &none),
      Some(app.clone())
    );
    assert_eq!(
      start_dir(Some(app.clone()), Some(&app.join("src"))),
      Some(app.join("src"))
    );
    assert_eq!(start_dir(Some(app.clone()), Some(&plain)), Some(app));
    assert_eq!(start_dir(None, Some(&plain)), Some(plain));
  }

  /// A remembered checkout is trusted only while it is still its project:
  /// its directory replaced by another repository, or without `.git`, it is
  /// forgotten (from `checkouts.toml` too) and the next noted one taken.
  /// Read with git once, then trusted while its `.git` is the same.
  #[test]
  fn a_remembered_checkout_no_longer_its_project_is_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let data = d.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let (app, old, plain) = (d.join("app"), d.join("old-app"), d.join("plain"));
    repo(&app, None);
    run(
      &d,
      &[
        "clone",
        "--quiet",
        app.to_str().unwrap(),
        old.to_str().unwrap(),
      ],
    );
    let project = project_of(&Checkout::of(&app).unwrap(), &[]);
    let mut seen = Checkouts::load(&data);
    seen.saw(&project, &old);
    seen.saw(&project, &app);
    assert_eq!(seen.of(&project, &[]), Some(app.clone()));
    let key = (project.to_string(), app.clone());
    assert_eq!(seen.checked.get(&key), GitStamp::of(&app).as_ref());
    assert_eq!(seen.of(&project, &[]), Some(app.clone()), "from the cache");

    // Another repository where it was: forgotten, the clone noted before
    // it taken, and so the next node start reads it too.
    std::fs::remove_dir_all(&app).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    run(&app, &["init", "--quiet"]);
    run(
      &app,
      &[
        "-c",
        "user.name=T",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "another project",
      ],
    );
    assert_ne!(project_of(&Checkout::of(&app).unwrap(), &[]), project);
    assert_eq!(seen.of(&project, &[]), Some(old.clone()));
    assert_eq!(
      Checkouts::load(&data).seen[project.as_str()],
      vec![old.clone()]
    );

    // No `.git` any more (a plain folder now): forgotten, and nothing left.
    std::fs::remove_dir_all(old.join(".git")).unwrap();
    assert_eq!(seen.of(&project, &[]), None);
    assert!(!Checkouts::load(&data).seen.contains_key(project.as_str()));

    // A folder moved inside another checkout: not it either.
    repo(&plain, None);
    let inner = plain.join("sub");
    std::fs::create_dir_all(&inner).unwrap();
    let mut seen = Checkouts::load(&data);
    seen.saw(&project, &inner);
    assert_eq!(seen.of(&project, &[]), None);
  }

  /// The node and the CLI each hold a copy of `checkouts.toml`: a change
  /// by one is a read-modify-write of the file, not a write of its copy.
  #[test]
  fn checkouts_changes_re_read_the_file_shared_with_another_copy() {
    let dir = tempfile::tempdir().unwrap();
    let data = std::fs::canonicalize(dir.path()).unwrap().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let repo = "proj-y".parse::<RepoId>().unwrap();
    let other = "proj-z".parse::<RepoId>().unwrap();
    let (x, y) = (
      PathBuf::from("/tmp/x-checkout"),
      PathBuf::from("/tmp/y-checkout"),
    );
    let mut a = Checkouts::load(&data);
    let mut b = Checkouts::load(&data);
    a.saw(&repo, &x);

    // The other copy (the CLI) forgot X and wrote the file without it:
    // `a` still has X first in memory, and must write it back.
    std::fs::write(data.join("checkouts.toml"), "").unwrap();
    a.saw(&repo, &x);
    assert_eq!(Checkouts::load(&data).seen[repo.as_str()], vec![x.clone()]);

    // A checkout noted by `b` meanwhile is not dropped by `a`'s change.
    b.saw(&other, &y);
    a.saw(&repo, &y);
    let file = Checkouts::load(&data);
    assert_eq!(file.seen[other.as_str()], vec![y.clone()]);
    assert_eq!(file.seen[repo.as_str()], vec![y.clone(), x.clone()]);
  }

  #[test]
  fn some_folders_never_sync_and_the_rest_ask() {
    use stitch_store::{RecordRoot, Role};
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    assert!(never_synced(&home.join(".cargo/registry")).is_some());
    assert!(never_synced(&home.join("notes")).is_none());
    assert!(never_synced(&std::env::temp_dir().join("x")).is_some());
    assert!(never_synced(Path::new("/usr/share")).is_some());
    assert!(never_synced(Path::new("/srv/data")).is_none());

    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(dir.path(), "home".parse().unwrap(), Role::Home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    let node: stitch_records::NodeName = "mbp".parse().unwrap();
    let sync = |index: &mut Index, allow| {
      root
        .publish(&node, sync_draft(&node, "~/notes", allow).unwrap())
        .unwrap();
      index.refresh(&root).unwrap();
      Folders::load(index).sync("~/notes")
    };
    assert_eq!(Folders::load(&index).sync("~/notes"), None);
    assert_eq!(sync(&mut index, true), Some(true));
    // The latest answer counts.
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(sync(&mut index, false), Some(false));
  }

  #[test]
  fn folders_are_one_across_nodes_unless_told_otherwise() {
    use stitch_records::{Author, Draft, NodeName};
    use stitch_store::{RecordRoot, Role};
    let dir = tempfile::tempdir().unwrap();
    let home = RecordRoot::init(dir.path(), "home".parse().unwrap(), Role::Home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&home).unwrap();
    let (mbp, nas): (NodeName, NodeName) = ("mbp".parse().unwrap(), "nas".parse().unwrap());
    let folders = |index: &Index| Folders::load(index);

    // The same path under the home directory is one folder on every node,
    // however each wrote its home directory.
    let f = folders(&index);
    let notes = f.key("mbp", "~/notes");
    assert_eq!(notes, "~/notes");
    assert_eq!(f.key("nas", "/home/someone/notes"), notes);
    assert_eq!(f.key("pc", "/Users/someone-else/notes"), notes);
    // Elsewhere, each node's own.
    assert_ne!(f.key("mbp", "/srv/data"), f.key("nas", "/srv/data"));

    // Linked, two paths are one folder; each node names it by its own.
    home
      .publish(
        &mbp,
        link_draft(&mbp, "mbp:/srv/data", "nas:/mnt/data").unwrap(),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    let f = folders(&index);
    let data = f.key("mbp", "/srv/data");
    assert_eq!(f.key("nas", "/mnt/data"), data);
    assert_eq!(f.label(&data, "mbp"), "/srv/data");
    assert_eq!(f.label(&data, "nas"), "/mnt/data");
    assert_eq!(f.label(&data, "pc"), "/srv/data on mbp");

    // A relative path on another node is in its home directory: linking
    // `mbp:house` from `~/house` keeps the folder `~/house`, not a folder
    // named by mbp's location.
    assert_eq!(location("mbp:house"), "mbp:~/house");
    home
      .publish(&nas, link_draft(&nas, "nas:~/house", "mbp:house").unwrap())
      .unwrap();
    index.refresh(&home).unwrap();
    let f = folders(&index);
    assert_eq!(f.key("nas", "~/house"), "~/house");
    assert_eq!(f.key("mbp", "~/house"), "~/house");

    // Unlinked, a node's folder keeps to it.
    home
      .publish(&nas, unlink_draft(&nas, "nas:~/notes").unwrap())
      .unwrap();
    index.refresh(&home).unwrap();
    let f = folders(&index);
    assert_eq!(f.key("mbp", "~/notes"), "~/notes");
    assert_eq!(f.key("nas", "~/notes"), "nas:~/notes");

    // A folder with threads that became a checkout is folded into its
    // project, and its threads are listed there.
    let mut folder = toml::Table::new();
    folder.insert("node".into(), "mbp".into());
    folder.insert("path".into(), "~/tool".into());
    let thread = home
      .publish(
        &mbp,
        Draft::new(kinds::THREAD, Author::user(mbp.clone()))
          .unwrap()
          .field("folder", toml::Value::Table(folder))
          .unwrap()
          .body("# notes on the tool\n"),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    let repo: RepoId = "p-0123456789abcdef".parse().unwrap();
    let co = Checkout {
      top: expand_home("~/tool"),
      root: "0123456789abcdef0123".into(),
      remote: None,
    };
    let project = Place::Project(repo.clone());
    assert!(fold(&mut index, &home, &mbp, &project, Some(&co)).unwrap());
    assert!(
      !fold(&mut index, &home, &mbp, &project, Some(&co)).unwrap(),
      "once"
    );
    let r = index.record(&thread.id).unwrap().unwrap();
    assert_eq!(
      Place::of_thread(&r, Some(home.repo()), &Places::load(&index)),
      project
    );
  }

  #[test]
  fn a_link_needs_both_sides_known() {
    use stitch_records::{Author, Draft, NodeName};
    use stitch_store::{RecordRoot, Role};
    let dir = tempfile::tempdir().unwrap();
    let home =
      RecordRoot::init(&dir.path().join("w"), "home".parse().unwrap(), Role::Home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&home).unwrap();
    let (mbp, nas): (NodeName, NodeName) = ("mbp".parse().unwrap(), "nas".parse().unwrap());
    let here_dir = std::fs::canonicalize(dir.path()).unwrap().join("house");
    std::fs::create_dir_all(&here_dir).unwrap();
    let here = format!("nas:{}", here_dir.display());
    let drafted = |r: Result<Link, String>| match r {
      Ok(Link::Draft(_)) => true,
      other => panic!("{other:?}"),
    };

    // Already one folder (the same `~` path, however typed): nothing.
    for other in ["mbp:~/home-notes", "mbp:home-notes"] {
      match link(&index, &nas, "nas:~/home-notes", other, false) {
        Ok(Link::Same(label)) => assert_eq!(label, "~/home-notes"),
        r => panic!("{other}: {r:?}"),
      }
    }

    // A location no record shows on its node is refused, maybe a typo,
    // unless forced.
    let err = link(&index, &nas, &here, "mbp:home-ntoes", false).unwrap_err();
    assert!(
      err.contains("mbp:~/home-ntoes") && err.contains("--force"),
      "{err}"
    );
    assert!(drafted(link(&index, &nas, &here, "mbp:home-ntoes", true)));
    // Nor is a side here that is not a directory.
    let gone = format!("nas:{}", here_dir.join("gone").display());
    assert!(link(&index, &nas, &gone, "nas:/", false).is_err());
    // A directory on this node is known.
    assert!(drafted(link(&index, &nas, &here, "nas:/", false)));

    // A thread that ran there makes it known.
    let mut folder = toml::Table::new();
    folder.insert("node".into(), "mbp".into());
    folder.insert("path".into(), "/Users/me/Documents/house".into());
    home
      .publish(
        &mbp,
        Draft::new(kinds::THREAD, Author::user(mbp.clone()))
          .unwrap()
          .field("folder", toml::Value::Table(folder))
          .unwrap()
          .body("# house\n"),
      )
      .unwrap();
    // So does a folder sync snapshot that node published.
    home
      .publish(
        &mbp,
        Draft::new(crate::folders::SNAPSHOT, Author::node(mbp.clone()))
          .unwrap()
          .field("folder", "mbp:/srv/house")
          .unwrap(),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    assert!(drafted(link(
      &index,
      &nas,
      &here,
      "mbp:Documents/house",
      false
    )));
    assert!(drafted(link(&index, &nas, &here, "mbp:/srv/house", false)));
    // Not on another node.
    assert!(link(&index, &nas, &here, "pc:/srv/house", false).is_err());
  }

  #[test]
  fn other_nodes_paths_of_a_place_name_it_here() {
    let here = Path::new("/data/folders/f-1/files");
    let a = Aliases::new(vec!["~/house".into(), "/srv/house".into()], here);
    assert_eq!(a.map("~/house"), Some(here.to_owned()));
    assert_eq!(
      a.map("~/house/april/AGENTS.md"),
      Some(here.join("april/AGENTS.md"))
    );
    // However the other node wrote its home directory.
    assert_eq!(a.map("/Users/someone/house/a.md"), Some(here.join("a.md")));
    assert_eq!(a.map("/home/someone/house/a.md"), Some(here.join("a.md")));
    assert_eq!(a.map(" /srv/house/b/c.txt "), Some(here.join("b/c.txt")));
    // Not a prefix of a name, nor anything else.
    assert_eq!(a.map("~/houseboat/a.md"), None);
    assert_eq!(a.map("~/notes/a.md"), None);
    assert_eq!(a.map("a.md"), None);
    // A home directory is never one.
    assert_eq!(Aliases::new(vec!["~".into()], here).map("~/a.md"), None);

    // A folder's paths: its own, and those of the locations linked to it.
    let dir = tempfile::tempdir().unwrap();
    let home = stitch_store::RecordRoot::init(
      dir.path(),
      "home".parse().unwrap(),
      stitch_store::Role::Home,
    )
    .unwrap();
    let mut index = Index::open_in_memory().unwrap();
    let mbp: stitch_records::NodeName = "mbp".parse().unwrap();
    home
      .publish(
        &mbp,
        link_draft(&mbp, "mbp:~/house", "nas:/srv/house").unwrap(),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    let f = Folders::load(&index);
    assert_eq!(f.paths(&f.key("mbp", "~/house")), ["/srv/house", "~/house"]);
  }

  #[test]
  fn a_checkout_whose_git_goes_away_is_a_folder_again() {
    use stitch_records::{Author, Draft, NodeName, RecordId};
    use stitch_store::{RecordRoot, Role};
    let dir = tempfile::tempdir().unwrap();
    let home = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&home).unwrap();
    let mbp: NodeName = "mbp".parse().unwrap();
    let top = dir.path().join("house");
    std::fs::create_dir_all(top.join(".git")).unwrap();
    let top = std::fs::canonicalize(&top).unwrap();
    let path = top.display().to_string();

    // A folder's thread, then `git init` and a commit: folded.
    let mut folder = toml::Table::new();
    folder.insert("node".into(), "mbp".into());
    folder.insert("path".into(), path.clone().into());
    let thread = home
      .publish(
        &mbp,
        Draft::new(kinds::THREAD, Author::user(mbp.clone()))
          .unwrap()
          .field("folder", toml::Value::Table(folder))
          .unwrap()
          .body("# the house\n"),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    let repo: RepoId = "p-0123456789abcdef".parse().unwrap();
    let co = Checkout {
      top: top.clone(),
      root: "0123456789abcdef0123".into(),
      remote: None,
    };
    let project = Place::Project(repo.clone());
    assert!(fold(&mut index, &home, &mbp, &project, Some(&co)).unwrap());
    home
      .publish(
        &mbp,
        Draft::new(kinds::PROJECT, Author::node(mbp.clone()))
          .unwrap()
          .field("repo", repo.as_str())
          .unwrap()
          .field("root", co.root.as_str())
          .unwrap(),
      )
      .unwrap();
    index.refresh(&home).unwrap();
    // A thread started in the project, in its own records.
    let store = RecordRoot::init(&dir.path().join("project"), repo.clone(), Role::Project).unwrap();
    let later = store
      .publish(
        &mbp,
        Draft::new(kinds::THREAD, Author::user(mbp.clone()))
          .unwrap()
          .body("# the lights\n"),
      )
      .unwrap();
    index.refresh(&store).unwrap();
    let key = Places::load(&index).folders.key("mbp", &path);
    let place_of = |index: &Index, id: &RecordId| {
      let r = index.record(id).unwrap().unwrap();
      Place::of_thread(&r, Some(home.repo()), &Places::load(index))
    };
    let listed = |index: &Index| Places::load(index).listed().any(|p| p.repo == repo);
    assert_eq!(place_of(&index, &thread.id), project);
    assert_eq!(place_of(&index, &later.id), project);
    assert!(listed(&index));
    // A turn that began in the folder: its shells are the project's now.
    let now = |index: &Index, p: &Place| Places::load(index).now(p);
    assert_eq!(now(&index, &Place::Folder(key.clone())), project);
    assert_eq!(now(&index, &project), project);

    // While it has a `.git`, or on another node, it stays folded.
    assert!(unfold(&mut index, &home, &mbp).unwrap().is_empty());
    std::fs::remove_dir_all(top.join(".git")).unwrap();
    let nas: NodeName = "nas".parse().unwrap();
    assert!(unfold(&mut index, &home, &nas).unwrap().is_empty());
    assert_eq!(place_of(&index, &thread.id), project);

    // `.git` gone: a folder again, once, every thread with it, those
    // started in the project included; the project is no place.
    let folder = Place::Folder(key.clone());
    assert!(fold(&mut index, &home, &mbp, &folder, None).unwrap());
    assert_eq!(Places::load(&index).folders.project(&key), None);
    assert_eq!(place_of(&index, &thread.id), folder);
    assert_eq!(place_of(&index, &later.id), folder);
    assert!(!listed(&index));
    assert_eq!(now(&index, &project), folder);
    assert_eq!(now(&index, &folder), folder);
    assert!(unfold(&mut index, &home, &mbp).unwrap().is_empty());

    // A checkout again: folded again.
    std::fs::create_dir_all(top.join(".git")).unwrap();
    assert!(fold(&mut index, &home, &mbp, &project, Some(&co)).unwrap());
    assert_eq!(place_of(&index, &thread.id), project);
    assert_eq!(place_of(&index, &later.id), project);
    assert!(listed(&index));
  }

  /// A repository re-created from a new first commit at the same origin:
  /// its project lists the new roots (`reroot` records, which have no
  /// `root`), and checkouts of them are its own; a `reroot` of a project
  /// not listed counts for nothing.
  #[test]
  fn a_project_follows_its_repository_rerooted() {
    use stitch_records::{Author, Draft, NodeName};
    use stitch_store::{RecordRoot, Role};
    let dir = tempfile::tempdir().unwrap();
    let home = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      Role::Home,
    )
    .unwrap();
    let mbp: NodeName = "mbp".parse().unwrap();
    let (a, b, c, d) = (
      "a".repeat(40),
      "b".repeat(40),
      "c".repeat(40),
      "d".repeat(40),
    );
    let remote = "github.com/org/lib";
    let old = project_repo(&a, None);
    let newer = project_repo(&b, None);
    // A second apart each: records in the same millisecond are in ID order.
    let next = std::cell::Cell::new(1_000_000_i64);
    let publish = |fields: &[(&str, &str)]| {
      let mut draft = Draft::new(kinds::PROJECT, Author::node(mbp.clone())).unwrap();
      for (k, v) in fields {
        draft = draft.field(k, *v).unwrap();
      }
      next.set(next.get() + 1);
      let at = jiff::Timestamp::from_second(next.get()).unwrap();
      home.publish_at(&mbp, draft, at).unwrap();
    };
    publish(&[("repo", old.as_str()), ("root", &a), ("remote", remote)]);
    publish(&[("repo", newer.as_str()), ("root", &b), ("remote", remote)]);
    publish(&[("repo", old.as_str()), ("reroot", &b), ("remote", remote)]);
    publish(&[("repo", old.as_str()), ("reroot", &b), ("remote", remote)]);
    publish(&[("repo", old.as_str()), ("reroot", &c), ("remote", remote)]);
    publish(&[
      ("repo", "p-eeeeeeeeeeeeeeee"),
      ("reroot", &d),
      ("remote", remote),
    ]);
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&home).unwrap();
    let ps = projects(&index);
    assert_eq!(ps.len(), 2, "{ps:?}");
    let p = ps.iter().find(|p| p.repo == old).unwrap();
    assert_eq!(p.root, a);
    assert_eq!(p.reroots, [b.clone(), c.clone()]);

    let co = |root: &str, remote: Option<&str>| Checkout {
      top: dir.path().to_owned(),
      root: root.into(),
      remote: remote.map(str::to_owned),
    };
    // The old root, and the roots it was re-created from: the old project.
    for root in [&a, &b, &c] {
      assert_eq!(project_of(&co(root, Some(remote)), &ps), old);
      assert!(holds(&co(root, Some(remote)), &old, &ps));
      assert!(!recreates(&co(root, Some(remote)), &old, &ps));
    }
    // The project listed for the new root still holds its checkout.
    assert!(holds(&co(&b, Some(remote)), &newer, &ps));
    assert!(!holds(&co(&b, Some("github.com/org/other")), &newer, &ps));
    // Another new root at the same origin: a re-creation; at another
    // origin, or with none, another project.
    assert_eq!(
      project_of(&co(&d, Some(remote)), &ps),
      project_repo(&d, None)
    );
    assert!(recreates(&co(&d, Some(remote)), &old, &ps));
    assert!(!recreates(&co(&d, Some("github.com/org/other")), &old, &ps));
    assert!(!recreates(&co(&d, None), &old, &ps));
    assert!(!holds(&co(&d, Some(remote)), &old, &ps));
  }

  /// A noted checkout replaced by its repository re-created (a new first
  /// commit, the same origin) is forgotten as any other repository there:
  /// identity is not merged by path or origin. Once its project follows the
  /// new root (`reroot`, the user's `stitch project rebind`), it is kept.
  #[test]
  fn a_noted_checkout_recreated_at_its_origin_is_kept_only_once_rebound() {
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let data = d.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let lib = d.join("lib");
    let origin = d.join("origin").display().to_string();
    repo(&lib, Some(&origin));
    let first = Checkout::of(&lib).unwrap();
    let project = project_of(&first, &[]);
    let mut ps = vec![Project {
      repo: project.clone(),
      root: first.root.clone(),
      remote: first.remote.clone(),
      name: "lib".into(),
      split: false,
      reroots: Vec::new(),
      created: jiff::Timestamp::UNIX_EPOCH,
    }];
    let mut seen = Checkouts::load(&data);
    seen.saw(&project, &lib);
    assert_eq!(seen.of(&project, &ps), Some(lib.clone()));

    std::fs::remove_dir_all(&lib).unwrap();
    // A distinct first commit (another time).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    repo(&lib, Some(&origin));
    let recreated = Checkout::of(&lib).unwrap();
    assert_ne!(recreated.root, first.root);
    assert!(recreates(&recreated, &project, &ps));
    assert_eq!(seen.of(&project, &ps), None);
    assert!(!Checkouts::load(&data).seen.contains_key(project.as_str()));

    // Rebound: its project follows the new root, and keeps the checkout.
    ps[0].reroots.push(recreated.root.clone());
    seen.saw(&project, &lib);
    assert_eq!(seen.of(&project, &ps), Some(lib.clone()));
  }

  /// Re-rooting only goes forward. Two projects share an origin: the old
  /// one, and one listed later for the new root (a chat ran in the reset
  /// checkout first). An old checkout, not yet reset, is no re-creation
  /// of the newer project; the new root is one of the old project's. And a
  /// cycle written anyway (two nodes at once) is ignored by `project_of`.
  #[test]
  fn rerooting_only_goes_forward() {
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    let remote = Some("github.com/org/lib".to_owned());
    let at = |secs| jiff::Timestamp::from_second(secs).unwrap();
    let project = |root: &str, created| Project {
      repo: project_repo(root, None),
      root: root.into(),
      remote: remote.clone(),
      name: "lib".into(),
      split: false,
      reroots: Vec::new(),
      created,
    };
    let (old, newer) = (project(&a, at(100)), project(&b, at(200)));
    let co = |root: &str| Checkout {
      top: PathBuf::from("/lib"),
      root: root.into(),
      remote: remote.clone(),
    };
    let ps = vec![old.clone(), newer.clone()];
    // Backward: refused. Forward: allowed.
    assert!(!recreates(&co(&a), &newer.repo, &ps));
    assert!(recreates(&co(&b), &old.repo, &ps));
    // Re-rooted forward: the new root is the old project's, and the newer
    // project still holds its own root's checkout.
    let mut rerooted = old.clone();
    rerooted.reroots.push(b.clone());
    let ps = vec![rerooted.clone(), newer.clone()];
    assert_eq!(project_of(&co(&b), &ps), old.repo);
    assert_eq!(project_of(&co(&a), &ps), old.repo);
    assert!(holds(&co(&b), &newer.repo, &ps));
    assert!(!recreates(&co(&a), &newer.repo, &ps));
    // A cycle written anyway: each root is its own project again.
    let mut back = newer.clone();
    back.reroots.push(a.clone());
    let ps = vec![rerooted, back];
    assert_eq!(project_of(&co(&a), &ps), old.repo);
    assert_eq!(project_of(&co(&b), &ps), newer.repo);
  }
}
