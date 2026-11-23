//! Apps (design §2.10): HTML and JavaScript an agent writes, shown to the
//! user. The node serves them, the same way to every client: the iOS app's
//! web view through a URL scheme of its own, a desktop browser over HTTP on
//! this machine (`http`), each app at an origin of its own.
//!
//! - **A version** is what `show_app` took of a directory (or one HTML
//!   file): its files as blobs and trees, as a folder's (`folders::store_tree`),
//!   and a manifest blob naming the tree, the app's name, whether it may use
//!   the network, and the thread whose state it keeps. The manifest is the
//!   tool message's attachment (`MEDIA`), so the chat shows each version
//!   where it was made; the tool result lists every blob, so nodes that
//!   pull all blobs pull them.
//! - **Pinned:** an `app` record in the home workspace names a version, a
//!   name and an icon; pinned again, a newer record supersedes it, so its
//!   versions are its history. Phones show pinned apps on their home list.
//!   A pin follows its thread: a newer version of its name shown there
//!   moves it (`follow`), unless it was pinned at a fixed version.
//! - **State:** what a page keeps (`localStorage`, `stitch.storage`) is
//!   its thread's: `set` records on the thread, field `app:<key>`, a JSON
//!   value as text (empty: removed). A page's writes are gathered for a
//!   second, written, and synced at once; every version and pin of the
//!   thread's apps shares them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::{Attachment, Author, BlobHash, Draft, RecordId, Ref, kinds};
use stitch_store::{BlobStore, Index, IndexedRecord};

use crate::folders::{Flat, Ignore, Leaf};
use crate::node::Shared;
use crate::tools::ToolOutcome;
use crate::{Error, Result};

#[cfg(not(mobile))]
pub(crate) mod blob;

/// A file name for a header or a record: no path, no control characters
/// (an upload's name, `blob`; an attachment's, `node::attachment_arg`).
pub(crate) fn clean_name(name: &str) -> String {
  let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
  let s: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
  if s.trim().is_empty() {
    "file".into()
  } else {
    s
  }
}
pub mod containers;
pub(crate) mod feed;
pub mod files;
pub(crate) mod guard;
#[cfg(not(mobile))]
pub mod http;
mod libs;
pub mod live;
pub mod serve;
pub mod threads;
#[cfg(not(mobile))]
pub(crate) mod ws;

/// The record kind of a pinned app.
pub const KIND: &str = "app";
/// The media type of an app's manifest, as an attachment.
pub const MEDIA: &str = "application/vnd.stitch.app+json";
/// A version holds at most this many files, and this many bytes.
const MAX_FILES: usize = 2000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The prefix of a state key's `set` field.
const STATE: &str = "app:";
/// How long a page's writes are gathered before they are written.
const GATHER: Duration = Duration::from_secs(1);

/// A version of an app, as its manifest blob holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
  /// The manifest's format.
  pub app: u32,
  pub name: String,
  /// The root tree of its files.
  pub tree: BlobHash,
  /// The page opened first, a path in the tree.
  pub entry: String,
  /// It may reach the network; else its pages reach this app only.
  #[serde(default)]
  pub networking: bool,
  /// The thread whose state it keeps (and which it reports to).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thread: Option<RecordId>,
  /// What it needs of the node (`stitch-app.json`): threads of its own,
  /// within a grant (design §2.22).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub needs: Option<threads::Needs>,
}

impl Manifest {
  /// Reads a manifest blob that is here.
  pub fn read(blobs: &BlobStore, hash: &BlobHash) -> Option<Manifest> {
    let bytes = std::fs::read(blobs.path(hash)).ok()?;
    serde_json::from_slice::<Manifest>(&bytes)
      .ok()
      .filter(|m| m.app == 1)
  }
}

/// What `snapshot` took.
pub struct Version {
  pub manifest: BlobHash,
  pub name: String,
  /// Its files' bytes, together.
  pub bytes: u64,
  pub files: usize,
  /// Every blob it is made of: the manifest, its trees, its files.
  pub blobs: Vec<BlobHash>,
}

/// Takes the app at `path` (a directory with `index.html`, or one HTML
/// file, served as `index.html`) into the blob store.
pub fn snapshot(
  blobs: &BlobStore,
  path: &Path,
  name: &str,
  networking: bool,
  thread: Option<&RecordId>,
) -> Result<Version> {
  let meta = std::fs::metadata(path).map_err(Error::io(path.display()))?;
  let mut flat = Flat::new();
  let (mut bytes, mut all) = (0u64, Vec::new());
  let needs = if meta.is_dir() {
    threads::Needs::read(path)?
  } else {
    None
  };
  let mut add = |rel: String, file: &Path, flat: &mut Flat| -> Result<()> {
    let (hash, size) = blobs.put_file(file)?;
    bytes += size;
    if flat.len() >= MAX_FILES || bytes > MAX_BYTES {
      return Err(Error::Config(format!(
        "an app holds at most {MAX_FILES} files and {} MiB: {} has more; keep only what the \
         app loads in its directory",
        MAX_BYTES >> 20,
        path.display()
      )));
    }
    all.push(hash);
    flat.insert(rel, Leaf::File { hash, size });
    Ok(())
  };
  if meta.is_file() {
    let ext = path
      .extension()
      .and_then(|e| e.to_str())
      .unwrap_or("")
      .to_ascii_lowercase();
    if ext != "html" && ext != "htm" {
      return Err(Error::Config(format!(
        "{}: an app is a directory with index.html, or one .html file",
        path.display()
      )));
    }
    add("index.html".into(), path, &mut flat)?;
  } else {
    if !path.join("index.html").is_file() {
      return Err(Error::Config(format!(
        "{} has no index.html: the page an app opens with",
        path.display()
      )));
    }
    let ignore = Ignore::of(path);
    let walk = walkdir::WalkDir::new(path)
      .follow_links(false)
      .sort_by_file_name()
      .into_iter()
      .filter_entry(|e| {
        let rel = e.path().strip_prefix(path).unwrap_or(e.path());
        let rel = rel.to_string_lossy().replace('\\', "/");
        let hidden = e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.');
        rel.is_empty() || !(hidden || ignore.ignores(&rel))
      });
    for e in walk {
      let e = e.map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
      if !e.file_type().is_file() {
        continue;
      }
      let rel = e
        .path()
        .strip_prefix(path)
        .unwrap_or(e.path())
        .to_string_lossy()
        .replace('\\', "/");
      add(rel, e.path(), &mut flat)?;
    }
  }
  let needs = match needs {
    Some(n) => Some(with_pins(blobs, &flat, n.named(name))?),
    None => None,
  };
  version(blobs, flat, all, bytes, name, networking, thread, needs)
}

/// The needs with their container's pins computed from the version's
/// files (design §2.23).
fn with_pins(blobs: &BlobStore, flat: &Flat, mut needs: threads::Needs) -> Result<threads::Needs> {
  let Some(c) = &mut needs.container else {
    return Ok(needs);
  };
  let files: BTreeMap<String, String> = flat
    .iter()
    .filter_map(|(p, l)| match l {
      Leaf::File { hash, .. } => Some((p.clone(), hash.to_string())),
      _ => None,
    })
    .collect();
  let cf = match flat.get(&format!("{}/Containerfile", c.dir)) {
    Some(Leaf::File { hash, .. }) => {
      std::fs::read(blobs.path(hash)).map_err(Error::io(blobs.path(hash).display()))?
    }
    _ => {
      return Err(Error::Config(format!(
        "{}: no {}/Containerfile in the app (container.dir)",
        threads::NEEDS_FILE,
        c.dir
      )));
    }
  };
  c.pins = Some(
    containers::spec::pins_of(&c.dir, &files, &cf)
      .map_err(|e| Error::Config(format!("{}: {e}", threads::NEEDS_FILE)))?,
  );
  Ok(needs)
}

