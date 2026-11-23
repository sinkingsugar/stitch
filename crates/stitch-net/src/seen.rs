//! When each peer was last reached, and how (`peers-seen.toml`, beside
//! `peers.toml`): what `stitch peer ls` shows, written by the running node.
//! Local, never synced.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// How a connection to a peer runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathKind {
  /// Straight to one of its addresses (LAN, or a hole punched).
  Direct,
  /// Through a relay server.
  Relay,
}

/// What this node last knew of one peer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen {
  /// Unix seconds of the last connection that worked.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reached: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<PathKind>,
  /// Round-trip time on the selected path, in milliseconds.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rtt_ms: Option<u64>,
  /// The address of a direct path.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub addr: Option<String>,
  /// Unix seconds of the last attempt that failed, when after `reached`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failed: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  /// The last sync with it, in milliseconds.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sync_ms: Option<u64>,
}

/// Every peer's `Seen`, by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SeenPeers {
  #[serde(default)]
  pub peer: BTreeMap<String, Seen>,
}

pub(crate) fn now_secs() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

impl SeenPeers {
  pub fn load(path: &Path) -> SeenPeers {
    fs::read_to_string(path)
      .ok()
      .and_then(|t| toml::from_str(&t).ok())
      .unwrap_or_default()
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    let text = toml::to_string(self).map_err(|e| Error::Config(e.to_string()))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(|e| Error::Io {
      path: tmp.display().to_string(),
      source: e,
    })?;
    fs::rename(&tmp, path).map_err(|e| Error::Io {
      path: path.display().to_string(),
      source: e,
    })
  }
}

impl Seen {
  /// Whether the peer was last reached on the local network: straight to
  /// a private, link-local or loopback address. `None`: not known (never
  /// reached, or no address kept).
  pub fn on_lan(&self) -> Option<bool> {
    match self.path? {
      PathKind::Relay => Some(false),
      PathKind::Direct => {
        let addr: std::net::SocketAddr = self.addr.as_deref()?.parse().ok()?;
        Some(is_local_ip(addr.ip()))
      }
    }
  }
}

/// An address on the local network: private (10/8, 172.16/12,
/// 192.168/16, IPv6 unique local), link-local or loopback. A tailnet's
/// (100.64/10) and public ones are not.
pub fn is_local_ip(ip: std::net::IpAddr) -> bool {
  match ip {
    std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
    std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
      Some(v4) => is_local_ip(v4.into()),
      None => {
        let first = v6.segments()[0];
        v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
      }
    },
  }
}

/// The path a connection runs on now (the selected one), its round trip,
/// and the direct addresses it holds open.
pub(crate) fn paths(
  conn: &Connection,
) -> (
  Option<PathKind>,
  Option<Duration>,
  Vec<std::net::SocketAddr>,
) {
  let list = conn.paths();
  let mut kind = None;
  let mut rtt = None;
  let mut direct = Vec::new();
  for p in list.iter() {
    if let iroh::TransportAddr::Ip(a) = p.remote_addr() {
      direct.push(*a);
    }
    if p.is_selected() {
      kind = Some(if p.is_ip() {
        PathKind::Direct
      } else {
        PathKind::Relay
      });
      rtt = Some(p.rtt());
    }
  }
  (kind, rtt, direct)
}

/// "3m ago", "2h ago", "5d ago".
pub fn ago(secs: u64) -> String {
  let d = now_secs().saturating_sub(secs);
  match d {
    0..60 => format!("{d}s ago"),
    60..3600 => format!("{}m ago", d / 60),
    3600..86400 => format!("{}h ago", d / 3600),
    _ => format!("{}d ago", d / 86400),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn local_network_addresses() {
    for a in [
      "192.168.0.10",
      "10.0.0.2",
      "172.20.1.1",
      "169.254.3.4",
      "127.0.0.1",
      "::1",
      "fe80::1",
      "fd12::7",
      "::ffff:192.168.0.2",
    ] {
      assert!(is_local_ip(a.parse().unwrap()), "{a}");
    }
    for a in [
      "100.101.102.103",
      "8.8.8.8",
      "2001:db8::1",
      "::ffff:1.2.3.4",
    ] {
      assert!(!is_local_ip(a.parse().unwrap()), "{a}");
    }
    let seen = |path, addr: &str| Seen {
      path: Some(path),
      addr: Some(addr.into()),
      ..Seen::default()
    };
    assert_eq!(
      seen(PathKind::Direct, "192.168.0.2:4433").on_lan(),
      Some(true)
    );
    assert_eq!(
      seen(PathKind::Direct, "100.64.0.9:4433").on_lan(),
      Some(false)
    );
    assert_eq!(
      seen(PathKind::Relay, "192.168.0.2:4433").on_lan(),
      Some(false)
    );
    assert_eq!(Seen::default().on_lan(), None);
  }
}
