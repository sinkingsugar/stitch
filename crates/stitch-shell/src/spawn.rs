//! Starting a shell without `fork`.
//!
//! The node is multithreaded and links system frameworks, and `fork` runs
//! their `pthread_atfork` child handlers in the child before any code of
//! ours. On macOS, Network.framework's (`nw_settings_child_has_forked`) was
//! seen killing every child with SIGSEGV once a node had run for a while,
//! so no new shell started until a restart. So shells are started with
//! `posix_spawn`, which runs no atfork handler and nothing of ours in the
//! child: everything is computed here, and the kernel (macOS) or libc
//! (Linux, a `CLONE_VFORK` child) does the rest.
//!
//! The child becomes a session leader (`POSIX_SPAWN_SETSID`) with the
//! terminal as stdin, stdout and stderr, opened by path without
//! `O_NOCTTY`. On Linux that open makes it the controlling terminal. macOS
//! runs file actions before `setsid`, so there the node spawns the shell
//! helper (`helper`), which forks the shell the classic way (bash's jobs
//! failing `setpgid` now and then is a macOS race the helper does not
//! avoid: see `helper`), after a handshake
//! (`PROTOCOL`): a helper of another version starts nothing, and that is
//! an error. Without the helper (none given, or it could not be spawned),
//! the child is `/bin/sh`, which opens the terminal again as the new
//! session's leader, taking it, and executes the shell in its place (same
//! pid). A spawn that returns a
//! pid has executed its program: failing to open the terminal, change
//! directory or execute is an error returned here (on macOS, the shell's
//! own execution fails in `sh`, with its message and status 127).
//!
//! iOS and visionOS start no programs (the sandbox forbids it): there
//! `spawn` says so, and the rest goes unused.

#![allow(unsafe_code)]
#![cfg_attr(
  not(any(target_os = "macos", target_os = "linux")),
  allow(dead_code, unused_imports)
)]

use std::collections::BTreeMap;
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::io::{BufRead, BufReader, PipeReader};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::process::Pid;

#[cfg(not(any(
  target_os = "macos",
  target_os = "linux",
  target_os = "ios",
  target_os = "visionos"
)))]
compile_error!("shells are spawned with macOS's or Linux's posix_spawn");

/// Whether this OS lets the node start programs.
pub const CAN_SPAWN: bool = cfg!(any(target_os = "macos", target_os = "linux"));