/// A version of files given whole (a built-in app's), by their paths.
pub(crate) fn snapshot_files(
  blobs: &BlobStore,
  files: &[(&str, &[u8])],
  name: &str,
  networking: bool,
  thread: Option<&RecordId>,
) -> Result<Version> {
  let mut flat = Flat::new();
  let (mut bytes, mut all) = (0u64, Vec::new());
  for (path, data) in files {
    let hash = blobs.put_bytes(data)?;
    bytes += data.len() as u64;
    all.push(hash);
    flat.insert(
      (*path).to_owned(),
      Leaf::File {
        hash,
        size: data.len() as u64,
      },
    );
  }
  version(blobs, flat, all, bytes, name, networking, thread, None)
}

/// The version of `flat` (its files' blobs `all`, `bytes` in all): its
/// trees and manifest stored.
#[allow(clippy::too_many_arguments)]
fn version(
  blobs: &BlobStore,
  flat: Flat,
  mut all: Vec<BlobHash>,
  bytes: u64,
  name: &str,
  networking: bool,
  thread: Option<&RecordId>,
  needs: Option<threads::Needs>,
) -> Result<Version> {
  let files = flat.len();
  let (tree, trees) = crate::folders::store_tree(blobs, &flat)?;
  all.extend(trees.into_iter().map(|(h, _, _)| h));
  all.push(tree);
  let manifest = Manifest {
    app: 1,
    name: name.to_owned(),
    tree,
    entry: "index.html".into(),
    networking,
    thread: thread.cloned(),
    needs,
  };
  let text = serde_json::to_vec_pretty(&manifest).map_err(|e| Error::Config(e.to_string()))?;
  let hash = blobs.put_bytes(&text)?;
  all.push(hash);
  all.sort_by_key(BlobHash::hex);
  all.dedup();
  Ok(Version {
    manifest: hash,
    name: name.to_owned(),
    bytes,
    files,
    blobs: all,
  })
}

/// An app's origin: the host its pages are served from. A version's is its
/// manifest's hash in two labels (a label holds 63 characters at most); a
/// pinned app's, `p` and 16 hex digits of its ID's hash, so it keeps its
/// origin, and its pages their state, from version to version.
pub fn version_host(manifest: &BlobHash) -> String {
  let hex = manifest.hex();
  format!("{}.{}", &hex[..32], &hex[32..])
}

pub fn pinned_host(app: &RecordId) -> String {
  format!("p{}", &blake3::hash(app.as_str().as_bytes()).to_hex()[..16])
}

/// What a host names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
  Version(BlobHash),
  Pinned(String),
}

impl Host {
  /// The host without a port and a `.localhost` (the desktop's).
  pub fn parse(host: &str) -> Option<Host> {
    let host = host.to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or("");
    let host = host.strip_suffix(".localhost").unwrap_or(host);
    let hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    match host.split_once('.') {
      Some((a, b)) if a.len() == 32 && b.len() == 32 && hex(a) && hex(b) => {
        format!("blake3:{a}{b}").parse().ok().map(Host::Version)
      }
      None if host.len() == 17 && host.starts_with('p') && hex(&host[1..]) => {
        Some(Host::Pinned(host.to_owned()))
      }
      _ => None,
    }
  }
}

/// A pinned app, as its latest `app` record has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pinned {
  /// Its ID: its first record's.
  pub id: RecordId,
  pub name: String,
  /// An SF Symbol's name, for the phone.
  pub icon: Option<String>,
  pub manifest: BlobHash,
  /// The thread it was pinned from.
  pub thread: Option<RecordId>,
  /// It stays on its version; else a newer version of its name its
  /// thread shows moves it.
  pub fixed: bool,
  /// When it was last pinned.
  pub at: jiff::Timestamp,
  pub host: String,
  /// The user let it read live state (`Some(true)`), or would not
  /// (`Some(false)`); `None`, never asked (`live`).
  pub live: Option<bool>,
  /// A built-in app it is (`dashboard`), and of which version.
  pub builtin: Option<(String, u64)>,
}

/// What a pin carries besides its version, name and icon; `None` keeps
/// what the pin had.
#[derive(Default)]
pub(crate) struct Extra {
  pub(crate) live: Option<bool>,
  pub(crate) builtin: Option<(String, u64)>,
}

/// The heads of each pinned app's records (those nothing supersedes), by
/// the app's ID.
fn heads(index: &Index) -> Result<BTreeMap<String, Vec<IndexedRecord>>> {
  let records = index.records(Some(KIND))?;
  let mut superseded = HashSet::new();
  for r in &records {
    for e in index.edges_from(&r.id)? {
      if e.rel == "supersedes" {
        superseded.insert(e.dst.id.to_string());
      }
    }
  }
  let mut out: BTreeMap<String, Vec<IndexedRecord>> = BTreeMap::new();
  for r in records {
    if superseded.contains(r.id.as_str()) {
      continue;
    }
    let app = r
      .fields
      .get("app")
      .and_then(Value::as_str)
      .map(str::to_owned)
      .unwrap_or_else(|| r.id.to_string());
    out.entry(app).or_default().push(r);
  }
  Ok(out)
}

/// The pinned apps the user sees and manages, in the order they were first
/// pinned: **without the built-in web client**, which is the node's own UI
/// for a browser (the iOS app has its own and cannot run it). Every list,
/// unpin, follow, export and allow goes through this, so none can forget
/// to leave it out; the node's own serving and `AppOpen`/`Client` use
/// [`pinned_all`].
pub fn pinned(index: &Index) -> Result<Vec<Pinned>> {
  Ok(
    pinned_all(index)?
      .into_iter()
      .filter(|p| !live::is_client(p))
      .collect(),
  )
}

/// Every pinned app, the built-in client included: for the node's own use
/// (serving the client, its origin checks, pinning it), never a list shown
/// to the user.
pub fn pinned_all(index: &Index) -> Result<Vec<Pinned>> {
  let mut out = Vec::new();
  for (app, heads) in heads(index)? {
    // Two nodes pinned it again offline: the later one stands.
    let Some(r) = heads
      .iter()
      .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)))
    else {
      continue;
    };
    if r.fields.get("removed").and_then(Value::as_bool) == Some(true) {
      continue;
    }
    let (Ok(id), Some(manifest)) = (
      app.parse::<RecordId>(),
      r.fields
        .get("manifest")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<BlobHash>().ok()),
    ) else {
      continue;
    };
    let field = |k: &str| r.fields.get(k).and_then(Value::as_str).map(str::to_owned);
    out.push(Pinned {
      host: pinned_host(&id),
      name: field("name").unwrap_or_else(|| "App".into()),
      icon: field("icon"),
      thread: field("thread").and_then(|t| t.parse().ok()),
      fixed: r.fields.get("fixed").and_then(Value::as_bool) == Some(true),
      live: r.fields.get("live").and_then(Value::as_bool),
      builtin: field("builtin").map(|b| {
        let v = r
          .fields
          .get("builtin_version")
          .and_then(Value::as_u64)
          .unwrap_or(0);
        (b, v)
      }),
      at: r.created,
      manifest,
      id,
    });
  }
  out.sort_by(|a, b| a.id.cmp(&b.id));
  Ok(out)
}

