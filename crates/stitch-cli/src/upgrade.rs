//! Builds kept, health and rollback (docs/self-improve.md §2.1, steps 4
//! and 6): each build a node runs is kept in `<data>/bin/<slug>/`; a new
//! build is on trial until it has run `healthy_after()` and answers on its
//! socket; one that fails to start `MAX_ATTEMPTS` times, ends with an error
//! before it is healthy, or hangs at its start, is replaced by the last
//! healthy build, installed back where the binary is, unless the store's
//! format differs between them (then nothing is rolled back: the user is
//! asked, by the log and a notice once a node runs).
//!
//! A build is known by the hash of its binary (`Id`): the commits it was
//! built from (`STITCH_BUILD`) say what it is, not which file it is (an
//! Otter- or engram-only change, two `-dirty` builds of one commit).
//!
//! `<data>/bin/state.json` holds what is known: the healthy build, the one
//! before it, the one on trial and its attempts, builds that failed (never
//! restarted into again), and an event to tell the user.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How long a new build runs before it counts as healthy (60 s;
/// `STITCH_HEALTHY_AFTER_SECS` for tests).
pub fn healthy_after() -> Duration {
  let secs = std::env::var("STITCH_HEALTHY_AFTER_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(60);
  Duration::from_secs(secs)
}

/// How long past `healthy_after()` a build on trial may go without
/// answering on its socket before it gives up (exits with an error, so the
/// healthy build comes back): a build that hangs at its start.
pub fn trial_grace() -> Duration {
  let secs = std::env::var("STITCH_TRIAL_GRACE_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(120);
  Duration::from_secs(secs)
}

/// Starts of a build on trial that did not get it healthy, before the
/// healthy build comes back.
pub const MAX_ATTEMPTS: u32 = 3;

/// Builds kept besides the healthy one, the one before it and the one on
/// trial.
const KEEP: usize = 2;

/// Which build a binary is: the hash of its contents, and what it says it
/// was built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Id {
  /// blake3 of the file, hex.
  pub hash: String,
  /// `STITCH_BUILD`: commits, for people.
  pub label: String,
}

impl Id {
  /// The directory name: the commit, then the start of the hash.
  pub fn slug(&self) -> String {
    slug(&self.label, &self.hash)
  }
}

/// `<commit>-<hash, 12>`: the commit for people, the hash for identity.
pub fn slug(label: &str, hash: &str) -> String {
  let head: String = label
    .split_whitespace()
    .next()
    .unwrap_or("unknown")
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
    .take(24)
    .collect();
  let head = if head.is_empty() {
    "unknown".into()
  } else {
    head
  };
  format!("{head}-{}", &hash[..hash.len().min(12)])
}

/// The hash of a file's contents (blake3, hex).
pub fn hash_file(path: &Path) -> std::io::Result<String> {
  hash_reader(std::fs::File::open(path)?)
}

fn hash_reader(mut f: impl Read) -> std::io::Result<String> {
  let mut h = blake3::Hasher::new();
  let mut buf = vec![0u8; 1 << 20];
  loop {
    let n = f.read(&mut buf)?;
    if n == 0 {
      break;
    }
    h.update(&buf[..n]);
  }
  Ok(h.finalize().to_hex().to_string())
}

/// Where this process's binary is installed: `current_exe`, without the
/// " (deleted)" Linux gives it once the file was replaced by another
/// build (an install renames a new file over it). With that suffix the
/// path names no file: a node that started before an install never saw
/// the build installed (incident 2026-11-02), and a restart could not
/// start it.
pub fn installed_exe() -> std::io::Result<PathBuf> {
  let exe = std::env::current_exe()?;
  Ok(strip_deleted(exe))
}

fn strip_deleted(exe: PathBuf) -> PathBuf {
  if exe.exists() {
    return exe;
  }
  match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
    Some(s) => PathBuf::from(s),
    None => exe,
  }
}

/// The image this process runs, open: what it reads stays the file this
/// process was started from after an install renamed another build over
/// its path (a node waiting for the lock meanwhile: incident 2026-11-02).
pub struct Image {
  file: std::fs::File,
  /// What it was opened as, for messages.
  path: PathBuf,
}