#[cfg(target_os = "macos")]
const FLAGS: libc::c_short = (0x0400 // POSIX_SPAWN_SETSID (sys/spawn.h)
  | libc::POSIX_SPAWN_SETSIGDEF
  | libc::POSIX_SPAWN_SETSIGMASK
  // Nothing but the terminal is inherited.
  | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as libc::c_short;
#[cfg(target_os = "linux")]
const FLAGS: libc::c_short = libc::POSIX_SPAWN_SETSID
  | (libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK) as libc::c_short;

/// What to start, and where.
pub(crate) struct Spawn {
  pub program: OsString,
  pub args: Vec<OsString>,
  /// The whole environment.
  pub env: BTreeMap<OsString, OsString>,
  pub cwd: PathBuf,
  /// The terminal's path: stdin, stdout, stderr and controlling terminal.
  pub tty: PathBuf,
  /// The shell helper (`helper`), used on macOS: without it, or when it is
  /// not there, the `/bin/sh` trampoline is. On Linux it confines a
  /// sandboxed shell.
  pub helper: Option<PathBuf>,
  /// Where the shell may write (`sandbox`); none: anywhere.
  pub sandbox: Option<crate::sandbox::Sandbox>,
}

/// What the node and the shell helper speak. The node passes the one it
/// speaks (`--protocol=N`); the helper says its own first (`hello N`) and
/// starts nothing for another. A helper from before the handshake takes
/// the flag for the shell's path and fails to execute it.
pub(crate) const PROTOCOL: u32 = 2;

/// A started shell. `wait` says how it ended; dropped unwaited, it is
/// waited for on a thread of its own (whoever drops it has it killed
/// first).
pub(crate) struct Child {
  shell: libc::pid_t,
  waiter: Option<Waiter>,
  /// Set the moment the shell is known reaped (by the node, or as the
  /// helper says): its pid, and its group's, may be another process's
  /// from then on.
  reaped: Arc<AtomicBool>,
}

/// Who reaps the shell.
enum Waiter {
  /// The node: the shell is its child.
  Own,
  /// The helper, which reports how it ended (macOS).
  #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
  Helper {
    pid: libc::pid_t,
    report: BufReader<PipeReader>,
  },
}

impl Waiter {
  fn wait(self, shell: libc::pid_t, reaped: &AtomicBool) -> io::Result<ExitStatus> {
    match self {
      Waiter::Own => {
        let status = reap(shell);
        reaped.store(true, Ordering::SeqCst);
        status
      }
      Waiter::Helper { pid, mut report } => {
        let mut line = String::new();
        // The helper reaped it before it said so, or it is gone.
        let read = report.read_line(&mut line);
        reaped.store(true, Ordering::SeqCst);
        let _ = reap(pid);
        read?;
        line
          .trim_end()
          .strip_prefix("status ")
          .and_then(|s| s.parse::<i32>().ok())
          .filter(|&s| s >= 0)
          .map(ExitStatus::from_raw)
          .ok_or_else(|| {
            io::Error::other("the shell helper ended without saying how the shell ended")
          })
      }
    }
  }
}

impl Child {
  pub fn id(&self) -> libc::pid_t {
    self.shell
  }

  /// Whether the shell was reaped (see `reaped`); shared, it says so the
  /// moment it is known.
  pub fn reaped(&self) -> Arc<AtomicBool> {
    self.reaped.clone()
  }

  /// Waits for it to end. Dropped while waiting, the wait goes on.
  pub async fn wait(mut self) -> io::Result<ExitStatus> {
    let waiter = self.waiter.take().expect("not waited");
    let (shell, reaped) = (self.shell, self.reaped.clone());
    tokio::task::spawn_blocking(move || waiter.wait(shell, &reaped))
      .await
      .map_err(io::Error::other)?
  }
}

impl Drop for Child {
  fn drop(&mut self) {
    if let Some(waiter) = self.waiter.take() {
      let (shell, reaped) = (self.shell, self.reaped.clone());
      let _ = std::thread::Builder::new()
        .name("stitch-shell-reap".into())
        .spawn(move || waiter.wait(shell, &reaped));
    }
  }
}

fn reap(pid: libc::pid_t) -> io::Result<ExitStatus> {
  loop {
    let mut status = 0;
    // SAFETY: plain syscall on our own child.
    if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
      return Ok(ExitStatus::from_raw(status));
    }
    let e = io::Error::last_os_error();
    if e.kind() != io::ErrorKind::Interrupted {
      return Err(e);
    }
  }
}

fn c(s: &OsStr) -> io::Result<CString> {
  CString::new(s.as_bytes()).map_err(|_| {
    io::Error::new(
      io::ErrorKind::InvalidInput,
      format!("{} contains a NUL byte", s.display()),
    )
  })
}

/// `program` as `execvp` would find it, in the child's PATH.
pub(crate) fn resolve(
  program: &OsStr,
  env: &BTreeMap<OsString, OsString>,
  cwd: &Path,
) -> io::Result<PathBuf> {
  if program.as_bytes().contains(&b'/') {
    return Ok(program.into());
  }
  let path = env
    .get(OsStr::new("PATH"))
    .cloned()
    .or_else(|| std::env::var_os("PATH"))
    .unwrap_or_else(|| "/usr/bin:/bin".into());
  std::env::split_paths(&path)
    .map(|dir| cwd.join(dir).join(program))
    .find(|p| {
      std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
    .ok_or_else(|| {
      io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} is not in PATH ({})", program.display(), path.display()),
      )
    })
}

fn check(code: libc::c_int) -> io::Result<()> {
  match code {
    0 => Ok(()),
    e => Err(io::Error::from_raw_os_error(e)),
  }
}

struct Attr(libc::posix_spawnattr_t);

impl Drop for Attr {
  fn drop(&mut self) {
    // SAFETY: initialized in `spawn`.
    unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
  }
}

struct Actions(libc::posix_spawn_file_actions_t);

impl Drop for Actions {
  fn drop(&mut self) {
    // SAFETY: initialized in `spawn`.
    unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
  }
}

/// Run by `/bin/sh` on macOS with the terminal and the command: opening
/// the terminal (no `O_NOCTTY`) as a session leader without one takes it.
#[cfg(target_os = "macos")]
const TAKE_TTY: &str = r#"tty=$1; shift; exec "$@" <>"$tty" >&0 2>&0"#;

