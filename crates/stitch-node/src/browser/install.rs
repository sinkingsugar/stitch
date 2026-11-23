//! `stitch browser install` (design §2.18): the extension's files, to load
//! unpacked, and the native messaging host's manifest for every Chromium
//! browser found (Chrome, Chromium, Brave, Edge), pointing at a small
//! script that runs this binary as `stitch browser-host` for this data
//! directory. Sideloaded for now: Chrome ignores `--load-extension` since
//! 137, so the user clicks "Load unpacked" once per profile (B0, §15.1).

use std::path::{Path, PathBuf};

use serde_json::json;

use super::{EXTENSION_ID, HOST_NAME};

/// The extension, as built into this binary.
pub const FILES: &[(&str, &str)] = &[
  (
    "manifest.json",
    include_str!("../../../../apps/chrome/manifest.json"),
  ),
  ("sw.js", include_str!("../../../../apps/chrome/sw.js")),
  (
    "content.js",
    include_str!("../../../../apps/chrome/content.js"),
  ),
  (
    "popup.html",
    include_str!("../../../../apps/chrome/popup.html"),
  ),
  ("popup.js", include_str!("../../../../apps/chrome/popup.js")),
];

/// A short hash of the extension as built into this binary: what its
/// service worker reports as `build`, so the node can tell a Chrome still
/// running older files.
pub fn build_id() -> String {
  let mut h = blake3::Hasher::new();
  for (name, text) in FILES {
    h.update(name.as_bytes());
    h.update(&(text.len() as u64).to_le_bytes());
    h.update(text.as_bytes());
  }
  h.finalize().to_hex()[..12].to_owned()
}

/// The extension's files as `install` writes them (sw.js stamped).
fn files() -> Vec<(&'static str, String)> {
  let id = build_id();
  FILES
    .iter()
    .map(|(n, t)| {
      let t = if *n == "sw.js" {
        t.replace(
          "const BUILD = \"dev\";",
          &format!("const BUILD = \"{id}\";"),
        )
      } else {
        (*t).to_owned()
      };
      (*n, t)
    })
    .collect()
}

/// Rewrites the extension files of an existing install when they differ
/// from this binary's. Returns whether it rewrote any. Nothing is written
/// when there is no install (that is `stitch browser install`'s).
pub fn refresh(data: &Path) -> std::io::Result<bool> {
  let ext = data.join("browser").join("extension");
  if !ext.is_dir() {
    return Ok(false);
  }
  let mut changed = false;
  for (name, text) in files() {
    let path = ext.join(name);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
      std::fs::write(&path, text)?;
      changed = true;
    }
  }
  Ok(changed)
}

/// What `install` did.
pub struct Installed {
  pub extension: PathBuf,
  pub host: PathBuf,
  /// Browsers whose host manifest was written: (name, manifest path).
  pub manifests: Vec<(String, PathBuf)>,
}

/// Where each browser keeps native messaging hosts for the user, on this
/// system: (name, the browser's user data directory).
fn browsers(home: &Path) -> Vec<(&'static str, PathBuf)> {
  if cfg!(target_os = "macos") {
    let base = home.join("Library/Application Support");
    vec![
      ("Chrome", base.join("Google/Chrome")),
      ("Chrome Beta", base.join("Google/Chrome Beta")),
      ("Chrome Canary", base.join("Google/Chrome Canary")),
      ("Chromium", base.join("Chromium")),
      ("Brave", base.join("BraveSoftware/Brave-Browser")),
      ("Edge", base.join("Microsoft Edge")),
    ]
  } else {
    let base = std::env::var_os("XDG_CONFIG_HOME")
      .map(PathBuf::from)
      .unwrap_or_else(|| home.join(".config"));
    vec![
      ("Chrome", base.join("google-chrome")),
      ("Chrome Beta", base.join("google-chrome-beta")),
      ("Chrome Dev", base.join("google-chrome-unstable")),
      ("Chromium", base.join("chromium")),
      ("Brave", base.join("BraveSoftware/Brave-Browser")),
      ("Edge", base.join("microsoft-edge")),
    ]
  }
}

/// The host manifest, for a host script at `host`.
pub fn manifest(host: &Path) -> String {
  serde_json::to_string_pretty(&json!({
    "name": HOST_NAME,
    "description": "Stitch: relays the Stitch extension to the local Stitch node",
    "path": host,
    "type": "stdio",
    "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
  }))
  .expect("a manifest serialises")
}