impl Image {
  /// The file at `path`.
  pub fn open(path: &Path) -> std::io::Result<Image> {
    Ok(Image {
      file: std::fs::File::open(path)?,
      path: path.to_owned(),
    })
  }

  /// Its contents from the start, read at offsets (the open file's own
  /// position is not used, so one read never moves another's).
  fn reader(&self) -> impl Read + '_ {
    struct At<'a>(&'a std::fs::File, u64);
    impl Read for At<'_> {
      fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::os::unix::fs::FileExt;
        let n = self.0.read_at(buf, self.1)?;
        self.1 += n as u64;
        Ok(n)
      }
    }
    At(&self.file, 0)
  }

  pub fn hash(&self) -> std::io::Result<String> {
    hash_reader(self.reader())
  }
}

/// The image this process runs, `kept` the folder of the builds kept
/// (`<data>/bin`). Linux: `/proc/self/exe`, the running file whatever its
/// path became. macOS has no such path, and an install may rename another
/// build over the binary before this process's first instruction (a debug
/// build's start takes seconds there), so the file is found by its
/// identity: the device and inode its code is mapped from (`mapped_image`).
/// That is the file `current_exe` names, when no install replaced it;
/// else the build kept as a hard link to it (`Builds::keep` links the
/// installed file when it is the build that runs, as the node before this
/// one did). `Err` when neither is (a build no node here ran and kept).
pub fn own_image(kept: &Path) -> std::io::Result<Image> {
  #[cfg(target_os = "linux")]
  {
    let _ = kept;
    Image::open(Path::new("/proc/self/exe"))
  }
  #[cfg(target_os = "macos")]
  {
    use std::os::unix::fs::MetadataExt;
    let running = mapped_image().ok_or_else(|| {
      std::io::Error::other("the file this process runs is not known (proc_pidinfo)")
    })?;
    // Opened, then its identity read from the open file: what is read is
    // what was checked.
    let is_it = |path: &Path| {
      let image = Image::open(path).ok()?;
      let m = image.file.metadata().ok()?;
      ((m.dev() as u32, m.ino()) == running).then_some(image)
    };
    let exe = std::env::current_exe()?;
    if let Some(image) = is_it(&exe) {
      return Ok(image);
    }
    let found = std::fs::read_dir(kept)
      .into_iter()
      .flatten()
      .flatten()
      .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
      .find_map(|e| is_it(&e.path().join("stitch")));
    if let Some(image) = &found {
      eprintln!(
        "upgrade: another build was installed since this process started: it runs {}",
        image.path.display()
      );
    }
    found.ok_or_else(|| {
      std::io::Error::other(format!(
        "{} is not the file this process runs any more (another build was installed since \
         it started), and no build kept in {} is",
        exe.display(),
        kept.display()
      ))
    })
  }
  #[cfg(not(any(target_os = "linux", target_os = "macos")))]
  {
    let _ = kept;
    Image::open(&std::env::current_exe()?)
  }
}

