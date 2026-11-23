//! Where a thread's sandboxed shells may write (design §2.3, the
//! sandbox): its place (a checkout with its git directory and every
//! worktree of it, or a folder), temporary directories, the build caches
//! and toolchains agents use, the records in the node's data directory
//! (the `stitch` record commands write them; its configuration and keys
//! stay read-only), and what `[shell] writable` adds.
//! Everything else is read-only to them; writing there takes a command
//! run outside the sandbox, which asks. A read-only thread's shells write
//! only temporary directories (design §2.2).

use std::path::{Path, PathBuf};

use stitch_shell::Sandbox;

/// Whether this process runs in a sandbox (a thread's sandboxed shell's,
/// which its commands carry however they detach): the check the node makes
/// of a caller before a question that is the user's alone. For tests that
/// need to run unsandboxed (an agent's `just quick`).
pub fn confined_here() -> bool {
  crate::peer::confined(std::process::id() as i32)
}

/// The parts of the node's data directory a sandboxed shell writes: the
/// records and their index, blobs and folders (`stitch new`, `stitch ls`).
const DATA_WRITABLE: &[&str] = &[
  "home",
  "projects",
  "blobs",
  "folders",
  "downloads",
  "index.sqlite",
  "index.sqlite-wal",
  "index.sqlite-shm",
  "index.sqlite-journal",
];

/// The sandbox of a thread whose place is `place` (none: a global
/// thread), its shells starting in `start`, with `env` the environment
/// they get (its `CARGO_TARGET_DIR` and the like). A read-only thread's
/// (`read_only`) writes only temporary directories.
pub fn sandbox(
  place: Option<&Path>,
  start: &Path,
  data: &Path,
  extra: &[String],
  env: &[(String, String)],
  read_only: bool,
) -> Sandbox {
  if read_only {
    return Sandbox {
      writable: temporary(),
      ..Sandbox::default()
    };
  }
  let home = std::env::var_os("HOME").map(PathBuf::from);
  let var = |name: &str| {
    env
      .iter()
      .find(|(k, _)| k == name)
      .map(|(_, v)| PathBuf::from(v))
      .or_else(|| std::env::var_os(name).map(PathBuf::from))
      .filter(|p| p.is_absolute())
  };
  let in_home = |rel: &str| home.as_ref().map(|h| h.join(rel));
  let mut writable: Vec<PathBuf> = Vec::new();
  let mut siblings: Vec<PathBuf> = Vec::new();

  // The place: a global thread started in a directory of its own (not
  // home, not the root) has that one.
  let project = place.map(Path::to_owned).or_else(|| {
    (Some(start) != home.as_deref() && start.parent().is_some()).then(|| start.to_owned())
  });
  if let Some(project) = &project {
    writable.push(project.clone());
    if let Some(git) = repository(project) {
      writable.extend(git.dirs);
      siblings.push(git.main);
    }
  }

  writable.extend(temporary());

  // Toolchains, package caches and build directories.
  writable.extend(var("CARGO_HOME").or_else(|| in_home(".cargo")));
  writable.extend(var("RUSTUP_HOME").or_else(|| in_home(".rustup")));
  writable.extend(var("CARGO_TARGET_DIR"));
  writable.extend(cargo_target_dirs(project.as_deref(), home.as_deref()));
  writable.extend(var("GOPATH").or_else(|| in_home("go")));
  for rel in [".cache", ".npm", ".bun", ".gradle", ".m2", ".pnpm-store"] {
    writable.extend(in_home(rel));
  }
  if cfg!(target_os = "macos") {
    // Xcode's DerivedData, simulators and caches; every tool's caches.
    writable.extend(in_home("Library/Developer"));
    writable.extend(in_home("Library/Caches"));
  }
  // What the record commands write; not the configuration, keys or
  // sign-ins (no command turns yolo on, or replaces a key).
  for part in DATA_WRITABLE {
    writable.push(data.join(part));
  }
  writable.extend(extra.iter().map(|p| crate::place::expand_home(p)));

  // Linux has no patterns: the worktrees there now are listed.
  if !cfg!(target_os = "macos") {
    for main in &siblings {
      writable.extend(worktree_siblings(main));
    }
  }
  let mut seen = std::collections::HashSet::new();
  writable.retain(|p| p.is_absolute() && seen.insert(p.clone()));
  Sandbox {
    writable,
    siblings,
    ..Sandbox::default()
  }
}

