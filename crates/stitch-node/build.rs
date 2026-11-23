//! `cfg(mobile)`: iOS and visionOS, where the node starts no programs (no
//! shells, no Chrome) and keeps secrets in the data protection keychain.

fn main() {
  println!("cargo::rustc-check-cfg=cfg(mobile)");
  let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
  if matches!(os.as_str(), "ios" | "visionos") {
    println!("cargo::rustc-cfg=mobile");
  }
}
