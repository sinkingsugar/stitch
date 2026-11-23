//! Provider API keys (design §2.2): in the OS keychain, or where there is
//! none (a headless Linux box) in `credentials.toml`, readable by the owner
//! only. Never in records, so they never sync.
//!
//! Read in this order: the data directory's `credentials.toml`, the
//! environment (`STITCH_<PROVIDER>_API_KEY`, then the provider's usual
//! variable), the keychain. A keychain entry belongs to one data directory
//! (its account carries a hash of the directory's path, as OAuth tokens'
//! do), so a second node on the machine, or a test, never reads or
//! overwrites another's key, and never asks the keychain for one it does
//! not have. Entries from before that (account = the provider) are still
//! read for the default data directory, `~/.stitch`. On iOS and visionOS
//! the account is the provider: the app has one data directory
//! (`secret::scope`).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::{Error, Result, secret};

const SERVICE: &str = "stitch";
const FILE: &str = "credentials.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
  Keychain,
  File,
}

fn read_file(data: &Path) -> BTreeMap<String, String> {
  std::fs::read_to_string(data.join(FILE))
    .ok()
    .and_then(|t| toml::from_str(&t).ok())
    .unwrap_or_default()
}

fn write_file(data: &Path, keys: &BTreeMap<String, String>) -> Result<()> {
  let path = data.join(FILE);
  let tmp = data.join("credentials.toml.tmp");
  let text = toml::to_string(keys).map_err(|e| Error::Config(e.to_string()))?;
  let mut opts = std::fs::OpenOptions::new();
  opts.write(true).create(true).truncate(true);
  #[cfg(unix)]
  std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
  let mut f = opts.open(&tmp).map_err(Error::io(tmp.display()))?;
  f.write_all(text.as_bytes())
    .map_err(Error::io(tmp.display()))?;
  f.sync_all().map_err(Error::io(tmp.display()))?;
  std::fs::rename(&tmp, &path).map_err(Error::io(path.display()))
}

/// The keychain account for a provider's key in `data`.
fn account(data: &Path, provider: &str) -> String {
  match secret::scope(data) {
    Ok(None) => provider.to_owned(),
    Ok(Some(id)) => format!("{provider}:{id}"),
    // Not there yet: as written.
    Err(_) => format!(
      "{provider}:{}",
      blake3::hash(data.to_string_lossy().as_bytes())
    ),
  }
}

/// Whether `data` is the default data directory, whose keys were once
/// stored under the bare provider name.
fn is_default(data: &Path) -> bool {
  let Some(home) = std::env::var_os("HOME") else {
    return false;
  };
  let default = Path::new(&home).join(".stitch");
  match (std::fs::canonicalize(data), std::fs::canonicalize(&default)) {
    (Ok(a), Ok(b)) => a == b,
    _ => false,
  }
}

fn keychain_get(data: &Path, provider: &str) -> Option<String> {
  let get = |account: &str| secret::get(SERVICE, account).ok().flatten();
  get(&account(data, provider)).or_else(|| is_default(data).then(|| get(provider)).flatten())
}

/// What this node keeps under `entry` (a provider's name, or a scoped
/// secret's `<scope>/<name>`): the file, then the keychain, never the
/// environment (secrets, design §2.19).
pub(crate) fn stored(data: &Path, entry: &str) -> Option<String> {
  read_file(data)
    .remove(entry)
    .or_else(|| keychain_get(data, entry))
}

/// Where this node keeps what it is not told where to keep (secrets that
/// arrive from other nodes, its sealing key): the file when it already
/// has one (a node without a keychain, or one the user chose the file
/// for) or `STITCH_SECRETS_FILE` is set (tests), else the keychain.
pub(crate) fn store_for(data: &Path) -> Store {
  if data.join(FILE).exists() || std::env::var_os("STITCH_SECRETS_FILE").is_some() {
    Store::File
  } else {
    Store::Keychain
  }
}

/// The names `credentials.toml` holds.
pub(crate) fn file_entries(data: &Path) -> Vec<String> {
  read_file(data).into_keys().collect()
}

