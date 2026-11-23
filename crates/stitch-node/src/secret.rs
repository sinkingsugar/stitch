//! The OS keychain, for provider keys and OAuth tokens: the `keyring`
//! crate's store on macOS and Linux; on iOS and visionOS, which it does not
//! cover, the data protection keychain through Security.framework, on this
//! device only and readable after the first unlock (background refresh runs
//! while the phone is locked).

use std::path::Path;

/// The keychain cannot be reached (locked, no secret service): not the same
/// as an entry that is not there.
#[derive(Debug)]
pub(crate) struct Unavailable;

/// What makes a data directory's entries its own, put in their account
/// names: a hash of its path, so a second node on the machine, or a test,
/// never reads another's. None on iOS and visionOS: the app has one data
/// directory, and its path changes when the app is reinstalled or updated.
pub(crate) fn scope(data: &Path) -> std::io::Result<Option<String>> {
  if cfg!(mobile) {
    return Ok(None);
  }
  let canonical = std::fs::canonicalize(data)?;
  Ok(Some(
    blake3::hash(canonical.to_string_lossy().as_bytes()).to_string(),
  ))
}

#[cfg(not(mobile))]
mod imp {
  use super::Unavailable;

  fn entry(service: &str, account: &str) -> Result<keyring::Entry, Unavailable> {
    keyring::Entry::new(service, account).map_err(|_| Unavailable)
  }

  pub(crate) fn get(service: &str, account: &str) -> Result<Option<String>, Unavailable> {
    match entry(service, account)?.get_password() {
      Ok(s) => Ok(Some(s)),
      Err(keyring::Error::NoEntry) => Ok(None),
      Err(_) => Err(Unavailable),
    }
  }

  pub(crate) fn set(service: &str, account: &str, secret: &str) -> Result<(), Unavailable> {
    entry(service, account)?
      .set_password(secret)
      .map_err(|_| Unavailable)
  }

  pub(crate) fn delete(service: &str, account: &str) {
    if let Ok(e) = entry(service, account) {
      let _ = e.delete_credential();
    }
  }
}

#[cfg(mobile)]
mod imp {
  use super::Unavailable;
  use security_framework::access_control::{ProtectionMode, SecAccessControl};
  use security_framework::passwords::{
    PasswordOptions, delete_generic_password, generic_password, set_generic_password_options,
  };

  const NOT_FOUND: i32 = -25300; // errSecItemNotFound

  pub(crate) fn get(service: &str, account: &str) -> Result<Option<String>, Unavailable> {
    match generic_password(PasswordOptions::new_generic_password(service, account)) {
      Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| Unavailable),
      Err(e) if e.code() == NOT_FOUND => Ok(None),
      Err(_) => Err(Unavailable),
    }
  }

  pub(crate) fn set(service: &str, account: &str, secret: &str) -> Result<(), Unavailable> {
    // Replaced whole: an update would match on the protection too.
    delete(service, account);
    let mut options = PasswordOptions::new_generic_password(service, account);
    let protection = SecAccessControl::create_with_protection(
      Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
      0,
    )
    .map_err(|_| Unavailable)?;
    options.set_access_control(protection);
    set_generic_password_options(secret.as_bytes(), options).map_err(|_| Unavailable)
  }

  pub(crate) fn delete(service: &str, account: &str) {
    let _ = delete_generic_password(service, account);
  }
}

pub(crate) use imp::{delete, get, set};
