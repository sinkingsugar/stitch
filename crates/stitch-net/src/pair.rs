//! Pairing (design §2.6): a node that is already trusted shows a ticket (a QR
//! code or text) with its endpoint ID, addresses and a one-time secret. The
//! new node connects to that ID, which proves it reached the right node, and
//! returns the secret, which proves it saw the ticket. The inviter then
//! admits the new node with a `peer` record in the home workspace, which
//! spreads to every node by sync, or in its local list without one.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use data_encoding::BASE32_NOPAD;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{EndpointAddr, EndpointId, PublicKey};
use serde::{Deserialize, Serialize};
use stitch_records::NodeName;

use crate::link::{Net, refusal};
use crate::node::{Event, NodeState};
use crate::proto::{read_message, write_message};
use crate::{Error, Peer, Result};

pub const PAIR_ALPN: &[u8] = b"stitch/pair/1";
const PAIR_VERSION: u32 = 1;
const TICKET_PREFIX: &str = "STITCH1";
/// Wrong secrets an invitation survives before it is withdrawn.
const MAX_ATTEMPTS: u32 = 5;

/// What a new node needs to join: whom to reach, where, and the secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
  pub id: [u8; 32],
  pub secret: [u8; 16],
  pub addrs: Vec<SocketAddr>,
}

impl Ticket {
  pub fn endpoint_id(&self) -> Result<EndpointId> {
    PublicKey::from_bytes(&self.id)
      .map_err(|e| Error::Config(format!("ticket: bad endpoint ID: {e}")))
  }

  pub fn addr(&self) -> Result<EndpointAddr> {
    let mut addr = EndpointAddr::new(self.endpoint_id()?);
    for a in &self.addrs {
      addr = addr.with_ip_addr(*a);
    }
    Ok(addr)
  }
}

/// Upper case base32, which a QR code stores in its compact alphanumeric mode.
impl std::fmt::Display for Ticket {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let bytes = postcard::to_stdvec(self).expect("a ticket serializes");
    write!(f, "{TICKET_PREFIX}{}", BASE32_NOPAD.encode(&bytes))
  }
}

impl std::str::FromStr for Ticket {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self> {
    let s = s.trim().to_ascii_uppercase();
    let body = s
      .strip_prefix(TICKET_PREFIX)
      .ok_or_else(|| Error::Config("not a Stitch pairing ticket".into()))?;
    let bytes = BASE32_NOPAD
      .decode(body.as_bytes())
      .map_err(|e| Error::Config(format!("ticket: {e}")))?;
    postcard::from_bytes(&bytes).map_err(|e| Error::Config(format!("ticket: {e}")))
  }
}

pub(crate) struct Invitation {
  secret: [u8; 16],
  expires: Instant,
  failures: u32,
}

#[derive(Serialize, Deserialize)]
enum PairFrame {
  Join {
    version: u32,
    secret: [u8; 16],
    name: String,
  },
  Welcome {
    name: String,
    home: Option<String>,
    fleet: Vec<WirePeer>,
  },
  Refused {
    reason: String,
  },
}

/// `Peer` on the wire: postcard is not self-describing, so no field may be
/// skipped, as `Peer` skips empty addresses in `peers.toml`.
#[derive(Serialize, Deserialize)]
struct WirePeer {
  name: String,
  id: String,
  addrs: Vec<SocketAddr>,
}

impl From<Peer> for WirePeer {
  fn from(p: Peer) -> Self {
    WirePeer {
      name: p.name,
      id: p.id,
      addrs: p.addrs,
    }
  }
}

impl From<WirePeer> for Peer {
  fn from(p: WirePeer) -> Self {
    Peer {
      name: p.name,
      id: p.id,
      addrs: p.addrs,
    }
  }
}

/// What a node learns by joining.
#[derive(Clone, Debug)]
pub struct Joined {
  pub inviter: Peer,
  /// The home workspace's repository ID, if the inviter holds it.
  pub home: Option<String>,
  /// The other nodes the inviter trusts.
  pub fleet: Vec<Peer>,
}

fn same(a: &[u8; 16], b: &[u8; 16]) -> bool {
  a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Net {
  /// Opens an invitation for `ttl` and returns its ticket. A new invitation
  /// replaces the previous one. `addrs` overrides the advertised addresses.
  pub fn invite(&self, ttl: Duration, addrs: Option<Vec<SocketAddr>>) -> Ticket {
    let mut secret = [0u8; 16];
    rand::fill(&mut secret);
    *self.state().invitation.lock().expect("invitation lock") = Some(Invitation {
      secret,
      expires: Instant::now() + ttl,
      failures: 0,
    });
    let addrs = addrs.unwrap_or_else(|| self.endpoint().addr().ip_addrs().copied().collect());
    Ticket {
      id: *self.endpoint().id().as_bytes(),
      secret,
      addrs,
    }
  }

  /// Withdraws the open invitation, if any.
  pub fn withdraw_invitation(&self) {
    *self.state().invitation.lock().expect("invitation lock") = None;
  }

  /// Joins the node that showed `ticket`, and remembers it and the nodes it
  /// trusts in the local list.
  pub async fn join(&self, ticket: &Ticket) -> Result<Joined> {
    let conn = self
      .endpoint()
      .connect(ticket.addr()?, PAIR_ALPN)
      .await
      .map_err(Error::net)?;
    let result = async {
      let (mut send, mut recv) = conn.open_bi().await.map_err(Error::net)?;
      let join = PairFrame::Join {
        version: PAIR_VERSION,
        secret: ticket.secret,
        name: self.state().node.to_string(),
      };
      write_message(&mut send, &join, "Join").await?;
      send.finish().map_err(Error::net)?;
      match read_message(&mut recv).await? {
        PairFrame::Welcome { name, home, fleet } => Ok((name, home, fleet)),
        PairFrame::Refused { reason } => Err(Error::Refused(reason)),
        PairFrame::Join { .. } => Err(Error::Protocol("expected Welcome".into())),
      }
    }
    .await
    .map_err(|e| refusal(&conn, e));
    conn.close(0u32.into(), b"done");
    let (name, home, fleet) = result?;
    let inviter = Peer {
      name,
      id: ticket.endpoint_id()?.to_string(),
      addrs: ticket.addrs.clone(),
    };
    let own = self.endpoint().id().to_string();
    let fleet: Vec<Peer> = fleet
      .into_iter()
      .map(Peer::from)
      .filter(|p| p.id != own && p.id != inviter.id)
      .collect();
    self
      .state()
      .remember(std::iter::once(inviter.clone()).chain(fleet.iter().cloned()))?;
    Ok(Joined {
      inviter,
      home,
      fleet,
    })
  }
}

#[derive(Clone)]
pub(crate) struct PairHandler {
  pub(crate) state: Arc<NodeState>,
  /// This node's own endpoint ID, for its own `peer` record.
  pub(crate) id: EndpointId,
}

impl std::fmt::Debug for PairHandler {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "PairHandler({})", self.state.node)
  }
}

