//! What the hazard guard shows the model besides the command: the files
//! the command runs (design §2.28). Measured: a hazard hidden in a
//! program was caught 5 times in 28 from the command alone, 27 in 28 with
//! the files it runs attached. So the guard gathers, in this order: files
//! the line names (`python3 x.py`, `bash x.sh`, `./x`, `$(cat x)`), the
//! recipe a `just` or `make` runs, `package.json` for npm, yarn, pnpm and
//! bun, for cargo the `build.rs` and the sources of what it builds and
//! runs and the Rust sources changed lately, `conftest.py` for pytest, the
//! files the thread wrote this turn, and one hop on: modules, scripts and
//! config files those name. Within a cap, the input says what was cut.
//!
//! Gathering runs nothing (no git, no program the checkout could
//! configure: it reads files only) and reads through one bounded reader
//! (`Gatherer::read`): regular files only (no FIFO, no device), inside the
//! thread's place (or a fixer's worktree) by canonical path, never one
//! that may hold secrets by its requested or its resolved name, within a
//! size, a total and a deadline. What is outside the place is said, never
//! read. It is mistake detection, not a boundary against an adversary:
//! the sandbox is that.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::rules::guard_commands;

/// One file's most, before cutting its middle.
const FILE_MAX: usize = 24_000;
/// Files larger than this are not read at all.
const READ_MAX: u64 = 4 << 20;
/// All files read in one gathering, in bytes at most.
const READ_TOTAL: u64 = 8 << 20;
/// Recently changed sources considered at most.
const DIFF_MAX: usize = 400;
/// What counts as changed lately, for cargo's sources.
const RECENT: Duration = Duration::from_secs(7 * 24 * 3600);
/// The longest a gathering takes.
pub(crate) const DEADLINE: Duration = Duration::from_millis(1500);

/// What was gathered.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Gathered {
  /// `(name as shown, text)`, in order of priority.
  pub files: Vec<(String, String)>,
  /// What was cut or left out, said in the input.
  pub notes: Vec<String>,
  /// What each file read was, to tell later whether it changed.
  pub prints: Vec<Print>,
}

/// A file as it was read: its path and a hash of what was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Print {
  pub path: PathBuf,
  pub hash: [u8; 32],
}

impl Print {
  /// Whether the file is not what was read any more (changed, gone).
  pub(crate) fn changed(&self) -> bool {
    match bounded_bytes(&self.path, READ_MAX) {
      Some(bytes) => *blake3::hash(&bytes).as_bytes() != self.hash,
      None => true,
    }
  }
}

