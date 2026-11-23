//! Interactive shell sessions for agents and people (design §2.3).
//!
//! A session is a shell on a PTY, driven from the async runtime: no reader
//! threads, no locks held across I/O. State is read, not guessed:
//! - whether a program is running comes from the terminal's foreground
//!   process group (`tcgetpgrp`);
//! - that a command finished, and its exit status, comes from a marker the
//!   session's own rc prints before each prompt, carrying a per-session
//!   secret, so output cannot fake it.
//!
//! When neither has happened by the caller's deadline, the call says so
//! ("still running"); the caller decides to wait, type, or interrupt.
//! Output is kept twice: as a line log (what a line-oriented command
//! printed) and as a terminal screen (what a full-screen program shows).

mod helper;
mod keys;
mod log;
mod pty;
mod remote;
mod sandbox;
mod session;
mod spawn;

pub use helper::helper_main;
pub use keys::key;
pub use regex::Regex;
pub use remote::{BoxFuture, Remote, RemoteConfig, Resumed, remote_rc};
pub use sandbox::Sandbox;
pub use session::{
  Command, Config, Error, Outcome, Redact, Screen, ScreenUntil, Session, ShellKind, Status,
  StopSignal, Until, Wait, Waited, signal_group,
};
pub use spawn::CAN_SPAWN;

pub type Result<T, E = Error> = std::result::Result<T, E>;
