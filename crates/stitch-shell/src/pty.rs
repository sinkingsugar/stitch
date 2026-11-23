//! Allocating a terminal.
//!
//! macOS now and then fails `posix_openpt` with -6: XNU's `EREDRIVEOPEN`,
//! the kernel's own signal that two opens raced for one PTY minor
//! (`tty_ptmx.c`, retried in `vfs_vnops.c`), let out to userspace. It is
//! not `ENXIO` (+6), and the next try succeeds, so allocation is tried
//! again, a few times with a growing pause, on that error alone, before any
//! shell is started. What fails is said with its stage and raw errno.

#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::Duration;

/// XNU's `EREDRIVEOPEN` (`bsd/sys/errno.h`, kernel-internal).
const EREDRIVEOPEN: i32 = -6;
/// Tries in all, and the pause after the first (doubled after each).
const TRIES: u32 = 8;
const PAUSE: Duration = Duration::from_millis(1);

/// A terminal allocation that failed.
#[derive(Debug, thiserror::Error)]
#[error(
  "allocating a terminal failed at {stage}: {} (errno {errno}){}",
  describe(*.errno),
  if *.tries > 1 { format!(", after {} tries", .tries) } else { String::new() }
)]
pub struct OpenError {
  /// The call that failed.
  pub stage: &'static str,
  /// Its raw errno, as the kernel gave it.
  pub errno: i32,
  pub tries: u32,
}

fn describe(errno: i32) -> String {
  if errno == EREDRIVEOPEN {
    "EREDRIVEOPEN, the kernel's PTY allocation race".into()
  } else {
    std::io::Error::from_raw_os_error(errno).to_string()
  }
}

/// One try's failure.
#[derive(Debug)]
struct Failed {
  stage: &'static str,
  errno: i32,
}

fn at(stage: &'static str) -> impl FnOnce(rustix::io::Errno) -> Failed {
  move |e| Failed {
    stage,
    errno: e.raw_os_error(),
  }
}

/// A terminal: the master (non-blocking, on the runtime), the slave open,
/// and its path.
pub(crate) struct Opened {
  pub pty: pty_process::Pty,
  pub pts: OwnedFd,
  pub tty: PathBuf,
}

/// Allocates a terminal, trying again on `EREDRIVEOPEN`.
pub(crate) async fn open() -> Result<Opened, OpenError> {
  retrying(open_once).await
}

async fn retrying<T>(mut once: impl FnMut() -> Result<T, Failed>) -> Result<T, OpenError> {
  let mut pause = PAUSE;
  let mut tries = 0;
  loop {
    tries += 1;
    match once() {
      Ok(t) => return Ok(t),
      Err(f) if f.errno == EREDRIVEOPEN && tries < TRIES => {
        tokio::time::sleep(pause).await;
        pause *= 2;
      }
      Err(f) => {
        return Err(OpenError {
          stage: f.stage,
          errno: f.errno,
          tries,
        });
      }
    }
  }
}

/// What `pty_process::open` does, stage by stage.
fn open_once() -> Result<Opened, Failed> {
  use rustix::pty::OpenptFlags;
  // Close-on-exec from the start on Linux (a program started meanwhile
  // by another thread inherits no copy); elsewhere the flag does not
  // exist, and it is set right after.
  #[cfg(target_os = "linux")]
  let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC;
  #[cfg(not(target_os = "linux"))]
  let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY;
  let master = rustix::pty::openpt(flags).map_err(at("posix_openpt"))?;
  rustix::pty::grantpt(&master).map_err(at("grantpt"))?;
  rustix::pty::unlockpt(&master).map_err(at("unlockpt"))?;
  #[cfg(not(target_os = "linux"))]
  {
    let flags = rustix::io::fcntl_getfd(&master).map_err(at("fcntl(F_GETFD)"))?;
    rustix::io::fcntl_setfd(&master, flags | rustix::io::FdFlags::CLOEXEC)
      .map_err(at("fcntl(F_SETFD)"))?;
  }
  let name = rustix::pty::ptsname(&master, Vec::new()).map_err(at("ptsname"))?;
  let tty = PathBuf::from(OsStr::from_bytes(name.as_bytes()));
  let pts: OwnedFd = std::fs::OpenOptions::new()
    .read(true)
    .write(true)
    .custom_flags(libc::O_NOCTTY)
    .open(&tty)
    .map_err(|e| Failed {
      stage: "opening the terminal",
      errno: e.raw_os_error().unwrap_or(libc::EIO),
    })?
    .into();
  let fl = rustix::fs::fcntl_getfl(master.as_fd()).map_err(at("fcntl(F_GETFL)"))?;
  rustix::fs::fcntl_setfl(master.as_fd(), fl | rustix::fs::OFlags::NONBLOCK)
    .map_err(at("fcntl(F_SETFL)"))?;
  // SAFETY: a PTY master, open, non-blocking: what `from_fd` asks.
  let pty = unsafe { pty_process::Pty::from_fd(master) }.map_err(|e| Failed {
    stage: "registering it with the runtime",
    errno: match e {
      pty_process::Error::Io(e) => e.raw_os_error().unwrap_or(libc::EIO),
      pty_process::Error::Rustix(e) => e.raw_os_error(),
      _ => libc::EIO,
    },
  })?;
  Ok(Opened { pty, pts, tty })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn failing(stage: &'static str, errno: i32, times: u32) -> impl FnMut() -> Result<u32, Failed> {
    let mut n = 0;
    move || {
      n += 1;
      if n <= times {
        Err(Failed { stage, errno })
      } else {
        Ok(n)
      }
    }
  }

  #[tokio::test]
  async fn the_allocation_race_is_tried_again_and_other_errors_are_not() {
    // EREDRIVEOPEN a few times: the next try has it.
    assert_eq!(
      retrying(failing("posix_openpt", EREDRIVEOPEN, 3))
        .await
        .unwrap(),
      4
    );
    // Every time: given up within the bound, the stage and errno kept.
    let e = retrying(failing("posix_openpt", EREDRIVEOPEN, u32::MAX))
      .await
      .unwrap_err();
    assert_eq!((e.stage, e.errno, e.tries), ("posix_openpt", -6, TRIES));
    assert!(e.to_string().contains("EREDRIVEOPEN"), "{e}");
    assert!(e.to_string().contains("errno -6"), "{e}");
    // Anything else (ENXIO is +6) is said at once.
    let e = retrying(failing("ptsname", libc::ENXIO, 1))
      .await
      .unwrap_err();
    assert_eq!((e.stage, e.errno, e.tries), ("ptsname", libc::ENXIO, 1));
    assert!(!e.to_string().contains("tries"), "{e}");
  }

  #[tokio::test]
  async fn a_terminal_is_allocated() {
    use std::os::fd::AsFd;
    let t = open().await.unwrap();
    assert!(t.tty.starts_with("/dev/"), "{:?}", t.tty);
    // No program started holds a copy of the master.
    let flags = rustix::io::fcntl_getfd(t.pty.as_fd()).unwrap();
    assert!(flags.contains(rustix::io::FdFlags::CLOEXEC));
  }
}
