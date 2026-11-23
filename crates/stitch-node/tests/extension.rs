//! The Chrome extension's service worker and content script against mocks
//! of Chrome's APIs and the DOM (`apps/chrome/test/sw.test.mjs`,
//! `content.test.mjs`), in Node. Skipped (passing) where no `node` is found.

use std::path::Path;
use std::process::Command;

fn run_node(name: &str) {
  let script = Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("../../apps/chrome/test")
    .join(name);
  let out = match Command::new("node").arg(&script).output() {
    Ok(out) => out,
    Err(e) => {
      eprintln!("skipped: no node ({e})");
      return;
    }
  };
  let text = format!(
    "{}{}",
    String::from_utf8_lossy(&out.stdout),
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(out.status.success(), "{text}");
  assert!(text.contains("ok "), "{text}");
}

#[test]
fn the_service_worker_passes_its_cases() {
  run_node("sw.test.mjs");
}

#[test]
fn the_content_script_passes_its_cases() {
  run_node("content.test.mjs");
}
