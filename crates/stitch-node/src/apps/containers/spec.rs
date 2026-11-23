//! What an app declares for containers (`container` in `stitch-app.json`,
//! design §2.23): where its Containerfile and build context are, the
//! build's limits, its commands with their typed arguments, pinned inputs
//! and limits, a cache, a shell; and the pins the node computes from the
//! version's files. Checked when `show_app` takes the app, and again when
//! a grant is read.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The node's caps on what a manifest may ask.
pub const MAX_TIMEOUT: u64 = 3600;
pub const MAX_MEMORY_MB: u64 = 16 * 1024;
pub const MAX_CPUS: f64 = 16.0;
pub const MAX_PIDS: u64 = 1024;
pub const MAX_OUT_MB: u64 = 2048;
pub const MAX_OUT_FILES: u64 = 1000;
pub const MAX_DISK_MB: u64 = 64 * 1024;
pub const MAX_CACHE_MB: u64 = 16 * 1024;
pub const MAX_BUILD_MINUTES: u64 = 120;
pub const MAX_DAILY_MINUTES: u64 = 24 * 60;
/// The defaults.
pub const TIMEOUT: u64 = 300;
pub const MEMORY_MB: u64 = 2048;
pub const CPUS: f64 = 2.0;
pub const PIDS: u64 = 256;
pub const OUT_MB: u64 = 256;
pub const OUT_FILES: u64 = 64;
pub const DISK_MB: u64 = 4096;
pub const BUILD_MEMORY_MB: u64 = 4096;
pub const BUILD_MINUTES: u64 = 30;
pub const DAILY_MINUTES: u64 = 60;
/// The NVIDIA Container Toolkit with the fixes for CVE-2024-0132
/// (1.16.2) and CVE-2025-23266 (1.17.8).
pub const MIN_NVIDIA_CTK: (u32, u32, u32) = (1, 17, 8);

/// What an app declares for its containers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerNeeds {
  /// The app's directory holding its `Containerfile` and the build
  /// context (its tools bundle).
  #[serde(default = "default_dir")]
  pub dir: String,
  #[serde(default)]
  pub build: BuildNeeds,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache: Option<CacheNeeds>,
  #[serde(default)]
  pub commands: BTreeMap<String, Command>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub shell: Option<ShellNeeds>,
  /// Container minutes the family may use a day.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub daily_minutes: Option<u64>,
  /// Computed by the node from the version's files, never declared.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pins: Option<Pins>,
}

fn default_dir() -> String {
  "container".into()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildNeeds {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory_mb: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub minutes: Option<u64>,
  /// The build reaches the network (the default), or not at all.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub network: Option<bool>,
  /// The app's images together, at most.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub disk_mb: Option<u64>,
}

/// A persistent volume of the app's, mounted in its commands.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheNeeds {
  /// Where in the container (absolute).
  pub path: String,
  pub mb: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellNeeds {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory_mb: Option<u64>,
}

/// A declared command.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  /// Its argv: an absolute program in the image, then arguments, a
  /// `{name}` standing alone for each typed argument.
  pub run: Vec<String>,
  #[serde(default)]
  pub args: BTreeMap<String, Arg>,
  /// Files from outside the app, each pinned, copied in read-only at
  /// `/in/<name>/`.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub inputs: BTreeMap<String, Input>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub memory_mb: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cpus: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pids: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_files: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_mb: Option<u64>,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub gpu: bool,
}

/// A typed argument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Arg {
  /// A file of the app's data folder under `dir`, with one of `ext`:
  /// copied in read-only, given as its path in the container.
  File {
    #[serde(default)]
    dir: String,
    #[serde(default)]
    ext: Vec<String>,
  },
  /// The output folder: `/out` (made by the node), or with `fresh`, a
  /// path inside it that does not exist yet (`/out/result`), for tools
  /// that refuse an existing one.
  Out {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    fresh: bool,
  },
  Enum {
    values: Vec<String>,
  },
  Int {
    min: i64,
    max: i64,
  },
  /// Letters, digits, `-` and `_`, at most 64.
  Slug,
}

