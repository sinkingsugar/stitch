//! Pairing over real iroh connections on localhost: tickets, admission by
//! `peer` records in the home workspace, the fleet spreading by sync, and
//! revocation.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use iroh::{EndpointAddr, SecretKey};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_records::{Author, Draft, Ref, kinds};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

const POLICY: Policy = Policy {
  blobs: BlobPolicy::Lazy,
};

struct TestNode {
  net: Net,
  key: SecretKey,
}

impl TestNode {
  fn state(&self) -> &Arc<NodeState> {
    self.net.state()
  }

  fn local_addr(&self) -> SocketAddr {
    let port = self
      .net
      .endpoint()
      .bound_sockets()
      .into_iter()
      .find(|a| a.is_ipv4())
      .unwrap()
      .port();
    ([127, 0, 0, 1], port).into()
  }

  fn addr(&self) -> EndpointAddr {
    EndpointAddr::new(self.key.public()).with_ip_addr(self.local_addr())
  }

  fn invite(&self) -> Ticket {
    self
      .net
      .invite(Duration::from_secs(60), Some(vec![self.local_addr()]))
  }

  fn home(&self) -> RecordRoot {
    self.state().home().unwrap()
  }

  async fn sync_home(&self, with: &TestNode) -> stitch_net::Result<stitch_net::SessionReport> {
    self.net.sync_with(with.addr(), self.home().repo()).await
  }
}

/// A node, holding the home workspace at `home` when given. `peers` is its
/// local list, carried over when a test restarts a node.
async fn node(
  dir: &Path,
  name: &str,
  key: &SecretKey,
  home: Option<&Path>,
  peers: Peers,
) -> TestNode {
  let mut roots = BTreeMap::new();
  if let Some(h) = home {
    let root = RecordRoot::open(h)
      .unwrap_or_else(|_| RecordRoot::init(h, "home".parse().unwrap(), Role::Home).unwrap());
    roots.insert(root.repo().clone(), root);
  }
  let state = NodeState::new(
    name.parse().unwrap(),
    roots,
    Index::open_in_memory().unwrap(),
    BlobStore::open(&dir.join(format!("{name}-blobs"))).unwrap(),
    peers,
    None,
    POLICY,
  );
  let endpoint = bind(key.clone(), Discovery::Local, None).await.unwrap();
  TestNode {
    net: Net::spawn(endpoint, Arc::new(state)),
    key: key.clone(),
  }
}

fn local_peers(n: &TestNode) -> Peers {
  n.state().peers.lock().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn nodes_join_by_ticket_and_the_fleet_spreads_through_the_home_workspace() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (ka, kb, kc, kd) = (
    SecretKey::generate(),
    SecretKey::generate(),
    SecretKey::generate(),
    SecretKey::generate(),
  );
  let laptop = node(d, "mbp", &ka, Some(&d.join("mbp-home")), Peers::default()).await;

  // Nothing to join before an invitation.
  let phone = node(d, "phone", &kb, None, Peers::default()).await;
  let mut ticket = laptop.invite();
  laptop.net.withdraw_invitation();
  assert!(phone.net.join(&ticket).await.is_err());

  // A wrong secret is refused; the right one admits, once.
  ticket = laptop.invite();
  let mut wrong = ticket.clone();
  wrong.secret[0] ^= 1;
  let err = phone.net.join(&wrong).await.unwrap_err();
  assert!(err.to_string().contains("wrong or expired"), "{err}");
  let joined = phone.net.join(&ticket).await.unwrap();
  assert_eq!(joined.inviter.name, "mbp");
  assert_eq!(joined.home.as_deref(), Some("home"));
  assert!(joined.fleet.is_empty());
  assert!(phone.state().allows(&ka.public()).is_some());
  let stranger = node(d, "nas", &kc, None, Peers::default()).await;
  let err = stranger.net.join(&ticket).await.unwrap_err();
  assert!(
    err.to_string().contains("not accepting new nodes"),
    "a ticket works once: {err}"
  );

  // The laptop wrote records for itself and the phone.
  assert_eq!(
    laptop.state().allows(&kb.public()).as_deref(),
    Some("phone")
  );
  {
    let index = laptop.state().index.lock().unwrap();
    assert_eq!(index.peer_records().unwrap().len(), 2);
  }

  // The phone takes the home workspace and syncs it: it now has the records.
  let phone = node(
    d,
    "phone",
    &kb,
    Some(&d.join("phone-home")),
    local_peers(&phone),
  )
  .await;
  let report = phone.sync_home(&laptop).await.unwrap();
  assert_eq!(report.received.len(), 2);

  // A third node joins through the laptop and learns of the phone.
  let nas = node(d, "nas", &kc, None, Peers::default()).await;
  let joined = nas.net.join(&laptop.invite()).await.unwrap();
  assert_eq!(
    joined
      .fleet
      .iter()
      .map(|p| p.name.as_str())
      .collect::<Vec<_>>(),
    ["phone"]
  );
  let nas = node(d, "nas", &kc, Some(&d.join("nas-home")), local_peers(&nas)).await;
  nas.sync_home(&laptop).await.unwrap();

  // The phone has never met the nas, but trusts it once the home workspace
  // brings its record; then the two sync directly.
  assert!(nas.sync_home(&phone).await.is_err());
  phone.sync_home(&laptop).await.unwrap();
  nas.sync_home(&phone).await.unwrap();

  // Names are unique.
  let impostor = node(d, "phone", &kd, None, Peers::default()).await;
  let err = impostor.net.join(&laptop.invite()).await.unwrap_err();
  assert!(err.to_string().contains("taken"), "{err}");

  // The phone revokes the nas; once the laptop has the revocation, the nas
  // is turned away there too.
  let nas_record = {
    let index = phone.state().index.lock().unwrap();
    index
      .peer_records()
      .unwrap()
      .into_iter()
      .find(|r| r.fields["name"] == "nas")
      .unwrap()
      .id
  };
  phone
    .home()
    .publish(
      &phone.state().node,
      Draft::new(kinds::REVOKE, Author::user(phone.state().node.clone()))
        .unwrap()
        .reference("revokes", Ref::to(nas_record))
        .unwrap(),
    )
    .unwrap();
  phone.sync_home(&laptop).await.unwrap();
  let err = nas.sync_home(&laptop).await.unwrap_err();
  assert!(
    err.to_string().contains("not one of this user's nodes"),
    "{err}"
  );
}