/// Temporary directories: macOS's per-user one with its siblings (the
/// caches compilers keep beside it, `…/C`), `/tmp` and `/var/tmp`.
fn temporary() -> Vec<PathBuf> {
  let mut out = Vec::new();
  let tmp = std::env::temp_dir();
  if cfg!(target_os = "macos")
    && let Some(parent) = tmp
      .canonicalize()
      .ok()
      .and_then(|t| t.parent().map(Path::to_owned))
  {
    out.push(parent);
  }
  out.extend([tmp, "/tmp".into(), "/var/tmp".into()]);
  let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
  for name in RUNTIME_OWN {
    out.extend(runtime_own(run.clone(), name));
  }
  out
}

/// The tools' own directories in `$XDG_RUNTIME_DIR` a sandbox writes:
/// `just`'s scripts (shebang recipes), and ccache's temporary files (its
/// default `temporary_dir`; llama.cpp's CMake build uses ccache where it
/// is installed, and failed in fixers' sandboxes without it, so they ran
/// builds with `CCACHE_TEMPDIR=/tmp`).
const RUNTIME_OWN: &[&str] = &["just", "ccache-tmp"];

/// `$XDG_RUNTIME_DIR/<name>` when it is set (`run`): that directory only,
/// made if missing (the rest of the runtime directory holds the user's
/// sockets).
fn runtime_own(run: Option<PathBuf>, name: &str) -> Option<PathBuf> {
  let run = run.filter(|r| r.is_absolute() && r.is_dir())?;
  let own = run.join(name);
  (own.is_dir() || std::fs::create_dir(&own).is_ok()).then_some(own)
}

/// A checkout's git directories and worktrees.
struct Repository {
  /// The git directory (shared by its worktrees), and every worktree.
  dirs: Vec<PathBuf>,
  /// The main worktree, whose siblings `<main>-<name>` are worktrees by
  /// convention (`just worktree`).
  main: PathBuf,
}

/// The repository `dir` is a checkout of, read from its files (no git
/// started): `.git` a directory, or a file naming the worktree's git
/// directory, whose `commondir` is the repository's.
fn repository(dir: &Path) -> Option<Repository> {
  let dot = dir.join(".git");
  let common = if dot.is_dir() {
    dot
  } else {
    let text = std::fs::read_to_string(&dot).ok()?;
    let gitdir = dir.join(text.trim().strip_prefix("gitdir:")?.trim());
    match std::fs::read_to_string(gitdir.join("commondir")) {
      Ok(c) => gitdir.join(c.trim()),
      Err(_) => gitdir,
    }
  };
  let common = common.canonicalize().unwrap_or(common);
  let main = match common.file_name() {
    Some(n) if n == ".git" => common.parent()?.to_owned(),
    _ => dir.to_owned(),
  };
  let mut dirs = vec![common.clone(), main.clone()];
  if let Ok(entries) = std::fs::read_dir(common.join("worktrees")) {
    for e in entries.flatten() {
      // `gitdir` names the worktree's `.git` file.
      if let Ok(text) = std::fs::read_to_string(e.path().join("gitdir"))
        && let Some(worktree) = Path::new(text.trim()).parent()
        && worktree.is_dir()
      {
        dirs.push(worktree.to_owned());
      }
    }
  }
  Some(Repository { dirs, main })
}

/// Another checkout a sandbox writes as it writes its place's: a fixer's
/// paired repository (docs/self-improve.md §6, the core an app's fix
/// changes too). Its git directories and worktrees, its main checkout as a
/// sibling (`fixer::own_worktree` makes the fix's worktree beside it), and
/// on Linux, which has no patterns, the worktrees there now. Nothing when
/// `dir` is no checkout.
pub fn add_checkout(sandbox: &mut Sandbox, dir: &Path) {
  let Some(git) = repository(dir) else {
    return;
  };
  let mut add = git.dirs;
  if !cfg!(target_os = "macos") {
    add.extend(worktree_siblings(&git.main));
  }
  for p in add {
    if p.is_absolute() && !sandbox.writable.contains(&p) {
      sandbox.writable.push(p);
    }
  }
  if !sandbox.siblings.contains(&git.main) {
    sandbox.siblings.push(git.main);
  }
}

/// The directories beside `main` named `<main>-<something>`.
fn worktree_siblings(main: &Path) -> Vec<PathBuf> {
  let (Some(parent), Some(name)) = (main.parent(), main.file_name()) else {
    return Vec::new();
  };
  let prefix = format!("{}-", name.to_string_lossy());
  std::fs::read_dir(parent)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
    .map(|e| e.path())
    .filter(|p| p.is_dir())
    .collect()
}

