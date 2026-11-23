//! A node's key and the peers it trusts. The key is the node's iroh
//! identity; the peers file is the allowlist of the user's other nodes.

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use iroh::{EndpointAddr, EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
  move |source| Error::Io {
    path: path.display().to_string(),
    source,
  }
}

/// Reads the node's secret key, creating it (readable by the owner only) on
/// first use.
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
  match fs::read(path) {
    Ok(bytes) => {
      let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Config(format!("{}: not a 32-byte key", path.display())))?;
      Ok(SecretKey::from_bytes(&bytes))
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
      let key = SecretKey::generate();
      if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io(parent))?;
      }
      let mut opts = fs::OpenOptions::new();
      opts.write(true).create_new(true);
      #[cfg(unix)]
      std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
      let mut f = opts.open(path).map_err(io(path))?;
      f.write_all(&key.to_bytes()).map_err(io(path))?;
      f.sync_all().map_err(io(path))?;
      Ok(key)
    }
    Err(e) => Err(io(path)(e)),
  }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
  pub name: String,
  pub id: String,
  /// Direct addresses, for networks without relays and discovery.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub addrs: Vec<SocketAddr>,
}

impl Peer {
  pub fn endpoint_id(&self) -> Result<EndpointId> {
    self
      .id
      .parse()
      .map_err(|e| Error::Config(format!("peer {}: bad id: {e}", self.name)))
  }

  pub fn addr(&self) -> Result<EndpointAddr> {
    let mut addr = EndpointAddr::new(self.endpoint_id()?);
    for a in &self.addrs {
      addr = addr.with_ip_addr(*a);
    }
    Ok(addr)
  }
}

/// `peers.toml`: the nodes allowed to connect, by endpoint ID.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peers {
  #[serde(default, rename = "peer")]
  pub peers: Vec<Peer>,
}

impl Peers {
  pub fn load(path: &Path) -> Result<Self> {
    match fs::read_to_string(path) {
      Ok(text) => {
        toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Peers::default()),
      Err(e) => Err(io(path)(e)),
    }
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
  }

  /// Adds or replaces the peer with this name.
  pub fn add(&mut self, peer: Peer) -> Result<()> {
    peer.endpoint_id()?;
    self
      .peers
      .retain(|p| p.name != peer.name && p.id != peer.id);
    self.peers.push(peer);
    Ok(())
  }

  /// A peer by name or endpoint ID.
  pub fn find(&self, name_or_id: &str) -> Option<&Peer> {
    self
      .peers
      .iter()
      .find(|p| p.name == name_or_id || p.id == name_or_id)
  }

  pub fn allows(&self, id: &EndpointId) -> Option<&Peer> {
    let id = id.to_string();
    self.peers.iter().find(|p| p.id == id)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_key_is_created_once_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node.key");
    let a = load_or_create_key(&path).unwrap();
    let b = load_or_create_key(&path).unwrap();
    assert_eq!(a.public(), b.public());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
      );
    }
  }

  #[test]
  fn peers_round_trip_and_allow_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.toml");
    let id = SecretKey::generate().public();
    let mut peers = Peers::load(&path).unwrap();
    peers
      .add(Peer {
        name: "phone".into(),
        id: id.to_string(),
        addrs: vec!["127.0.0.1:4433".parse().unwrap()],
      })
      .unwrap();
    peers.save(&path).unwrap();
    let back = Peers::load(&path).unwrap();
    assert_eq!(back, peers);
    assert_eq!(back.allows(&id).unwrap().name, "phone");
    assert!(back.allows(&SecretKey::generate().public()).is_none());
    assert!(
      peers
        .add(Peer {
          name: "x".into(),
          id: "nope".into(),
          addrs: vec![]
        })
        .is_err()
    );
  }
}