/// The device and inode of the file this process's code is mapped from:
/// the region of `proc_pidinfo(PROC_PIDREGIONPATHINFO)` holding this very
/// function (Rust links the node's code into its executable), which names
/// its file's vnode however that file's path changed since.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn mapped_image() -> Option<(u32, u64)> {
  /// `struct proc_regioninfo` (`<sys/proc_info.h>`), which `libc` lacks
  /// (the kernel fills it; two of its fields are read).
  #[allow(dead_code)]
  #[repr(C)]
  struct RegionInfo {
    pri_protection: u32,
    pri_max_protection: u32,
    pri_inheritance: u32,
    pri_flags: u32,
    pri_offset: u64,
    pri_behavior: u32,
    pri_user_wired_count: u32,
    pri_user_tag: u32,
    pri_pages_resident: u32,
    pri_pages_shared_now_private: u32,
    pri_pages_swapped_out: u32,
    pri_pages_dirtied: u32,
    pri_ref_count: u32,
    pri_shadow_depth: u32,
    pri_share_mode: u32,
    pri_private_pages_resident: u32,
    pri_shared_pages_resident: u32,
    pri_obj_id: u32,
    pri_depth: u32,
    pri_address: u64,
    pri_size: u64,
  }
  /// `struct proc_regionwithpathinfo`.
  #[repr(C)]
  struct RegionWithPath {
    prp_prinfo: RegionInfo,
    prp_vip: libc::vnode_info_path,
  }
  const PROC_PIDREGIONPATHINFO: libc::c_int = 8;
  let here = mapped_image as fn() -> Option<(u32, u64)> as usize as u64;
  let size = std::mem::size_of::<RegionWithPath>() as libc::c_int;
  let mut info = std::mem::MaybeUninit::<RegionWithPath>::zeroed();
  // SAFETY: the buffer is `size` bytes of a zeroed `repr(C)` struct the
  // kernel fills (all its fields are plain integers); it returns how many
  // bytes it wrote.
  let n = unsafe {
    libc::proc_pidinfo(
      libc::getpid(),
      PROC_PIDREGIONPATHINFO,
      here,
      info.as_mut_ptr().cast(),
      size,
    )
  };
  if n != size {
    return None;
  }
  // SAFETY: written whole by the kernel (`n == size`), from zeroes.
  let info = unsafe { info.assume_init() };
  let r = &info.prp_prinfo;
  // The region at or after the address asked: it must hold it.
  if !(r.pri_address <= here && here < r.pri_address.saturating_add(r.pri_size)) {
    return None;
  }
  let st = &info.prp_vip.vip_vi.vi_stat;
  (st.vst_ino != 0).then_some((st.vst_dev, st.vst_ino))
}

/// Whether a binary is a build output (`…/target/…`): not kept (tests and
/// `cargo run` would copy hundreds of MB each start), so never rolled back to.
pub fn is_build_output(exe: &Path) -> bool {
  let name = |p: Option<&Path>| {
    p.and_then(Path::file_name)
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_default()
  };
  let parent = exe.parent();
  let profile = |n: &str| matches!(n, "debug" | "release");
  // `target/<profile>/stitch`, `target/<triple>/<profile>/stitch`, and
  // `…/<profile>/deps/stitch-<hash>`.
  profile(&name(parent))
    || (name(parent) == "deps" && profile(&name(parent.and_then(Path::parent))))
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
  /// The last build that ran healthy here (a slug).
  pub healthy: Option<String>,
  /// When it became the healthy one (RFC 3339): the rollout's trial
  /// counts from there (`stitch node builds --healthy-for`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub healthy_since: Option<String>,
  /// The healthy one before it.
  pub previous: Option<String>,
  /// The build on trial, and how many times it started.
  pub trying: Option<String>,
  #[serde(default)]
  pub attempts: u32,
  /// Builds that failed here: never restarted into again.
  #[serde(default)]
  pub failed: Vec<String>,
  /// What to tell the user once a node runs (a rollback, a refusal).
  pub event: Option<String>,
}

pub struct Builds {
  dir: PathBuf,
}

fn sync_dir(dir: &Path) {
  if let Ok(d) = std::fs::File::open(dir) {
    let _ = d.sync_all();
  }
}

/// Copies `from` to `to`, its data on disk before it returns.
fn copy_synced(from: &Path, to: &Path) -> Result<()> {
  let mode = std::fs::metadata(from)?.permissions();
  write_synced(&mut std::fs::File::open(from)?, mode, to)
}

/// Writes what `src` reads to `to`, with permissions `mode`, its data on
/// disk before it returns.
fn write_synced(src: &mut impl Read, mode: std::fs::Permissions, to: &Path) -> Result<()> {
  let mut dst = std::fs::File::create(to)?;
  std::io::copy(src, &mut dst)?;
  dst.flush()?;
  dst.sync_all()?;
  std::fs::set_permissions(to, mode)?;
  Ok(())
}

impl Builds {
  pub fn new(data: &Path) -> Builds {
    Builds {
      dir: data.join("bin"),
    }
  }

  /// Where the builds are kept, one folder each.
  pub fn dir(&self) -> &Path {
    &self.dir
  }

  fn state_path(&self) -> PathBuf {
    self.dir.join("state.json")
  }