/// Pins a version: as a new app, or as `app`'s newer version (`None`:
/// a pinned app of the same thread and name, else a new one); `fixed`, it
/// stays there, else it follows its thread.
pub(crate) fn pin(
  s: &Shared,
  manifest: &BlobHash,
  name: Option<&str>,
  icon: Option<&str>,
  app: Option<&RecordId>,
  fixed: bool,
) -> Result<Pinned> {
  pin_with(s, manifest, name, icon, app, fixed, Extra::default())
}

/// `pin`, with what else the pin carries (what it had, unless `extra`
/// says otherwise): consent to read live state, and a built-in app's key.
pub(crate) fn pin_with(
  s: &Shared,
  manifest: &BlobHash,
  name: Option<&str>,
  icon: Option<&str>,
  app: Option<&RecordId>,
  fixed: bool,
  extra: Extra,
) -> Result<Pinned> {
  let state = s.state();
  let m = Manifest::read(&state.blobs, manifest)
    .ok_or_else(|| Error::Config(format!("no app {manifest} here")))?;
  let name = name
    .map(str::trim)
    .filter(|n| !n.is_empty())
    .unwrap_or(&m.name)
    .to_owned();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace".into()))?;
  let (app, supersedes, icon, had) = {
    let index = state.index.lock().expect("index lock");
    let all = heads(&index)?;
    let current = pinned_all(&index)?;
    let same = match app {
      Some(a) => Some(
        current
          .iter()
          .find(|p| &p.id == a)
          .ok_or_else(|| Error::Config(format!("no pinned app {a}")))?,
      ),
      None => current
        .iter()
        .find(|p| p.thread.is_some() && p.thread == m.thread && p.name == name),
    };
    match same {
      Some(p) => (
        Some(p.id.clone()),
        all
          .get(p.id.as_str())
          .map(|h| h.iter().map(|r| r.id.clone()).collect())
          .unwrap_or_default(),
        icon.map(str::to_owned).or_else(|| p.icon.clone()),
        (p.live, p.builtin.clone()),
      ),
      None => (None, Vec::new(), icon.map(str::to_owned), (None, None)),
    }
  };
  // A new pin of a thread's app takes the answer its conversation gave.
  let live = extra
    .live
    .or(had.0)
    .or_else(|| app.is_none().then(|| live::consent(s, &m, None)).flatten());
  let builtin = extra.builtin.or(had.1);
  let mut draft = Draft::new(KIND, Author::user(state.node.clone()))?
    .field("name", name.as_str())?
    .field("manifest", manifest.to_string())?
    .attach(Attachment {
      hash: *manifest,
      media_type: MEDIA.into(),
      name: Some(name.clone()),
      bytes: std::fs::metadata(state.blobs.path(manifest)).map_or(0, |m| m.len()),
    });
  if let Some(a) = &app {
    draft = draft.field("app", a.to_string())?;
  }
  if let Some(i) = icon.as_deref().filter(|i| !i.is_empty()) {
    draft = draft.field("icon", i)?;
  }
  if let Some(t) = &m.thread {
    draft = draft.field("thread", t.to_string())?;
  }
  if fixed {
    draft = draft.field("fixed", true)?;
  }
  if let Some(l) = live {
    draft = draft.field("live", l)?;
  }
  if let Some((b, v)) = &builtin {
    draft = draft
      .field("builtin", b.as_str())?
      .field("builtin_version", *v as i64)?;
  }
  for h in supersedes {
    draft = draft.reference("supersedes", Ref::to(h))?;
  }
  let record = home.publish(&state.node, draft)?;
  let mut index = state.index.lock().expect("index lock");
  index.refresh(&home)?;
  let id = app.unwrap_or(record.id);
  pinned_all(&index)?
    .into_iter()
    .find(|p| p.id == id)
    .ok_or_else(|| Error::Config("the pin was written but is not read back".into()))
}

/// Unpins an app (a record saying so; its versions stay).
pub(crate) fn unpin(s: &Shared, app: &RecordId) -> Result<()> {
  let state = s.state();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace".into()))?;
  let (p, supersedes) = {
    let index = state.index.lock().expect("index lock");
    let p = pinned(&index)?
      .into_iter()
      .find(|p| &p.id == app)
      .ok_or_else(|| Error::Config(format!("no pinned app {app}")))?;
    let heads: Vec<RecordId> = heads(&index)?
      .remove(app.as_str())
      .unwrap_or_default()
      .into_iter()
      .map(|r| r.id)
      .collect();
    (p, heads)
  };
  let mut draft = Draft::new(KIND, Author::user(state.node.clone()))?
    .field("app", app.to_string())?
    .field("name", p.name.as_str())?
    .field("manifest", p.manifest.to_string())?
    .field("removed", true)?;
  for h in supersedes {
    draft = draft.reference("supersedes", Ref::to(h))?;
  }
  home.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok(())
}

/// A pinned app as clients list it.
pub(crate) fn info(s: &Shared, p: &Pinned) -> crate::api::AppInfo {
  crate::api::AppInfo {
    id: p.id.to_string(),
    name: p.name.clone(),
    icon: p.icon.clone(),
    version: p.manifest.to_string(),
    thread: p.thread.as_ref().map(|t| t.to_string()),
    url: s.apps.desktop.get().map(|d| d.url(&p.host, "/")),
    fixed: p.fixed,
    live: p.live,
    builtin: p.builtin.as_ref().map(|(b, _)| b.clone()),
  }
}

/// A thread showed `manifest`: the pinned apps that follow it, of that
/// app's name, move to it, keeping their name, icon and data. The pinned
/// apps moved.
pub(crate) fn follow(s: &Shared, manifest: &BlobHash) -> Result<Vec<Pinned>> {
  let state = s.state();
  let m = Manifest::read(&state.blobs, manifest)
    .ok_or_else(|| Error::Config(format!("no app {manifest} here")))?;
  let Some(thread) = &m.thread else {
    return Ok(Vec::new());
  };
  let followers: Vec<Pinned> = pinned(&state.index.lock().expect("index lock"))?
    .into_iter()
    .filter(|p| !p.fixed && p.thread.as_ref() == Some(thread) && p.manifest != *manifest)
    // Of the same app: its version's name (the pin may be named
    // otherwise), or the pin's when that version is not here.
    .filter(|p| {
      Manifest::read(&state.blobs, &p.manifest).map_or(p.name == m.name, |pm| pm.name == m.name)
    })
    .collect();
  let mut moved = Vec::new();
  for p in followers {
    moved.push(pin(s, manifest, Some(&p.name), None, Some(&p.id), false)?);
  }
  Ok(moved)
}

/// The home workspace synced with every peer, in the background: the other
/// devices have a pin at once.
pub(crate) fn sync_home(s: &Arc<Shared>) {
  let Some(home) = s.state().home() else { return };
  let s = s.clone();
  tokio::spawn(async move {
    crate::node::sync_one(&s, home.repo()).await;
  });
}

