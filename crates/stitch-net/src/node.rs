//! A node on the network: an iroh endpoint that serves the protocol to the
//! user's other nodes and syncs with them.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointId, RelayMode, SecretKey};
use stitch_records::{NodeName, RepoId, kinds};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

use crate::pair::{Invitation, PAIR_ALPN};
use crate::proto::ALPN;
use crate::seen::SeenPeers;
use crate::{Error, Peer, Peers, Result};

/// How a node finds others and is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery {
  /// iroh's defaults: number 0's relays and DNS address lookup, which work
  /// across NATs and publish the node's ID and addresses to their servers.
  N0,
  /// Direct addresses only: no relays, nothing published.
  Local,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobPolicy {
  /// Pull every referenced blob this node lacks (always-on nodes).
  All,
  /// Fetch blobs on demand only (phones).
  Lazy,
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {
  pub blobs: BlobPolicy,
}

/// How long the trusted peers are kept before they are read again.
const TRUSTED_FOR: Duration = Duration::from_secs(5);

/// The trusted peers kept: when read, `peers.toml`'s time then, the peers.
type Trusted = (Instant, Option<std::time::SystemTime>, Vec<Peer>);

/// What happened on the node, for whoever runs it.
pub enum Event {
  /// A session this node served, with the peer's name.
  Session {
    peer: String,
    result: Result<SessionReport>,
  },
  /// A node joined through an invitation.
  Paired(Peer),
}

type Observer = Box<dyn Fn(&Event) + Send + Sync>;

/// Why `Net::ask_within` has no answer, or `Net::sync_within` no report.
#[derive(Debug)]
pub enum AskError {
  /// The peer could not be reached: nothing was asked or exchanged.
  Unreachable(String),
  /// It was asked (the exchange began), and failed: it may have been
  /// answered, or records taken in part.
  Failed(Error),
}

/// Answers a peer's question (the peer's name, the question), both in the
/// host's own encoding.
pub type Answerer =
  Box<dyn Fn(String, Vec<u8>) -> Pin<Box<dyn Future<Output = Vec<u8>> + Send>> + Send + Sync>;

/// What the protocol works on. The index is behind a mutex and only touched
/// from blocking tasks.
pub struct NodeState {
  pub node: NodeName,
  /// The repositories held here; one can be added while the node runs.
  roots: RwLock<BTreeMap<RepoId, RecordRoot>>,
  /// Where the record stores of projects live (`hold_projects`).
  projects_dir: Option<PathBuf>,
  pub index: Mutex<Index>,
  pub blobs: BlobStore,
  /// The local allowlist, and where it is saved.
  pub peers: Mutex<Peers>,
  pub peers_path: Option<PathBuf>,
  pub policy: Policy,
  pub observer: Option<Observer>,
  /// Set once the host can answer; until then questions are refused.
  pub(crate) answerer: OnceLock<Answerer>,
  pub(crate) invitation: Mutex<Option<Invitation>>,
  /// When each peer was last reached, and how (`peers-seen.toml`).
  seen: Mutex<SeenPeers>,
  seen_saved: Mutex<Option<Instant>>,
  /// `known_peers`, kept a few seconds: every incoming connection asks.
  trusted: Mutex<Option<Trusted>>,
  /// The connections kept with peers (link.rs).
  pub(crate) pool: Mutex<crate::link::Pool>,
  /// The newest protocol version this node speaks: below the current one
  /// only to test how a node of an older version is met.
  #[doc(hidden)]
  pub speaks: u32,
}

impl NodeState {
  pub fn new(
    node: NodeName,
    roots: BTreeMap<RepoId, RecordRoot>,
    index: Index,
    blobs: BlobStore,
    peers: Peers,
    peers_path: Option<PathBuf>,
    policy: Policy,
  ) -> Self {
    NodeState {
      node,
      roots: RwLock::new(roots),
      projects_dir: None,
      index: Mutex::new(index),
      blobs,
      peers: Mutex::new(peers),
      peers_path,
      policy,
      observer: None,
      answerer: OnceLock::new(),
      invitation: Mutex::new(None),
      seen: Mutex::new(SeenPeers::default()),
      seen_saved: Mutex::new(None),
      trusted: Mutex::new(None),
      pool: Mutex::new(crate::link::Pool::default()),
      speaks: crate::proto::VERSION,
    }
  }

  /// Where `peers-seen.toml` is: beside `peers.toml`.
  pub fn seen_path(&self) -> Option<PathBuf> {
    self
      .peers_path
      .as_ref()
      .map(|p| p.with_file_name("peers-seen.toml"))
  }

  /// What this node knows of each peer's reachability.
  pub fn seen(&self) -> SeenPeers {
    self.seen.lock().expect("seen lock").clone()
  }

  /// A connection with `id` works: when, on which path, and its direct
  /// addresses, kept in the local list for the next dial.
  ///
  /// Called from async code: the peer's name comes from the trusted peers
  /// kept (`known_peers`), never read afresh here.
  pub fn note_reached(&self, id: &EndpointId, conn: &Connection) {
    let Some(name) = self.cached_name(id) else {
      return;
    };
    let direct = self.sample_path(&name, conn, true);
    if !direct.is_empty() {
      self.refresh_addrs(id, direct);
    }
  }

  /// A kept connection with `id` is used again: its path is sampled afresh
  /// (the handshake's sample is usually the relay, before iroh punches a
  /// direct path). Only the path, round trip and address change; a held
  /// connection is not a reach.
  pub fn note_path(&self, id: &EndpointId, conn: &Connection) {
    let Some(name) = self.cached_name(id) else {
      return;
    };
    self.sample_path(&name, conn, false);
  }

  /// Test only: overwrites what is seen of `name`'s path.
  #[doc(hidden)]
  pub fn set_seen_path(&self, name: &str, kind: Option<crate::seen::PathKind>) {
    let mut seen = self.seen.lock().expect("seen lock");
    seen.peer.entry(name.to_owned()).or_default().path = kind;
  }

  /// Records `conn`'s path for `name`; with `reached`, also the reach.
  /// Returns its direct addresses.
  fn sample_path(&self, name: &str, conn: &Connection, reached: bool) -> Vec<std::net::SocketAddr> {
    let (kind, rtt, direct) = crate::seen::paths(conn);
    let changed = {
      let mut seen = self.seen.lock().expect("seen lock");
      let entry = seen.peer.entry(name.to_owned()).or_default();
      let before = (entry.path, entry.failed.is_some());
      if reached {
        entry.reached = Some(crate::seen::now_secs());
        entry.failed = None;
        entry.error = None;
      }
      if kind.is_some() {
        entry.path = kind;
      }
      entry.rtt_ms = rtt.map(|r| r.as_millis() as u64).or(entry.rtt_ms);
      if let Some(a) = direct.first() {
        entry.addr = Some(a.to_string());
      }
      before != (entry.path, !reached && before.1)
    };
    self.save_seen(changed);
    direct
  }

  /// A peer could not be reached (or its session failed).
  pub fn note_failed(&self, name: &str, why: &str) {
    {
      let mut seen = self.seen.lock().expect("seen lock");
      let entry = seen.peer.entry(name.to_owned()).or_default();
      entry.failed = Some(crate::seen::now_secs());
      entry.error = Some(why.to_owned());
    }
    self.save_seen(true);
  }

  /// A sync with a peer took `took`: it was reached, whether on a new
  /// connection (`note_reached` said so then) or on one kept since.
  pub fn note_synced(&self, name: &str, took: Duration) {
    let changed = {
      let mut seen = self.seen.lock().expect("seen lock");
      let entry = seen.peer.entry(name.to_owned()).or_default();
      entry.sync_ms = Some(took.as_millis() as u64);
      entry.reached = Some(crate::seen::now_secs());
      entry.error = None;
      entry.failed.take().is_some()
    };
    self.save_seen(changed);
  }

  /// Writes `peers-seen.toml`: at once when something changed that a
  /// reader would act on, else at most every 30 seconds.
  fn save_seen(&self, now: bool) {
    let Some(path) = self.seen_path() else {
      return;
    };
    {
      let mut saved = self.seen_saved.lock().expect("seen lock");
      if !now && saved.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
        return;
      }
      *saved = Some(Instant::now());
    }
    let seen = self.seen();
    if let Err(e) = seen.save(&path) {
      eprintln!("{}: {e}", path.display());
    }
  }

  /// A peer of the local list reached on other addresses than those it
  /// lists: the list takes the ones that work (addresses from an older
  /// network only slow the next dial).
  fn refresh_addrs(&self, id: &EndpointId, mut direct: Vec<std::net::SocketAddr>) {
    let Some(path) = &self.peers_path else {
      return;
    };
    let id = id.to_string();
    let mut peers = self.peers.lock().expect("peers lock");
    if let Ok(p) = Peers::load(path) {
      *peers = p;
    }
    let Some(peer) = peers.peers.iter_mut().find(|p| p.id == id) else {
      return;
    };
    direct.sort();
    direct.dedup();
    let mut had = peer.addrs.clone();
    had.sort();
    if had == direct {
      return;
    }
    peer.addrs = direct;
    if let Err(e) = peers.save(path) {
      eprintln!("{}: {e}", path.display());
    }
    *self.trusted.lock().expect("trusted lock") = None;
  }

  /// The home workspace's root, if this node holds it.
  pub fn home(&self) -> Option<RecordRoot> {
    self
      .roots
      .read()
      .expect("roots lock")
      .values()
      .find(|r| r.config().role == Role::Home)
      .cloned()
  }

  /// A repository held here.
  pub fn root(&self, repo: &RepoId) -> Option<RecordRoot> {
    self.roots.read().expect("roots lock").get(repo).cloned()
  }

  /// Every repository held here.
  pub fn roots(&self) -> Vec<RecordRoot> {
    self
      .roots
      .read()
      .expect("roots lock")
      .values()
      .cloned()
      .collect()
  }

  /// Keeps the record stores of projects in `dir`.
  pub fn with_projects(mut self, dir: PathBuf) -> Self {
    self.projects_dir = Some(dir);
    self
  }

  /// Holds a record store for every project the home workspace lists
  /// (`project` records), creating the missing ones empty; how many are new.
  pub fn hold_projects(&self) -> Result<usize> {
    let Some(dir) = &self.projects_dir else {
      return Ok(0);
    };
    let mut index = self.index.lock().expect("index lock");
    if let Some(home) = self.home() {
      index.refresh(&home)?;
    }
    let repos: Vec<RepoId> = index
      .records(Some(kinds::PROJECT))?
      .into_iter()
      .filter_map(|r| r.fields.get("repo")?.as_str()?.parse().ok())
      .collect();
    let mut new = 0;
    for repo in repos {
      if self.root(&repo).is_some() {
        continue;
      }
      let d = dir.join(repo.as_str());
      let root = match RecordRoot::open(&d) {
        Ok(r) => r,
        Err(_) => RecordRoot::init(&d, repo, Role::Project)?,
      };
      index.refresh(&root)?;
      if self.add_root(root) {
        new += 1;
      }
    }
    Ok(new)
  }

  /// Holds one more repository from now on; false if it already was.
  pub fn add_root(&self, root: RecordRoot) -> bool {
    let mut roots = self.roots.write().expect("roots lock");
    if roots.contains_key(root.repo()) {
      return false;
    }
    roots.insert(root.repo().clone(), root);
    true
  }

  /// Every other node this one trusts: the local list, then `peer` records
  /// in the home workspace that are not revoked.
  ///
  /// Both are read afresh at most `TRUSTED_FOR` apart (every incoming
  /// connection asks), so a change another process made (a peer added to
  /// the file, a `revoke` written into the home workspace) counts within
  /// seconds.
  pub fn known_peers(&self) -> Result<Vec<Peer>> {
    let stamp = self.peers_stamp();
    if let Some((at, kept_stamp, peers)) = &*self.trusted.lock().expect("trusted lock")
      && at.elapsed() < TRUSTED_FOR
      && *kept_stamp == stamp
    {
      return Ok(peers.clone());
    }
    let mut out = {
      let mut peers = self.peers.lock().expect("peers lock");
      if let Some(path) = &self.peers_path {
        *peers = Peers::load(path)?;
      }
      peers.peers.clone()
    };
    let records = {
      let mut index = self.index.lock().expect("index lock");
      if let Some(home) = self.home() {
        index.refresh(&home)?;
      }
      index.peer_records()?
    };
    for r in records {
      let field = |k: &str| r.fields.get(k).and_then(|v| v.as_str()).map(str::to_owned);
      if let (Some(name), Some(id)) = (field("name"), field("endpoint"))
        && name != self.node.as_str()
        && !out.iter().any(|p| p.id == id)
      {
        out.push(Peer {
          name,
          id,
          addrs: Vec::new(),
        });
      }
    }
    *self.trusted.lock().expect("trusted lock") = Some((Instant::now(), stamp, out.clone()));
    Ok(out)
  }

  /// When `peers.toml` last changed: a peer added by a command counts at
  /// once.
  fn peers_stamp(&self) -> Option<std::time::SystemTime> {
    let path = self.peers_path.as_ref()?;
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
  }

  /// The trusted peers are read afresh at the next connection.
  pub fn forget_trusted(&self) {
    *self.trusted.lock().expect("trusted lock") = None;
  }

  /// The trusted peers read afresh (home records came: a `revoke` among
  /// them), and the kept connections of a node no longer trusted closed.
  /// Blocking: it reads the index.
  pub fn recheck_trust(&self) {
    self.forget_trusted();
    let Ok(peers) = self.known_peers() else {
      return;
    };
    let ids: std::collections::HashSet<String> = peers.into_iter().map(|p| p.id).collect();
    for conn in self.pool.lock().expect("pool lock").untrusted(&ids) {
      conn.close(1u32.into(), b"not one of this user's nodes");
    }
  }

  /// The name of a trusted node, from what is kept only.
  pub(crate) fn cached_name(&self, id: &EndpointId) -> Option<String> {
    let id = id.to_string();
    if let Some((_, _, peers)) = &*self.trusted.lock().expect("trusted lock")
      && let Some(p) = peers.iter().find(|p| p.id == id)
    {
      return Some(p.name.clone());
    }
    let peers = self.peers.lock().expect("peers lock");
    peers
      .peers
      .iter()
      .find(|p| p.id == id)
      .map(|p| p.name.clone())
  }

  /// The name of a trusted node, if `id` is one.
  ///
  /// One not among the peers kept is looked for afresh: a node admitted a
  /// moment ago (its `peer` record just synced) is let in at once.
  pub fn allows(&self, id: &EndpointId) -> Option<String> {
    let id = id.to_string();
    let find = |peers: Vec<Peer>| peers.into_iter().find(|p| p.id == id).map(|p| p.name);
    if let Some(name) = find(self.known_peers().ok()?) {
      return Some(name);
    }
    *self.trusted.lock().expect("trusted lock") = None;
    find(self.known_peers().ok()?)
  }

  /// A trusted node by name or endpoint ID.
  pub fn find_peer(&self, name_or_id: &str) -> Result<Option<Peer>> {
    Ok(
      self
        .known_peers()?
        .into_iter()
        .find(|p| p.name == name_or_id || p.id == name_or_id),
    )
  }

  /// Adds peers to the local list and saves it.
  pub fn remember(&self, new: impl IntoIterator<Item = Peer>) -> Result<()> {
    let mut peers = self.peers.lock().expect("peers lock");
    for p in new {
      peers.add(p)?;
    }
    *self.trusted.lock().expect("trusted lock") = None;
    if let Some(path) = &self.peers_path {
      peers.save(path)?;
    }
    Ok(())
  }

  /// Writes a `peer` record for a node into the home workspace, unless one
  /// for that endpoint exists; returns false without a home workspace.
  pub(crate) fn admit(&self, name: &str, id: &EndpointId) -> Result<bool> {
    let Some(home) = self.home() else {
      return Ok(false);
    };
    let mut index = self.index.lock().expect("index lock");
    index.refresh(&home)?;
    let id = id.to_string();
    let exists = index
      .peer_records()?
      .iter()
      .any(|r| r.fields.get("endpoint").and_then(|v| v.as_str()) == Some(id.as_str()));
    if !exists {
      let draft =
        stitch_records::Draft::new(kinds::PEER, stitch_records::Author::node(self.node.clone()))?
          .field("name", name)?
          .field("endpoint", id.as_str())?;
      home.publish(&self.node, draft)?;
      index.refresh(&home)?;
      *self.trusted.lock().expect("trusted lock") = None;
    }
    Ok(true)
  }

  /// Who answers peers' questions; only the first one set counts.
  pub fn set_answerer(&self, answerer: Answerer) {
    let _ = self.answerer.set(answerer);
  }

  pub(crate) fn emit(&self, event: Event) {
    if let Some(observe) = &self.observer {
      observe(&event);
    }
  }
}