  /// What is known; a state that cannot be read is kept aside
  /// (`state.json.bad`) and said, and the node starts from nothing.
  pub fn state(&self) -> State {
    let path = self.state_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
      return State::default();
    };
    match serde_json::from_str(&text) {
      Ok(s) => s,
      Err(e) => {
        let bad = self.dir.join("state.json.bad");
        let _ = std::fs::rename(&path, &bad);
        eprintln!(
          "builds: {} could not be read ({e}): kept as {}, starting from nothing",
          path.display(),
          bad.display()
        );
        State::default()
      }
    }
  }

  pub fn save(&self, state: &State) -> Result<()> {
    std::fs::create_dir_all(&self.dir)?;
    let tmp = self.dir.join("state.json.tmp");
    {
      let mut f = std::fs::File::create(&tmp)?;
      f.write_all(serde_json::to_string_pretty(state)?.as_bytes())?;
      f.sync_all()?;
    }
    std::fs::rename(tmp, self.state_path())?;
    sync_dir(&self.dir);
    Ok(())
  }

  pub fn path(&self, slug: &str) -> PathBuf {
    self.dir.join(slug)
  }

  /// The store format a kept build needs.
  pub fn format(&self, slug: &str) -> Option<u32> {
    std::fs::read_to_string(self.path(slug).join("format"))
      .ok()?
      .trim()
      .parse()
      .ok()
  }

  /// The store format of a build that can be rolled back to: kept whole
  /// (`None` for one damaged since, or not kept: a rollback to it would
  /// fail and start the failed build again, for ever).
  pub fn kept_format(&self, slug: &str) -> Option<u32> {
    self.is_kept(slug).then(|| self.format(slug)).flatten()
  }

  /// Whether `slug` is kept whole: its binary there, with the hash it
  /// was kept with.
  pub fn is_kept(&self, slug: &str) -> bool {
    let dir = self.path(slug);
    let Ok(want) = std::fs::read_to_string(dir.join("hash")) else {
      return false;
    };
    hash_file(&dir.join("stitch")).is_ok_and(|h| h == want.trim())
  }

  /// Keeps the running build `id` (its image `image`; the shell helper
  /// beside `exe`) once: copied, synced and checked against its hash
  /// before it takes its name. Folders left by a keep that was cut short
  /// go.
  pub fn keep(&self, id: &Id, image: &Image, exe: &Path, format: u32) -> Result<()> {
    std::fs::create_dir_all(&self.dir)?;
    if let Ok(entries) = std::fs::read_dir(&self.dir) {
      for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with('.') && n.ends_with(".tmp") {
          let _ = std::fs::remove_dir_all(e.path());
        }
      }
    }
    let slug = id.slug();
    if self.is_kept(&slug) {
      return Ok(());
    }
    let tmp = self.dir.join(format!(".{slug}.tmp"));
    std::fs::create_dir_all(&tmp)?;
    // A hard link to the installed file when it is still this build (no
    // copy: an install puts a new file in its place, and this one stays as
    // it is); else a copy of the running image.
    let linked = std::fs::hard_link(exe, tmp.join("stitch")).is_ok()
      && hash_file(&tmp.join("stitch")).is_ok_and(|h| h == id.hash);
    if !linked {
      let _ = std::fs::remove_file(tmp.join("stitch"));
      let mode = image.file.metadata()?.permissions();
      write_synced(&mut image.reader(), mode, &tmp.join("stitch"))?;
    }
    let copied = hash_file(&tmp.join("stitch"))?;
    if copied != id.hash {
      let _ = std::fs::remove_dir_all(&tmp);
      return Err(
        format!(
          "the copy of {} is not the running build (it changed meanwhile)",
          image.path.display()
        )
        .into(),
      );
    }
    if let Some(helper) = exe.parent().map(|d| d.join("stitch-shell-helper"))
      && helper.exists()
    {
      copy_synced(&helper, &tmp.join("stitch-shell-helper"))?;
    }
    std::fs::write(tmp.join("build"), &id.label)?;
    std::fs::write(tmp.join("hash"), &id.hash)?;
    std::fs::write(tmp.join("format"), format.to_string())?;
    sync_dir(&tmp);
    let dir = self.path(&slug);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&tmp, &dir)?;
    sync_dir(&self.dir);
    Ok(())
  }

  /// Installs kept build `slug` where `exe` is (with its helper), each file
  /// written beside, synced and renamed over, so a running process keeps
  /// its own. A kept build that is not whole any more is refused.
  pub fn install(&self, slug: &str, exe: &Path) -> Result<()> {
    if !self.is_kept(slug) {
      return Err(format!("build {slug} is not kept whole here: not installed").into());
    }
    let dir = self.path(slug);
    let put = |from: &Path, to: &Path| -> Result<()> {
      let tmp = to.with_extension("rollback");
      copy_synced(from, &tmp)?;
      std::fs::rename(&tmp, to)?;
      if let Some(d) = to.parent() {
        sync_dir(d);
      }
      Ok(())
    };
    let helper = dir.join("stitch-shell-helper");
    if helper.exists()
      && let Some(d) = exe.parent()
    {
      put(&helper, &d.join("stitch-shell-helper"))?;
    }
    put(&dir.join("stitch"), exe)?;
    Ok(())
  }

  /// Removes kept builds but the healthy, the previous, the one on trial
  /// and the `KEEP` newest others.
  pub fn prune(&self, state: &State) {
    let Ok(entries) = std::fs::read_dir(&self.dir) else {
      return;
    };
    let mut others: Vec<(std::time::SystemTime, PathBuf)> = entries
      .flatten()
      .filter(|e| e.path().is_dir())
      .filter(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        !n.starts_with('.')
          && Some(&n) != state.healthy.as_ref()
          && Some(&n) != state.previous.as_ref()
          && Some(&n) != state.trying.as_ref()
      })
      .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
      .collect();
    others.sort();
    let n = others.len().saturating_sub(KEEP);
    for (_, p) in others.into_iter().take(n) {
      let _ = std::fs::remove_dir_all(p);
    }
  }
}

