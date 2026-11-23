//! Who is on the other end of the socket (docs/self-improve.md §4): the
//! user, or a thread's shell, told from the connecting process itself,
//! never from what it says (`STITCH_THREAD` is the caller's to set).
//!
//! The peer's process (SO_PEERCRED on Linux, LOCAL_PEERPID on macOS) is
//! walked up its parents: one that reaches a shell session this node
//! started is that session's thread; one that reaches the node itself (an
//! MCP server's) is the node's; only one with neither is the user. A
//! process in a shell's session (its session ID is the shell's) is that
//! thread's too, however it was reparented (a command left running in the
//! background once its shell is gone keeps its session).
//!
//! Ancestry alone misses a process a thread's command started in a session
//! of its own (`setsid`) and then orphaned (a double fork): it is taken
//! for the user. What only the user may do (answer a landing's question,
//! `the_user_alone`) also needs the peer outside any sandbox (`confined`):
//! a sandboxed shell's commands carry their sandbox however they detach,
//! and a fixer's shells are always sandboxed (where there is no sandbox,
//! each of its commands asks). An unsandboxed command of another thread
//! runs with the user's powers because the user let it run outside.

use std::sync::Arc;

use stitch_records::RecordId;

use crate::node::Shared;

/// Who asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
  /// The user: their terminal, their app.
  User,
  /// A thread's shell (and what its commands start).
  Thread(RecordId),
  /// Something of the node's own that is no thread's shell (an MCP
  /// server's process, an orphan the node took in): never the user.
  Node,
}

impl Caller {
  pub(crate) fn thread(&self) -> Option<&RecordId> {
    match self {
      Caller::Thread(t) => Some(t),
      _ => None,
    }
  }

  pub(crate) fn is_user(&self) -> bool {
    matches!(self, Caller::User)
  }
}

/// A process's parent and session.
#[allow(unsafe_code)]
fn parent_and_session(pid: i32) -> Option<(i32, i32)> {
  // SAFETY: getsid has no preconditions; it fails for a process gone.
  let sid = unsafe { libc::getsid(pid) };
  #[cfg(target_os = "linux")]
  {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let ppid: i32 = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((ppid, sid))
  }
  #[cfg(target_os = "macos")]
  {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the buffer is a proc_bsdinfo of the size given.
    let n = unsafe {
      libc::proc_pidinfo(
        pid,
        libc::PROC_PIDTBSDINFO,
        0,
        info.as_mut_ptr().cast(),
        size,
      )
    };
    if n != size {
      return None;
    }
    // SAFETY: filled whole, as `n` says.
    let info = unsafe { info.assume_init() };
    Some((info.pbi_ppid as i32, sid))
  }
  // No shells here (iOS, visionOS): no ancestry to read.
  #[cfg(not(any(target_os = "linux", target_os = "macos")))]
  {
    let _ = sid;
    None
  }
}

/// The shells this node runs, by process: their threads.
async fn shells(s: &Arc<Shared>) -> Vec<(i32, RecordId)> {
  let sessions = s.shells.sessions.lock().await;
  sessions
    .iter()
    .filter_map(|(key, sess)| {
      let pid = sess.shell_pid()?;
      let thread = key.split(':').next()?.parse().ok()?;
      Some((pid, thread))
    })
    .collect()
}

/// The other end of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Peer {
  /// In this process: the app that links the node (the phone's). Like
  /// `Web`, a chat typed into another node's thread runs on its owner by
  /// default (`Peer::owner_default`).
  App,
  /// The web client's WebSocket (`apps/ws.rs`): the user, as `App`. The
  /// WebSocket is desktop-only, so a phone never makes one.
  #[cfg_attr(mobile, allow(dead_code))]
  Web,
  /// A process on the socket, when its ID could be read.
  Process(Option<u32>),
}

impl Peer {
  /// Whether a chat this peer types into an unpinned thread another node
  /// owns runs on that owner (design §2.7, "Continue on another node"): the
  /// user's clients (the app, the web client) yes; the TUI and other socket
  /// processes run it where typed.
  pub(crate) fn owner_default(self) -> bool {
    matches!(self, Peer::App | Peer::Web)
  }
}

/// Who the peer is. A socket peer whose process cannot be told is never
/// the user.
pub(crate) async fn identify(s: &Arc<Shared>, peer: Peer) -> Caller {
  let pid = match peer {
    Peer::App | Peer::Web => return Caller::User,
    Peer::Process(None) => return Caller::Node,
    Peer::Process(Some(p)) => p,
  };
  let shells = shells(s).await;
  classify(
    pid as i32,
    std::process::id() as i32,
    &shells,
    parent_and_session,
  )
}