/// A version of an app, as a client is told of it: its name, and its
/// address here with the node's token (the client's alone: it is never
/// stored), when the node serves apps to a browser.
pub(crate) fn shown(s: &Shared, manifest: &BlobHash, name: String) -> crate::api::Reply {
  crate::api::Reply::App {
    name,
    url: s
      .apps
      .desktop
      .get()
      .map(|d| d.url(&version_host(manifest), "/")),
  }
}

/// The version `what` names that is not a pinned app: a version
/// (`blake3:…`, fetched when missing) or a thread (the latest app it
/// showed); with its name.
async fn version_of(s: &Arc<Shared>, what: &str) -> Result<(BlobHash, String)> {
  match what.parse::<BlobHash>() {
    Ok(h) => {
      if !s.state().blobs.contains(&h) {
        crate::node::fetch_blobs(s, &[h]).await;
      }
      let name = Manifest::read(&s.state().blobs, &h)
        .map(|m| m.name)
        .ok_or_else(|| Error::Config(format!("no app {h} here")))?;
      Ok((h, name))
    }
    Err(_) => {
      let thread: RecordId = what.parse().map_err(|_| {
        Error::Config(format!(
          "no pinned app `{what}`: name one (stitch app list), a thread, or a version \
           (blake3:…)"
        ))
      })?;
      latest_in(s, &thread)?
        .ok_or_else(|| Error::Config(format!("thread {thread} has shown no app")))
    }
  }
}

/// An app version's files, for `stitch app export`: `what` a pinned app,
/// a thread or a version; its trees and files fetched when missing here.
async fn export(s: &Arc<Shared>, what: &str) -> Result<crate::api::Reply> {
  let found = {
    let pinned = pinned(&s.state().index.lock().expect("index lock"))?;
    find_pinned(&pinned, what).map(|p| (p.manifest, p.name.clone()))
  };
  let (manifest, name) = match found {
    Some((m, name)) => {
      if !s.state().blobs.contains(&m) {
        crate::node::fetch_blobs(s, &[m]).await;
      }
      (m, name)
    }
    None => version_of(s, what).await?,
  };
  let blobs = s.state().blobs.clone();
  let m = Manifest::read(&blobs, &manifest)
    .ok_or_else(|| Error::Config(format!("no app {manifest} here")))?;
  let not_here = || {
    Error::Config(format!(
      "{name}: its files are not on this device, and no other of your devices that has them \
       could be reached"
    ))
  };
  // Trees a level at a time, then the files.
  let flat = loop {
    match crate::folders::load_tree(&blobs, &m.tree) {
      Ok(f) => break f,
      Err(missing) => {
        if crate::node::fetch_blobs(s, &missing).await == 0 {
          return Err(not_here());
        }
      }
    }
  };
  let mut files = Vec::new();
  let mut missing = Vec::new();
  for (path, leaf) in &flat {
    if let crate::folders::Leaf::File { hash, .. } = leaf {
      if !blobs.contains(hash) {
        missing.push(*hash);
      }
      files.push((path.clone(), hash.to_string()));
    }
  }
  if !missing.is_empty() && crate::node::fetch_blobs(s, &missing).await < missing.len() {
    return Err(not_here());
  }
  Ok(crate::api::Reply::AppFiles {
    name,
    version: manifest.to_string(),
    files,
  })
}

/// The directory `stitch app export` writes an app to by default: its
/// name, lower-case, with dashes for what is not a letter or digit.
pub fn slug(name: &str) -> String {
  let mut out = String::new();
  for c in name.chars() {
    if c.is_alphanumeric() {
      out.extend(c.to_lowercase());
    } else if !out.is_empty() && !out.ends_with('-') {
      out.push('-');
    }
  }
  let out = out.trim_end_matches('-');
  if out.is_empty() {
    "app".into()
  } else {
    out.into()
  }
}

/// Writes an app version's files (`Reply::AppFiles`: each path with its
/// blob) from `blobs` into `dir`; how many. A directory that is there and
/// not empty is refused unless `force`; a path that would leave `dir`
/// is refused before anything is written.
pub fn export_files(
  blobs: &BlobStore,
  files: &[(String, String)],
  dir: &Path,
  force: bool,
) -> Result<usize> {
  if !force
    && std::fs::read_dir(dir)
      .ok()
      .is_some_and(|mut d| d.next().is_some())
  {
    return Err(Error::Config(format!(
      "{} is not empty: name another directory, or --force to write over it",
      dir.display()
    )));
  }
  let mut todo = Vec::new();
  for (path, hash) in files {
    let hash: BlobHash = hash
      .parse()
      .map_err(|e| Error::Config(format!("{path}: {e}")))?;
    if path.is_empty()
      || path.starts_with('/')
      || path.contains('\\')
      || path
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
      return Err(Error::Config(format!("{path}: not a path inside the app")));
    }
    if !blobs.contains(&hash) {
      return Err(Error::Config(format!("{path}: its content is not here")));
    }
    todo.push((dir.join(path), hash));
  }
  for (to, hash) in &todo {
    if let Some(parent) = to.parent() {
      std::fs::create_dir_all(parent).map_err(Error::io(parent.display()))?;
    }
    // Read and written, not copied: a blob's permissions are the store's.
    let from = blobs.path(hash);
    let bytes = std::fs::read(&from).map_err(Error::io(from.display()))?;
    std::fs::write(to, bytes).map_err(Error::io(to.display()))?;
  }
  Ok(todo.len())
}

