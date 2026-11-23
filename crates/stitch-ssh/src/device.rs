//! This device's own SSH key: what "Authorize on host" adds to a host's
//! `authorized_keys`.
//!
//! On iOS (a device, not the simulator) a P-256 key made in the Secure
//! Enclave (`ecdsa-sha2-nistp256`): its private half never leaves it, and
//! it signs without asking each time (the app lock covers the app). It is
//! found again by its label. Elsewhere, and where the Secure Enclave is
//! not there, an Ed25519 key the node keeps as a secret (`software_key`,
//! `to_openssh`/`from_openssh`).

use russh::keys::{Algorithm, PrivateKey, PublicKey};

use crate::{Error, Result};

/// A new Ed25519 key.
pub fn software_key(comment: &str) -> Result<PrivateKey> {
  let mut key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
    .map_err(|e| Error::Key(e.to_string()))?;
  key.set_comment(comment);
  Ok(key)
}

/// A key as kept (OpenSSH's private key format).
pub fn to_openssh(key: &PrivateKey) -> Result<String> {
  key
    .to_openssh(russh::keys::ssh_key::LineEnding::LF)
    .map(|s| s.to_string())
    .map_err(|e| Error::Key(e.to_string()))
}

pub fn from_openssh(text: &str) -> Result<PrivateKey> {
  PrivateKey::from_openssh(text).map_err(|e| Error::Key(e.to_string()))
}

/// A public key as `authorized_keys` takes it.
pub fn authorized_line(key: &PublicKey) -> String {
  key.to_openssh().unwrap_or_default()
}

/// A public key from its `authorized_keys` line.
pub fn parse_public(line: &str) -> Result<PublicKey> {
  PublicKey::from_openssh(line.trim()).map_err(|e| Error::Key(e.to_string()))
}

/// An ECDSA signature in DER (`SEQUENCE { INTEGER r, INTEGER s }`), as
/// Security.framework gives it, in SSH's form for `ecdsa-sha2-nistp256`:
/// the algorithm's name, then `mpint r, mpint s` as a string.
pub fn ecdsa_ssh_signature(der: &[u8]) -> Option<Vec<u8>> {
  fn read_len(d: &[u8], at: &mut usize) -> Option<usize> {
    let first = *d.get(*at)?;
    *at += 1;
    if first < 0x80 {
      return Some(first as usize);
    }
    let n = (first & 0x7f) as usize;
    let mut len = 0usize;
    for _ in 0..n {
      len = (len << 8) | *d.get(*at)? as usize;
      *at += 1;
    }
    Some(len)
  }
  fn int<'a>(d: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    if *d.get(*at)? != 0x02 {
      return None;
    }
    *at += 1;
    let len = read_len(d, at)?;
    let v = d.get(*at..*at + len)?;
    *at += len;
    Some(v)
  }
  fn mpint(out: &mut Vec<u8>, v: &[u8]) {
    let v: &[u8] = {
      let mut v = v;
      while v.len() > 1 && v[0] == 0 {
        v = &v[1..];
      }
      v
    };
    let pad = v.first().is_some_and(|b| b & 0x80 != 0);
    let len = v.len() + usize::from(pad);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    if pad {
      out.push(0);
    }
    out.extend_from_slice(v);
  }
  let mut at = 0;
  if *der.first()? != 0x30 {
    return None;
  }
  at += 1;
  read_len(der, &mut at)?;
  let r = int(der, &mut at)?;
  let s = int(der, &mut at)?;
  let mut inner = Vec::new();
  mpint(&mut inner, r);
  mpint(&mut inner, s);
  let name = b"ecdsa-sha2-nistp256";
  let mut out = Vec::new();
  out.extend_from_slice(&(name.len() as u32).to_be_bytes());
  out.extend_from_slice(name);
  out.extend_from_slice(&(inner.len() as u32).to_be_bytes());
  out.extend_from_slice(&inner);
  Some(out)
}

/// A P-256 public key from its uncompressed point (`04 || x || y`).
pub fn p256_public(sec1: &[u8], comment: &str) -> Result<PublicKey> {
  let ec = russh::keys::ssh_key::public::EcdsaPublicKey::from_sec1_bytes(sec1)
    .map_err(|e| Error::Key(e.to_string()))?;
  Ok(PublicKey::new(
    russh::keys::ssh_key::public::KeyData::Ecdsa(ec),
    comment,
  ))
}

#[cfg(any(target_os = "ios", target_os = "visionos"))]
pub mod enclave {
  //! The Secure Enclave key, through Security.framework.

  #![allow(unsafe_code)]

  use core_foundation::base::{CFType, TCFType};
  use core_foundation::boolean::CFBoolean;
  use core_foundation::dictionary::CFDictionary;
  use core_foundation::number::CFNumber;
  use core_foundation::string::CFString;
  use security_framework::access_control::{ProtectionMode, SecAccessControl};
  use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
  use security_framework::key::{Algorithm, SecKey};
  use security_framework_sys::item::{
    kSecAttrAccessControl, kSecAttrIsPermanent, kSecAttrKeySizeInBits, kSecAttrKeyType,
    kSecAttrKeyTypeECSECPrimeRandom, kSecAttrLabel, kSecAttrTokenID, kSecAttrTokenIDSecureEnclave,
    kSecPrivateKeyAttrs,
  };

