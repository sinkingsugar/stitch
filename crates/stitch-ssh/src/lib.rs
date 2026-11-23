//! SSH for Stitch (design §2.20): shell sessions on other machines that
//! are `stitch_shell::Session`s like local ones (so waits, jobs, watches
//! and full-screen programs work the same), run inside tmux where the
//! host has it, so they outlive a dropped connection and are picked up
//! again from any node.
//!
//! Over russh with ring: pure Rust, built for iOS and visionOS too.
//!
//! - `client`: connections (through a jump host), host keys trusted on
//!   first use and refused when changed, authentication by agent, key
//!   files, a password, or this device's key.
//! - `shell`: a session on a host (tmux in control mode, or a plain PTY).
//! - `config`: `~/.ssh/config`.
//! - `device`: this device's own key (the Secure Enclave on iOS).

pub mod client;
pub mod config;
pub mod device;
pub mod shell;
pub mod tmux;

/// An SSH server in the test's own process (and, under `cfg(test)`, this
/// crate's tests against it); the `testing` feature lends it to other
/// crates' tests.
#[cfg(all(
  any(test, feature = "testing"),
  not(any(target_os = "ios", target_os = "visionos"))
))]
#[path = "tests.rs"]
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub mod testing;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use client::{Auth, Connection, DeviceSigner, Exec, HostKeys, MemoryHostKeys, Target};
pub use russh::keys::{PrivateKey, PublicKey};
pub use shell::{Link, Opened, ShellOptions, open, session_name};

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("connecting to {host} failed: {why}")]
  Connect { host: String, why: String },
  #[error(
    "the host key of {host} ({at}) has changed: it was {}, it is now {now}. Someone may be \
     intercepting the connection, or the host was reinstalled; if the change is expected, \
     remove the old key (`stitch ssh hostkey rm {at}`, or Settings → SSH Hosts in the app) \
     and connect again",
    was.join(", ")
  )]
  HostKeyChanged {
    host: String,
    at: String,
    was: Vec<String>,
    now: String,
  },
  #[error(
    "{user}@{host} refused every way this device has to authenticate (tried {}). Add this \
     device's key to the host (`stitch ssh authorize {host}`, or `stitch ssh key` to see it), \
     or give the host a password secret",
    if tried.is_empty() { "nothing: none is set up".to_owned() } else { tried.join(", ") }
  )]
  Auth {
    host: String,
    user: String,
    tried: Vec<String>,
  },
  #[error("{host}: {why}")]
  Channel { host: String, why: String },
  #[error("starting a shell on {host} failed: {why}")]
  Setup { host: String, why: String },
  #[error(transparent)]
  Shell(#[from] stitch_shell::Error),
  #[error("{0}")]
  Key(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// One connection per host, shared by its sessions; a dropped one is made
/// again when next asked for.
#[derive(Default)]
pub struct Pool {
  conns: tokio::sync::Mutex<HashMap<String, Arc<Connection>>>,
}

impl Pool {
  /// The connection to `target` (by name), connecting when there is none
  /// or it dropped; whether it is new.
  pub async fn get(
    &self,
    target: &Target,
    keys: Arc<dyn HostKeys>,
    known_hosts: Option<PathBuf>,
  ) -> Result<(Arc<Connection>, bool)> {
    let mut conns = self.conns.lock().await;
    if let Some(c) = conns.get(&target.name)
      && !c.is_closed()
    {
      return Ok((c.clone(), false));
    }
    let c = Connection::connect(target, keys, known_hosts).await?;
    conns.insert(target.name.clone(), c.clone());
    Ok((c, true))
  }

  /// Forgets (and ends) the connection to `name`.
  pub async fn drop_host(&self, name: &str) {
    let c = self.conns.lock().await.remove(name);
    if let Some(c) = c {
      c.disconnect().await;
    }
  }

  /// Ends every connection.
  pub async fn close_all(&self) {
    let all: Vec<Arc<Connection>> = self.conns.lock().await.drain().map(|(_, c)| c).collect();
    for c in all {
      c.disconnect().await;
    }
  }
}