/// How starting through the helper failed.
#[cfg(target_os = "macos")]
enum HelperFailed {
  /// The helper itself could not be spawned: the trampoline starts the
  /// shell instead.
  NotSpawned(io::Error),
  /// The helper ran and said why it started nothing (or was of another
  /// version): that is the error.
  Said(io::Error),
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn spawn(_: &Spawn) -> io::Result<Child> {
  Err(io::Error::new(
    io::ErrorKind::Unsupported,
    "this OS starts no programs, so no shells",
  ))
}

/// Blocks, on the helper's report: called off the async threads.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn spawn(s: &Spawn) -> io::Result<Child> {
  // Said as itself, not as a program (the helper, `/bin/sh`) not found.
  if !s.cwd.is_dir() {
    return Err(io::Error::new(
      io::ErrorKind::NotFound,
      format!("the directory {} is not there", s.cwd.display()),
    ));
  }
  let program = resolve(&s.program, &s.env, &s.cwd)?;
  let argv: Vec<OsString> = std::iter::once(s.program.clone())
    .chain(s.args.iter().cloned())
    .collect();
  // Sandboxed, what is started executes the shell in its place.
  let (program, argv) = match &s.sandbox {
    Some(sandbox) => sandbox.wrap(&program, argv, s.helper.as_deref())?,
    None => (program, argv),
  };
  #[cfg(target_os = "macos")]
  if let Some(helper) = &s.helper {
    match spawn_helper(s, helper, &program, &argv) {
      Ok(child) => return Ok(child),
      Err(HelperFailed::Said(e)) => return Err(e),
      Err(HelperFailed::NotSpawned(e)) => {
        eprintln!(
          "shells: the shell helper {} could not be started ({e}); this shell starts through /bin/sh",
          helper.display()
        );
      }
    }
  }
  #[cfg(target_os = "macos")]
  let (program, args): (PathBuf, Vec<OsString>) = (
    PathBuf::from("/bin/sh"),
    ["sh", "-c", TAKE_TTY, "sh"]
      .map(OsString::from)
      .into_iter()
      .chain([s.tty.clone().into(), program.into()])
      .chain(argv.into_iter().skip(1))
      .collect(),
  );
  #[cfg(target_os = "linux")]
  let args = argv;
  let shell = posix_spawn(s, &program, &args, FLAGS, None)?;
  Ok(Child {
    shell,
    waiter: Some(Waiter::Own),
    reaped: Arc::default(),
  })
}

/// Starts the shell through the helper: it says its protocol, forks the
/// shell and reports its pid, then how it ended.
#[cfg(target_os = "macos")]
fn spawn_helper(
  s: &Spawn,
  helper: &Path,
  program: &Path,
  argv: &[OsString],
) -> Result<Child, HelperFailed> {
  let said = |e: io::Error| HelperFailed::Said(e);
  let (reader, writer) = std::io::pipe().map_err(said)?;
  let protocol = format!("--protocol={PROTOCOL}");
  let args: Vec<OsString> = [
    helper.as_os_str(),
    OsStr::new(&protocol),
    program.as_os_str(),
  ]
  .map(OsString::from)
  .into_iter()
  .chain(argv.iter().cloned())
  .collect();
  // Not a session leader: the helper's child is, made the classic way.
  let pid = posix_spawn(s, helper, &args, FLAGS & !0x0400, Some(writer.as_raw_fd()))
    .map_err(HelperFailed::NotSpawned)?;
  drop(writer);
  let mut report = BufReader::new(reader);
  let mut line = String::new();
  let read = report.read_line(&mut line);
  let hello = line.trim_end().strip_prefix("hello ").map(str::to_owned);
  if hello.as_deref() != Some(PROTOCOL.to_string().as_str()) {
    // Of another version: it started nothing (an older one, before the
    // handshake, failed to execute the flag; whatever it did ends here).
    if let Some(shell) = line
      .trim_end()
      .strip_prefix("pid ")
      .and_then(|p| p.parse().ok())
    {
      // SAFETY: plain syscall; the helper has not reaped it, so the pid is
      // still the shell's.
      unsafe { libc::kill(shell, libc::SIGKILL) };
    }
    let _ = reap(pid);
    read.map_err(said)?;
    return Err(said(io::Error::other(format!(
      "the shell helper {} is of another version (it speaks {}, this node {PROTOCOL}): \
       install stitch and its helper together (e.g. `cargo install --path crates/stitch-cli`), \
       or point STITCH_SHELL_HELPER at a matching one",
      helper.display(),
      hello.map_or_else(
        || "no protocol: it is older".to_owned(),
        |v| format!("protocol {v}")
      ),
    ))));
  }
  line.clear();
  let read = report.read_line(&mut line);
  let said_line = line.trim_end();
  if let Some(shell) = said_line.strip_prefix("pid ").and_then(|p| p.parse().ok()) {
    return Ok(Child {
      shell,
      waiter: Some(Waiter::Helper { pid, report }),
      reaped: Arc::default(),
    });
  }
  let _ = reap(pid);
  read.map_err(said)?;
  Err(said(
    match said_line
      .strip_prefix("error ")
      .and_then(|e| e.parse().ok())
    {
      Some(errno) => {
        let e = io::Error::from_raw_os_error(errno);
        io::Error::new(
          e.kind(),
          format!("starting {} (by the shell helper): {e}", program.display()),
        )
      }
      None => io::Error::other(format!(
        "the shell helper ({}) did not start the shell",
        helper.display()
      )),
    },
  ))
}