/// What starting build `me` decides.
#[derive(Debug, PartialEq)]
pub enum Start {
  /// Start it (healthy already, or on trial).
  Run,
  /// Install the healthy build back and start that.
  RollBack { to: String, why: String },
}

/// The decision at the start of build `me` (its format `format`), with
/// the state updated (its attempt counted). Pure, for the tests.
pub fn starting(
  state: &mut State,
  me: &str,
  format: u32,
  format_of: impl Fn(&str) -> Option<u32>,
) -> Start {
  if state.healthy.as_deref() == Some(me) {
    state.trying = None;
    state.attempts = 0;
    return Start::Run;
  }
  // A build that failed here before (started again by a reboot, a
  // restart, a client): the healthy one comes back at once.
  if state.failed.iter().any(|f| f == me) {
    let why = format!("build {me} failed here before");
    return rollback_to(state, me, format, format_of, why);
  }
  if state.trying.as_deref() == Some(me) {
    state.attempts += 1;
  } else {
    state.trying = Some(me.to_owned());
    state.attempts = 1;
  }
  if state.attempts <= MAX_ATTEMPTS {
    return Start::Run;
  }
  let why = format!(
    "build {me} started {} times without getting healthy",
    state.attempts - 1
  );
  rollback_to(state, me, format, format_of, why)
}

/// Build `me` stopped because it was asked to (a stop, a restart, a
/// hand-over): that start does not count against it.
pub fn stopped_cleanly(state: &mut State, me: &str) {
  if state.trying.as_deref() == Some(me) {
    state.attempts = state.attempts.saturating_sub(1);
  }
}