/// Writes the extension into `<data>/browser/extension`, the host script,
/// and the host manifest into every browser found, or only into
/// `user_data_dirs` when given (test profiles).
pub fn install(
  data: &Path,
  exe: &Path,
  home: &Path,
  user_data_dirs: &[PathBuf],
) -> std::io::Result<Installed> {
  let dir = data.join("browser");
  let ext = dir.join("extension");
  std::fs::create_dir_all(&ext)?;
  for (name, text) in files() {
    std::fs::write(ext.join(name), text)?;
  }
  let host = dir.join("stitch-browser-host");
  let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
  std::fs::write(
    &host,
    format!(
      "#!/bin/sh\n# Started by Chrome for Stitch's extension (written by `stitch browser install`).\nexec {} --data {} browser-host \"$@\"\n",
      quote(exe),
      quote(data)
    ),
  )?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755))?;
  }
  let targets: Vec<(String, PathBuf)> = if user_data_dirs.is_empty() {
    browsers(home)
      .into_iter()
      .filter(|(_, d)| d.is_dir())
      .map(|(n, d)| (n.to_owned(), d))
      .collect()
  } else {
    user_data_dirs
      .iter()
      .map(|d| (d.display().to_string(), d.clone()))
      .collect()
  };
  let mut manifests = Vec::new();
  for (name, d) in targets {
    let hosts = d.join("NativeMessagingHosts");
    std::fs::create_dir_all(&hosts)?;
    let path = hosts.join(format!("{HOST_NAME}.json"));
    std::fs::write(&path, manifest(&host))?;
    manifests.push((name, path));
  }
  Ok(Installed {
    extension: ext,
    host,
    manifests,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn installs_into_the_browsers_found() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let profile = tmp.path().join("profile");
    let done = install(
      &data,
      Path::new("/usr/bin/stitch"),
      &home,
      std::slice::from_ref(&profile),
    )
    .unwrap();
    assert!(done.extension.join("manifest.json").is_file());
    let script = std::fs::read_to_string(&done.host).unwrap();
    assert!(script.contains("'/usr/bin/stitch' --data '"), "{script}");
    let m: serde_json::Value =
      serde_json::from_str(&std::fs::read_to_string(&done.manifests[0].1).unwrap()).unwrap();
    assert_eq!(
      m["allowed_origins"][0],
      format!("chrome-extension://{EXTENSION_ID}/")
    );
    // The pinned key in the manifest gives that ID.
    let manifest: serde_json::Value = serde_json::from_str(FILES[0].1).unwrap();
    let key = base64::Engine::decode(
      &base64::engine::general_purpose::STANDARD,
      manifest["key"].as_str().unwrap(),
    )
    .unwrap();
    let digest = ring::digest::digest(&ring::digest::SHA256, &key);
    let id: String = digest.as_ref()[..16]
      .iter()
      .flat_map(|b| [b >> 4, b & 15])
      .map(|n| (b'a' + n) as char)
      .collect();
    assert_eq!(id, EXTENSION_ID);
  }

  #[test]
  fn the_installed_worker_carries_the_build_and_refresh_rewrites_stale_files() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let profile = tmp.path().join("profile");
    let done = install(&data, Path::new("/usr/bin/stitch"), tmp.path(), &[profile]).unwrap();
    let id = build_id();
    assert!(id.len() >= 8, "{id}");
    let sw = std::fs::read_to_string(done.extension.join("sw.js")).unwrap();
    assert!(sw.contains(&format!("const BUILD = \"{id}\";")));
    assert!(!sw.contains("const BUILD = \"dev\";"));
    // A current install is left alone.
    assert!(!refresh(&data).unwrap());
    // A stale one is rewritten.
    std::fs::write(done.extension.join("sw.js"), "old").unwrap();
    assert!(refresh(&data).unwrap());
    assert_eq!(
      std::fs::read_to_string(done.extension.join("sw.js")).unwrap(),
      sw
    );
    assert!(!refresh(&data).unwrap());
    // No install: nothing is created.
    let none = tmp.path().join("none");
    assert!(!refresh(&none).unwrap());
    assert!(!none.join("browser").exists());
  }
}