/// `posix_spawn` of `program` with `args` (`argv[0]` first) and the
/// environment, directory and terminal of `s`; `report` becomes fd 3.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn posix_spawn(
  s: &Spawn,
  program: &Path,
  args: &[OsString],
  flags: libc::c_short,
  report: Option<RawFd>,
) -> io::Result<libc::pid_t> {
  let path = c(program.as_os_str())?;
  let args = args.iter().map(|a| c(a)).collect::<io::Result<Vec<_>>>()?;
  let env = s
    .env
    .iter()
    .map(|(k, v)| {
      let mut kv = k.clone();
      kv.push("=");
      kv.push(v);
      c(&kv)
    })
    .collect::<io::Result<Vec<_>>>()?;
  let tty = c(s.tty.as_os_str())?;
  let cwd = c(s.cwd.as_os_str())?;
  let mut argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
  argv.push(std::ptr::null_mut());
  let mut envp: Vec<*mut libc::c_char> = env.iter().map(|e| e.as_ptr().cast_mut()).collect();
  envp.push(std::ptr::null_mut());

  // SAFETY: each structure is initialized before use and destroyed by its
  // guard; every pointer handed over outlives the `posix_spawn` call.
  unsafe {
    let mut attr = MaybeUninit::uninit();
    check(libc::posix_spawnattr_init(attr.as_mut_ptr()))?;
    let mut attr = Attr(attr.assume_init());
    check(libc::posix_spawnattr_setflags(&mut attr.0, flags))?;
    // Every signal at its default (the node ignores SIGPIPE), none blocked.
    let mut set = MaybeUninit::uninit();
    libc::sigfillset(set.as_mut_ptr());
    let mut all = set.assume_init();
    libc::sigdelset(&mut all, libc::SIGKILL);
    libc::sigdelset(&mut all, libc::SIGSTOP);
    check(libc::posix_spawnattr_setsigdefault(&mut attr.0, &all))?;
    let mut set = MaybeUninit::uninit();
    libc::sigemptyset(set.as_mut_ptr());
    check(libc::posix_spawnattr_setsigmask(&mut attr.0, set.as_ptr()))?;

    let mut actions = MaybeUninit::uninit();
    check(libc::posix_spawn_file_actions_init(actions.as_mut_ptr()))?;
    let mut actions = Actions(actions.assume_init());
    check(libc::posix_spawn_file_actions_addopen(
      &mut actions.0,
      0,
      tty.as_ptr(),
      libc::O_RDWR,
      0,
    ))?;
    check(libc::posix_spawn_file_actions_adddup2(&mut actions.0, 0, 1))?;
    check(libc::posix_spawn_file_actions_adddup2(&mut actions.0, 0, 2))?;
    if let Some(fd) = report {
      check(libc::posix_spawn_file_actions_adddup2(
        &mut actions.0,
        fd,
        crate::helper::REPORT_FD,
      ))?;
    }
    check(libc::posix_spawn_file_actions_addchdir_np(
      &mut actions.0,
      cwd.as_ptr(),
    ))?;

    let mut pid = 0;
    check(libc::posix_spawn(
      &mut pid,
      path.as_ptr(),
      &actions.0,
      &attr.0,
      argv.as_ptr(),
      envp.as_ptr(),
    ))
    .map_err(|e| {
      io::Error::new(
        e.kind(),
        format!("starting {}: {e}", path.to_string_lossy()),
      )
    })?;
    Ok(pid)
  }
}