/// The key for a provider, from wherever it is.
pub fn get(data: &Path, provider: &str, kind: &str) -> Option<String> {
  if let Some(k) = read_file(data).remove(provider) {
    return Some(k);
  }
  let var = format!(
    "STITCH_{}_API_KEY",
    provider.to_ascii_uppercase().replace('-', "_")
  );
  if let Ok(k) = std::env::var(var) {
    return Some(k);
  }
  standard_vars(provider, kind)
    .iter()
    .find_map(|v| std::env::var(v).ok().filter(|k| !k.is_empty()))
    .or_else(|| keychain_get(data, provider))
}

/// The environment variables a provider's own tools read its key from.
fn standard_vars(provider: &str, kind: &str) -> &'static [&'static str] {
  match (provider, kind) {
    // Before the kind: Kimi is on the Anthropic adapter, with its own key.
    ("kimi", _) => &["MOONSHOT_API_KEY", "KIMI_API_KEY"],
    ("deepseek", _) => &["DEEPSEEK_API_KEY"],
    ("openrouter", _) => &["OPENROUTER_API_KEY"],
    (_, "anthropic") => &["ANTHROPIC_API_KEY"],
    ("openai", _) => &["OPENAI_API_KEY"],
    ("grok", _) => &["XAI_API_KEY"],
    // Google's SDKs read either, `GEMINI_API_KEY` first.
    ("gemini", _) | (_, "gemini") => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
    ("brave", _) => &["BRAVE_API_KEY"],
    ("perplexity", _) => &["PERPLEXITY_API_KEY"],
    _ => &[],
  }
}

/// Stores a key in the keychain, or in the file where there is none or
/// `store` says so. Returns where it went.
pub fn set(data: &Path, provider: &str, key: &str, store: Store) -> Result<Store> {
  // The account names the directory as it will be found again.
  std::fs::create_dir_all(data).map_err(Error::io(data.display()))?;
  if store == Store::Keychain && secret::set(SERVICE, &account(data, provider), key).is_ok() {
    // A copy in the file would outlive a change in the keychain.
    remove_from_file(data, provider)?;
    return Ok(Store::Keychain);
  }
  let mut keys = read_file(data);
  keys.insert(provider.to_owned(), key.to_owned());
  write_file(data, &keys)?;
  Ok(Store::File)
}

fn remove_from_file(data: &Path, provider: &str) -> Result<()> {
  let mut keys = read_file(data);
  if keys.remove(provider).is_some() {
    write_file(data, &keys)?;
  }
  Ok(())
}

/// Removes a provider's key from the keychain and the file.
pub fn remove(data: &Path, provider: &str) -> Result<()> {
  secret::delete(SERVICE, &account(data, provider));
  if is_default(data) {
    secret::delete(SERVICE, provider);
  }
  remove_from_file(data, provider)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn kimi_and_deepseek_read_their_vendors_variables() {
    // Kimi is on the Anthropic adapter, but never takes Claude's key.
    assert_eq!(
      standard_vars("kimi", "anthropic"),
      &["MOONSHOT_API_KEY", "KIMI_API_KEY"]
    );
    assert_eq!(standard_vars("deepseek", "openai"), &["DEEPSEEK_API_KEY"]);
    assert_eq!(
      standard_vars("openrouter", "openai"),
      &["OPENROUTER_API_KEY"]
    );
    assert_eq!(
      standard_vars("anthropic", "anthropic"),
      &["ANTHROPIC_API_KEY"]
    );
    assert!(standard_vars("ollama", "openai").is_empty());
  }

  #[test]
  fn file_keys_are_private_and_removable() {
    let dir = tempfile::tempdir().unwrap();
    let provider = "stitch-test-provider-file";
    assert_eq!(
      set(dir.path(), provider, "sk-1", Store::File).unwrap(),
      Store::File
    );
    assert_eq!(get(dir.path(), provider, "openai").as_deref(), Some("sk-1"));
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(dir.path().join(FILE))
        .unwrap()
        .permissions()
        .mode();
      assert_eq!(mode & 0o777, 0o600);
    }
    remove_from_file(dir.path(), provider).unwrap();
    assert_eq!(get(dir.path(), provider, "openai"), None);
  }

  #[test]
  fn keychain_entries_belong_to_their_data_directory() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    assert_ne!(
      account(a.path(), "anthropic"),
      account(b.path(), "anthropic")
    );
    assert_eq!(
      account(a.path(), "anthropic"),
      account(&a.path().join("."), "anthropic"),
      "the same directory however it is written"
    );
    assert!(account(a.path(), "anthropic").starts_with("anthropic:"));
    assert!(!is_default(a.path()));
  }
}