impl ProtocolHandler for PairHandler {
  async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
    if self
      .state
      .invitation
      .lock()
      .expect("invitation lock")
      .is_none()
    {
      conn.close(1u32.into(), b"not accepting new nodes");
      return Ok(());
    }
    let remote = conn.remote_id();
    let Ok((mut send, mut recv)) = conn.accept_bi().await else {
      return Ok(());
    };
    let reply = match read_message::<PairFrame>(&mut recv).await {
      Ok(PairFrame::Join {
        version,
        secret,
        name,
      }) => {
        let state = self.state.clone();
        let own = self.id;
        tokio::task::spawn_blocking(move || admit(&state, own, remote, version, secret, &name))
          .await
          .unwrap_or_else(|e| Err(Error::net(e)))
      }
      Ok(_) => Err(Error::Protocol("expected Join".into())),
      Err(e) => Err(e),
    };
    let frame = match reply {
      Ok((frame, peer)) => {
        self.state.emit(Event::Paired(peer));
        frame
      }
      Err(Error::Refused(reason)) => PairFrame::Refused { reason },
      Err(e) => PairFrame::Refused {
        reason: e.to_string(),
      },
    };
    if write_message(&mut send, &frame, "pairing reply")
      .await
      .is_ok()
    {
      let _ = send.finish();
      // Wait for the joiner to read the reply and close.
      conn.closed().await;
    }
    Ok(())
  }
}

/// Checks the secret and admits the node; the reply to send and the peer.
fn admit(
  state: &NodeState,
  own: EndpointId,
  remote: EndpointId,
  version: u32,
  secret: [u8; 16],
  name: &str,
) -> Result<(PairFrame, Peer)> {
  if version != PAIR_VERSION {
    return Err(Error::Refused(format!(
      "pairing version {version}, this node speaks {PAIR_VERSION}"
    )));
  }
  {
    let mut slot = state.invitation.lock().expect("invitation lock");
    let valid = match slot.as_mut() {
      Some(inv) if inv.expires > Instant::now() => {
        if same(&inv.secret, &secret) {
          true
        } else {
          inv.failures += 1;
          if inv.failures >= MAX_ATTEMPTS {
            *slot = None;
          }
          false
        }
      }
      _ => false,
    };
    if !valid {
      return Err(Error::Refused("wrong or expired invitation".into()));
    }
    *slot = None;
  }
  let name: NodeName = name
    .parse()
    .map_err(|_| Error::Refused(format!("`{name}` is not a valid node name")))?;
  let remote_id = remote.to_string();
  let known = state.known_peers()?;
  if name == state.node
    || known
      .iter()
      .any(|p| p.name == name.as_str() && p.id != remote_id)
  {
    return Err(Error::Refused(format!(
      "the name `{name}` is taken by another node"
    )));
  }
  let peer = Peer {
    name: name.to_string(),
    id: remote_id.clone(),
    addrs: Vec::new(),
  };
  // In the home workspace both nodes get a record, so the whole fleet learns
  // of them; without one, only this node's local list changes.
  if state.admit(state.node.as_str(), &own)? {
    state.admit(name.as_str(), &remote)?;
  } else {
    state.remember([peer.clone()])?;
  }
  let fleet = state
    .known_peers()?
    .into_iter()
    .filter(|p| p.id != remote_id)
    .map(WirePeer::from)
    .collect();
  let welcome = PairFrame::Welcome {
    name: state.node.to_string(),
    home: state.home().map(|h| h.repo().to_string()),
    fleet,
  };
  Ok((welcome, peer))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tickets_round_trip_as_upper_case_base32() {
    let t = Ticket {
      id: *iroh::SecretKey::generate().public().as_bytes(),
      secret: [7; 16],
      addrs: vec!["192.168.0.20:47123".parse().unwrap()],
    };
    let s = t.to_string();
    assert!(s.starts_with("STITCH1"));
    assert!(
      s.bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    );
    assert_eq!(s.parse::<Ticket>().unwrap(), t);
    assert_eq!(s.to_lowercase().parse::<Ticket>().unwrap(), t);
    assert!("STITCH1AAAA".parse::<Ticket>().is_err());
    assert!("hello".parse::<Ticket>().is_err());
  }
}