/// The local protocol's requests about apps.
pub(crate) async fn request(
  s: &Arc<Shared>,
  req: crate::api::Request,
) -> Result<crate::api::Reply> {
  use crate::api::{Reply, Request};
  let launcher = s.apps.desktop.get().map(|d| d.url("", "/"));
  match req {
    Request::Apps => {
      // `pinned` leaves out the built-in web client (`stitch client`).
      let pinned = pinned(&s.state().index.lock().expect("index lock"))?;
      Ok(Reply::Apps {
        apps: pinned.iter().map(|p| info(s, p)).collect(),
        launcher,
      })
    }
    Request::AppOpen { what } => {
      let what = what.trim();
      let found = {
        let index = s.state().index.lock().expect("index lock");
        // The user's apps first; then the built-in client, by any name.
        let user = pinned(&index)?;
        match find_pinned(&user, what) {
          Some(p) => Some(p.clone()),
          None => {
            let all = pinned_all(&index)?;
            find_pinned(&all, what)
              .filter(|p| live::is_client(p))
              .cloned()
          }
        }
      };
      if let Some(p) = found {
        return Ok(Reply::App {
          name: p.name.clone(),
          url: s.apps.desktop.get().map(|d| d.url(&p.host, "/")),
        });
      }
      let (manifest, name) = version_of(s, what).await?;
      Ok(shown(s, &manifest, name))
    }
    Request::AppPin {
      what,
      name,
      icon,
      fixed,
    } => {
      let version = what.parse::<BlobHash>().ok();
      let fixed = fixed.unwrap_or(version.is_some());
      let manifest = match version {
        Some(h) => h,
        None => {
          let thread: RecordId = what.parse().map_err(|_| {
            Error::Config(format!(
              "`{what}`: name an app's version (blake3:…) or a thread"
            ))
          })?;
          latest_in(s, &thread)?
            .ok_or_else(|| Error::Config(format!("thread {thread} has shown no app")))?
            .0
        }
      };
      if !s.state().blobs.contains(&manifest) {
        crate::node::fetch_blobs(s, &[manifest]).await;
      }
      let p = pin(s, &manifest, name.as_deref(), icon.as_deref(), None, fixed)?;
      sync_home(s);
      Ok(Reply::Apps {
        apps: vec![info(s, &p)],
        launcher,
      })
    }
    Request::AppUnpin { app } => {
      let id = {
        let index = s.state().index.lock().expect("index lock");
        let user = pinned(&index)?;
        match find_pinned(&user, &app) {
          Some(p) => p.id.clone(),
          None => {
            let all = pinned_all(&index)?;
            return Err(
              match find_pinned(&all, &app).filter(|p| live::is_client(p)) {
                Some(p) => Error::Config(format!(
                  "{} is the built-in web client: it cannot be unpinned (open it with `stitch client`)",
                  p.name
                )),
                None => Error::Config(format!("no pinned app {app}")),
              },
            );
          }
        }
      };
      unpin(s, &id)?;
      sync_home(s);
      Ok(Reply::Ok)
    }
    Request::AppLive { app, thread, allow } => {
      let said = live::allow(s, &app, thread.as_deref(), allow)?;
      sync_home(s);
      if let Some(t) = thread
        .as_deref()
        .and_then(|t| t.parse::<RecordId>().ok())
        .and_then(|t| s.thread_place(&t).ok())
      {
        let s = s.clone();
        tokio::spawn(async move { crate::node::sync_one(&s, t.0.repo()).await });
      }
      s.apps.bump();
      Ok(Reply::Lines(vec![said]))
    }
    Request::AppThreads {
      app,
      thread,
      allow,
      needs,
    } => {
      let said = threads::allow(s, &app, thread.as_deref(), allow, needs.as_deref())?;
      // The grant syncs with the app's conversation at once.
      if let Some(t) = said.0.as_ref().and_then(|t| s.thread_place(t).ok()) {
        let s = s.clone();
        tokio::spawn(async move { crate::node::sync_one(&s, t.0.repo()).await });
      }
      Ok(Reply::Lines(said.1))
    }
    Request::AppContainer {
      app,
      thread,
      deny,
      build,
      run,
      shell,
    } => {
      let mut hashes = Vec::new();
      for (k, h) in [
        (containers::Kind::Build, build),
        (containers::Kind::Run, run),
        (containers::Kind::Shell, shell),
      ] {
        if let Some(h) = h {
          hashes.push((k, h));
        }
      }
      let said = containers::allow(s, &app, thread.as_deref(), deny, &hashes).await?;
      if (deny || !hashes.is_empty())
        && let Some(t) = thread
          .as_deref()
          .and_then(|t| t.parse::<RecordId>().ok())
          .and_then(|t| s.thread_place(&t).ok())
      {
        let s = s.clone();
        tokio::spawn(async move { crate::node::sync_one(&s, t.0.repo()).await });
      }
      Ok(Reply::Lines(said))
    }
    Request::AppStorage { remove } => Ok(Reply::Lines(
      containers::storage::report(s, remove.as_deref())
        .await
        .map_err(Error::Config)?,
    )),
    Request::AppExport { what } => export(s, what.trim()).await,
    Request::Client => {
      if s.apps.desktop.get().is_none() {
        return Err(Error::Config(
          "this node serves no apps to a browser: `[apps] port` is 0 in its config.toml, or it \
           is a node that cannot (the phone runs the app's own UI); run `stitch client` on a \
           computer, or set a port"
            .into(),
        ));
      }
      let found = pinned_all(&s.state().index.lock().expect("index lock"))?
        .into_iter()
        .find(live::is_client);
      let p = match found {
        Some(p) => p,
        None => {
          let s2 = s.clone();
          let p = tokio::task::spawn_blocking(move || live::client(&s2))
            .await
            .map_err(|e| Error::Config(e.to_string()))??;
          sync_home(s);
          p
        }
      };
      Ok(Reply::App {
        name: p.name.clone(),
        url: s.apps.desktop.get().map(|d| d.url(&p.host, "/")),
      })
    }
    Request::Dashboard => {
      let s2 = s.clone();
      let p = tokio::task::spawn_blocking(move || live::dashboard(&s2))
        .await
        .map_err(|e| Error::Config(e.to_string()))??;
      sync_home(s);
      Ok(Reply::Apps {
        apps: vec![info(s, &p)],
        launcher,
      })
    }
    _ => Err(Error::Config("not a request about apps".into())),
  }
}

/// A pinned app by its name (case aside), its ID, or its host.
pub fn find_pinned<'a>(pinned: &'a [Pinned], what: &str) -> Option<&'a Pinned> {
  pinned
    .iter()
    .find(|p| p.id.as_str() == what || p.host == what)
    .or_else(|| pinned.iter().find(|p| p.name.eq_ignore_ascii_case(what)))
}

/// A thread's app state: each key's current value (JSON text), from its
/// `set` records.
pub fn stored(index: &Index, thread: &RecordId) -> Result<BTreeMap<String, String>> {
  let sets: Vec<IndexedRecord> = index
    .sources(kinds::SET, "target", thread, None)?
    .into_iter()
    .filter(|r| {
      r.fields
        .get("field")
        .and_then(Value::as_str)
        .is_some_and(|f| f.starts_with(STATE))
    })
    .collect();
  let mut superseded = HashSet::new();
  for r in &sets {
    for e in index.edges_from(&r.id)? {
      if e.rel == "supersedes" {
        superseded.insert(e.dst.id.to_string());
      }
    }
  }
  // Sets come oldest first: of a key's heads, the latest stands.
  let mut out = BTreeMap::new();
  for r in sets {
    if superseded.contains(r.id.as_str()) {
      continue;
    }
    let key = r.fields["field"].as_str().unwrap_or("")[STATE.len()..].to_owned();
    match r.fields.get("value").and_then(Value::as_str) {
      Some(v) if !v.is_empty() => out.insert(key, v.to_owned()),
      _ => out.remove(&key),
    };
  }
  Ok(out)
}

/// The node's apps: pages' writes not yet written, what changed, and how
/// the desktop serves them.
#[derive(Default)]
pub(crate) struct Apps {
  /// Bumped when any app state may have changed (a page wrote, a sync
  /// brought records): pages waiting for changes look again.
  changed: tokio::sync::Notify,
  generation: std::sync::atomic::AtomicU64,
  /// Writes gathered, per thread: a key's value, or `None` removed.
  pending: Mutex<HashMap<RecordId, BTreeMap<String, Option<String>>>>,
  /// A thread's reports: when it was last told, and how many times.
  reports: Mutex<HashMap<RecordId, (Instant, u32)>>,
  /// Versions whose error was told already.
  errors: Mutex<HashSet<BlobHash>>,
  /// Errors told to each thread.
  fixes: Mutex<HashMap<RecordId, usize>>,
  /// Where this node serves apps to a browser (`http`).
  pub(crate) desktop: OnceLock<http::Desktop>,
  /// Live state for apps, and its changes (`live`).
  pub(crate) live: live::Live,
  /// App threads: owners of threads starting, rate limits (`threads`).
  pub(crate) family: threads::Family,
}

