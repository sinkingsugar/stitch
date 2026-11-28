//! The `antigravity` provider (design §2.8, Antigravity): models on the
//! Google Antigravity plan, through Google's `agy` CLI. The bridge
//! (`stitch-antigravity`) is desktop only; this is the node's side.

use std::path::Path;
use std::sync::Arc;

use stitch_providers::Provider;

pub use stitch_antigravity::HOOK_SUBCOMMAND;
pub use stitch_antigravity::KIND;
/// `stitch antigravity-hook`: agy's PreToolUse gate.
pub use stitch_antigravity::hook::run as hook;

/// The provider for the data directory `data`, its effort as
/// `thought_level`.
pub fn provider(data: &Path, effort: Option<String>) -> Provider {
  let bridge = stitch_antigravity::bridge(stitch_antigravity::Options::new(data));
  Provider::External(Arc::new(stitch_antigravity::Antigravity {
    bridge,
    effort: effort.filter(|e| !e.is_empty()),
  }))
}

/// Whether agy is installed and signed in here.
pub fn signed_in(data: &Path) -> bool {
  stitch_antigravity::bridge(stitch_antigravity::Options::new(data))
    .ready()
    .is_ok()
}
