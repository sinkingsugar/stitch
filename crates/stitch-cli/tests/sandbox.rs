//! Sandboxed shells (`stitch_shell::Sandbox`): they write only where
//! they may, and signal only what they started; the session works as
//! any other.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use stitch_shell::{Config, Sandbox, Session, Status, Wait};

#[path = "../../stitch-shell/tests/common/mod.rs"]
mod common;
use common::without_setpgid;

fn helper() -> PathBuf {
  env!("CARGO_BIN_EXE_stitch-shell-helper").into()
}

fn wait() -> Wait {
  Wait::prompt(Duration::from_secs(10))
}

/// A sandboxed session in `cwd`, writing in `sandbox`; none where this
/// machine cannot sandbox (a kernel without Landlock), said.
async fn sandboxed(cwd: &Path, sandbox: Sandbox) -> Option<Session> {
  if let Err(why) = Sandbox::available(Some(&helper())) {
    eprintln!("not sandboxed here: {why}");
    return None;
  }
  // With paths unreadable, everything else is granted by listing `/` down;
  // inside another sandbox that cannot list `/` (a fixer's), no program
  // starts. Try one the way such a shell starts.
  if cfg!(target_os = "linux") && !sandbox.unreadable.is_empty() {
    let tried = std::process::Command::new(helper())
      .arg("--sandbox")
      .arg(format!("--write={}", cwd.display()))
      .arg(format!(
        "--unreadable={}",
        cwd.join("not-there-gfau").display()
      ))
      .args(["--", "/bin/sh", "sh", "-c", "exit 0"])
      .output()
      .unwrap();
    if !tried.status.success() {
      eprintln!(
        "not sandboxed here: unreadable paths cannot be set inside a sandbox that hides `/` ({})",
        String::from_utf8_lossy(&tried.stderr).trim()
      );
      return None;
    }
  }
  let mut cfg = Config::new(cwd);
  cfg.helper = Some(helper());
  cfg.sandbox = Some(sandbox);
  let s = Session::spawn(cfg).await.unwrap();
  assert!(s.sandboxed());
  Some(s)
}

async fn run(s: &Session, line: &str) -> String {
  let o = s.run(line, wait()).await.unwrap();
  assert!(matches!(o.status, Status::AtPrompt { .. }), "{o:?}");
  without_setpgid(&o.output)
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_go_only_where_the_sandbox_allows() {
  let root = tempfile::tempdir().unwrap();
  let inside = root.path().join("project");
  let outside = root.path().join("elsewhere");
  std::fs::create_dir_all(&inside).unwrap();
  std::fs::create_dir_all(&outside).unwrap();
  let Some(s) = sandboxed(
    &inside,
    Sandbox {
      writable: vec![inside.clone()],
      ..Sandbox::default()
    },
  )
  .await
  else {
    return;
  };
  assert_eq!(
    run(&s, "echo a > in.txt && mkdir -p d/e && echo ok; cat in.txt").await,
    "ok\na"
  );
  let out = run(
    &s,
    &format!(
      "echo b > '{0}/out.txt' || echo denied; mkdir '{0}/d' 2>/dev/null || echo denied; \
       rm -rf '{0}' 2>/dev/null; [ -d '{0}' ] && echo still-there",
      outside.display()
    ),
  )
  .await;
  // Seatbelt says EPERM, Landlock EACCES.
  assert!(
    out.contains("Operation not permitted") || out.contains("Permission denied"),
    "{out}"
  );
  assert!(out.ends_with("denied\ndenied\nstill-there"), "{out}");
  assert!(!outside.join("out.txt").exists());
  // The terminal and /dev/null still take writes; reads go anywhere.
  assert_eq!(
    run(
      &s,
      &format!(
        "echo x >/dev/null; ls '{}' | wc -l | tr -d ' '; tty >/dev/null && echo tty",
        outside.display()
      )
    )
    .await,
    "0\ntty"
  );
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_its_own_processes_are_signalled() {
  if !Sandbox::confines_signals() {
    eprintln!("this kernel does not confine signals");
    return;
  }
  let root = tempfile::tempdir().unwrap();
  let Some(s) = sandboxed(
    root.path(),
    Sandbox {
      writable: vec![root.path().to_owned()],
      ..Sandbox::default()
    },
  )
  .await
  else {
    return;
  };
  let mut other = std::process::Command::new("sleep")
    .arg("30")
    .spawn()
    .unwrap();
  let out = run(
    &s,
    &format!(
      "kill {} 2>/dev/null || echo denied; sleep 30 & kill $! && echo own",
      other.id()
    ),
  )
  .await;
  assert!(out.starts_with("denied\n") && out.contains("own"), "{out}");
  assert!(other.try_wait().unwrap().is_none(), "it still runs");
  other.kill().unwrap();
  let _ = other.wait();
  s.close().await;
}

/// macOS: Swift's tools are reached through the session's shims, which
/// run the real ones (never themselves) with their own sandboxes off.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn xcodebuild_goes_through_its_shim_and_runs() {
  if std::process::Command::new("xcodebuild")
    .arg("-version")
    .output()
    .map_or(true, |o| !o.status.success())
  {
    eprintln!("no Xcode here");
    return;
  }
  let root = tempfile::tempdir().unwrap();
  let Some(s) = sandboxed(
    root.path(),
    Sandbox {
      writable: vec![root.path().to_owned()],
      ..Sandbox::default()
    },
  )
  .await
  else {
    return;
  };
  let out = run(
    &s,
    "type -p xcodebuild; xcodebuild -version | head -1; [ -n \"$XCODE_XCCONFIG_FILE\" ] && echo xcconfig",
  )
  .await;
  let lines: Vec<&str> = out.lines().collect();
  assert!(
    lines[0].ends_with("/bin/xcodebuild") && !lines[0].starts_with("/usr/"),
    "{out}"
  );
  assert!(lines[1].starts_with("Xcode "), "{out}");
  assert_eq!(lines[2], "xcconfig", "{out}");
  s.close().await;
}

/// macOS: worktrees named after the checkout (`<checkout>-<name>`), made
/// after the shell started, are writable; other siblings are not.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn worktrees_made_later_are_writable() {
  let root = tempfile::tempdir().unwrap();
  let checkout = root.path().join("proj");
  std::fs::create_dir_all(&checkout).unwrap();
  let Some(s) = sandboxed(
    &checkout,
    Sandbox {
      writable: vec![checkout.clone()],
      siblings: vec![checkout.clone()],
      ..Sandbox::default()
    },
  )
  .await
  else {
    return;
  };
  let out = run(
    &s,
    "mkdir ../proj-feature && echo a > ../proj-feature/x && echo made; \
     mkdir ../projx 2>/dev/null || echo denied; mkdir ../other 2>/dev/null || echo denied",
  )
  .await;
  assert_eq!(out, "made\ndenied\ndenied");
  s.close().await;
}

