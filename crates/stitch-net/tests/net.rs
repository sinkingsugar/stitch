//! Two nodes over real iroh connections on localhost, without relays.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use iroh::{EndpointAddr, SecretKey};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peer, PeerSync, Peers, Policy, bind};
use stitch_records::{Attachment, Author, BlobHash, Draft, NodeName, Ref};
use stitch_store::{BlobStore, Index, RecordRoot, Role, TaskStatus};

struct TestNode {
  net: Net,
  key: SecretKey,
}

impl TestNode {
  fn state(&self) -> &Arc<NodeState> {
    self.net.state()
  }

  fn root(&self) -> RecordRoot {
    self.state().roots().remove(0)
  }

  fn name(&self) -> &NodeName {
    &self.state().node
  }

  /// Its endpoint ID alone: reached only on a connection already kept.
  fn addr_unknown(&self) -> EndpointAddr {
    EndpointAddr::new(self.key.public())
  }

  fn addr(&self) -> EndpointAddr {
    let port = self
      .net
      .endpoint()
      .bound_sockets()
      .into_iter()
      .find(|a| a.is_ipv4())
      .unwrap()
      .port();
    EndpointAddr::new(self.key.public()).with_ip_addr(([127, 0, 0, 1], port).into())
  }
}

const PHONE: Policy = Policy {
  blobs: BlobPolicy::Lazy,
};
const LAPTOP: Policy = Policy {
  blobs: BlobPolicy::All,
};

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_and_a_laptop_sync_records_and_blobs() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  RecordRoot::init(
    &d.join("phone-root"),
    "proj".parse().unwrap(),
    Role::Project,
  )
  .unwrap();
  RecordRoot::init(
    &d.join("laptop-root"),
    "proj".parse().unwrap(),
    Role::Project,
  )
  .unwrap();
  let (pk, lk) = (SecretKey::generate(), SecretKey::generate());
  let phone = node(
    d,
    "phone",
    &d.join("phone-root"),
    PHONE,
    pk.clone(),
    &[("mbp", &lk)],
  )
  .await;
  let laptop = node(
    d,
    "mbp",
    &d.join("laptop-root"),
    LAPTOP,
    lk,
    &[("phone", &pk)],
  )
  .await;

  // The phone writes a task with a photo.
  let photo = vec![42u8; 300_000];
  let hash = phone.state().blobs.put_bytes(&photo).unwrap();
  let task = phone
    .root()
    .publish(
      phone.name(),
      Draft::new("task", Author::user(phone.name().clone()))
        .unwrap()
        .attach(Attachment {
          hash,
          media_type: "image/jpeg".into(),
          name: Some("photo.jpg".into()),
          bytes: photo.len() as u64,
        })
        .body("# From the phone\n"),
    )
    .unwrap();

  let report = phone
    .net
    .sync_with(laptop.addr(), phone.root().repo())
    .await
    .unwrap();
  assert_eq!(report.sent, 1);
  assert!(report.received.is_empty());
  assert_eq!(report.version, 3);

  // The laptop has the record, and fetches the photo once the session is
  // over, on the same connection.
  assert!(laptop.root().read_record(&task.id).is_ok());
  for _ in 0..100 {
    if laptop.state().blobs.contains(&hash) {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  assert!(laptop.state().blobs.contains(&hash));
  assert!(laptop.state().blobs.verify(&hash).unwrap());

  // The laptop answers with a run and its own attachment; the phone, lazy,
  // gets the records but not the blob until it asks.
  let log_blob = laptop.state().blobs.put_bytes(b"build log").unwrap();
  let run = laptop
    .root()
    .publish(
      laptop.name(),
      Draft::new("run", Author::node(laptop.name().clone()))
        .unwrap()
        .reference("task", Ref::to(task.id.clone()))
        .unwrap()
        .attach(Attachment {
          hash: log_blob,
          media_type: "text/plain".into(),
          name: None,
          bytes: 9,
        }),
    )
    .unwrap();
  let report = phone
    .net
    .sync_with(laptop.addr(), phone.root().repo())
    .await
    .unwrap();
  assert_eq!(
    report.received,
    vec![format!("records/{}/{}.md", run.id.month(), run.id)]
  );
  assert_eq!(report.blobs_received, 0);
  assert!(!phone.state().blobs.contains(&log_blob));
  {
    let index = phone.state().index.lock().unwrap();
    assert_eq!(index.task_status(&task.id).unwrap(), TaskStatus::Running);
  }
  assert!(
    phone
      .net
      .fetch_blob(laptop.addr(), &log_blob)
      .await
      .unwrap()
  );
  assert!(phone.state().blobs.contains(&log_blob));
  assert!(
    !phone
      .net
      .fetch_blob(laptop.addr(), &BlobHash::of(b"nobody has this"))
      .await
      .unwrap()
  );

  // From whichever peer holds it, asked at once: one that cannot be
  // reached costs nothing.
  let notes = laptop.state().blobs.put_bytes(b"meeting notes").unwrap();
  let gone =
    EndpointAddr::new(SecretKey::generate().public()).with_ip_addr(([127, 0, 0, 1], 9).into());
  let find = std::time::Duration::from_secs(5);
  assert!(
    phone
      .net
      .fetch_blob_any(&[gone.clone(), laptop.addr()], &notes, find)
      .await
      .unwrap()
  );
  assert!(phone.state().blobs.contains(&notes));
  assert!(
    !phone
      .net
      .fetch_blob_any(&[gone, laptop.addr()], &BlobHash::of(b"nobody"), find)
      .await
      .unwrap()
  );

  // Nothing left to exchange: one round.
  let report = phone
    .net
    .sync_with(laptop.addr(), phone.root().repo())
    .await
    .unwrap();
  assert_eq!(
    (report.rounds, report.sent, report.received.len()),
    (1, 0, 0)
  );

  // A summary: the same on both sides, no session at all.
  let how = PeerSync {
    seal: true,
    connect: std::time::Duration::from_secs(5),
    parallel: 4,
  };
  let repos = [phone.root().repo().clone(), "elsewhere".parse().unwrap()];
  let r = phone
    .net
    .sync_peer(&laptop.addr(), &repos[..1], how)
    .await
    .unwrap();
  assert_eq!((r.same.len(), r.synced.len()), (1, 0), "{r:?}");
  // Something new on the laptop: one session, that repository's only. A
  // repository the peer does not hold is said so, not tried.
  let note = laptop
    .root()
    .publish(
      laptop.name(),
      Draft::new("note", Author::user(laptop.name().clone())).unwrap(),
    )
    .unwrap();
  let r = phone
    .net
    .sync_peer(&laptop.addr(), &repos, how)
    .await
    .unwrap();
  assert_eq!(r.synced.len(), 1, "{r:?}");
  assert_eq!(r.not_held.len(), 1, "{r:?}");
  assert!(r.synced[0].1.as_ref().unwrap().received[0].contains(note.id.as_str()));
  // The laptop syncs back on the connection the phone dialed.
  let r = laptop
    .net
    .sync_peer(&phone.addr_unknown(), &repos[..1], how)
    .await
    .unwrap();
  assert_eq!((r.same.len(), r.synced.len()), (1, 0), "{r:?}");
}

async fn node(
  dir: &Path,
  name: &str,
  root_dir: &Path,
  policy: Policy,
  key: SecretKey,
  peers: &[(&str, &SecretKey)],
) -> TestNode {
  let root = RecordRoot::open(root_dir).unwrap();
  let mut allow = Peers::default();
  for (n, k) in peers {
    allow
      .add(Peer {
        name: n.to_string(),
        id: k.public().to_string(),
        addrs: vec![],
      })
      .unwrap();
  }
  let state = NodeState::new(
    name.parse().unwrap(),
    BTreeMap::from([(root.repo().clone(), root)]),
    Index::open_in_memory().unwrap(),
    BlobStore::open(&dir.join(format!("{name}-blobs"))).unwrap(),
    allow,
    None,
    policy,
  );
  let endpoint = bind(key.clone(), Discovery::Local, None).await.unwrap();
  TestNode {
    net: Net::spawn(endpoint, Arc::new(state)),
    key,
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn strangers_and_unknown_repositories_are_refused() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let a_key = SecretKey::generate();
  let b_key = SecretKey::generate();
  let stranger_key = SecretKey::generate();
  RecordRoot::init(&d.join("a"), "proj".parse().unwrap(), Role::Project).unwrap();
  RecordRoot::init(&d.join("b"), "other".parse().unwrap(), Role::Project).unwrap();
  RecordRoot::init(&d.join("s"), "proj".parse().unwrap(), Role::Project).unwrap();
  let a = node(d, "a", &d.join("a"), PHONE, a_key.clone(), &[("b", &b_key)]).await;
  let b = node(
    d,
    "b",
    &d.join("b"),
    LAPTOP,
    b_key.clone(),
    &[("a", &a_key)],
  )
  .await;
  let stranger = node(d, "s", &d.join("s"), PHONE, stranger_key, &[("a", &a_key)]).await;

  // b holds no `proj`.
  let err = a
    .net
    .sync_with(b.addr(), a.root().repo())
    .await
    .unwrap_err();
  assert!(err.to_string().contains("not held here"), "{err}");
  // a does not know the stranger: the connection is closed on it.
  let err = stranger
    .net
    .sync_with(a.addr(), stranger.root().repo())
    .await
    .unwrap_err();
  assert!(
    err.to_string().contains("not one of this user's nodes"),
    "{err}"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_of_version_2_is_synced_as_before() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  RecordRoot::init(&d.join("new"), "proj".parse().unwrap(), Role::Project).unwrap();
  RecordRoot::init(&d.join("old"), "proj".parse().unwrap(), Role::Project).unwrap();
  let (nk, ok) = (SecretKey::generate(), SecretKey::generate());
  let new = node(
    d,
    "new",
    &d.join("new"),
    LAPTOP,
    nk.clone(),
    &[("old", &ok)],
  )
  .await;
  let old = {
    let root = RecordRoot::open(&d.join("old")).unwrap();
    let mut allow = Peers::default();
    allow
      .add(Peer {
        name: "new".into(),
        id: nk.public().to_string(),
        addrs: vec![],
      })
      .unwrap();
    let mut state = NodeState::new(
      "old".parse().unwrap(),
      BTreeMap::from([(root.repo().clone(), root)]),
      Index::open_in_memory().unwrap(),
      BlobStore::open(&d.join("old-blobs")).unwrap(),
      allow,
      None,
      LAPTOP,
    );
    state.speaks = 2;
    let endpoint = bind(ok.clone(), Discovery::Local, None).await.unwrap();
    TestNode {
      net: Net::spawn(endpoint, Arc::new(state)),
      key: ok,
    }
  };
  let photo = vec![7u8; 100_000];
  let hash = old.state().blobs.put_bytes(&photo).unwrap();
  old
    .root()
    .publish(
      old.name(),
      Draft::new("task", Author::user(old.name().clone()))
        .unwrap()
        .attach(Attachment {
          hash,
          media_type: "image/jpeg".into(),
          name: None,
          bytes: photo.len() as u64,
        }),
    )
    .unwrap();
  let how = PeerSync {
    seal: true,
    connect: std::time::Duration::from_secs(5),
    parallel: 4,
  };
  // The new node's summary goes unanswered: every repository is synced in
  // version 2, blobs inside the session.
  let r = new
    .net
    .sync_peer(&old.addr(), &[new.root().repo().clone()], how)
    .await
    .unwrap();
  assert_eq!(r.version, 2);
  let s = r.synced[0].1.as_ref().unwrap();
  assert_eq!(
    (s.version, s.received.len(), s.blobs_received),
    (2, 1, 1),
    "{s:?}"
  );
  assert!(new.state().blobs.contains(&hash));
  // The old node syncs with the new one as it always did.
  new
    .root()
    .publish(
      new.name(),
      Draft::new("note", Author::user(new.name().clone())).unwrap(),
    )
    .unwrap();
  let s = old
    .net
    .sync_with(new.addr(), old.root().repo())
    .await
    .unwrap();
  assert_eq!((s.version, s.received.len()), (2, 1), "{s:?}");
  // And again from the new side, on what it learned.
  let r = new
    .net
    .sync_peer(&old.addr(), &[new.root().repo().clone()], how)
    .await
    .unwrap();
  assert_eq!((r.version, r.synced.len()), (2, 1));
}

/// A sync that worked reached the peer, over a connection kept since or
/// a new one: `peer ls` (and "not delivered yet") read when it last was.
#[test]
fn a_sync_that_worked_counts_as_reaching_the_peer() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let root_dir = d.join("a");
  RecordRoot::init(&root_dir, "proj".parse().unwrap(), Role::Project).unwrap();
  let root = RecordRoot::open(&root_dir).unwrap();
  let state = NodeState::new(
    "a".parse().unwrap(),
    BTreeMap::from([(root.repo().clone(), root)]),
    Index::open_in_memory().unwrap(),
    BlobStore::open(&d.join("blobs")).unwrap(),
    Peers::default(),
    None,
    PHONE,
  );
  state.note_failed("b", "not reached within 10s");
  state.note_synced("b", std::time::Duration::from_millis(90));
  let seen = state.seen().peer["b"].clone();
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap()
    .as_secs();
  assert!(seen.reached.is_some_and(|r| now - r < 5), "{seen:?}");
  assert_eq!(
    (seen.failed, seen.error, seen.sync_ms),
    (None, None, Some(90))
  );
}

/// A kept connection is re-sampled: the handshake's path (the relay, before
/// iroh punches through) must not stay in `peers-seen` forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_kept_connection_refreshes_the_seen_path() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  for r in ["phone-root", "laptop-root"] {
    RecordRoot::init(&d.join(r), "proj".parse().unwrap(), Role::Project).unwrap();
  }
  let (pk, lk) = (SecretKey::generate(), SecretKey::generate());
  let phone = node(
    d,
    "phone",
    &d.join("phone-root"),
    PHONE,
    pk.clone(),
    &[("mbp", &lk)],
  )
  .await;
  let laptop = node(
    d,
    "mbp",
    &d.join("laptop-root"),
    LAPTOP,
    lk,
    &[("phone", &pk)],
  )
  .await;

  phone
    .net
    .sync_with(laptop.addr(), phone.root().repo())
    .await
    .unwrap();
  assert_eq!(
    phone.state().seen().peer["mbp"].path,
    Some(stitch_net::PathKind::Direct)
  );

  // The stale handshake sample.
  phone
    .state()
    .set_seen_path("mbp", Some(stitch_net::PathKind::Relay));
  // Second exchange, on the kept connection.
  phone
    .net
    .sync_with(laptop.addr_unknown(), phone.root().repo())
    .await
    .unwrap();
  assert_eq!(
    phone.state().seen().peer["mbp"].path,
    Some(stitch_net::PathKind::Direct)
  );
}