#[cfg(mobile)]
pub(crate) mod http {
  /// No server where no other process could reach it.
  pub(crate) struct Desktop;
  impl Desktop {
    pub(crate) fn url(&self, _: &str, _: &str) -> String {
      String::new()
    }
    pub(crate) fn address(&self, _: &str, _: &str) -> String {
      String::new()
    }
    pub(crate) fn admit(&self, _: &str) -> Option<String> {
      None
    }
  }
}

/// Reports to a thread at most this often, and this many times a run.
const REPORT_EVERY: Duration = Duration::from_secs(2);
const REPORTS: u32 = 50;

impl Apps {
  pub(crate) fn generation(&self) -> u64 {
    self.generation.load(std::sync::atomic::Ordering::SeqCst)
  }

  /// Something may have changed: pages waiting look again.
  pub(crate) fn bump(&self) {
    self
      .generation
      .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    self.changed.notify_waiters();
  }

  /// Waits until the generation is past `seen`, or `limit` passes.
  pub(crate) async fn wait(&self, seen: u64, limit: Duration) {
    let changed = self.changed.notified();
    if self.generation() != seen {
      return;
    }
    let _ = tokio::time::timeout(limit, changed).await;
  }

  /// A thread's state now: what is written, with what is gathered over it.
  pub(crate) fn state(&self, index: &Index, thread: &RecordId) -> Result<BTreeMap<String, String>> {
    let mut out = stored(index, thread)?;
    if let Some(p) = self.pending.lock().expect("pending lock").get(thread) {
      for (k, v) in p {
        match v {
          Some(v) => out.insert(k.clone(), v.clone()),
          None => out.remove(k),
        };
      }
    }
    Ok(out)
  }
}

/// A key a page may use.
fn good_key(k: &str) -> bool {
  !k.is_empty() && k.len() <= 256 && !k.chars().any(char::is_control)
}

/// Largest value a key holds, as JSON text.
const MAX_VALUE: usize = 256 * 1024;

/// A page writes `set` and removes `remove` from its thread's state:
/// gathered, then written and synced.
pub(crate) fn write(
  s: &Arc<Shared>,
  thread: &RecordId,
  set: BTreeMap<String, Value>,
  remove: Vec<String>,
) -> Result<()> {
  for k in set.keys().chain(&remove) {
    if !good_key(k) {
      return Err(Error::Config(format!("`{k}` is not a key a page may use")));
    }
  }
  let mut writes = BTreeMap::new();
  for (k, v) in set {
    let text = v.to_string();
    if text.len() > MAX_VALUE {
      return Err(Error::Config(format!(
        "`{k}`: {} bytes, more than a key holds ({MAX_VALUE})",
        text.len()
      )));
    }
    writes.insert(k, Some(text));
  }
  for k in remove {
    writes.insert(k, None);
  }
  if writes.is_empty() {
    return Ok(());
  }
  let first = {
    let mut pending = s.apps.pending.lock().expect("pending lock");
    let first = !pending.contains_key(thread);
    pending.entry(thread.clone()).or_default().extend(writes);
    first
  };
  s.apps.bump();
  if first {
    let (s, thread) = (s.clone(), thread.clone());
    tokio::spawn(async move {
      tokio::time::sleep(GATHER).await;
      flush(&s, &thread).await;
    });
  }
  Ok(())
}

/// Writes a thread's gathered state as `set` records, and syncs them.
async fn flush(s: &Arc<Shared>, thread: &RecordId) {
  let Some(writes) = s.apps.pending.lock().expect("pending lock").remove(thread) else {
    return;
  };
  let (s2, t) = (s.clone(), thread.clone());
  let written = tokio::task::spawn_blocking(move || -> Result<stitch_records::RepoId> {
    let stored = stored(&s2.state().index.lock().expect("index lock"), &t)?;
    for (k, v) in writes {
      // Unchanged keys write nothing.
      if stored.get(&k) == v.as_ref() {
        continue;
      }
      crate::meta::set(
        &s2,
        &t,
        &format!("{STATE}{k}"),
        v.unwrap_or_default().into(),
        Author::user(s2.state().node.clone()),
      )?;
    }
    Ok(s2.thread_place(&t)?.0.repo().clone())
  })
  .await;
  match written {
    Ok(Ok(repo)) => {
      s.apps.bump();
      crate::node::sync_one(s, &repo).await;
    }
    Ok(Err(e)) => eprintln!("apps: thread {thread}: state not written: {e}"),
    Err(e) => eprintln!("apps: thread {thread}: state not written: {e}"),
  }
}

/// What a page sends its thread's agent (`stitch.report`), not too often
/// (per thread, whichever version or pin sends it), as a message sent to
/// the thread (`tell`). `from` names the app as the agent reads it.
pub(crate) async fn report(
  s: &Arc<Shared>,
  thread: &RecordId,
  from: &str,
  data: &str,
) -> std::result::Result<String, (u16, String)> {
  {
    let mut reports = s.apps.reports.lock().expect("reports lock");
    let now = Instant::now();
    let r = reports
      .entry(thread.clone())
      .or_insert((now - REPORT_EVERY, 0));
    if now.duration_since(r.0) < REPORT_EVERY {
      return Err((429, "reports come at most every 2 seconds".into()));
    }
    if r.1 >= REPORTS {
      return Err((429, format!("this thread had {REPORTS} reports already")));
    }
    *r = (now, r.1 + 1);
  }
  let data: String = data.chars().take(4000).collect();
  tell(s, thread, format!("[{from} sent you data]\n{data}")).await
}

/// Tells a thread's agent what one of its apps says, as a message sent to
/// it (`weave::send` with `interject`): woven into the loop running on it,
/// else it starts a turn, a detached thread that reported included (a
/// note would not wake it); on the node that runs it, which may not be
/// the one serving the page (a phone's pinned app, of a thread on the
/// Mac): there as an assignment, synced to it at once. Where it went, or
/// why it could not go: 503 when that node could not be reached now (the
/// assignment waits for the next sync).
async fn tell(
  s: &Arc<Shared>,
  thread: &RecordId,
  text: String,
) -> std::result::Result<String, (u16, String)> {
  let (owner, sent) = crate::weave::send(s, None, thread, &text, true, true)
    .await
    .map_err(|e| (409, e.to_string()))?;
  if sent != crate::weave::Sent::Passed {
    return Ok(format!("told the thread here ({})", sent.said()));
  }
  let Ok((root, _)) = s.thread_place(thread) else {
    return Ok(format!("passed on to {owner}, which runs the thread"));
  };
  if crate::weave::reach(s, &owner, root.repo()).await {
    Ok(format!("passed on to {owner}, which runs the thread"))
  } else {
    Err((
      503,
      format!(
        "the thread runs on {owner}, which could not be reached now: it takes this when it \
         next syncs with this device"
      ),
    ))
  }
}

/// The latest version shown in a thread: its manifest and name.
pub(crate) fn latest_in(s: &Shared, thread: &RecordId) -> Result<Option<(BlobHash, String)>> {
  let (root, _) = s.thread_place(thread)?;
  let state = s.state();
  let ctx = crate::chat::ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let mut latest = None;
  for e in crate::chat::thread_events(&ctx, thread)? {
    if let stitch_records::Event::Msg(m) = e {
      for p in m.parts {
        if let stitch_records::Part::Attachment { att } = p
          && att.media_type == MEDIA
        {
          latest = Some((att.hash, att.name.unwrap_or_default()));
        }
      }
    }
  }
  Ok(latest)
}