/// Gathers what `line`, run in `cwd`, runs; `touched` are the files the
/// thread wrote this turn; `roots`, where files may be read from (the
/// thread's place, a fixer's worktree; none: `cwd`); at most `cap` bytes
/// in all.
pub(crate) fn gather(
  line: &str,
  cwd: &Path,
  home: Option<&Path>,
  touched: &[PathBuf],
  roots: &[PathBuf],
  cap: usize,
) -> Gathered {
  let roots: Vec<PathBuf> = if roots.is_empty() {
    vec![cwd.to_owned()]
  } else {
    roots.to_vec()
  };
  let mut g = Gatherer {
    cwd: cwd.to_owned(),
    home: home.map(Path::to_owned),
    roots: roots
      .iter()
      .filter_map(|r| std::fs::canonicalize(r).ok())
      .collect(),
    seen: HashSet::new(),
    found: Vec::new(),
    notes: Vec::new(),
    prints: Vec::new(),
    read: 0,
    deadline: Instant::now() + DEADLINE,
  };
  g.line(line, cwd, 0);
  for t in touched.iter().rev() {
    g.add(t.clone(), Why::Touched);
  }
  // One hop on: what the files found name.
  let first: Vec<(PathBuf, String)> = g
    .found
    .iter()
    .map(|(p, _, text)| (p.clone(), text.clone()))
    .collect();
  for (path, text) in first {
    let dir = path.parent().unwrap_or(cwd).to_owned();
    for named in referenced(&text, &path) {
      for base in [&dir, &g.cwd.clone()] {
        let p = base.join(&named);
        if g.add(p, Why::Named) {
          break;
        }
      }
    }
  }
  g.finish(cap)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Why {
  /// Named on the line, or the recipe it runs.
  Line,
  /// What a build runs (`build.rs`, the diff's sources).
  Build,
  /// Written by the thread this turn.
  Touched,
  /// Named by a file found.
  Named,
}

struct Gatherer {
  cwd: PathBuf,
  home: Option<PathBuf>,
  /// Where files may be read from, canonical.
  roots: Vec<PathBuf>,
  seen: HashSet<PathBuf>,
  /// `(path, why, text)`.
  found: Vec<(PathBuf, Why, String)>,
  notes: Vec<String>,
  prints: Vec<Print>,
  /// Bytes read so far.
  read: u64,
  deadline: Instant,
}

/// A regular file's bytes, at most `max` of them, opened without blocking
/// (a FIFO is never waited on) and checked by its handle.
fn bounded_bytes(path: &Path, max: u64) -> Option<Vec<u8>> {
  use std::io::Read;
  let mut opts = std::fs::OpenOptions::new();
  opts.read(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
  }
  let f = opts.open(path).ok()?;
  let meta = f.metadata().ok()?;
  if !meta.is_file() || meta.len() > max {
    return None;
  }
  let mut bytes = Vec::with_capacity(meta.len() as usize);
  f.take(max).read_to_end(&mut bytes).ok()?;
  Some(bytes)
}

/// `p` without `.` and with `..` taken back, as written (no symlink
/// followed): what a requested name is checked against.
pub(crate) fn lexical(p: &Path) -> PathBuf {
  let mut out = PathBuf::new();
  for c in p.components() {
    match c {
      Component::CurDir => {}
      Component::ParentDir => {
        out.pop();
      }
      other => out.push(other.as_os_str()),
    }
  }
  out
}

impl Gatherer {
  /// The one reader: a file's text when it may be read (see the module's
  /// doc); `None` otherwise, said in the notes when it is outside the
  /// place.
  fn read(&mut self, path: &Path) -> Option<(PathBuf, String)> {
    if Instant::now() > self.deadline || self.read >= READ_TOTAL {
      return None;
    }
    if secret(&lexical(path), self.home.as_deref()) {
      return None;
    }
    let real = std::fs::canonicalize(path).ok()?;
    if secret(&real, self.home.as_deref()) {
      return None;
    }
    if !self.roots.iter().any(|r| real.starts_with(r)) {
      let note = format!(
        "{} is outside the thread's place: not read",
        shown(&real, &self.cwd, self.home.as_deref())
      );
      if !self.notes.contains(&note) && self.notes.len() < 16 {
        self.notes.push(note);
      }
      return None;
    }
    let bytes = bounded_bytes(&real, READ_MAX.min(READ_TOTAL - self.read))?;
    self.read += bytes.len() as u64;
    // Text only.
    if bytes.iter().take(8192).any(|b| *b == 0) {
      return None;
    }
    Some((real, String::from_utf8_lossy(&bytes).into_owned()))
  }

  /// Adds a file when it is one to read; whether it was (or is) there.
  fn add(&mut self, path: PathBuf, why: Why) -> bool {
    if let Ok(real) = std::fs::canonicalize(&path)
      && self.seen.contains(&real)
    {
      return true;
    }
    let Some((real, text)) = self.read(&path) else {
      return false;
    };
    self.prints.push(Print {
      path: real.clone(),
      hash: *blake3::hash(text.as_bytes()).as_bytes(),
    });
    self.seen.insert(real.clone());
    self.found.push((real, why, text));
    true
  }

  /// What a line runs, read from `dir` (a `cd` in it moves it).
  fn line(&mut self, line: &str, dir: &Path, depth: usize) {
    let mut dir = dir.to_owned();
    for words in guard_commands(line) {
      self.command(&words, &mut dir, depth);
    }
  }

  fn command(&mut self, words: &[String], dir: &mut PathBuf, depth: usize) {
    let mut words = words;
    while words
      .first()
      .is_some_and(|w| matches!(w.as_str(), "sudo" | "doas"))
    {
      words = &words[1..];
    }
    let Some(first) = words.first() else {
      return;
    };
    let program = first.rsplit('/').next().unwrap_or(first).to_owned();
    if program == "cd" {
      let to = words.get(1).map_or("~", String::as_str);
      if let Some(p) = self.resolve(dir, to)
        && p.is_dir()
      {
        *dir = p;
      }
      return;
    }
    // Scripts given whole (`bash -c '…'`, `python3 -c '…'`) are read as
    // lines too: the files they name.
    for w in words {
      if w.contains(char::is_whitespace) && depth < 2 {
        let d = dir.clone();
        self.line(w, &d, depth + 1);
      }
    }
    // Every word naming a file there (the program too: `./x`, `x.sh`).
    for w in words {
      let w = w.strip_prefix('@').unwrap_or(w);
      let value = w.split_once('=').map_or(w, |(k, v)| {
        if k.starts_with('-') || k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
          v
        } else {
          w
        }
      });
      if value.is_empty() || (value.starts_with('-') && !value.contains('/')) {
        continue;
      }
      if let Some(p) = self.resolve(dir, value) {
        self.add(p, Why::Line);
      }
    }
    let sub = words
      .iter()
      .skip(1)
      .find(|w| !w.starts_with('-'))
      .map(String::as_str)
      .unwrap_or("");
    match program.as_str() {
      "just" => self.just(words, dir),
      "make" | "gmake" => self.make(words, dir),
      "npm" | "yarn" | "pnpm" | "bun" | "npx" => {
        if let Some(p) = upward(dir, &["package.json"]) {
          self.add(p, Why::Line);
        }
      }
      "cargo"
        if !matches!(
          sub,
          "fmt" | "metadata" | "tree" | "search" | "--version" | ""
        ) =>
      {
        self.cargo(words, dir)
      }
      "pytest" | "py.test" => self.conftests(words, dir),
      p if p.starts_with("python") => {
        if words.iter().any(|w| w == "pytest") {
          self.conftests(words, dir);
        }
        // `-m pkg.module`.
        if let Some(i) = words.iter().position(|w| w == "-m")
          && let Some(m) = words.get(i + 1)
        {
          let rel = m.replace('.', "/");
          let d = dir.clone();
          self.add(d.join(format!("{rel}.py")), Why::Line);
          self.add(d.join(&rel).join("__main__.py"), Why::Line);
        }
      }
      _ => {}
    }
  }

  /// A word as a path from `dir`: `~` and `$HOME` expanded, other
  /// variables not.
  fn resolve(&self, dir: &Path, w: &str) -> Option<PathBuf> {
    let home = self.home.as_deref();
    let p = if let Some(rest) = w.strip_prefix("~/").or_else(|| w.strip_prefix("$HOME/")) {
      home?.join(rest)
    } else if w == "~" {
      home?.to_owned()
    } else if w.contains('$') || w.contains('*') {
      return None;
    } else {
      dir.join(w)
    };
    Some(p)
  }

  /// The recipes `just` runs (or its first, the default) and what they
  /// depend on, with the justfile's settings and variables; a small
  /// justfile whole.
  fn just(&mut self, words: &[String], dir: &Path) {
    let file = words
      .windows(2)
      .find(|w| matches!(w[0].as_str(), "-f" | "--justfile"))
      .map(|w| dir.join(&w[1]))
      .or_else(|| upward(dir, &["justfile", "Justfile", ".justfile"]));
    let Some(file) = file else {
      return;
    };
    let names: Vec<String> = words
      .iter()
      .skip(1)
      .filter(|w| !w.starts_with('-') && !w.contains('='))
      .cloned()
      .collect();
    self.recipes(&file, &names, Syntax::Just);
  }

  fn make(&mut self, words: &[String], dir: &Path) {
    let dir = words
      .windows(2)
      .find(|w| w[0] == "-C")
      .map_or_else(|| dir.to_owned(), |w| dir.join(&w[1]));
    let file = words
      .windows(2)
      .find(|w| w[0] == "-f")
      .map(|w| dir.join(&w[1]))
      .or_else(|| {
        ["GNUmakefile", "makefile", "Makefile"]
          .iter()
          .map(|n| dir.join(n))
          .find(|p| p.is_file())
      });
    let Some(file) = file else {
      return;
    };
    let names: Vec<String> = words
      .iter()
      .skip(1)
      .filter(|w| !w.starts_with('-') && !w.contains('='))
      .cloned()
      .collect();
    self.recipes(&file, &names, Syntax::Make);
  }

  /// A recipe file: whole when small, else the recipes named (the first
  /// when none is) with their dependencies and the file's variables.
  fn recipes(&mut self, file: &Path, names: &[String], syntax: Syntax) {
    if std::fs::canonicalize(file).is_ok_and(|r| self.seen.contains(&r)) {
      return;
    }
    let Some((real, text)) = self.read(file) else {
      return;
    };
    self.prints.push(Print {
      path: real.clone(),
      hash: *blake3::hash(text.as_bytes()).as_bytes(),
    });
    let shown = if text.len() <= 8_000 {
      text
    } else {
      extract_recipes(&text, names, syntax)
    };
    self.seen.insert(real.clone());
    self.found.push((real, Why::Line, shown));
  }

  /// For cargo: `build.rs` of the packages named (`-p`) or of the
  /// package where it runs, the sources named by `--bin`, `--example`,
  /// `--test`, `--bench`, and the checkout's diff (since the branch left
  /// its base, uncommitted and untracked files included): its `build.rs`
  /// files first, then binaries, examples, tests and benches, then the
  /// rest of its Rust sources.
  fn cargo(&mut self, words: &[String], dir: &Path) {
    let root = cargo_root(dir).unwrap_or_else(|| dir.to_owned());
    let val = |opts: &[&str]| -> Vec<String> {
      let mut out = Vec::new();
      for (i, w) in words.iter().enumerate() {
        for o in opts {
          if w == o
            && let Some(v) = words.get(i + 1)
          {
            out.push(v.clone());
          } else if let Some(v) = w.strip_prefix(&format!("{o}=")) {
            out.push(v.to_owned());
          }
        }
      }
      out
    };
    let packages = val(&["-p", "--package"]);
    let mut dirs: Vec<PathBuf> = packages
      .iter()
      .filter_map(|p| package_dir(&root, p))
      .collect();
    if packages.is_empty() {
      dirs.push(cargo_root_nearest(dir).unwrap_or_else(|| dir.to_owned()));
    }
    for d in &dirs {
      self.add(d.join("build.rs"), Why::Build);
    }
    for (opt, sub) in [
      ("--bin", "src/bin"),
      ("--example", "examples"),
      ("--test", "tests"),
      ("--bench", "benches"),
    ] {
      for name in val(&[opt]) {
        for d in dirs.iter().chain(std::iter::once(&root)) {
          let base = d.join(sub);
          self.add(base.join(format!("{name}.rs")), Why::Build);
          self.add(base.join(&name).join("main.rs"), Why::Build);
        }
      }
    }
    if words.iter().any(|w| w == "run") && val(&["--bin", "--example"]).is_empty() {
      for d in &dirs {
        self.add(d.join("src/main.rs"), Why::Build);
      }
    }
    let changed = recent_sources(&root);
    let rank = |p: &str| {
      if p.ends_with("build.rs") {
        0
      } else if p.contains("/bin/")
        || p.starts_with("src/bin/")
        || p.starts_with("examples/")
        || p.contains("/examples/")
        || p.starts_with("tests/")
        || p.contains("/tests/")
        || p.contains("/benches/")
        || p.ends_with("main.rs")
      {
        1
      } else {
        2
      }
    };
    let mut rs: Vec<&String> = changed.iter().collect();
    rs.sort_by_key(|p| rank(p));
    for p in rs {
      self.add(root.join(p), Why::Build);
    }
  }

  /// `conftest.py` where pytest runs and beside the test files named.
  fn conftests(&mut self, words: &[String], dir: &Path) {
    let mut dirs = vec![dir.to_owned()];
    for w in words.iter().skip(1).filter(|w| !w.starts_with('-')) {
      let p = dir.join(w.split("::").next().unwrap_or(w));
      if let Some(parent) = p.parent() {
        dirs.push(parent.to_owned());
      }
      if p.is_dir() {
        dirs.push(p);
      }
    }
    for d in dirs {
      self.add(d.join("conftest.py"), Why::Line);
    }
  }

  /// The files found, in order of priority, within `cap` bytes.
  fn finish(mut self, cap: usize) -> Gathered {
    self.found.sort_by_key(|(_, why, _)| *why);
    let mut g = Gathered {
      notes: std::mem::take(&mut self.notes),
      prints: std::mem::take(&mut self.prints),
      ..Gathered::default()
    };
    if Instant::now() > self.deadline {
      g.notes
        .push("gathering took too long: some files not read".into());
    }
    let mut left = cap;
    for (path, _, text) in self.found {
      let name = shown(&path, &self.cwd, self.home.as_deref());
      if left < 400 {
        g.notes.push(format!("{name} left out (no room)"));
        continue;
      }
      let most = FILE_MAX.min(left);
      let text = if text.len() > most {
        let head = floor(&text, most * 4 / 5);
        let tail_from = ceil(&text, text.len() - (most - head.len()).min(text.len()));
        g.notes.push(format!(
          "{name}: {} of its {} bytes shown (its middle cut)",
          head.len() + text.len() - tail_from,
          text.len()
        ));
        format!(
          "{head}\n[… {} bytes cut …]\n{}",
          tail_from - head.len(),
          &text[tail_from..]
        )
      } else {
        text
      };
      left = left.saturating_sub(text.len());
      g.files.push((name, text));
    }
    g
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Syntax {
  Just,
  Make,
}

/// The longest start of `s` within `n` bytes, on a character boundary.
fn floor(s: &str, n: usize) -> &str {
  let mut n = n.min(s.len());
  while !s.is_char_boundary(n) {
    n -= 1;
  }
  &s[..n]
}

fn ceil(s: &str, n: usize) -> usize {
  let mut n = n.min(s.len());
  while !s.is_char_boundary(n) {
    n += 1;
  }
  n
}

/// A path as the input shows it: from where the command runs, or from
/// home.
fn shown(path: &Path, cwd: &Path, home: Option<&Path>) -> String {
  let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_owned());
  if let Ok(rel) = path.strip_prefix(&cwd) {
    return rel.display().to_string();
  }
  if let Some(h) = home
    && let Ok(rel) = path.strip_prefix(std::fs::canonicalize(h).unwrap_or_else(|_| h.to_owned()))
  {
    return format!("~/{}", rel.display());
  }
  path.display().to_string()
}

/// Files never read for the guard: keys, credentials, environment files.
fn secret(path: &Path, home: Option<&Path>) -> bool {
  let name = path
    .file_name()
    .map(|n| n.to_string_lossy().to_ascii_lowercase())
    .unwrap_or_default();
  let in_dir = |d: &str| {
    path.components().any(|c| c.as_os_str() == d)
      || home.is_some_and(|h| path.starts_with(h.join(d)))
  };
  name.starts_with(".env")
    || name.starts_with("id_")
    || [
      ".netrc",
      "_netrc",
      ".git-credentials",
      ".npmrc",
      ".yarnrc",
      ".yarnrc.yml",
      ".pypirc",
      ".pgpass",
      ".my.cnf",
      ".s3cfg",
      ".boto",
      ".vault-token",
      ".htpasswd",
      "hosts.yml",
      "credentials",
      "credentials.toml",
      "credentials.json",
      "credentials.db",
      "access_tokens.db",
      "application_default_credentials.json",
      "kubeconfig",
      "auth.json",
    ]
    .contains(&name.as_str())
    || name.contains("secret")
    || name.contains("token")
    || name.contains("password")
    || name.contains("credential")
    || [
      ".pem",
      ".key",
      ".p12",
      ".pfx",
      ".keychain",
      ".kdbx",
      ".gpg",
      ".asc",
    ]
    .iter()
    .any(|e| name.ends_with(e))
    || [
      ".ssh",
      ".aws",
      ".gnupg",
      ".kube",
      ".docker",
      ".azure",
      ".oci",
      ".terraform.d",
      ".password-store",
      ".stitch",
    ]
    .iter()
    .any(|d| in_dir(d))
    || [
      ".config/gh",
      ".config/hub",
      ".config/gcloud",
      ".config/doctl",
      ".config/op",
      ".config/rclone",
      ".config/stitch",
      ".cargo/credentials",
      ".cargo/credentials.toml",
      ".gem/credentials",
      ".local/share/keyrings",
      "Library/Keychains",
    ]
    .iter()
    .any(|d| {
      home.is_some_and(|h| path.starts_with(h.join(d)))
        || path.to_string_lossy().contains(&format!("/{d}"))
    })
    || path.starts_with("/proc")
    || path.starts_with("/sys")
    || path.starts_with("/dev")
}

/// The first of `names` in `dir` or a directory above it (up to a
/// repository's root).
fn upward(dir: &Path, names: &[&str]) -> Option<PathBuf> {
  let mut d = Some(dir);
  while let Some(at) = d {
    for n in names {
      let p = at.join(n);
      if p.is_file() {
        return Some(p);
      }
    }
    if at.join(".git").exists() {
      return None;
    }
    d = at.parent();
  }
  None
}

/// The outermost directory above `dir` with a `Cargo.toml`, within its
/// repository.
fn cargo_root(dir: &Path) -> Option<PathBuf> {
  let mut found = None;
  let mut d = Some(dir);
  while let Some(at) = d {
    if at.join("Cargo.toml").is_file() {
      found = Some(at.to_owned());
    }
    if at.join(".git").exists() {
      break;
    }
    d = at.parent();
  }
  found.or_else(|| git_root(dir))
}

/// The nearest directory with a `Cargo.toml`: the package where it runs.
fn cargo_root_nearest(dir: &Path) -> Option<PathBuf> {
  let mut d = Some(dir);
  while let Some(at) = d {
    if at.join("Cargo.toml").is_file() {
      return Some(at.to_owned());
    }
    if at.join(".git").exists() {
      return None;
    }
    d = at.parent();
  }
  None
}

/// The directory of package `name` under `root`: one whose `Cargo.toml`
/// names it, else one called so (a few levels down, build output and
/// dependencies skipped).
fn package_dir(root: &Path, name: &str) -> Option<PathBuf> {
  let mut by_dir = None;
  let mut stack = vec![(root.to_owned(), 0)];
  while let Some((d, depth)) = stack.pop() {
    let manifest = d.join("Cargo.toml");
    if let Some(text) =
      bounded_bytes(&manifest, 1 << 20).map(|b| String::from_utf8_lossy(&b).into_owned())
      && text.lines().any(|l| {
        let l = l.replace(' ', "");
        l == format!("name=\"{name}\"")
      })
    {
      return Some(d);
    }
    if by_dir.is_none() && d.file_name().is_some_and(|n| n == name) {
      by_dir = Some(d.clone());
    }
    if depth >= 3 {
      continue;
    }
    let Ok(entries) = std::fs::read_dir(&d) else {
      continue;
    };
    for e in entries.flatten() {
      let n = e.file_name();
      let n = n.to_string_lossy();
      if n.starts_with('.') || matches!(n.as_ref(), "target" | "node_modules" | "build") {
        continue;
      }
      if e.file_type().is_ok_and(|t| t.is_dir()) {
        stack.push((e.path(), depth + 1));
      }
    }
  }
  by_dir
}

/// The checkout's top: the nearest directory above `dir` with `.git`.
fn git_root(dir: &Path) -> Option<PathBuf> {
  dir
    .ancestors()
    .find(|d| d.join(".git").exists())
    .map(Path::to_owned)
}

/// The Rust sources under `root` changed lately (within `RECENT`), newest
/// first, relative to `root`: what a build would compile that the agent
/// may have just written. Read from the file system alone: git is never
/// run here (a checkout's configuration could run its programs, design
/// §2.28).
fn recent_sources(root: &Path) -> Vec<String> {
  let now = SystemTime::now();
  let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
  let mut stack = vec![(root.to_owned(), 0)];
  let mut seen_dirs = 0;
  while let Some((d, depth)) = stack.pop() {
    seen_dirs += 1;
    if seen_dirs > 4_000 {
      break;
    }
    let Ok(entries) = std::fs::read_dir(&d) else {
      continue;
    };
    for e in entries.flatten() {
      let n = e.file_name();
      let n = n.to_string_lossy();
      if n.starts_with('.') || matches!(n.as_ref(), "target" | "node_modules" | "build") {
        continue;
      }
      let Ok(t) = e.file_type() else { continue };
      if t.is_dir() && depth < 8 {
        stack.push((e.path(), depth + 1));
      } else if t.is_file()
        && n.ends_with(".rs")
        && let Ok(m) = e.metadata().and_then(|m| m.modified())
        && now.duration_since(m).map_or(true, |a| a < RECENT)
      {
        found.push((m, e.path()));
      }
    }
  }
  found.sort_by_key(|f| std::cmp::Reverse(f.0));
  found.truncate(DIFF_MAX);
  found
    .into_iter()
    .filter_map(|(_, p)| p.strip_prefix(root).ok().map(|p| p.display().to_string()))
    .collect()
}

/// The recipes named (or the first) with their dependencies, the file's
/// settings and variables: what `just` or `make` runs.
fn extract_recipes(text: &str, names: &[String], syntax: Syntax) -> String {
  // Recipes: a header at the start of a line, its body indented after.
  let mut recipes: Vec<(String, Vec<String>, String)> = Vec::new();
  let mut globals = String::new();
  let mut current: Option<usize> = None;
  for line in text.lines() {
    let indented = line.starts_with(' ') || line.starts_with('\t');
    if indented && let Some(i) = current {
      recipes[i].2.push_str(line);
      recipes[i].2.push('\n');
      continue;
    }
    if line.trim().is_empty() {
      continue;
    }
    current = None;
    let assignment = match syntax {
      Syntax::Just => {
        line.contains(":=") || line.starts_with("set ") || line.starts_with("export ")
      }
      Syntax::Make => {
        line.contains(":=")
          || line.contains("?=")
          || line.contains("+=")
          || line
            .split_once('=')
            .is_some_and(|(k, _)| !k.contains(':') && !k.contains(' '))
          || line.starts_with("include ")
          || line.starts_with("export ")
      }
    };
    if line.starts_with('#') || line.starts_with('[') {
      // Attributes and comments go with the next recipe.
      globals.push_str(line);
      globals.push('\n');
      continue;
    }
    if assignment {
      globals.push_str(line);
      globals.push('\n');
      continue;
    }
    if let Some((head, deps)) = line.split_once(':') {
      let name = head
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('@')
        .to_owned();
      let deps: Vec<String> = deps
        .split_whitespace()
        .map(|d| d.trim_start_matches('(').to_owned())
        .filter(|d| !d.is_empty() && !d.starts_with('#'))
        .collect();
      recipes.push((name, deps, format!("{line}\n")));
      current = Some(recipes.len() - 1);
    }
  }
  let mut wanted: Vec<String> = names
    .iter()
    .filter(|n| recipes.iter().any(|r| &r.0 == *n))
    .cloned()
    .collect();
  if wanted.is_empty()
    && let Some(first) = recipes.iter().find(|r| !r.0.starts_with('.'))
  {
    wanted.push(first.0.clone());
  }
  let mut out = String::new();
  let mut done: HashSet<String> = HashSet::new();
  while let Some(n) = wanted.pop() {
    if !done.insert(n.clone()) {
      continue;
    }
    if let Some(r) = recipes.iter().find(|r| r.0 == n) {
      out.push_str(&r.2);
      out.push('\n');
      wanted.extend(r.1.iter().cloned());
    }
  }
  format!(
    "# (the recipes it runs, with the file's settings and variables; the rest left out)\n{globals}\n{out}"
  )
}

/// Paths a file names: modules it imports, scripts and config files it
/// mentions (relative paths, as written).
fn referenced(text: &str, path: &Path) -> Vec<String> {
  let ext = path
    .extension()
    .map(|e| e.to_string_lossy().into_owned())
    .unwrap_or_default();
  let mut out: Vec<String> = Vec::new();
  if ext == "py" {
    for line in text.lines().map(str::trim) {
      let module = line
        .strip_prefix("from ")
        .and_then(|r| r.split_whitespace().next())
        .or_else(|| {
          line
            .strip_prefix("import ")
            .and_then(|r| r.split([',', ' ']).next())
        });
      if let Some(m) = module {
        let m = m.trim_start_matches('.');
        if !m.is_empty() {
          let rel = m.replace('.', "/");
          out.push(format!("{rel}.py"));
          out.push(format!("{rel}/__init__.py"));
        }
      }
    }
  }
  // Quoted or bare words that look like files: `rotate.json`,
  // `./scripts/post.js`, `tools/x.sh`.
  const EXTS: &[&str] = &[
    "py", "sh", "bash", "zsh", "js", "mjs", "cjs", "ts", "json", "toml", "yaml", "yml", "rs", "rb",
    "pl", "cfg", "ini", "conf", "txt", "sql", "just", "mk",
  ];
  for word in text.split(|c: char| {
    c.is_whitespace()
      || matches!(
        c,
        '"' | '\'' | '`' | '(' | ')' | ',' | ';' | '=' | '[' | ']'
      )
  }) {
    let w = word.trim_end_matches([':', '.']);
    if w.len() < 3 || w.len() > 200 || w.contains("://") || w.starts_with('/') {
      continue;
    }
    let has_ext = w
      .rsplit_once('.')
      .is_some_and(|(stem, e)| !stem.is_empty() && EXTS.contains(&e));
    if has_ext {
      out.push(w.strip_prefix("./").unwrap_or(w).to_owned());
    } else if ext == "js" || ext == "mjs" || ext == "ts" || ext == "cjs" {
      // `require('./lib/x')`: without its extension.
      if w.starts_with("./") || w.starts_with("../") {
        for e in ["js", "mjs", "cjs", "ts"] {
          out.push(format!("{w}.{e}"));
        }
      }
    }
  }
  out.dedup();
  out.truncate(64);
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::process::Command;

  fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, text) in files {
      let path = d.path().join(p);
      std::fs::create_dir_all(path.parent().unwrap()).unwrap();
      std::fs::write(path, text).unwrap();
    }
    d
  }

  fn names(g: &Gathered) -> Vec<&str> {
    g.files.iter().map(|(n, _)| n.as_str()).collect()
  }

  #[test]
  fn files_the_line_names_and_what_they_name() {
    let d = tree(&[
      (
        "rotate.py",
        "import json\ncfg = json.load(open(\"rotate.json\"))\n",
      ),
      ("rotate.json", "{\"targets\": [\"~/devel/*\"]}"),
      ("tests/test_a.py", "from tests.helpers import ws\n"),
      ("tests/helpers.py", "import shutil\n"),
      ("tools/rm.sh", "rm -rf build\n"),
      (".env", "KEY=secret"),
    ]);
    let g = gather("python3 rotate.py", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["rotate.py", "rotate.json"]);
    let g = gather(
      "pytest -q tests/test_a.py",
      d.path(),
      None,
      &[],
      &[],
      60_000,
    );
    assert_eq!(names(&g), ["tests/test_a.py", "tests/helpers.py"]);
    let g = gather(
      "bash -c \"$(cat tools/rm.sh)\"",
      d.path(),
      None,
      &[],
      &[],
      60_000,
    );
    assert_eq!(names(&g), ["tools/rm.sh"]);
    let g = gather("cd tools && ./rm.sh", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["tools/rm.sh"]);
    let g = gather("source .env && ls", d.path(), None, &[], &[], 60_000);
    assert!(g.files.is_empty());
  }

  #[test]
  fn recipes_packages_and_builds() {
    let mut big = String::from("set shell := [\"bash\", \"-c\"]\nout := \"build\"\n\n");
    for i in 0..400 {
      big.push_str(&format!("other{i}:\n    echo {i}\n\n"));
    }
    big.push_str("clean: wipe\n    echo cleaning\n\nwipe:\n    rm -rf {{out}}/\n");
    let d = tree(&[
      ("justfile", &big),
      (
        "package.json",
        "{\"scripts\": {\"postinstall\": \"node scripts/post.js\"}}",
      ),
      (
        "scripts/post.js",
        "require('fs').rmSync(process.env.HOME, {recursive: true})",
      ),
      ("crates/parser/Cargo.toml", "[package]\nname = \"parser\"\n"),
      ("crates/parser/build.rs", "fn main() {}"),
      ("src/bin/cachectl.rs", "fn main() {}"),
    ]);
    let g = gather("just clean", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["justfile"]);
    let text = &g.files[0].1;
    assert!(text.contains("rm -rf {{out}}") && text.contains("out := \"build\""));
    assert!(!text.contains("other7:"), "only what runs");
    let g = gather("npm install", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["package.json", "scripts/post.js"]);
    // Its build.rs first, then the sources changed lately.
    let g = gather("cargo test -p parser", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["crates/parser/build.rs", "src/bin/cachectl.rs"]);
    let g = gather(
      "cargo run -q --bin cachectl -- --reset",
      d.path(),
      None,
      &[],
      &[],
      60_000,
    );
    assert_eq!(names(&g), ["src/bin/cachectl.rs", "crates/parser/build.rs"]);
  }

  /// Gathering runs nothing the checkout controls: a git fsmonitor or
  /// hook that would leave a marker never runs (design §2.28).
  #[test]
  fn gathering_runs_no_git_and_nothing_the_checkout_configures() {
    let d = tree(&[
      ("Cargo.toml", "[package]\nname = \"store\"\n"),
      ("src/lib.rs", "fn f() {}\n"),
    ]);
    let ok = Command::new("git")
      .args(["init", "-q"])
      .current_dir(d.path())
      .status()
      .is_ok_and(|s| s.success());
    if !ok {
      return;
    }
    let marker = d.path().join("ran");
    let hook = d.path().join("monitor.sh");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    for (k, v) in [
      ("core.fsmonitor", hook.display().to_string()),
      ("core.hooksPath", d.path().display().to_string()),
      ("diff.external", hook.display().to_string()),
    ] {
      Command::new("git")
        .args(["config", k, &v])
        .current_dir(d.path())
        .status()
        .unwrap();
    }
    let g = gather("cargo build", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["src/lib.rs"]);
    assert!(!marker.exists(), "a checkout's program ran while gathering");
  }

  /// One bounded reader: secrets by requested or resolved name (recipes
  /// too), credential stores, files outside the place (said, not read),
  /// FIFOs never waited on, `../` kept.
  #[test]
  fn the_reader_keeps_to_the_place_and_away_from_secrets() {
    let d = tree(&[
      (".env", "KEY=hunter2"),
      ("sub/run.py", "import x\nopen('../helper.py')\n"),
      ("helper.py", "import shutil; shutil.rmtree('/')\n"),
      (".npmrc", "//registry/:_authToken=npm-SECRET"),
      ("work/x.sh", "echo hi"),
    ]);
    let outside = tree(&[
      (".config/gh/hosts.yml", "oauth_token: gh-SECRET"),
      ("data.txt", "x"),
    ]);
    std::os::unix::fs::symlink(d.path().join(".env"), d.path().join("justfile")).unwrap();
    let text = |g: &Gathered| {
      g.files
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
    };
    // A recipe file that is a secret, by name or through a symlink.
    for line in ["make -f .env", "just x"] {
      let g = gather(line, d.path(), None, &[], &[], 60_000);
      assert!(!text(&g).contains("hunter2"), "{line}: {g:?}");
    }
    // A credential store named on the line, inside or outside the place.
    let hosts = outside.path().join(".config/gh/hosts.yml");
    let g = gather(
      &format!("cat .npmrc {}", hosts.display()),
      d.path(),
      Some(outside.path()),
      &[],
      &[],
      60_000,
    );
    assert!(g.files.is_empty(), "{g:?}");
    // Outside the place: said, not read (`..` and absolute paths alike).
    let data = outside.path().join("data.txt");
    let g = gather(
      &format!("python3 {}", data.display()),
      &d.path().join("work"),
      None,
      &[],
      &[d.path().to_owned()],
      60_000,
    );
    assert!(g.files.is_empty());
    assert!(
      g.notes
        .iter()
        .any(|n| n.contains("outside the thread's place")),
      "{:?}",
      g.notes
    );
    // `../helper.py` named by a file is that file, not `helper.py` here.
    let g = gather("python3 sub/run.py", d.path(), None, &[], &[], 60_000);
    assert_eq!(names(&g), ["sub/run.py", "helper.py"]);
    assert_eq!(
      referenced("open('../x.py')", Path::new("a.py")),
      ["../x.py"]
    );
    // A FIFO is never waited on.
    let fifo = d.path().join("Makefile");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    #[allow(unsafe_code)]
    // SAFETY: a NUL-terminated path; mkfifo only creates the node.
    let made = unsafe { libc::mkfifo(c.as_ptr(), 0o644) } == 0;
    if made {
      let started = Instant::now();
      let _ = gather("make", d.path(), None, &[], &[], 60_000);
      assert!(started.elapsed() < Duration::from_secs(2));
    }
  }

  /// A file that changes after it was read is told apart.
  #[test]
  fn prints_tell_a_changed_file() {
    let d = tree(&[("x.py", "print(1)\n")]);
    let g = gather("python3 x.py", d.path(), None, &[], &[], 60_000);
    assert_eq!(g.prints.len(), 1);
    assert!(!g.prints[0].changed());
    std::fs::write(d.path().join("x.py"), "import shutil\n").unwrap();
    assert!(g.prints[0].changed());
  }

  #[test]
  fn the_diff_and_the_turns_writes_count_and_the_cap_holds() {
    let d = tree(&[
      ("Cargo.toml", "[package]\nname = \"store\"\n"),
      ("src/lib.rs", "#[test]\nfn t() {}\n"),
      ("notes.md", "x"),
    ]);
    let ok = Command::new("git")
      .arg("init")
      .arg("-q")
      .current_dir(d.path())
      .status()
      .is_ok_and(|s| s.success());
    if ok {
      let g = gather("cargo test -p store", d.path(), None, &[], &[], 60_000);
      assert_eq!(names(&g), ["src/lib.rs"]);
    }
    let touched = vec![d.path().join("notes.md")];
    let g = gather("just x", d.path(), None, &touched, &[], 60_000);
    assert_eq!(names(&g), ["notes.md"]);
    // Cut to the cap, said.
    let long = "x".repeat(50_000);
    let d = tree(&[("a.py", &long), ("b.py", &long)]);
    let g = gather("python3 a.py b.py", d.path(), None, &[], &[], 30_000);
    let total: usize = g.files.iter().map(|(_, t)| t.len()).sum();
    assert!(total < 31_000, "{total}");
    assert!(
      g.notes
        .iter()
        .any(|n| n.contains("a.py") && n.contains("cut"))
    );
  }
}