  use super::{ecdsa_ssh_signature, p256_public};
  use crate::{DeviceSigner, Error, PublicKey, Result};

  const LABEL: &str = "dev.stitch.ssh.device-key";
  /// `kSecAccessControlPrivateKeyUsage`: signing with it, nothing asked.
  const PRIVATE_KEY_USAGE: core_foundation::base::CFOptionFlags = 1 << 30;

  /// The key, found or made.
  pub struct EnclaveKey {
    key: SecKey,
    public: PublicKey,
  }

  // SecKey is a CoreFoundation object, safe to use from any thread.
  unsafe impl Send for EnclaveKey {}
  unsafe impl Sync for EnclaveKey {}

  fn find() -> Option<SecKey> {
    let found = ItemSearchOptions::new()
      .class(ItemClass::key())
      .label(LABEL)
      .load_refs(true)
      .limit(Limit::Max(1))
      .search()
      .ok()?;
    found.into_iter().find_map(|r| match r {
      SearchResult::Ref(Reference::Key(k)) => Some(k),
      _ => None,
    })
  }

  /// Made with attributes of its own: `GenerateKeyOptions` (what
  /// `SecKey::new` takes) puts the private key's (permanent, its access
  /// control) only on macOS, so `generate` it is.
  #[allow(deprecated)]
  fn make() -> Result<SecKey> {
    let access = SecAccessControl::create_with_protection(
      Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
      PRIVATE_KEY_USAGE,
    )
    .map_err(|e| Error::Key(e.to_string()))?;
    // SAFETY: Security.framework's constant keys, valid for the process.
    let key = |k| unsafe { CFString::wrap_under_get_rule(k) };
    let private: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
      (
        key(unsafe { kSecAttrIsPermanent }),
        CFBoolean::true_value().as_CFType(),
      ),
      (
        key(unsafe { kSecAttrLabel }),
        CFString::new(LABEL).as_CFType(),
      ),
      (key(unsafe { kSecAttrAccessControl }), access.as_CFType()),
    ]);
    let attributes: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
      (
        key(unsafe { kSecAttrKeyType }),
        key(unsafe { kSecAttrKeyTypeECSECPrimeRandom }).as_CFType(),
      ),
      (
        key(unsafe { kSecAttrKeySizeInBits }),
        CFNumber::from(256).as_CFType(),
      ),
      (
        key(unsafe { kSecAttrTokenID }),
        key(unsafe { kSecAttrTokenIDSecureEnclave }).as_CFType(),
      ),
      (key(unsafe { kSecPrivateKeyAttrs }), private.as_CFType()),
    ]);
    SecKey::generate(attributes.to_untyped()).map_err(|e| Error::Key(e.to_string()))
  }

  impl EnclaveKey {
    /// The device's key, made the first time; `None` where there is no
    /// Secure Enclave (the simulator).
    pub fn load_or_make(comment: &str) -> Result<EnclaveKey> {
      let key = match find() {
        Some(k) => k,
        None => make()?,
      };
      let public = key
        .public_key()
        .and_then(|p| p.external_representation())
        .ok_or_else(|| Error::Key("the Secure Enclave key has no public half".into()))?;
      let public = p256_public(public.bytes(), comment)?;
      Ok(EnclaveKey { key, public })
    }
  }

  impl DeviceSigner for EnclaveKey {
    fn public_key(&self) -> PublicKey {
      self.public.clone()
    }

    fn sign(&self, data: &[u8]) -> std::result::Result<Vec<u8>, String> {
      let der = self
        .key
        .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, data)
        .map_err(|e| e.to_string())?;
      ecdsa_ssh_signature(&der).ok_or_else(|| "the Secure Enclave's signature did not parse".into())
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_software_key_round_trips() {
    let k = software_key("stitch@test").unwrap();
    let text = to_openssh(&k).unwrap();
    let back = from_openssh(&text).unwrap();
    assert_eq!(k.public_key(), back.public_key());
    let line = authorized_line(k.public_key());
    assert!(line.starts_with("ssh-ed25519 "), "{line}");
    assert!(line.ends_with(" stitch@test"), "{line}");
    assert_eq!(parse_public(&line).unwrap(), *k.public_key());
  }

  #[test]
  fn a_der_signature_becomes_ssh_s() {
    // r with its high bit set (padded), s short (leading zero dropped).
    let r = [0x80u8; 32];
    let mut s = vec![0u8; 31];
    s.push(7);
    // DER pads r with a zero (its high bit is set).
    let mut der = vec![0x30, 0, 0x02, 33, 0];
    der.extend_from_slice(&r);
    der.push(0x02);
    der.push(s.len() as u8);
    der.extend_from_slice(&s);
    der[1] = (der.len() - 2) as u8;
    let sig = ecdsa_ssh_signature(&der).unwrap();
    let name = b"ecdsa-sha2-nistp256";
    assert_eq!(&sig[4..4 + name.len()], name);
    let inner = &sig[4 + name.len() + 4..];
    // mpint r: 33 bytes (0x00 then 0x80…); mpint s: 1 byte.
    assert_eq!(&inner[..4], &33u32.to_be_bytes());
    assert_eq!(inner[4], 0);
    assert_eq!(&inner[37..41], &1u32.to_be_bytes());
    assert_eq!(inner[41], 7);
  }
}
