//! Confining what a shell writes (design §2.3, the sandbox): a sandboxed
//! shell, and everything it runs, writes only where `Sandbox::writable`
//! says (and to the terminal and `/dev/null`); it reads everywhere,
//! reaches the network, and signals only processes of its own sandbox.
//! What it starts outside it (a server it asks over a socket, an app
//! `open` launches) is not confined: the node asks before those. A
//! sandbox may also keep paths from being read at all
//! (`Sandbox::unreadable`: a fixer's, the credentials).
//!
//! macOS: the shell is executed by `sandbox-exec` with a Seatbelt profile.
//! Linux: by the shell helper (`helper`), which restricts itself with
//! Landlock and executes the shell in its place; signals are confined
//! from Linux 6.12 (Landlock ABI 6), and earlier kernels confine writes
//! only. Neither forks: the shell keeps the pid it was spawned with.

// iOS and visionOS start no programs (`spawn`), so none is sandboxed.
#![cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Where a sandboxed shell may write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sandbox {
  /// Directories (or files), each with what is under it.
  pub writable: Vec<PathBuf>,
  /// Directories named `<path>-<anything>`, made later included (a
  /// checkout's worktrees, `../stitch-<name>`). macOS only: on Linux the
  /// ones there when the shell starts go in `writable`.
  pub siblings: Vec<PathBuf>,
  /// Files and directories it does not read, nor anything under them (a
  /// fixer's: `~/.ssh`, `~/.config/gh`, the git credential stores). On
  /// Linux, Landlock knows no exception to a rule, so with any of these
  /// everything else is made readable entry by entry, down from `/`
  /// (`readable_beside`): the directories holding one of them are not
  /// listed (`ls ~` fails), and what appears in them later is not read.
  pub unreadable: Vec<PathBuf>,
}

#[cfg(target_os = "macos")]
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// What every shell writes besides its directories: the terminal, the
/// null and random devices, descriptors by path.
#[cfg(target_os = "macos")]
const DEVICES: &str = r#"(literal "/dev/null") (literal "/dev/zero") (literal "/dev/random") (literal "/dev/urandom") (literal "/dev/ptmx") (literal "/dev/dtracehelper") (regex #"^/dev/tty") (regex #"^/dev/fd/")"#;
#[cfg(target_os = "linux")]
const DEVICES: &[&str] = &[
  "/dev/null",
  "/dev/zero",
  "/dev/full",
  "/dev/random",
  "/dev/urandom",
  "/dev/tty",
  "/dev/ptmx",
  "/dev/pts",
  "/dev/shm",
];