/// Pinned files from a directory outside the app.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
  /// The directory (`~/…` or absolute).
  pub from: String,
  /// Each file's name in it and SHA-256.
  pub files: BTreeMap<String, String>,
}

/// What the node pins of an app's version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
  /// The Containerfile's SHA-256.
  pub containerfile: String,
  /// Its base images, as named (`…@sha256:…`).
  pub bases: Vec<String>,
  /// The context's tree hash (every file by path and content).
  pub context: String,
  /// The context's files, by path in it: their blobs.
  pub files: BTreeMap<String, String>,
}

impl Pins {
  /// A short hash of the pins, as images are labelled and tagged.
  pub fn id(&self) -> String {
    let mut h = blake3::Hasher::new();
    h.update(self.containerfile.as_bytes());
    h.update(&[0]);
    h.update(self.context.as_bytes());
    for b in &self.bases {
      h.update(&[0]);
      h.update(b.as_bytes());
    }
    h.finalize().to_hex()[..16].to_owned()
  }
}

fn hex64(s: &str) -> bool {
  s.len() == 64
    && s
      .bytes()
      .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
  let d = ring::digest::digest(&ring::digest::SHA256, bytes);
  d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// A relative path with plain parts only (no `..`, `.`, hidden parts,
/// empty parts, backslashes or NUL).
pub(crate) fn plain_rel(p: &str) -> bool {
  !p.is_empty()
    && p.len() <= 512
    && !p.starts_with('/')
    && !p.contains('\\')
    && !p.contains('\0')
    && p
      .split('/')
      .all(|c| !c.is_empty() && !c.starts_with('.') && c.chars().all(|ch| !ch.is_control()))
}

fn slug(s: &str, max: usize) -> bool {
  !s.is_empty()
    && s.len() <= max
    && s
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The placeholder an argument is (`{name}` alone), if it is one.
pub(crate) fn placeholder(a: &str) -> Option<&str> {
  a.strip_prefix('{')?.strip_suffix('}')
}

impl ContainerNeeds {
  /// Checked, with defaults; what cannot be declared, said.
  pub fn checked(mut self) -> Result<ContainerNeeds, String> {
    if !plain_rel(&self.dir) {
      return Err(format!(
        "container.dir: `{}` must be a plain path inside the app",
        self.dir
      ));
    }
    let b = &mut self.build;
    let mem = b.memory_mb.unwrap_or(BUILD_MEMORY_MB);
    if !(256..=MAX_MEMORY_MB).contains(&mem) {
      return Err(format!("container.build.memory_mb: 256 to {MAX_MEMORY_MB}"));
    }
    b.memory_mb = Some(mem);
    let min = b.minutes.unwrap_or(BUILD_MINUTES);
    if !(1..=MAX_BUILD_MINUTES).contains(&min) {
      return Err(format!("container.build.minutes: 1 to {MAX_BUILD_MINUTES}"));
    }
    b.minutes = Some(min);
    b.network = Some(b.network.unwrap_or(true));
    let disk = b.disk_mb.unwrap_or(DISK_MB);
    if !(64..=MAX_DISK_MB).contains(&disk) {
      return Err(format!("container.build.disk_mb: 64 to {MAX_DISK_MB}"));
    }
    b.disk_mb = Some(disk);
    if let Some(c) = &self.cache {
      if !abs_container_path(&c.path) || reserved(&c.path) {
        return Err(format!(
          "container.cache.path: `{}` must be an absolute path in the container, not /in, /out, \
           /tmp, /run, /dev, /proc or /sys",
          c.path
        ));
      }
      if !(1..=MAX_CACHE_MB).contains(&c.mb) {
        return Err(format!("container.cache.mb: 1 to {MAX_CACHE_MB}"));
      }
    }
    let daily = self.daily_minutes.unwrap_or(DAILY_MINUTES);
    if !(1..=MAX_DAILY_MINUTES).contains(&daily) {
      return Err(format!("container.daily_minutes: 1 to {MAX_DAILY_MINUTES}"));
    }
    self.daily_minutes = Some(daily);
    if self.commands.len() > 32 {
      return Err("container.commands: at most 32".into());
    }
    for (name, c) in self.commands.iter_mut() {
      if !slug(name, 64) {
        return Err(format!(
          "container.commands: `{name}`: a command's name is letters, digits, - and _"
        ));
      }
      c.check()
        .map_err(|e| format!("container.commands.{name}: {e}"))?;
    }
    if let Some(sh) = &mut self.shell {
      let t = sh.timeout.unwrap_or(TIMEOUT);
      if !(1..=MAX_TIMEOUT).contains(&t) {
        return Err(format!("container.shell.timeout: 1 to {MAX_TIMEOUT}"));
      }
      sh.timeout = Some(t);
      let m = sh.memory_mb.unwrap_or(MEMORY_MB);
      if !(64..=MAX_MEMORY_MB).contains(&m) {
        return Err(format!("container.shell.memory_mb: 64 to {MAX_MEMORY_MB}"));
      }
      sh.memory_mb = Some(m);
    }
    Ok(self)
  }

  /// The hash the build grant names: the pins, the build's limits and
  /// network, and the cache.
  pub fn build_hash(&self) -> String {
    hash_of(&(&self.pins, &self.build, &self.cache, &self.dir))
  }

  /// The hash the run grant names: the commands, the pins (with their
  /// inputs' hashes and devices in the commands), the cache and the
  /// day's minutes.
  pub fn run_hash(&self) -> String {
    hash_of(&(&self.pins, &self.commands, &self.cache, &self.daily_minutes))
  }

  /// The hash the shell grant names.
  pub fn shell_hash(&self) -> Option<String> {
    let sh = self.shell.as_ref()?;
    Some(hash_of(&(&self.pins, sh, &self.cache)))
  }
}

fn hash_of<T: Serialize>(v: &T) -> String {
  let bytes = serde_json::to_vec(v).unwrap_or_default();
  blake3::hash(&bytes).to_hex()[..16].to_owned()
}

fn abs_container_path(p: &str) -> bool {
  p.starts_with('/')
    && p.len() <= 256
    && p[1..]
      .split('/')
      .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains(['\0', ':', ',']))
}

fn reserved(p: &str) -> bool {
  ["/in", "/out", "/tmp", "/run", "/dev", "/proc", "/sys", "/"]
    .iter()
    .any(|r| p == *r || (r.len() > 1 && p.starts_with(&format!("{r}/"))))
}

impl Command {
  fn check(&mut self) -> Result<(), String> {
    let Some(first) = self.run.first() else {
      return Err("run: an argv, the program first".into());
    };
    if !abs_container_path(first) {
      return Err(format!(
        "run: `{first}`: the program is an absolute path in the image (never looked up)"
      ));
    }
    if self.run.len() > 128 || self.run.iter().any(|a| a.len() > 1024 || a.contains('\0')) {
      return Err("run: at most 128 arguments of 1024 bytes".into());
    }
    if placeholder(first).is_some() {
      return Err("run: the program cannot be an argument".into());
    }
    let mut used = Vec::new();
    for a in &self.run {
      match placeholder(a) {
        Some(n) => {
          if !self.args.contains_key(n) {
            return Err(format!("run: `{a}` names no argument in `args`"));
          }
          used.push(n.to_owned());
        }
        None if a.contains('{') || a.contains('}') => {
          return Err(format!(
            "run: `{a}`: an argument is substituted only standing alone (`{{name}}`), as one \
             whole argument"
          ));
        }
        None => {}
      }
    }
    let mut outs = 0;
    for (n, a) in &self.args {
      if !slug(n, 32) {
        return Err(format!("args: `{n}`: letters, digits, - and _"));
      }
      if !used.contains(n) {
        return Err(format!("args: `{n}` is not used in `run`"));
      }
      match a {
        Arg::File { dir, ext } => {
          if !dir.is_empty() && !plain_rel(dir) {
            return Err(format!(
              "args.{n}.dir: `{dir}` must be a plain path in the data folder"
            ));
          }
          if ext.is_empty() || ext.iter().any(|e| !slug(e, 16)) {
            return Err(format!(
              "args.{n}.ext: the extensions it may have (`json`), at least one"
            ));
          }
        }
        Arg::Out { .. } => outs += 1,
        Arg::Enum { values } => {
          if values.is_empty() || values.iter().any(|v| !slug(v, 64)) {
            return Err(format!(
              "args.{n}.values: words of letters, digits, - and _"
            ));
          }
        }
        Arg::Int { min, max } => {
          if min > max {
            return Err(format!("args.{n}: min above max"));
          }
        }
        Arg::Slug => {}
      }
    }
    if outs > 1 {
      return Err("args: one `out` at most".into());
    }
    for (n, i) in &self.inputs {
      if !slug(n, 32) || n == "args" {
        return Err(format!(
          "inputs: `{n}`: letters, digits, - and _ (not `args`)"
        ));
      }
      if self.args.contains_key(n) {
        return Err(format!("inputs: `{n}` is also an argument"));
      }
      if !(i.from.starts_with("~/") || i.from.starts_with('/')) || i.from.contains('\0') {
        return Err(format!("inputs.{n}.from: `~/…` or an absolute directory"));
      }
      let hidden = i.from.split('/').any(|c| c.starts_with('.') && c != "~");
      if hidden {
        return Err(format!(
          "inputs.{n}.from: `{}`: no hidden directories (`~/.ssh`, `~/.stitch`, `~/.config`)",
          i.from
        ));
      }
      if i.files.is_empty() || i.files.len() > 64 {
        return Err(format!("inputs.{n}.files: 1 to 64 files"));
      }
      for (f, h) in &i.files {
        if !plain_rel(f) {
          return Err(format!(
            "inputs.{n}.files: `{f}` must be a plain relative path"
          ));
        }
        if !hex64(h) {
          return Err(format!(
            "inputs.{n}.files.{f}: its SHA-256, 64 lower-case hex digits"
          ));
        }
      }
    }
    let t = self.timeout.unwrap_or(TIMEOUT);
    if !(1..=MAX_TIMEOUT).contains(&t) {
      return Err(format!("timeout: 1 to {MAX_TIMEOUT} seconds"));
    }
    self.timeout = Some(t);
    let m = self.memory_mb.unwrap_or(MEMORY_MB);
    if !(64..=MAX_MEMORY_MB).contains(&m) {
      return Err(format!("memory_mb: 64 to {MAX_MEMORY_MB}"));
    }
    self.memory_mb = Some(m);
    let c = self.cpus.unwrap_or(CPUS);
    if !(c > 0.0 && c <= MAX_CPUS) {
      return Err(format!("cpus: more than 0, at most {MAX_CPUS}"));
    }
    self.cpus = Some(c);
    let p = self.pids.unwrap_or(PIDS);
    if !(8..=MAX_PIDS).contains(&p) {
      return Err(format!("pids: 8 to {MAX_PIDS}"));
    }
    self.pids = Some(p);
    let f = self.max_files.unwrap_or(OUT_FILES);
    if !(1..=MAX_OUT_FILES).contains(&f) {
      return Err(format!("max_files: 1 to {MAX_OUT_FILES}"));
    }
    self.max_files = Some(f);
    let mb = self.max_mb.unwrap_or(OUT_MB);
    if !(1..=MAX_OUT_MB).contains(&mb) {
      return Err(format!("max_mb: 1 to {MAX_OUT_MB}"));
    }
    self.max_mb = Some(mb);
    Ok(())
  }

  /// How the user reads it, a line each.
  pub fn said(&self, name: &str) -> Vec<String> {
    let mut out = vec![format!(
      "command {name}{}: {}",
      self
        .description
        .as_ref()
        .map(|d| format!(" ({d})"))
        .unwrap_or_default(),
      self
        .run
        .iter()
        .map(|a| if a.contains(' ') {
          format!("'{a}'")
        } else {
          a.clone()
        })
        .collect::<Vec<_>>()
        .join(" ")
    )];
    for (n, a) in &self.args {
      out.push(format!(
        "  {{{n}}}: {}",
        match a {
          Arg::File { dir, ext } => format!(
            "a .{} file of the app's data folder{}, copied in read-only",
            ext.join("/."),
            if dir.is_empty() {
              String::new()
            } else {
              format!(" under {dir}/")
            }
          ),
          Arg::Out { fresh } => format!(
            "the output folder (/out{}), made by the node; what it holds is read back as untrusted",
            if *fresh {
              "/result, not existing yet"
            } else {
              ""
            }
          ),
          Arg::Enum { values } => format!("one of {}", values.join(", ")),
          Arg::Int { min, max } => format!("a whole number from {min} to {max}"),
          Arg::Slug => "a word (letters, digits, - and _)".into(),
        }
      ));
    }
    for (n, i) in &self.inputs {
      out.push(format!(
        "  input {n}: {} files from {} (each pinned by SHA-256, copied in read-only at /in/{n})",
        i.files.len(),
        i.from
      ));
    }
    out.push(format!(
      "  limits: {} s, {} MiB memory, {} CPUs, {} processes; at most {} output files, {} MiB; \
       no network",
      self.timeout.unwrap_or(TIMEOUT),
      self.memory_mb.unwrap_or(MEMORY_MB),
      self.cpus.unwrap_or(CPUS),
      self.pids.unwrap_or(PIDS),
      self.max_files.unwrap_or(OUT_FILES),
      self.max_mb.unwrap_or(OUT_MB)
    ));
    if self.gpu {
      out.push(
        "  GPU: the NVIDIA GPU through CDI. The GPU's kernel driver and NVIDIA's container hooks \
         (run on the host as the container starts) are exposed to this command; it is less \
         isolated than the others."
          .into(),
      );
    }
    out
  }
}

/// What a Containerfile is checked for: its base images, pinned by
/// digest; downloads checksummed; no secrets or SSH forwarded.
pub fn check_containerfile(text: &str) -> Result<Vec<String>, String> {
  let mut bases = Vec::new();
  let mut stages: Vec<String> = Vec::new();
  // Logical lines: continuations joined.
  let mut lines = Vec::new();
  let mut cur = String::new();
  for l in text.lines() {
    let t = l.trim_end();
    if cur.is_empty() && t.trim_start().starts_with('#') {
      continue;
    }
    if let Some(s) = t.strip_suffix('\\') {
      cur.push_str(s);
      cur.push(' ');
    } else {
      cur.push_str(t);
      lines.push(std::mem::take(&mut cur));
    }
  }
  if !cur.is_empty() {
    lines.push(cur);
  }
  for l in &lines {
    let words: Vec<&str> = l.split_whitespace().collect();
    let Some(ins) = words.first() else { continue };
    let ins = ins.to_ascii_uppercase();
    let rest: Vec<&str> = words[1..].to_vec();
    match ins.as_str() {
      "FROM" => {
        let args: Vec<&&str> = rest.iter().filter(|w| !w.starts_with("--")).collect();
        let Some(image) = args.first() else {
          return Err("FROM names no image".into());
        };
        if rest.iter().any(|w| w.starts_with("--platform")) {
          return Err("FROM --platform: not allowed; the node builds for itself".into());
        }
        if image.contains('$') {
          return Err(format!("FROM {image}: no build arguments in base images"));
        }
        let stage_ref = stages.iter().any(|s| s.eq_ignore_ascii_case(image));
        if !stage_ref && **image != "scratch" {
          match image.split_once("@sha256:") {
            Some((_, d)) if hex64(d) => bases.push((**image).to_owned()),
            _ => {
              return Err(format!(
                "FROM {image}: a base image is pinned by digest (`name@sha256:…`), never a tag"
              ));
            }
          }
        }
        if let (Some(a), Some(n)) = (args.get(1), args.get(2))
          && a.eq_ignore_ascii_case("as")
        {
          stages.push((**n).to_owned());
        }
      }
      "ADD" => {
        let src: Vec<&&str> = rest.iter().filter(|w| !w.starts_with("--")).collect();
        let remote = src
          .iter()
          .take(src.len().saturating_sub(1))
          .any(|s| s.contains("://") || s.starts_with("git@"));
        let checksum = rest
          .iter()
          .any(|w| w.strip_prefix("--checksum=sha256:").is_some_and(hex64));
        if remote && !checksum {
          return Err(format!(
            "`{l}`: a download is pinned: ADD --checksum=sha256:<hash> <url>, or RUN with the \
             hash checked"
          ));
        }
        if rest.iter().any(|w| w.starts_with("--keep-git-dir")) {
          return Err("ADD --keep-git-dir: not allowed".into());
        }
      }
      "RUN" | "COPY" => {
        for w in &rest {
          if let Some(m) = w.strip_prefix("--mount=")
            && (m.contains("type=secret") || m.contains("type=ssh"))
          {
            return Err(format!("`{l}`: no secrets or SSH agent in a build"));
          }
          if ins == "RUN" && w.starts_with("--network") {
            return Err(format!("`{l}`: the build's network is the node's to set"));
          }
          if ins == "RUN" && w.starts_with("--security") {
            return Err(format!("`{l}`: RUN --security is not allowed"));
          }
          if ins == "COPY" && w.starts_with("--from=") {
            let from = &w["--from=".len()..];
            if !stages.iter().any(|s| s.eq_ignore_ascii_case(from)) && from.parse::<u32>().is_err()
            {
              match from.split_once("@sha256:") {
                Some((_, d)) if hex64(d) => bases.push(from.to_owned()),
                _ => {
                  return Err(format!("`{l}`: COPY --from an image pins it by digest"));
                }
              }
            }
          }
        }
      }
      _ => {}
    }
  }
  if bases.is_empty()
    && !lines
      .iter()
      .any(|l| l.to_ascii_uppercase().starts_with("FROM"))
  {
    return Err("no FROM: a Containerfile starts from a base image".into());
  }
  bases.sort();
  bases.dedup();
  Ok(bases)
}

/// The pins of a version's files (`flat`: path → blob hash, as
/// snapshotted), for `container`: refused when the Containerfile is
/// missing or does not check.
pub fn pins_of(
  dir: &str,
  files: &BTreeMap<String, String>,
  containerfile: &[u8],
) -> Result<Pins, String> {
  let text =
    std::str::from_utf8(containerfile).map_err(|_| format!("{dir}/Containerfile is not UTF-8"))?;
  let bases = check_containerfile(text).map_err(|e| format!("{dir}/Containerfile: {e}"))?;
  let prefix = format!("{dir}/");
  let ctx: BTreeMap<String, String> = files
    .iter()
    .filter_map(|(p, h)| Some((p.strip_prefix(&prefix)?.to_owned(), h.clone())))
    .collect();
  if !ctx.contains_key("Containerfile") {
    return Err(format!("{dir}/Containerfile: missing"));
  }
  let mut h = blake3::Hasher::new();
  for (p, b) in &ctx {
    h.update(p.as_bytes());
    h.update(&[0]);
    h.update(b.as_bytes());
    h.update(&[0]);
  }
  Ok(Pins {
    containerfile: sha256(containerfile),
    bases,
    context: h.finalize().to_hex()[..32].to_owned(),
    files: ctx,
  })
}

/// What the user reads of a build grant.
pub fn build_said(c: &ContainerNeeds) -> Vec<String> {
  let b = &c.build;
  let mut out = vec![
    format!(
      "build: the app's Containerfile ({}/Containerfile) with its tools ({} files). THE BUILD RUNS \
       THE APP'S CODE{}.",
      c.dir,
      c.pins.as_ref().map_or(0, |p| p.files.len()),
      if b.network != Some(false) {
        " WITH INTERNET ACCESS, and machines of your local network are reachable (not this \
         machine's own services)"
      } else {
        ", without network"
      }
    ),
    format!(
      "  limits: {} MiB memory, {} minutes; the app's images at most {} MiB on disk",
      b.memory_mb.unwrap_or(BUILD_MEMORY_MB),
      b.minutes.unwrap_or(BUILD_MINUTES),
      b.disk_mb.unwrap_or(DISK_MB)
    ),
  ];
  if let Some(p) = &c.pins {
    out.push(format!("  base images: {}", p.bases.join(", ")));
    out.push(format!(
      "  pinned: Containerfile sha256:{}, context {}",
      p.containerfile, p.context
    ));
  }
  if let Some(cache) = &c.cache {
    out.push(format!(
      "  a persistent cache volume at {} in its commands, at most {} MiB (emptied when past it)",
      cache.path, cache.mb
    ));
  }
  out
}

/// What the user reads of a run grant.
pub fn run_said(c: &ContainerNeeds) -> Vec<String> {
  let mut out = vec![format!(
    "commands, each in a container of the app's image: no network, the image read-only, no \
     capabilities, as a user that owns nothing of yours; {} container minutes a day",
    c.daily_minutes.unwrap_or(DAILY_MINUTES)
  )];
  for (n, cmd) in &c.commands {
    out.extend(cmd.said(n).into_iter().map(|l| format!("  {l}")));
  }
  out
}

/// What the user reads of a shell grant.
pub fn shell_said(c: &ContainerNeeds) -> Vec<String> {
  let Some(sh) = &c.shell else {
    return Vec::new();
  };
  vec![
    format!(
      "a shell: command lines the app's threads write, run in the app's image with the same \
       isolation (no network, {} s and {} MiB a call), its data folder's work/ writable.",
      sh.timeout.unwrap_or(TIMEOUT),
      sh.memory_mb.unwrap_or(MEMORY_MB)
    ),
    "  THIS CANCELS PINNING FOR THIS APP: what runs is no longer only the image's pinned tools, \
     but whatever its threads write."
      .into(),
  ]
}

#[cfg(test)]
mod tests {
  use super::*;

  const D: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

  fn cmd(run: &[&str]) -> Command {
    Command {
      run: run.iter().map(|s| s.to_string()).collect(),
      ..Default::default()
    }
  }

  #[test]
  fn containerfiles() {
    let ok = format!(
      "# x\nFROM docker.io/library/debian@sha256:{D} AS base\nRUN apt-get update \\\n && \
       apt-get install -y curl\nFROM base\nADD --checksum=sha256:{D} https://x/y.tar /opt/\n\
       COPY tools /opt/tools\n"
    );
    assert_eq!(
      check_containerfile(&ok).unwrap(),
      vec![format!("docker.io/library/debian@sha256:{D}")]
    );
    for bad in [
      "FROM debian:bookworm\n".to_owned(),
      "FROM debian\n".to_owned(),
      format!("FROM x@sha256:{D}\nADD https://x/y /y\n"),
      format!("FROM x@sha256:{D}\nRUN --mount=type=secret,id=a cat /run/secrets/a\n"),
      format!("FROM x@sha256:{D}\nRUN --mount=type=ssh git clone x\n"),
      format!("FROM x@sha256:{D}\nRUN --network=host curl x\n"),
      format!("FROM x@sha256:{D}\nCOPY --from=alpine:3 /a /a\n"),
      format!("FROM --platform=linux/arm64 x@sha256:{D}\n"),
      "FROM $BASE\n".to_owned(),
      "RUN true\n".to_owned(),
    ] {
      assert!(check_containerfile(&bad).is_err(), "{bad}");
    }
  }

  #[test]
  fn commands() {
    let mut c = cmd(&["/opt/b/blender", "-b", "--", "{recipe}", "{out}"]);
    c.args.insert(
      "recipe".into(),
      Arg::File {
        dir: "projects".into(),
        ext: vec!["json".into()],
      },
    );
    c.args.insert("out".into(), Arg::Out { fresh: true });
    assert!(c.clone().check().is_ok());
    // Not absolute, looked up.
    assert!(cmd(&["blender"]).check().is_err());
    // A placeholder inside an argument.
    let mut d = c.clone();
    d.run.push("--out={out}".into());
    assert!(d.check().is_err());
    // A placeholder naming nothing.
    let mut d = c.clone();
    d.run.push("{nope}".into());
    assert!(d.check().is_err());
    // An argument not used.
    let mut d = c.clone();
    d.args.insert("x".into(), Arg::Slug);
    assert!(d.check().is_err());
    // Limits past the node's.
    let mut d = c.clone();
    d.timeout = Some(MAX_TIMEOUT + 1);
    assert!(d.check().is_err());
    let mut d = c.clone();
    d.memory_mb = Some(MAX_MEMORY_MB * 2);
    assert!(d.check().is_err());
    // Inputs: pinned, not hidden.
    let mut d = c.clone();
    d.inputs.insert(
      "freya".into(),
      Input {
        from: "~/devel/sl-body/base".into(),
        files: [("rig.json".to_owned(), D.to_owned())].into(),
      },
    );
    assert!(d.clone().check().is_ok());
    d.inputs.get_mut("freya").unwrap().from = "~/.ssh".into();
    assert!(d.clone().check().is_err());
    d.inputs.get_mut("freya").unwrap().from = "~/x".into();
    d.inputs
      .get_mut("freya")
      .unwrap()
      .files
      .insert("a".into(), "abc".into());
    assert!(d.clone().check().is_err());
    d.inputs.get_mut("freya").unwrap().files.remove("a");
    d.inputs
      .get_mut("freya")
      .unwrap()
      .files
      .insert("../a".into(), D.into());
    assert!(d.check().is_err());
  }

  #[test]
  fn hashes_change_with_what_they_cover() {
    let mut c = ContainerNeeds {
      dir: "container".into(),
      ..Default::default()
    };
    c.commands.insert("x".into(), cmd(&["/bin/true"]));
    let c = c.checked().unwrap();
    let (b, r) = (c.build_hash(), c.run_hash());
    let mut d = c.clone();
    d.pins = Some(Pins {
      containerfile: D.into(),
      ..Default::default()
    });
    assert_ne!(d.build_hash(), b);
    assert_ne!(d.run_hash(), r);
    let mut e = c.clone();
    e.commands.get_mut("x").unwrap().gpu = true;
    assert_eq!(e.build_hash(), b);
    assert_ne!(e.run_hash(), r);
    assert!(c.shell_hash().is_none());
  }

  #[test]
  fn pins() {
    let cf = format!("FROM a@sha256:{D}\nCOPY tools /t\n");
    let mut files = BTreeMap::new();
    files.insert("index.html".to_owned(), "h0".to_owned());
    files.insert("container/Containerfile".to_owned(), "h1".to_owned());
    files.insert("container/tools/t.py".to_owned(), "h2".to_owned());
    let p = pins_of("container", &files, cf.as_bytes()).unwrap();
    assert_eq!(p.files.len(), 2);
    assert_eq!(p.bases, vec![format!("a@sha256:{D}")]);
    files.insert("container/tools/t.py".to_owned(), "h3".to_owned());
    let q = pins_of("container", &files, cf.as_bytes()).unwrap();
    assert_ne!(p.context, q.context);
    assert_ne!(p.id(), q.id());
    files.remove("container/Containerfile");
    assert!(pins_of("container", &files, cf.as_bytes()).is_err());
  }

  #[test]
  fn plain_paths() {
    assert!(plain_rel("projects/a/recipe.json"));
    for bad in [
      "",
      "/a",
      "a/../b",
      "a/./b",
      ".stitch/x",
      "a//b",
      "a\\b",
      "a/.git",
    ] {
      assert!(!plain_rel(bad), "{bad}");
    }
  }
}
