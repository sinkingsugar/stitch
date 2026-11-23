//! The shell helper: a small single-threaded process, without the node's
//! frameworks, that forks the shell the classic way.
//!
//! The node must not fork (`spawn`); this helper can: its child becomes a
//! session leader, takes the terminal (`TIOCSCTTY`) and executes the
//! shell, as `pty_process` did in the node. On macOS the node starts
//! shells through it.
//!
//! It was added for bash's jobs now and then failing `setpgid` with EPERM
//! (`bash: child setpgid (…): Operation not permitted`), then taken for an
//! effect of the `/bin/sh` trampoline. It is not, and the helper does not
//! avoid it: bash calls `setpgid` for a new job in both the shell and the
//! job, and on macOS the two calls racing now and then fail the job's. It
//! shows with the helper too, and in plain C with `fork` and `setpgid`
//! alone (parent and child racing; none when the parent's call comes
//! first). The parent's call gives the job its group, in every case looked
//! at; bash's message is in the output.
//!
//! Started by `spawn` with the terminal as stdin, stdout and stderr, the
//! directory and environment the shell gets, a report pipe as fd 3, and
//! `--protocol=<N> <program path> <argv0> <args>…`. It reports on fd 3, a
//! line each: `hello <its protocol>` (for another than `N` it stops
//! there), `pid <pid>` once the shell is executed (or `error <errno>`),
//! then `status <raw wait status>` when the shell has ended, and exits.
//! Without `--protocol` (a node from before the handshake) it starts the
//! shell as that node expects: no `hello`.
//!
//! With `--sandbox` first (Linux), it confines itself instead and executes
//! the shell in its place, reporting nothing (`sandbox`).

#![allow(unsafe_code)]

use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStringExt;

/// The report pipe's descriptor in the helper.
pub(crate) const REPORT_FD: i32 = 3;

fn errno() -> i32 {
  std::io::Error::last_os_error()
    .raw_os_error()
    .unwrap_or(libc::EIO)
}

/// The helper's whole program: never returns.
pub fn helper_main() -> ! {
  let args: Vec<OsString> = std::env::args_os().skip(1).collect();
  if args.first().is_some_and(|a| a == "--sandbox") {
    // No report pipe here: the terminal shows why.
    eprintln!("stitch-shell-helper: {}", crate::sandbox::exec(&args[1..]));
    std::process::exit(126);
  }
  // SAFETY: fd 3 is the report pipe `spawn` gave this process.
  let mut report = unsafe { File::from_raw_fd(REPORT_FD) };
  let code = match run(&mut report) {
    Ok(()) => 0,
    Err(e) => {
      let _ = writeln!(report, "error {e}");
      1
    }
  };
  std::process::exit(code)
}

fn run(report: &mut File) -> Result<(), i32> {
  let mut args: Vec<CString> = std::env::args_os()
    .skip(1)
    .map(|a| CString::new(OsString::into_vec(a)).map_err(|_| libc::EINVAL))
    .collect::<Result<_, _>>()?;
  if let Some(asked) = args
    .first()
    .and_then(|a| a.to_str().ok())
    .and_then(|a| a.strip_prefix("--protocol="))
  {
    let asked = asked.to_owned();
    args.remove(0);
    writeln!(report, "hello {}", crate::spawn::PROTOCOL).map_err(|_| libc::EIO)?;
    if asked != crate::spawn::PROTOCOL.to_string() {
      // The node sees the other protocol and says so; nothing is started.
      std::process::exit(1);
    }
  }
  if args.len() < 2 {
    return Err(libc::EINVAL);
  }
  let path = &args[0];
  let mut argv: Vec<*const libc::c_char> = args[1..].iter().map(|a| a.as_ptr()).collect();
  argv.push(std::ptr::null());

  // SAFETY: this process is single-threaded: between fork and exec the
  // child makes only async-signal-safe calls, on what was prepared here.
  unsafe {
    // Signals for the node's process group are not the helper's: it lives
    // until it has said how the shell ended.
    for sig in [
      libc::SIGHUP,
      libc::SIGINT,
      libc::SIGQUIT,
      libc::SIGTERM,
      libc::SIGPIPE,
    ] {
      libc::signal(sig, libc::SIG_IGN);
    }
    // The child says why it could not execute the shell here; executing
    // closes it.
    let mut failed = [0; 2];
    if libc::pipe(failed.as_mut_ptr()) != 0 {
      return Err(errno());
    }
    libc::fcntl(failed[1], libc::F_SETFD, libc::FD_CLOEXEC);

    let pid = libc::fork();
    if pid < 0 {
      return Err(errno());
    }
    if pid == 0 {
      for sig in 1..32 {
        libc::signal(sig, libc::SIG_DFL);
      }
      let mut none = std::mem::zeroed();
      libc::sigemptyset(&mut none);
      libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
      if libc::setsid() >= 0 && libc::ioctl(0, libc::TIOCSCTTY as _, 0) >= 0 {
        libc::close(REPORT_FD);
        libc::close(failed[0]);
        libc::execv(path.as_ptr(), argv.as_ptr());
      }
      let why = errno();
      libc::write(failed[1], (&raw const why).cast(), size_of::<i32>());
      libc::_exit(127);
    }
    libc::close(failed[1]);
    let mut why = [0u8; 4];
    let mut failed = File::from_raw_fd(failed[0]);
    if failed.read(&mut why).map_err(|_| libc::EIO)? == why.len() {
      reap(pid);
      return Err(i32::from_ne_bytes(why));
    }
    drop(failed);
    writeln!(report, "pid {pid}").map_err(|_| libc::EIO)?;
    // Let go of the terminal: the node sees the shell's end as its end.
    let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
    if null >= 0 {
      for fd in 0..3 {
        libc::dup2(null, fd);
      }
      libc::close(null);
    }
    let status = reap(pid);
    writeln!(report, "status {status}").map_err(|_| libc::EIO)?;
  }
  Ok(())
}

/// Waits for `pid`: its raw wait status.
fn reap(pid: libc::pid_t) -> i32 {
  loop {
    let mut status = 0;
    // SAFETY: plain syscall on our own child.
    if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
      return status;
    }
    if errno() != libc::EINTR {
      return -1;
    }
  }
}