impl Sandbox {
  /// Whether shells can be sandboxed on this machine, `helper` being the
  /// shell helper (Linux needs it); `Err` says why not.
  pub fn available(helper: Option<&Path>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
      let _ = helper;
      if !Path::new(SANDBOX_EXEC).is_file() {
        return Err(format!("{SANDBOX_EXEC} is not there"));
      }
      // Tried once: inside another sandbox (an agent's) it is there but
      // refuses to apply a profile ("sandbox_apply: Operation not
      // permitted"), and every shell would fail to start.
      static APPLIES: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
      APPLIES
        .get_or_init(|| {
          let out = std::process::Command::new(SANDBOX_EXEC)
            .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("{SANDBOX_EXEC} did not run: {e}"))?;
          if out.status.success() {
            Ok(())
          } else {
            Err(format!(
              "{SANDBOX_EXEC} applies no profile here (inside another sandbox?): {}",
              String::from_utf8_lossy(&out.stderr).trim()
            ))
          }
        })
        .clone()
    }
    #[cfg(target_os = "linux")]
    {
      use landlock::{ABI, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr};
      if !helper.is_some_and(Path::is_file) {
        return Err(
          "the shell helper (stitch-shell-helper) applies it, and it is not there".into(),
        );
      }
      // Made, never enforced: says whether the kernel has Landlock.
      Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_write(ABI::V1))
        .and_then(Ruleset::create)
        .map(drop)
        .map_err(|e| format!("Landlock is not available in this kernel ({e})"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
      let _ = helper;
      Err("this OS starts no programs".into())
    }
  }

  /// Whether a sandboxed shell signals only processes of its own sandbox
  /// (`kill` of another's is denied).
  pub fn confines_signals() -> bool {
    #[cfg(target_os = "macos")]
    {
      true
    }
    #[cfg(target_os = "linux")]
    {
      use landlock::{CompatLevel, Compatible, Ruleset, RulesetAttr, Scope};
      Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(Scope::Signal)
        .and_then(Ruleset::create)
        .is_ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
      false
    }
  }

  /// The Seatbelt profile: everything allowed but writing outside the
  /// directories, and signalling outside the sandbox.
  #[cfg(target_os = "macos")]
  pub(crate) fn profile(&self) -> String {
    let mut allowed = String::from(DEVICES);
    for dir in &self.writable {
      for path in spellings(dir) {
        allowed.push_str(&format!(" (subpath \"{}\")", quoted(&path)));
      }
    }
    for base in &self.siblings {
      // A regex literal takes no `"`: such a path gets no worktrees.
      for path in spellings(base).into_iter().filter(|p| !p.contains('"')) {
        allowed.push_str(&format!(" (regex #\"^{}-[^/]+(/|$)\")", escaped(&path)));
      }
    }
    let mut profile = format!(
      "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* {allowed})\n\
       (deny signal)\n(allow signal (target same-sandbox))\n"
    );
    // Last: a later rule wins over `(allow default)`.
    let mut denied = String::new();
    for path in &self.unreadable {
      for path in spellings(path) {
        denied.push_str(&format!(" (subpath \"{}\")", quoted(&path)));
      }
    }
    if !denied.is_empty() {
      profile.push_str(&format!("(deny file-read* file-write*{denied})\n"));
    }
    profile
  }

  /// The program to spawn and its arguments (`argv[0]` first), for
  /// `program` with `argv` confined: executed by `sandbox-exec` (macOS) or
  /// by the shell helper (Linux), which execute it in their place.
  pub(crate) fn wrap(
    &self,
    program: &Path,
    argv: Vec<OsString>,
    helper: Option<&Path>,
  ) -> io::Result<(PathBuf, Vec<OsString>)> {
    #[cfg(target_os = "macos")]
    {
      let _ = helper;
      // sandbox-exec executes its first operand with the rest: the
      // program's path stands for its `argv[0]`.
      let args = ["sandbox-exec".into(), "-p".into(), self.profile().into()]
        .into_iter()
        .chain([program.as_os_str().to_owned()])
        .chain(argv.into_iter().skip(1))
        .collect();
      Ok((SANDBOX_EXEC.into(), args))
    }
    #[cfg(not(target_os = "macos"))]
    {
      let helper = helper.ok_or_else(|| {
        io::Error::new(
          io::ErrorKind::NotFound,
          "a sandboxed shell starts through the shell helper (stitch-shell-helper), and there is none",
        )
      })?;
      let mut args: Vec<OsString> = vec!["stitch-shell-helper".into(), "--sandbox".into()];
      for dir in &self.writable {
        let mut w = OsString::from("--write=");
        w.push(dir);
        args.push(w);
      }
      for path in &self.unreadable {
        let mut r = OsString::from("--unreadable=");
        r.push(path);
        args.push(r);
      }
      args.push("--".into());
      args.push(program.as_os_str().to_owned());
      args.extend(argv);
      Ok((helper.to_owned(), args))
    }
  }
}

/// What a sandboxed shell gets in its environment, in the session's own
/// directory `dir`. macOS refuses a sandbox inside another, and Swift's
/// tools make their own: SwiftPM for package manifests and plugins, the
/// compiler for macros. Inside this one they run without theirs: an
/// `xcodebuild` and a `swift` first on PATH add what turns them off,
/// executing the ones PATH had (by their paths, so never themselves), and
/// `XCODE_XCCONFIG_FILE` (unless set) gives the compiler its flag.
#[cfg(target_os = "macos")]
pub(crate) fn prepare(
  dir: &Path,
  cwd: &Path,
  env: &mut std::collections::BTreeMap<OsString, OsString>,
) -> io::Result<()> {
  use std::os::unix::fs::PermissionsExt;
  let bin = dir.join("bin");
  std::fs::create_dir_all(&bin)?;
  for (name, script) in [("xcodebuild", XCODEBUILD), ("swift", SWIFT)] {
    let Ok(real) = crate::spawn::resolve(std::ffi::OsStr::new(name), env, cwd) else {
      continue;
    };
    let real = format!("'{}'", real.to_string_lossy().replace('\'', r"'\''"));
    let path = bin.join(name);
    std::fs::write(
      &path,
      format!("{SHIM_HEAD}{}", script.replace("REAL", &real)),
    )?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
  }
  let xcconfig = dir.join("sandbox.xcconfig");
  std::fs::write(
    &xcconfig,
    "OTHER_SWIFT_FLAGS = $(inherited) -Xfrontend -disable-sandbox\n",
  )?;
  env
    .entry("XCODE_XCCONFIG_FILE".into())
    .or_insert(xcconfig.into());
  let mut path = OsString::from(&bin);
  if let Some(rest) = env.get(std::ffi::OsStr::new("PATH")) {
    path.push(":");
    path.push(rest);
  }
  env.insert("PATH".into(), path);
  Ok(())
}