/// Whether to go back from `me` to the healthy build, and the state for it.
pub fn rollback_to(
  state: &mut State,
  me: &str,
  format: u32,
  format_of: impl Fn(&str) -> Option<u32>,
  why: String,
) -> Start {
  if !state.failed.iter().any(|f| f == me) {
    state.failed.push(me.to_owned());
  }
  let Some(to) = state.healthy.clone().filter(|h| h != me) else {
    state.event = Some(format!("{why}; no healthy build to go back to"));
    state.failed.retain(|f| f != me);
    return Start::Run;
  };
  match format_of(&to) {
    Some(f) if f == format => {
      state.trying = None;
      state.attempts = 0;
      state.event = Some(format!("{why}: rolled back to {to}"));
      Start::RollBack { to, why }
    }
    Some(f) => {
      state.event = Some(format!(
        "{why}; not rolled back to {to}: its store format ({f}) is not this one's ({format}). It needs you"
      ));
      // Counted no further: it is the user's now.
      state.failed.retain(|f| f != me);
      state.trying = Some(me.to_owned());
      state.attempts = MAX_ATTEMPTS;
      Start::Run
    }
    None => {
      state.event = Some(format!(
        "{why}; build {to} is not kept here: not rolled back"
      ));
      state.failed.retain(|f| f != me);
      state.trying = Some(me.to_owned());
      state.attempts = MAX_ATTEMPTS;
      Start::Run
    }
  }
}