/// Fixes of a failing app a thread is asked for, at most, while the node
/// runs (edge talk's lesson: an app failing again and again must not
/// spend without end).
const FIX_ROUNDS: usize = 2;

/// A page's first fatal error, told to its thread's agent once per
/// version, and only of the thread's latest version (opening an old one
/// further up the conversation, or a pin fixed at one, asks for nothing):
/// it may fix the app and show it again, which moves the pins that follow
/// it. `pinned`: it was opened as a pinned app.
pub(crate) fn error(s: &Arc<Shared>, manifest: &BlobHash, m: &Manifest, pinned: bool, error: &str) {
  let Some(thread) = &m.thread else { return };
  {
    let mut errors = s.apps.errors.lock().expect("errors lock");
    if errors.contains(manifest) {
      return;
    }
    if !matches!(latest_in(s, thread), Ok(Some((h, _))) if h == *manifest) {
      return;
    }
    let mut rounds = s.apps.fixes.lock().expect("fixes lock");
    let n = rounds.entry(thread.clone()).or_default();
    if *n >= FIX_ROUNDS {
      eprintln!(
        "apps: thread {thread}: app \"{}\" fails again; not asked to fix it",
        m.name
      );
      return;
    }
    *n += 1;
    errors.insert(*manifest);
  }
  let error: String = error.chars().take(2000).collect();
  let text = format!(
    "[App \"{}\" failed when the user opened it{}: fix it and show it again, or say why not]\n{error}",
    m.name,
    if pinned { " (pinned)" } else { "" }
  );
  let (s, thread) = (s.clone(), thread.clone());
  tokio::spawn(async move {
    if let Err((_, e)) = tell(&s, &thread, text).await {
      eprintln!("apps: thread {thread}: its app's error not told: {e}");
    }
  });
}

/// The `apps` tools.
pub(crate) fn definitions() -> Vec<ToolDef> {
  vec![ToolDef {
    name: "show_app".into(),
    description: "Show the user an app: HTML, CSS and JavaScript you wrote, from a directory \
      with index.html (its other files too: scripts, styles, images, data) or one HTML file. \
      It shows in the conversation, on the phone and in a browser on the user's computers, \
      and the user can pin it. Each call shows a new version: to change an app, edit its \
      files and show it again. Before writing one, read the `apps` skill (read_skill): what \
      pages can use (bundled libraries, saved state, reports to you) and how to make them \
      work."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "path": {"type": "string", "description": "The app's directory (with index.html) or HTML file: absolute, `~/…`, or relative."},
        "name": {"type": "string", "description": "A short name for it, the same for each version."},
        "networking": {"type": "boolean", "description": "Let its pages reach the network (fetch, external scripts, images, map tiles). Off by default: they reach only their own files and the bundled libraries."}
      },
      "required": ["path", "name"]
    }),
  }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Show {
  path: String,
  name: String,
  #[serde(default)]
  networking: bool,
}

/// `show_app`: a version of the app at `input.path`, attached to the tool
/// message.
pub(crate) async fn show(
  s: Option<Arc<Shared>>,
  cwd: PathBuf,
  thread: RecordId,
  input: Value,
) -> ToolOutcome {
  let Some(s) = s else {
    return ToolOutcome::err("the node is stopping");
  };
  let show: Show = match serde_json::from_value(input) {
    Ok(v) => v,
    Err(e) => return ToolOutcome::err(format!("show_app: {e}")),
  };
  let name = show.name.trim().to_owned();
  if name.is_empty() || name.chars().count() > 64 {
    return ToolOutcome::err("an app's name is a few words (at most 64 characters)");
  }
  let path = crate::place::expand_home(&show.path);
  let path = if path.is_absolute() {
    path
  } else {
    cwd.join(path)
  };
  let s2 = s.clone();
  let taken = tokio::task::spawn_blocking(move || {
    snapshot(
      &s2.state().blobs,
      &path,
      &name,
      show.networking,
      Some(&thread),
    )
  })
  .await;
  let v = match taken {
    Ok(Ok(v)) => v,
    Ok(Err(e)) => return ToolOutcome::err(e.to_string()),
    Err(e) => return ToolOutcome::err(e.to_string()),
  };
  let mut text = format!(
    "Shown: app \"{}\", version {} ({} files, {} KB). The user sees it in the conversation \
     (on the phone; a desktop client gets a link to open it in their browser, and `stitch app \
     open <thread>` opens the latest one a thread showed); it reports its first error to you, \
     if it fails when opened.",
    v.name,
    &v.manifest.hex()[..8],
    v.files,
    v.bytes.div_ceil(1024)
  );
  if let Some(d) = s.apps.desktop.get() {
    text.push_str(&format!(
      "\nOn this computer it is at {} (for browse_web or web_eval, to check it works: they get \
       in; the user's browser needs the link their client gives, so do not hand them this one).",
      d.address(&version_host(&v.manifest), "/")
    ));
  }
  // The pins that follow this thread's app move to it.
  let (s2, manifest) = (s.clone(), v.manifest);
  match tokio::task::spawn_blocking(move || follow(&s2, &manifest)).await {
    Ok(Ok(moved)) if !moved.is_empty() => {
      sync_home(&s);
      for p in moved {
        text.push_str(&format!(
          "\nThe pinned app \"{}\" moved to this version.",
          p.name
        ));
      }
    }
    Ok(Ok(_)) => {}
    Ok(Err(e)) => eprintln!("apps: pins not moved to {}: {e}", v.manifest),
    Err(e) => eprintln!("apps: pins not moved to {}: {e}", v.manifest),
  }
  let mut out = ToolOutcome::ok(text);
  out.attachments.push(Attachment {
    hash: v.manifest,
    media_type: MEDIA.into(),
    name: Some(v.name),
    bytes: v.bytes,
  });
  out.blobs = v.blobs;
  out
}

/// A skill of Stitch's own, read with `read_skill` (its files too); a
/// skill of the user's or the place's of the same name wins.
pub struct BuiltInSkill {
  pub name: &'static str,
  pub description: &'static str,
  pub text: &'static str,
  /// Its files, by the paths its SKILL.md gives.
  pub files: &'static [(&'static str, &'static str)],
  /// Its tools are lazy (design §2.17): not in the tool list, called with
  /// `Tool.Call` in `otter_run`, `read_skill` listing them (listed after
  /// all where the `otter` skill is off).
  pub lazy: bool,
}

