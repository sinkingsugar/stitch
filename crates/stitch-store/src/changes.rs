//! The change feed: what came into the repositories this node holds, as
//! the index learns of it (design §2.6). Sync pushes a changed repository
//! to the other nodes from it; anything else that ships changes (a backup,
//! a remote store) can follow the same feed.
//!
//! A change is reported once the index holds it: by `Index::refresh` for
//! files found in a root (written by this process or another), and by
//! `Index::poll_changes` for what another process indexed meanwhile.

use std::sync::{Arc, Mutex};

use stitch_records::RepoId;

/// Files that came into, or went from, one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
  pub repo: RepoId,
  /// Root-relative paths, as sync names them; empty when the change was
  /// seen only as a different fingerprint (another process indexed it).
  pub added: Vec<String>,
  pub removed: usize,
  /// The peer the files came from, when a sync brought them.
  pub origin: Option<String>,
}

type Listener = Box<dyn Fn(&Change) + Send + Sync>;

/// Who is told of changes. Cheap to clone; every clone is the same feed.
///
/// Listeners are called on the thread that refreshed, under the index's
/// lock: they hand the change on (a channel) and return, and never touch
/// the index themselves.
#[derive(Clone, Default)]
pub struct ChangeFeed {
  listeners: Arc<Mutex<Vec<Listener>>>,
}

impl std::fmt::Debug for ChangeFeed {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let n = self.listeners.lock().map(|l| l.len()).unwrap_or(0);
    write!(f, "ChangeFeed({n} listeners)")
  }
}

impl ChangeFeed {
  pub fn subscribe(&self, listener: impl Fn(&Change) + Send + Sync + 'static) {
    self
      .listeners
      .lock()
      .expect("feed lock")
      .push(Box::new(listener));
  }

  pub(crate) fn emit(&self, change: &Change) {
    for l in self.listeners.lock().expect("feed lock").iter() {
      l(change);
    }
  }
}