/// Whether a process is confined by a sandbox, which it cannot leave: a
/// sandboxed shell's commands, however they detach (`setsid`, a double
/// fork). macOS: Seatbelt says so of any process (`sandbox_check`).
/// Linux: the shells' Landlock needs `no_new_privs`, which no process
/// clears. Either way a process whose state cannot be read counts as
/// confined.
#[allow(unsafe_code)]
pub(crate) fn confined(pid: i32) -> bool {
  #[cfg(target_os = "macos")]
  {
    unsafe extern "C" {
      fn sandbox_check(
        pid: libc::pid_t,
        operation: *const libc::c_char,
        kind: libc::c_int,
        ...
      ) -> libc::c_int;
    }
    // SAFETY: no operation and no filter (`SANDBOX_FILTER_NONE`): it only
    // asks whether `pid` runs sandboxed; 1 when it does, -1 for no process.
    let r = unsafe { sandbox_check(pid, std::ptr::null(), 0) };
    r != 0
  }
  #[cfg(target_os = "linux")]
  {
    match std::fs::read_to_string(format!("/proc/{pid}/status")) {
      Ok(status) => status
        .lines()
        .find_map(|l| l.strip_prefix("NoNewPrivs:"))
        .is_none_or(|v| v.trim() != "0"),
      Err(_) => true,
    }
  }
  #[cfg(not(any(target_os = "linux", target_os = "macos")))]
  {
    let _ = pid;
    false
  }
}

/// Whether the peer may answer a question that is the user's alone (a
/// landing, the fixers' settings): the user, from a process no thread
/// started (its ancestry) and in no sandbox (which a thread's sandboxed
/// commands carry however they detach). `Err` says why not.
pub(crate) async fn the_user_alone(s: &Arc<Shared>, peer: Peer) -> Result<(), String> {
  const REFUSED: &str = "only the user answers this question (the app, or a terminal of \
    theirs), never a thread's commands";
  let pid = match peer {
    Peer::App | Peer::Web => return Ok(()),
    Peer::Process(None) => return Err(REFUSED.into()),
    Peer::Process(Some(p)) => p as i32,
  };
  if !identify(s, peer).await.is_user() {
    return Err(REFUSED.into());
  }
  if confined(pid) {
    return Err(format!("{REFUSED}: this one runs in a sandbox"));
  }
  Ok(())
}

/// `identify`, with the process table given (for tests).
fn classify(
  pid: i32,
  node: i32,
  shells: &[(i32, RecordId)],
  lookup: impl Fn(i32) -> Option<(i32, i32)>,
) -> Caller {
  let thread_of = |p: i32| shells.iter().find(|(s, _)| *s == p).map(|(_, t)| t.clone());
  // The node's own process (its code, not a child's): as the app is.
  if pid == node {
    return Caller::User;
  }
  let mut cur = pid;
  let mut first = true;
  for _ in 0..128 {
    if let Some(t) = thread_of(cur) {
      return Caller::Thread(t);
    }
    if cur == node {
      return Caller::Node;
    }
    let Some((parent, sid)) = lookup(cur) else {
      // Gone, or not to be read: not the user's for sure only when it is
      // the caller itself we cannot read.
      return if first { Caller::Node } else { Caller::User };
    };
    if let Some(t) = thread_of(sid) {
      return Caller::Thread(t);
    }
    first = false;
    if parent <= 1 {
      return Caller::User;
    }
    cur = parent;
  }
  Caller::Node
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn callers_are_told_by_their_ancestry() {
    let t: RecordId = "2026-10-28T120000Z-mbp-abcd".parse().unwrap();
    let shells = vec![(500, t.clone())];
    // pid -> (parent, session)
    let table = |p: i32| match p {
      // A command in the thread's shell, two levels down.
      700 => Some((600, 500)),
      600 => Some((500, 500)),
      500 => Some((100, 500)),
      100 => Some((1, 100)),
      // The same, in a session of its own, still under the shell.
      710 => Some((600, 710)),
      // Orphaned from the shell's session (reparented to init).
      720 => Some((1, 500)),
      // An MCP server under the node.
      800 => Some((100, 100)),
      // The user's terminal.
      900 => Some((850, 850)),
      850 => Some((1, 850)),
      _ => None,
    };
    assert_eq!(
      classify(700, 100, &shells, table),
      Caller::Thread(t.clone())
    );
    assert_eq!(
      classify(710, 100, &shells, table),
      Caller::Thread(t.clone())
    );
    assert_eq!(classify(720, 100, &shells, table), Caller::Thread(t));
    assert_eq!(classify(800, 100, &shells, table), Caller::Node);
    assert_eq!(classify(900, 100, &shells, table), Caller::User);
    // A caller whose process cannot be read is not taken for the user.
    assert_eq!(classify(999, 100, &shells, table), Caller::Node);
    // The node's own code, in its process.
    assert_eq!(classify(100, 100, &shells, table), Caller::User);
  }

  /// The test runs outside any sandbox (as `just quick` does): itself not
  /// confined; a child under `sandbox-exec` is, and stays so in a session
  /// of its own.
  #[cfg(target_os = "macos")]
  #[test]
  fn a_sandboxed_process_is_told() {
    if confined(std::process::id() as i32) {
      eprintln!("this test runs in a sandbox: not checked");
      return;
    }
    let mut child = std::process::Command::new("/usr/bin/sandbox-exec")
      .args([
        "-p",
        "(version 1)(allow default)",
        "/usr/bin/perl",
        "-MPOSIX",
        "-e",
        "POSIX::setsid(); sleep 5",
      ])
      .spawn()
      .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(confined(child.id() as i32));
    child.kill().unwrap();
    let _ = child.wait();
  }

  #[test]
  fn this_process_has_a_parent() {
    let (parent, _) = parent_and_session(std::process::id() as i32).expect("readable");
    assert!(parent >= 1);
  }
}