#[derive(Clone, Debug, Default)]
pub struct SessionReport {
  pub repo: String,
  /// The protocol version the session spoke.
  pub version: u32,
  pub rounds: usize,
  /// Paths written here.
  pub received: Vec<String>,
  /// Files sent to the peer.
  pub sent: usize,
  /// Paths held on both sides with different content.
  pub conflicts: Vec<String>,
  /// Files from the peer that failed verification, with why.
  pub rejected: Vec<(String, String)>,
  pub blobs_received: usize,
  pub blobs_sent: usize,
  /// Blobs this node wanted that the peer did not hold either.
  pub blobs_unavailable: usize,
  /// From the connection (or the first frame, served) to the end.
  pub elapsed: Duration,
}

pub async fn bind(key: SecretKey, discovery: Discovery, port: Option<u16>) -> Result<Endpoint> {
  let mut builder = match discovery {
    Discovery::N0 => Endpoint::builder(presets::N0),
    Discovery::Local => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
  };
  builder = builder
    .secret_key(key)
    .alpns(vec![ALPN.to_vec(), PAIR_ALPN.to_vec()]);
  if let Some(port) = port {
    builder = builder
      .bind_addr(std::net::SocketAddr::from(([0, 0, 0, 0], port)))
      .map_err(Error::net)?;
  }
  builder.bind().await.map_err(Error::net)
}