/// The shims' start; `REAL` in each is the program PATH had.
#[cfg(target_os = "macos")]
const SHIM_HEAD: &str = r#"#!/bin/sh
# Stitch's sandbox: macOS runs no sandbox inside another, so Swift's tools
# run without their own here.
"#;

#[cfg(target_os = "macos")]
// Not for the actions that build nothing: `-create-xcframework` (and
// `-version`, `-showsdks`) refuse the options.
const XCODEBUILD: &str = r#"case "$1" in
  -create-xcframework | -version | -showsdks | -list | -help | -usage) exec REAL "$@" ;;
esac
exec REAL -IDEPackageSupportDisableManifestSandbox=YES -IDEPackageSupportDisablePluginExecutionSandbox=YES "$@"
"#;

#[cfg(target_os = "macos")]
const SWIFT: &str = r#"case "$1" in
  build | test | run | package) sub=$1; shift; exec REAL "$sub" --disable-sandbox "$@" ;;
esac
exec REAL "$@"
"#;

/// A path as written and as resolved (`/tmp` is `/private/tmp` on macOS,
/// and Seatbelt matches the resolved one).
#[cfg(target_os = "macos")]
fn spellings(path: &Path) -> Vec<String> {
  let mut out = vec![path.to_string_lossy().trim_end_matches('/').to_owned()];
  if let Ok(real) = std::fs::canonicalize(path) {
    let real = real.to_string_lossy().trim_end_matches('/').to_owned();
    if !out.contains(&real) {
      out.push(real);
    }
  }
  out.retain(|p| !p.is_empty());
  out
}