/// Build `me` ran healthy: it is the healthy one now.
pub fn healthy(state: &mut State, me: &str) {
  if state.healthy.as_deref() != Some(me) {
    state.previous = state.healthy.take();
    state.healthy = Some(me.to_owned());
    state.healthy_since = Some(jiff::Timestamp::now().to_string());
  }
  state.failed.retain(|f| f != me);
  if state.trying.as_deref() == Some(me) {
    state.trying = None;
    state.attempts = 0;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fmt(_: &str) -> Option<u32> {
    Some(1)
  }

  #[test]
  fn a_replaced_binary_is_its_path_without_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("stitch");
    std::fs::write(&exe, "x").unwrap();
    let deleted = PathBuf::from(format!("{} (deleted)", exe.display()));
    assert_eq!(strip_deleted(deleted), exe);
    assert_eq!(strip_deleted(exe.clone()), exe);
  }

  #[test]
  fn a_new_build_runs_on_trial_then_is_healthy() {
    let mut s = State::default();
    assert_eq!(starting(&mut s, "a", 1, fmt), Start::Run);
    healthy(&mut s, "a");
    assert_eq!(s.healthy.as_deref(), Some("a"));
    assert_eq!(starting(&mut s, "b", 1, fmt), Start::Run);
    assert_eq!((s.trying.as_deref(), s.attempts), (Some("b"), 1));
    healthy(&mut s, "b");
    assert_eq!(
      (s.healthy.as_deref(), s.previous.as_deref()),
      (Some("b"), Some("a"))
    );
    assert_eq!(s.trying, None);
    assert_eq!(starting(&mut s, "b", 1, fmt), Start::Run);
  }

  #[test]
  fn a_build_that_keeps_failing_gives_way_to_the_healthy_one() {
    let mut s = State::default();
    healthy(&mut s, "a");
    for _ in 0..MAX_ATTEMPTS {
      assert_eq!(starting(&mut s, "b", 1, fmt), Start::Run);
    }
    match starting(&mut s, "b", 1, fmt) {
      Start::RollBack { to, .. } => assert_eq!(to, "a"),
      other => panic!("{other:?}"),
    }
    assert_eq!(s.failed, ["b"]);
    assert!(s.event.as_deref().unwrap().contains("rolled back to a"));
    // The healthy build starts as healthy.
    assert_eq!(starting(&mut s, "a", 1, fmt), Start::Run);
  }

  #[test]
  fn a_failed_build_started_again_rolls_back_at_once() {
    let mut s = State::default();
    healthy(&mut s, "a");
    s.failed.push("b".into());
    match starting(&mut s, "b", 1, fmt) {
      Start::RollBack { to, why } => {
        assert_eq!(to, "a");
        assert!(why.contains("failed here before"), "{why}");
      }
      other => panic!("{other:?}"),
    }
  }

  #[test]
  fn stops_asked_for_do_not_count_against_a_build_on_trial() {
    let mut s = State::default();
    healthy(&mut s, "a");
    // Started and stopped by request ten times (`node stop`, restarts):
    // still on trial, never rolled back.
    for _ in 0..10 {
      assert_eq!(starting(&mut s, "b", 1, fmt), Start::Run);
      stopped_cleanly(&mut s, "b");
    }
    assert_eq!((s.trying.as_deref(), s.attempts), (Some("b"), 0));
    assert!(s.failed.is_empty());
  }

  #[test]
  fn no_rollback_across_a_store_format() {
    let mut s = State::default();
    healthy(&mut s, "a");
    let old = |b: &str| Some(if b == "a" { 1 } else { 2 });
    for _ in 0..MAX_ATTEMPTS {
      starting(&mut s, "b", 2, old);
    }
    assert_eq!(starting(&mut s, "b", 2, old), Start::Run);
    assert!(s.event.as_deref().unwrap().contains("needs you"));
    assert!(s.failed.is_empty());
    // Not counted again and again.
    assert_eq!(starting(&mut s, "b", 2, old), Start::Run);
    assert_eq!(s.attempts, MAX_ATTEMPTS);
  }

  #[test]
  fn nothing_to_roll_back_to_runs_on() {
    let mut s = State::default();
    for _ in 0..=MAX_ATTEMPTS {
      assert_eq!(starting(&mut s, "b", 1, fmt), Start::Run);
    }
    assert!(s.event.as_deref().unwrap().contains("no healthy build"));
    assert!(s.failed.is_empty());
  }

  #[test]
  fn slugs_name_the_commit_and_the_binary() {
    let label = "fae3f9276faa-dirty otter 57366f8e1ebe engram 981299b9c31a";
    assert_eq!(
      slug(label, "0123456789abcdef0123"),
      "fae3f9276faa-dirty-0123456789ab"
    );
    // One commit, two binaries (an Otter-only change, two dirty builds):
    // two builds.
    assert_ne!(
      slug(label, "aaaaaaaaaaaaaaaa"),
      slug(label, "bbbbbbbbbbbbbbbb")
    );
    assert_eq!(slug("", "ab"), "unknown-ab");
  }

  fn tree() -> (tempfile::TempDir, Builds, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let exe_dir = t.path().join("bin");
    std::fs::create_dir_all(&exe_dir).unwrap();
    let exe = exe_dir.join("stitch");
    std::fs::write(&exe, b"build one").unwrap();
    std::fs::write(exe_dir.join("stitch-shell-helper"), b"helper one").unwrap();
    let b = Builds::new(&t.path().join("data"));
    (t, b, exe)
  }

  fn id_of(exe: &Path, label: &str) -> Id {
    Id {
      hash: hash_file(exe).unwrap(),
      label: label.into(),
    }
  }

  #[test]
  fn builds_are_kept_checked_and_installed_back() {
    let (_t, b, exe) = tree();
    let one = id_of(&exe, "c1 otter o1");
    // A keep cut short leaves a folder that goes at the next.
    std::fs::create_dir_all(b.dir.join(".x.tmp")).unwrap();
    b.keep(&one, &Image::open(&exe).unwrap(), &exe, 1).unwrap();
    assert!(!b.dir.join(".x.tmp").exists());
    assert!(b.is_kept(&one.slug()));
    assert_eq!(b.format(&one.slug()), Some(1));

    // Another build of the same commits (Otter changed under it).
    // Installed as installs do: a new file renamed over (the kept build
    // one, a hard link to the old file, stays as it was).
    let new = exe.with_extension("new");
    std::fs::write(&new, b"build two").unwrap();
    std::fs::rename(&new, &exe).unwrap();
    std::fs::write(exe.with_file_name("stitch-shell-helper"), b"helper two").unwrap();
    assert!(b.is_kept(&one.slug()));
    let two = id_of(&exe, "c1 otter o1");
    assert_ne!(one.slug(), two.slug());
    b.keep(&two, &Image::open(&exe).unwrap(), &exe, 1).unwrap();

    b.install(&one.slug(), &exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"build one");
    assert_eq!(
      std::fs::read(exe.with_file_name("stitch-shell-helper")).unwrap(),
      b"helper one"
    );

    // A kept binary damaged since is never installed, nor rolled back to.
    std::fs::write(b.path(&two.slug()).join("stitch"), b"garbage").unwrap();
    assert!(!b.is_kept(&two.slug()));
    assert!(b.install(&two.slug(), &exe).is_err());
    assert_eq!(std::fs::read(&exe).unwrap(), b"build one");
    assert_eq!(b.kept_format(&one.slug()), Some(1));
    assert_eq!(b.kept_format(&two.slug()), None);
    let mut s = State::default();
    healthy(&mut s, &two.slug());
    for _ in 0..MAX_ATTEMPTS {
      starting(&mut s, "three", 1, |x| b.kept_format(x));
    }
    assert_eq!(
      starting(&mut s, "three", 1, |x| b.kept_format(x)),
      Start::Run
    );
    assert!(
      s.event.as_deref().unwrap().contains("not kept here"),
      "{s:?}"
    );
  }

  #[test]
  fn build_outputs_are_told_by_their_profile_folder() {
    for p in [
      "/w/stitch/target/debug/stitch",
      "/w/stitch/target/release/stitch",
      "/w/t/aarch64-apple-darwin/release/stitch",
      "/w/stitch/target/debug/deps/stitch-0123abcd",
    ] {
      assert!(is_build_output(Path::new(p)), "{p}");
    }
    for p in [
      "/home/u/.cargo/bin/stitch",
      "/w/stitch/target/tmp/stitch-hang-1/bin/stitch",
    ] {
      assert!(!is_build_output(Path::new(p)), "{p}");
    }
  }

  /// This process's image is found (on macOS: the file its code is mapped
  /// from is the one `current_exe` names) and is that file.
  #[test]
  fn the_own_image_is_the_running_file() {
    let image = own_image(Path::new("/nonexistent")).unwrap();
    assert_eq!(
      image.hash().unwrap(),
      hash_file(&std::env::current_exe().unwrap()).unwrap()
    );
  }

  /// An image opened stays the build it was once another is renamed over
  /// its path, and is read whole however many times.
  #[test]
  fn an_open_image_outlives_an_install() {
    let (_t, b, exe) = tree();
    let one = id_of(&exe, "c1");
    let image = Image::open(&exe).unwrap();
    let new = exe.with_extension("new");
    std::fs::write(&new, b"build two").unwrap();
    std::fs::rename(&new, &exe).unwrap();
    assert_eq!(image.hash().unwrap(), one.hash);
    assert_eq!(image.hash().unwrap(), one.hash);
    b.keep(&one, &image, &exe, 1).unwrap();
    assert!(b.is_kept(&one.slug()));
  }

  #[test]
  fn a_keep_of_a_changed_image_is_refused() {
    let (_t, b, exe) = tree();
    let mut id = id_of(&exe, "c1");
    id.hash = "0".repeat(64);
    assert!(b.keep(&id, &Image::open(&exe).unwrap(), &exe, 1).is_err());
    assert!(!b.path(&id.slug()).exists());
  }

  #[test]
  fn prune_keeps_the_named_and_the_newest() {
    let (_t, b, _) = tree();
    for (i, n) in ["h", "p", "t", "o1", "o2", "o3", "o4"].iter().enumerate() {
      let d = b.path(n);
      std::fs::create_dir_all(&d).unwrap();
      let f = std::fs::File::open(&d).unwrap();
      f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(100 + i as u64))
        .unwrap();
    }
    let s = State {
      healthy: Some("h".into()),
      previous: Some("p".into()),
      trying: Some("t".into()),
      ..State::default()
    };
    b.prune(&s);
    let mut left: Vec<String> = std::fs::read_dir(&b.dir)
      .unwrap()
      .flatten()
      .map(|e| e.file_name().to_string_lossy().into_owned())
      .collect();
    left.sort();
    assert_eq!(left, ["h", "o3", "o4", "p", "t"]);
  }

  #[test]
  fn a_state_that_cannot_be_read_is_kept_aside() {
    let (_t, b, _) = tree();
    std::fs::create_dir_all(&b.dir).unwrap();
    std::fs::write(b.state_path(), "{ not json").unwrap();
    assert_eq!(b.state(), State::default());
    assert!(b.dir.join("state.json.bad").exists());
    let s = State {
      healthy: Some("h".into()),
      ..State::default()
    };
    b.save(&s).unwrap();
    assert_eq!(b.state(), s);
  }
}
