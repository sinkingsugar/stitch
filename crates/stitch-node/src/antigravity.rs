//! The `antigravity` provider (design §2.8, Antigravity): models on the
//! Google Antigravity plan, through Google's `agy` CLI. The bridge
//! (`stitch-antigravity`) is desktop only; this is the node's side.

use std::path::Path;
use std::sync::Arc;

use stitch_providers::Provider;

/// What `stitch auth logout antigravity` says: agy's sign-in is agy's.
pub use stitch_antigravity::LOGOUT;
/// `stitch antigravity-hook`: agy's PreToolUse gate.
pub use stitch_antigravity::hook::run as hook;
pub use stitch_antigravity::{HOOK_SUBCOMMAND, KIND};

fn options(data: &Path) -> stitch_antigravity::Options {
  stitch_antigravity::Options::new(data)
}

/// The provider for the data directory `data`, its effort as
/// `thought_level`.
pub fn provider(data: &Path, effort: Option<String>) -> Provider {
  let bridge = stitch_antigravity::bridge(options(data));
  Provider::External(Arc::new(stitch_antigravity::Antigravity {
    bridge,
    effort: effort.filter(|e| !e.is_empty()),
  }))
}

/// Whether agy is installed and signed in here.
pub fn signed_in(data: &Path) -> bool {
  stitch_antigravity::bridge(options(data)).ready().is_ok()
}

/// Its sign-in as `stitch auth status` says it.
pub fn status(data: &Path) -> String {
  options(data).status()
}

/// `stitch auth login antigravity`: agy's own sign-in on this terminal,
/// confirmed by `agy models`; what to say once done.
pub fn login(data: &Path) -> Result<String, String> {
  let opts = options(data);
  let models = opts.login()?;
  Ok(format!(
    "Signed in to Google Antigravity through agy ({}). Its models: {}.\n\
     Use antigravity/<model> (e.g. antigravity/gemini-4-hydrogen); the plan is used for them, \
     billed by subscription.",
    opts.token.display(),
    models.join(", ")
  ))
}