/// Literal in a Seatbelt regex (POSIX extended), whose `#"…"` takes
/// backslashes as they are.
#[cfg(target_os = "macos")]
fn escaped(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    if ".^$*+?()[]{}|\\".contains(c) {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// Inside a Seatbelt string.
#[cfg(target_os = "macos")]
fn quoted(s: &str) -> String {
  s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The shell helper's `--sandbox` mode: `--write=<path>`… and
/// `--unreadable=<path>`…, `--`, the program's path, its `argv[0]`, its
/// arguments. Confines this process and executes the program in its place;
/// returns only on failure, saying why.
pub(crate) fn exec(args: &[OsString]) -> String {
  let mut writable = Vec::new();
  let mut unreadable = Vec::new();
  let mut rest = args.iter();
  for a in rest.by_ref() {
    if a == "--" {
      break;
    }
    let s = a.to_str().unwrap_or("");
    if let Some(p) = s.strip_prefix("--write=") {
      writable.push(PathBuf::from(p));
    } else if let Some(p) = s.strip_prefix("--unreadable=") {
      unreadable.push(PathBuf::from(p));
    } else {
      return format!(
        "--sandbox: {} is no --write=<path> nor --unreadable=<path>",
        a.display()
      );
    }
  }
  let (Some(program), Some(argv0)) = (rest.next(), rest.next()) else {
    return "--sandbox: no program to start".into();
  };
  if let Err(e) = confine(&writable, &unreadable) {
    return e;
  }
  use std::os::unix::process::CommandExt;
  let e = std::process::Command::new(program)
    .arg0(argv0)
    .args(rest)
    .exec();
  format!("starting {}: {e}", Path::new(program).display())
}

#[cfg(target_os = "linux")]
fn confine(writable: &[PathBuf], unreadable: &[PathBuf]) -> Result<(), String> {
  use landlock::{
    ABI, AccessFs, BitFlags, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
    path_beneath_rules,
  };
  // Not ABI 9's ResolveUnix: connecting to the node's socket stays allowed.
  let abi = ABI::V6;
  let write = AccessFs::from_write(abi);
  // Reading is handled only when something is unreadable: then what is
  // readable is every entry beside those, down from `/` (executing stays
  // unhandled: a program is read to run it).
  let read: BitFlags<AccessFs> = if unreadable.is_empty() {
    BitFlags::empty()
  } else {
    AccessFs::ReadFile | AccessFs::ReadDir
  };
  let readable = if unreadable.is_empty() {
    Vec::new()
  } else {
    readable_beside(unreadable)
  };
  let gpus = gpu_devices(Path::new("/dev"));
  let status = Ruleset::default()
    .handle_access(write | read)
    .and_then(|r| r.scope(Scope::Signal))
    .and_then(Ruleset::create)
    .and_then(|r| {
      r.add_rules(path_beneath_rules(
        writable
          .iter()
          .map(PathBuf::as_path)
          .chain(DEVICES.iter().map(Path::new))
          .chain(gpus.iter().map(PathBuf::as_path)),
        write,
      ))
    })
    .and_then(|r| {
      if read.is_empty() {
        return Ok(r);
      }
      // The devices are read as well as written.
      r.add_rules(path_beneath_rules(
        DEVICES
          .iter()
          .map(Path::new)
          .chain(gpus.iter().map(PathBuf::as_path)),
        read,
      ))?
      .add_rules(path_beneath_rules(&readable, read))
    })
    .and_then(|r| r.restrict_self())
    .map_err(|e| format!("--sandbox: Landlock: {e}"))?;
  if status.ruleset == RulesetStatus::NotEnforced {
    return Err("--sandbox: Landlock is not enforced by this kernel".into());
  }
  Ok(())
}

/// The GPUs' compute devices under `dev`: NVIDIA's (`nvidia<N>`,
/// `nvidiactl`, `nvidia-uvm`, `nvidia-uvm-tools`) and the DRM render nodes
/// (`dri/renderD*`), opened read-write by CUDA and Vulkan compute, `nvcc
/// -arch=native` and `nvidia-smi` included, so a build or a test that uses
/// the GPU (llama.cpp's CUDA build, issue: fixers building with
/// `STITCH_LLAMA_GPU=off`) runs the same inside the sandbox as outside it.
/// Not the display's (`dri/card*`, `nvidia-modeset`): no mode setting.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gpu_devices(dev: &Path) -> Vec<PathBuf> {
  let compute = |n: &str| {
    n.strip_prefix("nvidia").is_some_and(|r| {
      (!r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
        || matches!(r, "ctl" | "-uvm" | "-uvm-tools")
    })
  };
  let mut out: Vec<PathBuf> = std::fs::read_dir(dev)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|e| compute(&e.file_name().to_string_lossy()))
    .map(|e| e.path())
    .collect();
  out.extend(
    std::fs::read_dir(dev.join("dri"))
      .into_iter()
      .flatten()
      .flatten()
      .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
      .map(|e| e.path()),
  );
  out.sort();
  out
}

#[cfg(not(target_os = "linux"))]
fn confine(_: &[PathBuf], _: &[PathBuf]) -> Result<(), String> {
  Err("--sandbox: the helper confines shells on Linux only".into())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// A path as it is on disk: its deepest existing ancestor resolved
/// (symbolic links followed), the rest as written.
fn resolved(path: &Path) -> PathBuf {
  let mut rest = Vec::new();
  let mut cur = path.to_owned();
  loop {
    if let Ok(real) = std::fs::canonicalize(&cur) {
      return rest.into_iter().rev().fold(real, |p, n| p.join(n));
    }
    match (cur.parent(), cur.file_name()) {
      (Some(parent), Some(name)) => {
        rest.push(name.to_owned());
        cur = parent.to_owned();
      }
      _ => return path.to_owned(),
    }
  }
}

/// What stays readable when `unreadable` may not be read: every entry of
/// every directory from `/` down to each of them, but them and the
/// directories on the way (which are entered, not granted: a rule covers
/// all beneath it). An entry that is a symbolic link to a directory
/// holding one of them, or to one of them, is left out too (a rule
/// follows it).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn readable_beside(unreadable: &[PathBuf]) -> Vec<PathBuf> {
  readable_beside_from(Path::new("/"), unreadable)
}

/// [`readable_beside`], walking down from `root` (which must hold every
/// path of `unreadable`) instead of `/`.
fn readable_beside_from(root: &Path, unreadable: &[PathBuf]) -> Vec<PathBuf> {
  let denied: Vec<PathBuf> = unreadable
    .iter()
    .filter(|p| p.is_absolute())
    .map(|p| resolved(p))
    .collect();
  let touches = |p: &Path| denied.iter().any(|d| d.starts_with(p) || p.starts_with(d));
  let mut out = Vec::new();
  if denied.is_empty() {
    return out;
  }
  let mut dirs = vec![root.to_path_buf()];
  while let Some(dir) = dirs.pop() {
    let Ok(entries) = std::fs::read_dir(&dir) else {
      continue;
    };
    for e in entries.flatten() {
      let p = e.path();
      if denied.contains(&p) {
        continue;
      }
      let link = e.file_type().is_ok_and(|t| t.is_symlink());
      if link {
        // Granted as where it leads, unless that touches a denied path.
        if std::fs::canonicalize(&p).is_ok_and(|real| !touches(&real)) {
          out.push(p);
        }
        continue;
      }
      if denied.iter().any(|d| d.starts_with(&p)) {
        dirs.push(p);
      } else {
        out.push(p);
      }
    }
  }
  out.sort();
  out
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
  use super::*;

  #[test]
  fn the_profile_quotes_paths_and_matches_worktrees() {
    let s = Sandbox {
      writable: vec![PathBuf::from("/x/a \"b\"")],
      siblings: vec![PathBuf::from("/x/stitch.v2")],
      unreadable: Vec::new(),
    };
    let p = s.profile();
    assert!(p.contains(r#"(subpath "/x/a \"b\"")"#), "{p}");
    assert!(p.contains(r#"(regex #"^/x/stitch\.v2-[^/]+(/|$)")"#), "{p}");
    assert!(p.contains("(deny file-write*)") && p.contains("(target same-sandbox)"));
    assert!(!p.contains("file-read"), "{p}");
    let s = Sandbox {
      unreadable: vec![PathBuf::from("/x/.ssh")],
      ..s
    };
    let p = s.profile();
    assert!(
      p.trim_end()
        .ends_with(r#"(deny file-read* file-write* (subpath "/x/.ssh"))"#),
      "{p}"
    );
  }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
  use super::*;

  #[test]
  fn what_is_readable_is_everything_beside_the_unreadable() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    for d in [
      ".ssh",
      ".config/gh",
      ".config/git",
      ".config/other",
      "devel/x",
    ] {
      std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join(".config/git/credentials"), "secret").unwrap();
    std::fs::write(home.join(".config/git/ignore"), "*.o").unwrap();
    std::fs::write(home.join(".bashrc"), "").unwrap();
    std::fs::create_dir(root.join("sibling")).unwrap();
    std::os::unix::fs::symlink(&home, home.join("loop")).unwrap();
    std::os::unix::fs::symlink(home.join("devel"), home.join("work")).unwrap();
    // Walk from the temp root: `/` is not readable in a Landlock sandbox.
    let r = readable_beside_from(
      &root,
      &[
        home.join(".ssh"),
        home.join(".config/gh"),
        home.join(".config/git/credentials"),
        home.join(".git-credentials"),
      ],
    );
    for p in [
      home.join("devel"),
      home.join(".bashrc"),
      home.join(".config/other"),
      home.join(".config/git/ignore"),
      home.join("work"),
      root.join("sibling"),
    ] {
      assert!(r.contains(&p), "{} in {r:?}", p.display());
    }
    for p in [
      home.join(".ssh"),
      home.join(".config"),
      home.join(".config/gh"),
      home.join(".config/git"),
      home.join(".config/git/credentials"),
      home.clone(),
      home.join("loop"),
      PathBuf::from("/"),
    ] {
      assert!(!r.contains(&p), "{} in {r:?}", p.display());
    }
    // Nothing unreadable: no rules at all.
    assert!(readable_beside_from(&root, &[]).is_empty());
  }

  #[test]
  fn from_the_root_what_is_readable_includes_usr() {
    if std::fs::read_dir("/").is_err() {
      eprintln!("skipped: `/` is not readable here (a sandboxed shell)");
      return;
    }
    let r = readable_beside(&[PathBuf::from("/nonexistent-stitch-test/x")]);
    assert!(r.contains(&PathBuf::from("/usr")), "/usr in {r:?}");
  }

  /// The GPUs' devices are a sandbox's too (CUDA builds and tests run
  /// inside it as outside); nothing else of `/dev` is added.
  #[test]
  fn the_gpus_devices_and_no_other() {
    let dev = tempfile::tempdir().unwrap();
    for f in [
      "nvidia0",
      "nvidiactl",
      "nvidia-uvm",
      "nvidia-modeset",
      "sda",
      "kmsg",
    ] {
      std::fs::write(dev.path().join(f), "").unwrap();
    }
    std::fs::create_dir(dev.path().join("dri")).unwrap();
    for f in ["card0", "renderD128"] {
      std::fs::write(dev.path().join("dri").join(f), "").unwrap();
    }
    let names: Vec<String> = gpu_devices(dev.path())
      .iter()
      .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
      .collect();
    // Compute only: no display (card0, nvidia-modeset).
    assert_eq!(names, ["renderD128", "nvidia-uvm", "nvidia0", "nvidiactl"]);
    assert!(gpu_devices(&dev.path().join("none")).is_empty());
  }
}