/// Edge talk's skills for embeds, as apps: making them, and music.
pub const SKILLS: &[BuiltInSkill] = &[
  BuiltInSkill {
    name: "apps",
    description: "Making apps for the user (dashboards, tools, calculators, charts, games, 3D \
      models to print, documents to download): HTML, CSS and JavaScript shown with show_app, on \
      the phone and in a browser, with state that syncs across their devices.",
    text: include_str!("SKILL.md"),
    files: &[],
    lazy: false,
  },
  BuiltInSkill {
    name: "leadsheet",
    description: "Compose multitrack music as leadsheet (.ls) text: validate, render, and play \
      it in an app with an embedded MIDI player.",
    text: include_str!("skills/leadsheet/SKILL.md"),
    files: &[
      (
        "references/FORMAT.md",
        include_str!("skills/leadsheet/references/FORMAT.md"),
      ),
      (
        "references/pop.ls",
        include_str!("skills/leadsheet/references/pop.ls"),
      ),
      (
        "assets/player.html",
        include_str!("skills/leadsheet/assets/player.html"),
      ),
    ],
    lazy: false,
  },
  // Its tools are Otter scripts (`crate::otter`), offered where Otter is.
  BuiltInSkill {
    name: "google-flights",
    description: "Search Google Flights for flights between two airports on given dates, and \
      find the cheapest dates over a range, with the google_flights_search and \
      google_flights_cheap_dates tools.",
    text: include_str!("skills/google-flights/SKILL.md"),
    files: &[
      (
        "tools/flights.otr",
        include_str!("skills/google-flights/tools/flights.otr"),
      ),
      (
        "tools/search-flights.otr",
        include_str!("skills/google-flights/tools/search-flights.otr"),
      ),
      (
        "tools/cheap-dates.otr",
        include_str!("skills/google-flights/tools/cheap-dates.otr"),
      ),
    ],
    lazy: true,
  },
  // Its tool is an Otter script over YouTube's innertube API.
  BuiltInSkill {
    name: "youtube",
    description: "Watch a YouTube video by reading it: its details, timestamped transcript (in \
      the language asked for, or the video's own) and top comments, with the youtube_video tool. Use \
      it when the user shares a YouTube link or asks about a video.",
    text: include_str!("skills/youtube/SKILL.md"),
    files: &[
      (
        "tools/youtube.otr",
        include_str!("skills/youtube/tools/youtube.otr"),
      ),
      (
        "tools/video.otr",
        include_str!("skills/youtube/tools/video.otr"),
      ),
    ],
    lazy: true,
  },
  // Its tools are Otter scripts over edge talk's database, listed only
  // where it is (`crate::otter::edge_talk`).
  BuiltInSkill {
    name: "edge-talk",
    description: "The user's edge talk history (the chat app they used before Stitch): search \
      and read its chats and the memories it distilled, with edge_talk_search, \
      edge_talk_memories, edge_talk_conversations and edge_talk_read. Use it when the user \
      mentions edge talk or chats from before Stitch, or when recall finds nothing about older \
      history.",
    text: include_str!("skills/edge-talk/SKILL.md"),
    files: &[
      (
        "tools/edge-talk.otr",
        include_str!("skills/edge-talk/tools/edge-talk.otr"),
      ),
      (
        "tools/search.otr",
        include_str!("skills/edge-talk/tools/search.otr"),
      ),
      (
        "tools/memories.otr",
        include_str!("skills/edge-talk/tools/memories.otr"),
      ),
      (
        "tools/conversations.otr",
        include_str!("skills/edge-talk/tools/conversations.otr"),
      ),
      (
        "tools/read.otr",
        include_str!("skills/edge-talk/tools/read.otr"),
      ),
    ],
    lazy: true,
  },
  // Its tools are the language's own (`otter_search`, `otter_describe`,
  // `otter_check`, `otter_run`), offered while it is on.
  BuiltInSkill {
    name: "otter",
    description: "Write, check and run code in Otter, the language Stitch's tools are written \
      in, with otter_search, otter_describe, otter_check and otter_run, on any node (the phone \
      too); and write a skill's own tools in it.",
    text: include_str!("skills/otter/SKILL.md"),
    files: &[],
    lazy: false,
  },
  // Stitch fixing itself (docs/self-improve.md §2.3): the procedure of
  // the fixers `self_improve` starts, and of any thread working on Stitch.
  BuiltInSkill {
    name: "stitch-dev",
    description: "Working on Stitch's own code (a harness issue, a fix, an improvement), in \
      the core's repository or the app's beside it: worktrees (paired for the app), failing \
      test, just quick and check-apple, CI, landing on main (the core first for a fix in both), \
      and the rollout (canary, its 30-minute trial, then the fleet and TestFlight).",
    text: include_str!("skills/stitch-dev/SKILL.md"),
    files: &[],
    lazy: false,
  },
  // For meeting threads (the meeting bot's): the bot's brief names it.
  BuiltInSkill {
    name: "meeting",
    description: "Google Meet calls: have Stitch join one (\"join my call <url>\": `meet join` \
      on the node where meet.service is installed), and, for a meeting thread the meeting bot \
      made, answer its asks with event_reply (the Meet chat, follow mode's private suggestions), \
      what not to do, and the notes when the call ends.",
    text: include_str!("skills/meeting/SKILL.md"),
    files: &[],
    lazy: false,
  },
  // For the `browser` tools (design §2.18), offered while a browser is
  // connected: `Shared::skills` leaves it out of the list otherwise.
  BuiltInSkill {
    name: "browser",
    description: "Work in the user's own Chrome (their profile, logged in as them) with the \
      browser_* tools, beside the user in a tab group of your own: read, act, respect their \
      input, hand logins and payments to them.",
    text: include_str!("skills/browser/SKILL.md"),
    files: &[],
    lazy: false,
  },
];

/// Where Stitch's own skills are, as `read_skill` says it.
pub const BUILT_IN: &str = "stitch";

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hosts_name_versions_and_pinned_apps() {
    let h = BlobHash::of(b"manifest");
    let host = version_host(&h);
    assert_eq!(Host::parse(&host), Some(Host::Version(h)));
    assert_eq!(
      Host::parse(&format!("{}.localhost:7177", host.to_uppercase())),
      Some(Host::Version(h))
    );
    let id: RecordId = "2026-10-14T120000Z-mbp-k3f9".parse().unwrap();
    let p = pinned_host(&id);
    assert_eq!(Host::parse(&p), Some(Host::Pinned(p.clone())));
    assert_eq!(Host::parse("localhost:7177"), None);
    assert_eq!(Host::parse("example.com"), None);
  }

  #[test]
  fn a_directory_is_taken_whole_and_an_html_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
    let app = dir.path().join("app");
    std::fs::create_dir_all(app.join("js")).unwrap();
    std::fs::create_dir_all(app.join("node_modules/x")).unwrap();
    std::fs::write(app.join("index.html"), "<h1>hi</h1>").unwrap();
    std::fs::write(app.join("js/main.js"), "1").unwrap();
    std::fs::write(app.join(".secret"), "no").unwrap();
    std::fs::write(app.join("node_modules/x/y.js"), "no").unwrap();
    let v = snapshot(&blobs, &app, "Hi", false, None).unwrap();
    assert_eq!(v.files, 2);
    let m = Manifest::read(&blobs, &v.manifest).unwrap();
    assert_eq!(m.name, "Hi");
    let flat = crate::folders::load_tree(&blobs, &m.tree).unwrap();
    assert_eq!(
      flat.keys().collect::<Vec<_>>(),
      vec!["index.html", "js/main.js"]
    );
    for h in &v.blobs {
      assert!(blobs.contains(h));
    }

    let one = snapshot(&blobs, &app.join("index.html"), "One", true, None).unwrap();
    let m = Manifest::read(&blobs, &one.manifest).unwrap();
    assert!(m.networking);
    assert_eq!(
      crate::folders::load_tree(&blobs, &m.tree)
        .unwrap()
        .keys()
        .collect::<Vec<_>>(),
      vec!["index.html"]
    );
    assert!(snapshot(&blobs, &app.join("js/main.js"), "No", false, None).is_err());
    assert!(snapshot(&blobs, &app.join("js"), "No", false, None).is_err());
  }
}