/// A read-only thread's shells (design §2.2) write only temporary
/// directories: not its place, nor a worktree beside it, nor the node's
/// data. Its place is outside temporary directories here (the target
/// directory), as a checkout is.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_threads_shells_write_only_temporary_directories() {
  let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
  let root = root.path().canonicalize().unwrap();
  let (place, wt, data) = (root.join("proj"), root.join("proj-wt"), root.join("data"));
  for d in [place.join(".git"), wt.clone(), data.join("home")] {
    std::fs::create_dir_all(d).unwrap();
  }
  let writable = stitch_node::sandbox::sandbox(Some(&place), &place, &data, &[], &[], false);
  assert!(writable.writable.contains(&place));
  let sandbox = stitch_node::sandbox::sandbox(Some(&place), &place, &data, &[], &[], true);
  let Some(s) = sandboxed(&place, sandbox).await else {
    return;
  };
  let tmp = std::env::temp_dir().join(format!("stitch-read-only-{}", std::process::id()));
  let out = run(
    &s,
    &format!(
      "touch x 2>/dev/null || echo place-denied; touch '{}/x' 2>/dev/null || echo wt-denied; \
       touch '{}/home/x' 2>/dev/null || echo data-denied; touch '{}' && echo tmp-ok; \
       touch /tmp/stitch-read-only-$$ && rm /tmp/stitch-read-only-$$ && echo slash-tmp-ok",
      wt.display(),
      data.display(),
      tmp.display()
    ),
  )
  .await;
  assert_eq!(
    out, "place-denied\nwt-denied\ndata-denied\ntmp-ok\nslash-tmp-ok",
    "{out}"
  );
  assert!(!place.join("x").exists() && !wt.join("x").exists());
  assert!(tmp.exists());
  let _ = std::fs::remove_file(&tmp);
  s.close().await;
}

/// A fixer's sandbox (docs/self-improve.md §5) reads none of its
/// `unreadable` paths, however named (a link to one, `cat`, a script),
/// and everything else as before; it still writes where it may. Linux
/// lists no directory holding one of them (Landlock has no exception to
/// a rule); macOS does.
#[tokio::test(flavor = "multi_thread")]
async fn unreadable_paths_are_not_read() {
  let root = tempfile::tempdir().unwrap();
  let root = root.path().canonicalize().unwrap();
  let (home, work) = (root.join("home"), root.join("work"));
  for d in [
    home.join(".ssh"),
    home.join(".config/gh"),
    home.join(".config/x"),
    work.clone(),
  ] {
    std::fs::create_dir_all(d).unwrap();
  }
  std::fs::write(home.join(".ssh/id_ed25519"), "KEY-SECRET").unwrap();
  std::fs::write(home.join(".config/gh/hosts.yml"), "TOKEN-SECRET").unwrap();
  std::fs::write(home.join(".config/x/settings"), "readable").unwrap();
  std::fs::write(home.join(".bashrc"), "readable too").unwrap();
  std::os::unix::fs::symlink(home.join(".ssh"), work.join("keys")).unwrap();
  let Some(s) = sandboxed(
    &work,
    Sandbox {
      writable: vec![work.clone()],
      unreadable: vec![home.join(".ssh"), home.join(".config/gh")],
      ..Sandbox::default()
    },
  )
  .await
  else {
    return;
  };
  let h = home.display();
  let out = run(
    &s,
    &format!(
      "cat '{h}/.ssh/id_ed25519' 2>/dev/null || echo key-denied; \
       cat keys/id_ed25519 2>/dev/null || echo link-denied; \
       echo 'cat \"$1\"' > r.sh && bash r.sh '{h}/.config/gh/hosts.yml' 2>/dev/null || echo script-denied; \
       python3 -c 'print(open(\"{h}/.config/gh/hosts.yml\").read())' 2>/dev/null || echo python-denied; \
       ls '{h}/.ssh' 2>/dev/null || echo list-denied; \
       cat '{h}/.config/x/settings'; echo; cat '{h}/.bashrc'; echo; \
       echo w > written && cat written; cat /etc/hostname >/dev/null && echo etc-ok"
    ),
  )
  .await;
  assert!(!out.contains("SECRET"), "{out}");
  assert!(
    out.starts_with(
      "key-denied\nlink-denied\nscript-denied\npython-denied\nlist-denied\nreadable\nreadable too\nw\n"
    ),
    "{out}"
  );
  assert!(
    out.ends_with("etc-ok") || !Path::new("/etc/hostname").exists(),
    "{out}"
  );
  eprintln!("unreadable paths: the sandbox ran");
  s.close().await;
}
