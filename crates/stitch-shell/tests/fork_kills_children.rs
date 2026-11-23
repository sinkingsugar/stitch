//! In a process where `fork` kills every child before it can exec (as a
//! system framework's atfork child handler did on macOS: Network.framework,
//! SIGSEGV, every shell from then on), shells still start: they are not
//! forked. Its own test binary: the handler stays for the process's life.

#![allow(unsafe_code)]

use std::time::Duration;

use stitch_shell::{Config, Session, Status, Wait};

mod common;
use common::without_setpgid;

extern "C" fn die_in_the_child() {
  // SAFETY: only signal calls, in the forked child.
  unsafe {
    libc::signal(libc::SIGSEGV, libc::SIG_DFL);
    libc::raise(libc::SIGSEGV);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_start_when_a_forked_child_dies_in_fork() {
  // SAFETY: registers a handler; forks a child that only runs it and exits.
  let status = unsafe {
    assert_eq!(libc::pthread_atfork(None, None, Some(die_in_the_child)), 0);
    let pid = libc::fork();
    if pid == 0 {
      libc::_exit(0);
    }
    let mut status = 0;
    assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
    status
  };
  assert!(
    libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGSEGV,
    "the handler did not kill the forked child: {status:#x}"
  );

  for _ in 0..3 {
    let s = Session::spawn(Config::new(std::env::temp_dir()))
      .await
      .unwrap();
    let o = s
      .run("echo started", Wait::prompt(Duration::from_secs(10)))
      .await
      .unwrap();
    assert_eq!(without_setpgid(&o.output), "started");
    assert_eq!(o.status, Status::AtPrompt { exit: 0 });
    s.close().await;
  }
}
