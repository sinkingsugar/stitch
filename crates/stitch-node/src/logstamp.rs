//! The node's log with each line's local time (design §2.3, "The node's
//! log"). `node.log` is the node's stdout and stderr, as launchd, systemd or
//! `connect_or_start` opened them; its lines had no time, so a late report
//! or a sync stall could not be placed. When that output is not a terminal,
//! `install` sends both through a pipe to a thread that writes each line
//! where they went, its time first. A panic is written there directly as
//! well, so one that ends the process is not lost in the pipe.

use std::io::{BufRead, Write};

/// A line's time as the log says it: local, to the millisecond.
fn now() -> String {
  jiff::Zoned::now()
    .strftime("%Y-%m-%d %H:%M:%S%.3f")
    .to_string()
}

/// `line` as the log writes it, its time first.
fn stamped(at: &str, line: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(at.len() + line.len() + 2);
  out.extend_from_slice(at.as_bytes());
  out.push(b' ');
  out.extend_from_slice(line);
  out.push(b'\n');
  out
}

/// Copies `from` to `to` line by line, each stamped, until `from` ends.
fn copy_stamped(from: impl std::io::Read, mut to: impl Write) {
  for line in std::io::BufReader::new(from).split(b'\n') {
    let Ok(line) = line else { break };
    let _ = to.write_all(&stamped(&now(), &line));
  }
}

/// Stamps the process's stdout and stderr from now on (unix; a terminal's
/// output is left as it is). Every failure leaves them as they were.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn install() {
  use std::os::fd::FromRawFd;
  // SAFETY: plain descriptor calls; the descriptors made here are owned by
  // the `File`s built from them, and 1 and 2 are only replaced (dup2).
  unsafe {
    if libc::isatty(2) == 1 {
      return;
    }
    let out = libc::dup(2);
    if out < 0 {
      return;
    }
    let mut fds = [0; 2];
    if libc::pipe(fds.as_mut_ptr()) != 0 {
      libc::close(out);
      return;
    }
    let (read, write) = (fds[0], fds[1]);
    for fd in [read, write, out] {
      libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let panics = libc::dup(out);
    let reader = std::fs::File::from_raw_fd(read);
    let log = std::fs::File::from_raw_fd(out);
    // The reader first: with nobody reading, a full pipe would block every
    // write to the log.
    let started = std::thread::Builder::new()
      .name("node-log".into())
      .spawn(move || copy_stamped(reader, log));
    if started.is_err() || libc::dup2(write, 2) < 0 {
      libc::close(write);
      if panics >= 0 {
        libc::close(panics);
      }
      return;
    }
    let _ = std::io::stdout().flush();
    libc::dup2(write, 1);
    libc::close(write);
    if panics >= 0 {
      libc::fcntl(panics, libc::F_SETFD, libc::FD_CLOEXEC);
      let panics = std::sync::Mutex::new(std::fs::File::from_raw_fd(panics));
      std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let bt = std::backtrace::Backtrace::capture();
        let text = format!("{} thread '{name}' {info}\n{bt}\n", now());
        if let Ok(mut f) = panics.lock() {
          let _ = f.write_all(text.as_bytes());
        }
      }));
    }
  }
}

#[cfg(not(unix))]
pub fn install() {}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn each_line_gets_its_time() {
    let mut out = Vec::new();
    copy_stamped(&b"one\ntwo\n\nlast without newline"[..], &mut out);
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    for (line, want) in lines.iter().zip(["one", "two", "", "last without newline"]) {
      // `2026-10-27 10:13:05.123 one`
      let (date, rest) = line.split_at(23);
      assert!(date.parse::<jiff::civil::DateTime>().is_ok(), "{line}");
      assert_eq!(&rest[1..], want, "{line}");
    }
  }
}