/// `build.target-dir` from Cargo's configuration: the project's, then the
/// user's (relative to the directory holding `.cargo`).
fn cargo_target_dirs(project: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
  let mut out = Vec::new();
  for base in project.into_iter().chain(home) {
    for name in ["config.toml", "config"] {
      let Ok(text) = std::fs::read_to_string(base.join(".cargo").join(name)) else {
        continue;
      };
      let dir = toml::from_str::<toml::Table>(&text).ok().and_then(|t| {
        t.get("build")?
          .get("target-dir")?
          .as_str()
          .map(|d| base.join(crate::place::expand_home(d)))
      });
      out.extend(dir);
      break;
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_checkout_brings_its_git_directory_and_worktrees() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let main = root.join("proj");
    let wt = root.join("proj-feature");
    std::fs::create_dir_all(main.join(".git/worktrees/feature")).unwrap();
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(
      wt.join(".git"),
      format!(
        "gitdir: {}\n",
        main.join(".git/worktrees/feature").display()
      ),
    )
    .unwrap();
    std::fs::write(main.join(".git/worktrees/feature/commondir"), "../..\n").unwrap();
    std::fs::write(
      main.join(".git/worktrees/feature/gitdir"),
      format!("{}\n", wt.join(".git").display()),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();

    // From the main checkout, and from the worktree: the same repository.
    for from in [&main, &wt] {
      let s = sandbox(
        Some(from),
        from,
        Path::new("/data"),
        &["~/extra".into()],
        &[],
        false,
      );
      for p in [&main, &wt, &main.join(".git")] {
        assert!(
          s.writable.contains(p),
          "{} from {}: {:?}",
          p.display(),
          from.display(),
          s.writable
        );
      }
      assert!(!s.writable.contains(&root.join("other")));
      assert_eq!(s.siblings, vec![main.clone()]);
      assert!(s.writable.contains(&PathBuf::from("/data/home")));
      assert!(
        !s.writable
          .iter()
          .any(|p| p == Path::new("/data") || p.ends_with("config.toml"))
      );
      assert!(s.writable.contains(&crate::place::expand_home("~/extra")));
    }
  }

  #[test]
  fn a_global_thread_in_home_writes_no_project() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let s = sandbox(None, &home, Path::new("/data"), &[], &[], false);
    assert!(!s.writable.contains(&home));
    assert!(s.siblings.is_empty());
    let s = sandbox(None, Path::new("/"), Path::new("/data"), &[], &[], false);
    assert!(!s.writable.contains(&PathBuf::from("/")));
  }

  #[test]
  fn a_read_only_thread_writes_only_temporary_directories() {
    let root = tempfile::tempdir().unwrap();
    let main = root.path().canonicalize().unwrap().join("proj");
    std::fs::create_dir_all(main.join(".git")).unwrap();
    let s = sandbox(
      Some(&main),
      &main,
      Path::new("/data"),
      &["~/extra".into()],
      &[("CARGO_TARGET_DIR".into(), "/builds/target".into())],
      true,
    );
    assert_eq!(s.writable, temporary(), "{:?}", s.writable);
    assert!(s.siblings.is_empty());
    for p in [
      main.clone(),
      main.join(".git"),
      PathBuf::from("/data/home"),
      PathBuf::from("/builds/target"),
      crate::place::expand_home("~/extra"),
      crate::place::expand_home("~/.cargo"),
      crate::place::expand_home("~/Library/Caches"),
    ] {
      assert!(!s.writable.contains(&p), "{}", p.display());
    }
    assert!(s.writable.contains(&std::env::temp_dir()));
    assert!(s.writable.contains(&PathBuf::from("/tmp")));
  }

  /// `just` runs its shebang recipes from `$XDG_RUNTIME_DIR/just` (issue
  /// 2026-10-29T120308Z-geek-3byv), ccache keeps its temporary files in
  /// `$XDG_RUNTIME_DIR/ccache-tmp`: writable, the runtime directory not.
  #[test]
  fn justs_and_ccaches_runtime_directories_are_writable() {
    let run = tempfile::tempdir().unwrap();
    let run = run.path().to_owned();
    // Made when missing, then as it is.
    assert_eq!(
      runtime_own(Some(run.clone()), "just"),
      Some(run.join("just"))
    );
    assert!(run.join("just").is_dir());
    assert_eq!(
      runtime_own(Some(run.clone()), "just"),
      Some(run.join("just"))
    );
    assert_eq!(
      runtime_own(Some(run.clone()), "ccache-tmp"),
      Some(run.join("ccache-tmp"))
    );
    assert_eq!(runtime_own(None, "just"), None);
    assert_eq!(runtime_own(Some("relative".into()), "just"), None);
    assert_eq!(runtime_own(Some(run.join("gone")), "just"), None);
    // In every sandbox, the runtime directory itself is in none.
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
      let t = temporary();
      assert!(!t.contains(&run));
      assert!(t.contains(&run.join("ccache-tmp")));
    }
  }

  #[test]
  fn the_shells_target_directory_is_writable() {
    let s = sandbox(
      None,
      Path::new("/"),
      Path::new("/data"),
      &[],
      &[("CARGO_TARGET_DIR".into(), "/builds/target".into())],
      false,
    );
    assert!(s.writable.contains(&PathBuf::from("/builds/target")));
  }

  /// This checkout's work runs in its sandbox: cargo (into the shared
  /// target directory), git, and on macOS Xcode when the checkout has the
  /// app's project; home stays read-only.
  /// Long, so by hand: `cargo test -p stitch-node --lib -- --ignored
  /// the_checkouts_own_work`.
  #[tokio::test(flavor = "multi_thread")]
  #[ignore]
  async fn the_checkouts_own_work_runs_in_its_sandbox() {
    use stitch_shell::{Config, Session, Status, Wait};
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
      .join("../..")
      .canonicalize()
      .unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(&checkout);
    cfg.sandbox = Some(sandbox(
      Some(&checkout),
      &checkout,
      data.path(),
      &[],
      &[],
      false,
    ));
    let s = Session::spawn(cfg).await.unwrap();
    let run = |line: &'static str| {
      let s = s.clone();
      async move {
        let o = s
          .run(line, Wait::prompt(std::time::Duration::from_secs(1800)))
          .await
          .unwrap();
        (o.status, o.output)
      }
    };
    let (status, out) = run("cargo build -q -p stitch-records && echo built").await;
    assert_eq!(status, Status::AtPrompt { exit: 0 }, "{out}");
    let (status, out) = run(
      "g=$(mktemp -d) && cd $g && git init -q && echo a > a && git add a && \
       git -c user.name=t -c user.email=t@t commit -qm a && echo committed; cd - >/dev/null",
    )
    .await;
    assert_eq!(status, Status::AtPrompt { exit: 0 }, "{out}");
    let (_, out) = run("touch ~/.stitch-sandbox-probe 2>/dev/null || echo denied").await;
    assert!(out.ends_with("denied"), "{out}");
    // The app's project, where this checkout has one (the iOS app lives in
    // its own repository; a checkout without it skips this part).
    if cfg!(target_os = "macos") && checkout.join("apps/ios/project.yml").is_file() {
      // From scratch: Swift packages resolved, macros expanded, inside.
      let (status, out) = run(
        "rm -rf apps/ios/.build/DerivedData-sandbox; \
         xcodebuild -quiet -project apps/ios/Stitch.xcodeproj -scheme Stitch -configuration Debug \
         -destination generic/platform=iOS -derivedDataPath apps/ios/.build/DerivedData-sandbox \
         CODE_SIGNING_ALLOWED=NO build 2>&1 | tail -5; echo xcode ${PIPESTATUS[0]}",
      )
      .await;
      assert_eq!(status, Status::AtPrompt { exit: 0 });
      assert!(out.ends_with("xcode 0"), "{out}");
    }
    s.close().await;
  }

  /// A fixer's sandbox (`fixer::confine_sandbox`): this checkout still
  /// builds in it, and it reads no credential. By hand, on Linux with the
  /// helper built: `STITCH_SHELL_HELPER=$PWD/target/debug/stitch-shell-helper
  /// cargo test -p stitch-node --lib -- --ignored a_fixers_work`.
  #[tokio::test(flavor = "multi_thread")]
  #[ignore]
  async fn a_fixers_work_runs_in_its_sandbox() {
    use stitch_shell::{Config, Session, Status, Wait};
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
      .join("../..")
      .canonicalize()
      .unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut sb = sandbox(Some(&checkout), &checkout, data.path(), &[], &[], false);
    crate::fixer::confine_sandbox(true, &mut sb, data.path(), &[]);
    let mut cfg = Config::new(&checkout);
    cfg.sandbox = Some(sb);
    let s = Session::spawn(cfg).await.unwrap();
    let run = |line: &'static str| {
      let s = s.clone();
      async move {
        let o = s
          .run(line, Wait::prompt(std::time::Duration::from_secs(1800)))
          .await
          .unwrap();
        (o.status, o.output)
      }
    };
    let (status, out) =
      run("cargo build -q -p stitch-records && git status --short | wc -l && echo built").await;
    assert_eq!(status, Status::AtPrompt { exit: 0 }, "{out}");
    let (_, out) = run(
      "cat ~/.config/gh/hosts.yml >/dev/null 2>&1 && echo gh-read; \
       ls ~/.ssh >/dev/null 2>&1 && echo ssh-read; \
       touch ~/.cargo/bin/.stitch-probe 2>/dev/null && echo cargo-bin-written; \
       touch ~/.cargo/config.toml 2>/dev/null && echo cargo-config-written; \
       echo checked",
    )
    .await;
    assert!(
      out.ends_with("checked") && !out.contains("read") && !out.contains("written"),
      "{out}"
    );
    s.close().await;
  }
}
