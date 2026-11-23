//! Shells forked by `stitch-shell-helper` (macOS): they start in a process
//! whose own forks die, their jobs get process groups of their own, and an
//! early end is said with its signal, relayed by the helper.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use stitch_shell::{Config, Session, Status, Wait};

#[path = "../../stitch-shell/tests/common/mod.rs"]
mod common;
use common::without_setpgid;

fn config() -> Config {
  let mut cfg = Config::new(std::env::temp_dir());
  cfg.helper = Some(env!("CARGO_BIN_EXE_stitch-shell-helper").into());
  cfg
}

fn wait() -> Wait {
  Wait::prompt(Duration::from_secs(10))
}

/// Whether this test runs in a Stitch sandbox (an agent's `just quick`),
/// said on stderr: there `/bin/ps`, which these tests read process groups
/// and terminals with, is not let run (setuid: "Operation not permitted").
fn sandboxed(test: &str) -> bool {
  let confined = stitch_node::sandbox::confined_here();
  if confined {
    eprintln!(
      "{test} skipped: it runs in a sandbox, where /bin/ps does not; it needs to run unsandboxed"
    );
  }
  confined
}

extern "C" fn die_in_the_child() {
  // SAFETY: only signal calls, in the forked child.
  unsafe {
    libc::signal(libc::SIGSEGV, libc::SIG_DFL);
    libc::raise(libc::SIGSEGV);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_start_when_this_process_cannot_fork() {
  if sandboxed("shells_start_when_this_process_cannot_fork") {
    return;
  }
  // SAFETY: registers a handler for this test binary's forks.
  assert_eq!(
    unsafe { libc::pthread_atfork(None, None, Some(die_in_the_child)) },
    0
  );
  let s = Session::spawn(config()).await.unwrap();
  let o = s.run("echo started", wait()).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "started");
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  // The helper forked it: the shell is its child, leads its group and
  // has its terminal.
  let o = s
    .run(
      "ps -o comm= -p $PPID | xargs basename; \
       [ \"$(ps -o pgid= -p $$ | tr -d ' ')\" = \"$$\" ] && echo own-group; \
       [ \"/dev/$(ps -o tty= -p $$ | tr -d ' ')\" = \"$(tty)\" ] && echo controlling",
      wait(),
    )
    .await
    .unwrap();
  assert_eq!(
    without_setpgid(&o.output),
    "stitch-shell-helper\nown-group\ncontrolling"
  );
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn jobs_get_process_groups_of_their_own() {
  if sandboxed("jobs_get_process_groups_of_their_own") {
    return;
  }
  let s = Session::spawn(config()).await.unwrap();
  for _ in 0..20 {
    let o = s
      .run(
        "sleep 5 & j=$!; [ \"$(ps -o pgid= -p $j | tr -d ' ')\" = \"$j\" ] && echo own; kill $j; wait $j 2>/dev/null",
        wait(),
      )
      .await
      .unwrap();
    assert_eq!(
      without_setpgid(&o.output).lines().last(),
      Some("own"),
      "{}",
      o.output
    );
  }
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_dies_at_once_is_said_with_its_signal() {
  let dir = tempfile::tempdir().unwrap();
  let bash = dir.path().join("bash");
  std::fs::write(&bash, "#!/bin/sh\nkill -SEGV $$\n").unwrap();
  std::fs::set_permissions(&bash, std::fs::Permissions::from_mode(0o755)).unwrap();
  let mut cfg = config();
  cfg.clean_env = true;
  cfg.env = vec![("PATH".into(), dir.path().display().to_string())];
  let e = Session::spawn(cfg)
    .await
    .err()
    .expect("it died")
    .to_string();
  assert!(e.contains("was executed"), "{e}");
  assert!(e.contains("killed by signal 11 (SIGSEGV)"), "{e}");
}

/// A stand-in helper: a script that answers on the report pipe (fd 3).
fn fake_helper(dir: &std::path::Path, answer: &str) -> std::path::PathBuf {
  let path = dir.join("stitch-shell-helper");
  std::fs::write(&path, format!("#!/bin/sh\n{answer}\n")).unwrap();
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
  path
}

async fn spawn_error(helper: std::path::PathBuf) -> String {
  let mut cfg = Config::new(std::env::temp_dir());
  cfg.helper = Some(helper);
  Session::spawn(cfg)
    .await
    .err()
    .expect("no shell")
    .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_helper_of_another_version_starts_nothing_and_says_so() {
  let dir = tempfile::tempdir().unwrap();
  // One from before the handshake: it took the flag for the shell's path.
  let e = spawn_error(fake_helper(dir.path(), "echo 'error 2' >&3; exit 1")).await;
  assert!(e.contains("another version") && e.contains("older"), "{e}");
  // A newer one.
  let e = spawn_error(fake_helper(dir.path(), "echo 'hello 99' >&3; exit 1")).await;
  assert!(
    e.contains("another version") && e.contains("protocol 99"),
    "{e}"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_helper_says_failed_is_the_error_not_a_reason_to_go_around_it() {
  let dir = tempfile::tempdir().unwrap();
  // It ran, and could not make the shell (EPERM): no trampoline instead.
  let e = spawn_error(fake_helper(
    dir.path(),
    "echo 'hello 2' >&3; echo 'error 1' >&3; exit 1",
  ))
  .await;
  assert!(e.contains("by the shell helper"), "{e}");
  // A missing directory is said as such, not as a program not found.
  let mut cfg = config();
  cfg.cwd = dir.path().join("not-here");
  let e = Session::spawn(cfg)
    .await
    .err()
    .expect("no shell")
    .to_string();
  assert!(e.contains("not-here") && e.contains("is not there"), "{e}");
}

#[test]
fn the_helper_says_its_protocol_and_keeps_to_the_one_asked() {
  let helper = env!("CARGO_BIN_EXE_stitch-shell-helper");
  // What it reports on fd 3; its terminal is no terminal here, so it fails
  // to start the shell, after the handshake.
  let report = |args: &str| {
    let out = std::process::Command::new("/bin/sh")
      .args([
        "-c",
        &format!("'{helper}' {args} 3>&1 </dev/null 2>/dev/null"),
      ])
      .output()
      .unwrap();
    String::from_utf8(out.stdout).unwrap()
  };
  let asked = report("--protocol=2 /usr/bin/true true");
  assert!(asked.starts_with("hello 2\nerror "), "{asked:?}");
  // Another protocol: it says its own and stops.
  assert_eq!(report("--protocol=1 /usr/bin/true true"), "hello 2\n");
  // A node from before the handshake: no hello, as that node expects.
  let legacy = report("/usr/bin/true true");
  assert!(legacy.starts_with("error "), "{legacy:?}");
}