/// The name of the program a process runs, as `ps -o comm=` says it
/// (macOS: its `argv[0]`'s file name; Linux: its `comm`), without starting a
/// process to ask.
pub(crate) fn program_name(pid: Pid) -> Option<String> {
  let raw = pid.as_raw_nonzero().get();
  #[cfg(target_os = "macos")]
  let name: Option<String> = {
    // Its arguments as the kernel keeps them: argc, the executable's path,
    // NULs, then argv.
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, raw];
    // SAFETY: `size` is the buffer's length; the kernel writes at most
    // that and says how much.
    let ok = unsafe {
      libc::sysctl(
        mib.as_mut_ptr(),
        mib.len() as u32,
        buf.as_mut_ptr().cast(),
        &mut size,
        std::ptr::null_mut(),
        0,
      )
    } == 0;
    ok.then(|| buf.get(4..size.min(buf.len())))
      .flatten()
      .and_then(|args| {
        let path_end = args.iter().position(|b| *b == 0)?;
        let rest = &args[path_end..];
        let start = rest.iter().position(|b| *b != 0)?;
        let argv0 = &rest[start..];
        let end = argv0.iter().position(|b| *b == 0).unwrap_or(argv0.len());
        Some(String::from_utf8_lossy(&argv0[..end]).into_owned())
      })
  };
  #[cfg(target_os = "linux")]
  let name: Option<String> = std::fs::read_to_string(format!("/proc/{raw}/comm"))
    .ok()
    .map(|c| c.trim().to_owned());
  #[cfg(not(any(target_os = "macos", target_os = "linux")))]
  let name: Option<String> = {
    let _ = raw;
    None
  };
  name
    .map(|n| n.rsplit('/').next().unwrap_or(&n).to_owned())
    .filter(|n| !n.is_empty())
}

/// The terminal's foreground process group; none when no process holds it
/// (on macOS, once the shell has gone). Not rustix's, which makes a `Pid`
/// of 0 unchecked.
pub(crate) fn foreground(tty: BorrowedFd<'_>) -> io::Result<Option<Pid>> {
  // SAFETY: plain call on an open descriptor.
  match unsafe { libc::tcgetpgrp(tty.as_raw_fd()) } {
    -1 => Err(io::Error::last_os_error()),
    pgrp => Ok(Pid::from_raw(pgrp)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::os::fd::AsFd;
  use std::time::{Duration, Instant};

  /// The terminal is the child's controlling terminal, with the child's
  /// group in the foreground, whatever the program does itself (`sleep`
  /// opens nothing).
  #[tokio::test]
  async fn the_terminal_is_the_childs_controlling_terminal() {
    takes_the_terminal(None).await;
  }

  /// A helper that is not there: the trampoline starts it.
  #[cfg(target_os = "macos")]
  #[tokio::test]
  async fn without_its_helper_the_shell_still_starts() {
    takes_the_terminal(Some("/nonexistent/stitch-shell-helper".into())).await;
  }

  async fn takes_the_terminal(helper: Option<PathBuf>) {
    let (pty, pts) = pty_process::open().unwrap();
    let tty = rustix::pty::ptsname(&pty, Vec::new()).unwrap();
    let child = spawn(&Spawn {
      program: "sleep".into(),
      args: vec!["10".into()],
      env: std::env::vars_os().collect(),
      cwd: std::env::temp_dir(),
      tty: OsStr::from_bytes(tty.as_bytes()).into(),
      helper,
      sandbox: None,
    })
    .unwrap();
    drop(pts);
    let pid = Pid::from_raw(child.id()).unwrap();
    let start = Instant::now();
    let mut seen = None;
    while start.elapsed() < Duration::from_secs(5) {
      seen = foreground(pty.as_fd()).unwrap();
      if seen == Some(pid) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    assert_eq!(seen, Some(pid));
  }

  #[test]
  fn a_program_not_found_is_said_before_starting() {
    let e = resolve(
      OsStr::new("no-such-program-here"),
      &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
      Path::new("/"),
    )
    .unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
  }
}
