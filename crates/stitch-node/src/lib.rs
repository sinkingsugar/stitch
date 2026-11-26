//! The Stitch node (design §2.1): one per data directory. It holds the
//! record roots, the index and the blobs, serves the user's other nodes over
//! iroh and syncs with them, runs chats against the providers, and serves
//! local clients (the CLI and TUI of the same binary) over a Unix socket.
//!
//! The data directory holds `config.toml`, `node.key`, `peers.toml`,
//! `credentials.toml` (only without a keychain), `index.sqlite`, `blobs/`,
//! and while the node runs `node.lock`, `node.sock` and `node.log`.

pub mod actions;
pub mod api;
pub mod apns;
pub mod apps;
mod artefacts;
pub mod auth;
pub mod browser;
pub mod chat;
mod checkout;
pub mod client;
pub mod clones;
pub mod config;
pub mod context;
pub mod cron;
mod decide;
mod delete;
mod detached;
mod digests;
mod edits;
mod elsewhere;
mod error;
pub mod events;
pub mod failed;
mod fixer;
pub mod folders;
pub mod fork;
pub mod github;
mod hazard;
mod imagegen;
mod images;
pub mod instructions;
mod issues;
mod jobs;
pub mod keys;
mod live;
pub mod logstamp;
mod loose;
pub mod mcp;
pub mod memory;
pub mod meta;
pub mod migrate;
mod mirror;
pub mod modelset;
mod node;
pub mod notices;
pub mod otter;
mod peer;
pub mod personal;
pub mod place;
pub mod placeset;
pub mod pricing;
pub mod profile;
mod prompt;
mod remote;
pub mod replies;
pub mod rewind;
mod rings;
pub mod rooms;
pub mod rules;
pub mod sandbox;
mod schedules;
pub mod scopes;
mod secret;
pub mod secrets;
mod selfimprove;
pub mod setup;
pub mod share;
mod ssh;
#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
mod toasts;
pub mod tools;
pub mod toolset;
mod viewer;
mod weave;
pub mod web;
mod window;

pub use error::Error;
pub use node::{
  NodeHandle, NodeLock, RESTARTING_FOR, Restart, Supervision, THREAD_FOLDERS, lock_wait,
  media_type, media_type_of, node_pid, restart_due, restarting_marker, running,
  set_installed_check, set_supervision, socket_path, start, start_locked,
};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The version local clients and the node must agree on: the package's,
/// and the local protocol's, bumped whenever `api` changes shape.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "/local-76");

/// The format of the store (records, segments, the index) this build
/// writes: bumped by a migration an older build cannot read after
/// (docs/self-improve.md §4, 7). A node never rolls back to a build of
/// another format by itself.
pub const STORE_FORMAT: u32 = 1;

static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Says which build this process is (docs/self-improve.md §2.1): the
/// commits it was built from, set once by the binary that links the node
/// (the CLI's build script computes it). `VERSION` names the protocol only,
/// so two builds of different commits can share it.
pub fn set_build(build: impl Into<String>) {
  let _ = BUILD.set(build.into());
}

/// This process's build, `unknown` when nothing said it.
pub fn build() -> &'static str {
  BUILD.get().map(String::as_str).unwrap_or("unknown")
}

/// The shape of `api` that `VERSION`'s local protocol number stands for: a
/// hash of its code without comments and blank lines. A test fails when
/// `api` changes without it, so the protocol number is bumped with the
/// change: postcard names variants by their position, and a client of the
/// same version but another shape misreads every message (an old
/// browser-host retried for hours against a node with two new requests).
#[cfg(test)]
const API_SHAPE: &str = "293364f743a1a162";

/// What `STORE_FORMAT` stands for: a hash of the code that reads and
/// writes the store's files (the migrations, the record and segment
/// formats, the store's journal, root, blobs and change feed), without
/// comments and blank lines. A test fails when it
/// changes, so whoever changes it decides: a change an older build cannot
/// read bumps `STORE_FORMAT` (no build rolls back across it by itself);
/// either way, `STORE_SHAPE` is set to the new hash.
#[cfg(test)]
// `Part::Spoken` (2026-11-12): an older build reads it as `Other`, so
// STORE_FORMAT stays. Its `note` (2026-11-13): a build that knows only
// `spoken` reads it as the first note, so it stays again. `Part::Scope`
// (2026-11-13, design §2.27): an older build reads it as `Other` and
// leaves it out, so STORE_FORMAT stays. `Part::RoomTurn` (2026-11-13,
// design §2.27): likewise read as `Other`, so it stays again.
const STORE_SHAPE: &str = "2633cd005a60fb11";

#[cfg(test)]
mod store_format {
  fn shape() -> String {
    let mut h = blake3::Hasher::new();
    for src in [
      include_str!("migrate.rs"),
      include_str!("../../stitch-records/src/record.rs"),
      include_str!("../../stitch-records/src/segment.rs"),
      include_str!("../../stitch-store/src/journal.rs"),
      include_str!("../../stitch-store/src/root.rs"),
      include_str!("../../stitch-store/src/blobs.rs"),
      include_str!("../../stitch-store/src/changes.rs"),
    ] {
      let code = src.find("\n#[cfg(test)]").map_or(src, |i| &src[..i]);
      for line in code.lines().map(str::trim) {
        if !line.is_empty() && !line.starts_with("//") {
          h.update(line.as_bytes());
          h.update(b"\n");
        }
      }
    }
    h.finalize().to_hex()[..16].to_string()
  }

  #[test]
  fn the_store_format_is_decided_with_its_code() {
    assert_eq!(
      shape(),
      crate::STORE_SHAPE,
      "the code of the store's files changed (migrate.rs, stitch-records' record.rs or \
       segment.rs, stitch-store's journal.rs, root.rs, blobs.rs or changes.rs): if a build from before cannot read what this one writes, bump \
       STORE_FORMAT (lib.rs; a node never rolls back across it); either way set \
       STORE_SHAPE to the new hash"
    );
  }
}
