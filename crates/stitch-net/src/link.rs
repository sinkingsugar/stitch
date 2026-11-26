//! The protocol on the wire: connections kept per peer, summaries, sync
//! sessions, blob transfers and questions (design §2.6, proto.rs).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{FuturesUnordered, StreamExt};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use stitch_records::{BlobHash, RepoId};
use stitch_store::sync::{Fingerprint, ItemSet, Outcome, Reconciler};
use stitch_store::{BlobStore, Error as StoreError, Inserted, ItemPath, RecordRoot, Role};
use tokio::sync::oneshot;

use crate::node::{AskError, BlobPolicy, Event, NodeState, SessionReport};
use crate::pair::{PAIR_ALPN, PairHandler};
use crate::proto::{
  ALPN, Frame, LEGACY, VERSION, read_frame, read_frame_idle, unexpected, write_frame,
};
use crate::{Error, Result};

/// How long a kept connection has to bring the first answer before it is
/// taken for dead (a peer gone to sleep leaves its connection open until
/// QUIC's idle timeout): it is dropped and the peer dialed afresh, once.
const FIRST_REPLY: Duration = Duration::from_secs(15);
/// Blobs fetched at once from one peer after a session.
const BLOB_FETCHES: usize = 8;
/// A blob a peer said it does not hold is not asked of it again for this
/// long.
const UNAVAILABLE_FOR: Duration = Duration::from_secs(600);

/// The connections kept with each peer: the one this node dialed and the
/// one it accepted (both may exist when both sides dialed at once).
#[derive(Default)]
pub(crate) struct Pool {
  peers: HashMap<EndpointId, Held>,
  /// Blobs being fetched in the background, so two sessions do not fetch
  /// the same one.
  fetching: HashSet<[u8; 32]>,
  /// Blobs a peer did not hold, and when it said so.
  unavailable: HashMap<(EndpointId, [u8; 32]), Instant>,
}

impl Pool {
  /// Takes out the connections of nodes not in `trusted` (endpoint IDs).
  pub(crate) fn untrusted(&mut self, trusted: &HashSet<String>) -> Vec<Connection> {
    let gone: Vec<EndpointId> = self
      .peers
      .keys()
      .filter(|id| !trusted.contains(&id.to_string()))
      .copied()
      .collect();
    let mut out = Vec::new();
    for id in gone {
      if let Some(h) = self.peers.remove(&id) {
        out.extend(h.dialed.map(|(c, _)| c));
        out.extend(h.accepted.map(|(c, _)| c));
      }
    }
    out
  }
}

#[derive(Default)]
struct Held {
  /// Dialed here, with the version the peer was found to speak on it.
  dialed: Option<(Connection, Option<u32>)>,
  /// Accepted here; usable for streams of this node's only once the peer
  /// showed it speaks version 3 on it (a node of version 2 serves only
  /// connections it accepted).
  accepted: Option<(Connection, bool)>,
}

fn alive(c: &Connection) -> bool {
  c.close_reason().is_none()
}

/// A connection to use, and whether it was just dialed.
struct Link {
  conn: Connection,
  fresh: bool,
  /// What the peer speaks on it, when known.
  version: Option<u32>,
}

/// What the peer holds of each repository a summary named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoState {
  /// The same files on both sides.
  Same,
  Differs,
  /// The peer does not hold it (a folder it has not joined).
  NotHeld,
}

/// How `Net::sync_peer` goes about it.
#[derive(Clone, Copy, Debug)]
pub struct PeerSync {
  /// Seal the turns open here (and ask the peer to) first: a sync someone
  /// asked for, or the interval's; a push of what changed does not.
  pub seal: bool,
  /// How long the peer has to be reached.
  pub connect: Duration,
  /// Repositories synced at once.
  pub parallel: usize,
}

/// What `Net::sync_peer` did.
#[derive(Debug, Default)]
pub struct PeerReport {
  /// The protocol version spoken.
  pub version: u32,
  /// Repositories found the same by the summary (nothing exchanged).
  pub same: Vec<RepoId>,
  pub not_held: Vec<RepoId>,
  /// Each repository synced, with its session.
  pub synced: Vec<(RepoId, Result<SessionReport>)>,
  pub elapsed: Duration,
}

pub struct Net {
  router: Router,
  state: Arc<NodeState>,
}

impl Net {
  /// Serves the protocol on `endpoint` to the peers in `state`.
  pub fn spawn(endpoint: Endpoint, state: Arc<NodeState>) -> Net {
    let id = endpoint.id();
    let router = Router::builder(endpoint)
      .accept(
        ALPN,
        Handler {
          state: state.clone(),
        },
      )
      .accept(
        PAIR_ALPN,
        PairHandler {
          state: state.clone(),
          id,
        },
      )
      .spawn();
    Net { router, state }
  }

  pub fn endpoint(&self) -> &Endpoint {
    self.router.endpoint()
  }

  pub fn state(&self) -> &Arc<NodeState> {
    &self.state
  }

  /// Drops every kept connection (the network changed: a phone back from
  /// suspension); the next operation dials afresh.
  pub fn forget_connections(&self) {
    let held: Vec<Held> = {
      let mut pool = self.state.pool.lock().expect("pool lock");
      pool.peers.drain().map(|(_, h)| h).collect()
    };
    for h in held {
      if let Some((c, _)) = h.dialed {
        c.close(0u32.into(), b"network changed");
      }
    }
  }

  /// The connection kept with `peer`, else one dialed within `connect`.
  async fn link(
    &self,
    peer: &EndpointAddr,
    connect: Option<Duration>,
  ) -> std::result::Result<Link, AskError> {
    if let Some(link) = self.held(&peer.id) {
      // The pool lock is released: sample the path afresh.
      self.state.note_path(&peer.id, &link.conn);
      return Ok(link);
    }
    let dial = self.endpoint().connect(peer.clone(), ALPN);
    let conn = match connect {
      None => dial
        .await
        .map_err(|e| AskError::Unreachable(Error::net(e).to_string()))?,
      Some(limit) => match tokio::time::timeout(limit, dial).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => return Err(AskError::Unreachable(Error::net(e).to_string())),
        Err(_) => {
          return Err(AskError::Unreachable(format!(
            "not reached within {}s",
            limit.as_secs()
          )));
        }
      },
    };
    self.state.note_reached(&peer.id, &conn);
    {
      let mut pool = self.state.pool.lock().expect("pool lock");
      let held = pool.peers.entry(peer.id).or_default();
      held.dialed = Some((conn.clone(), None));
    }
    // The peer may open streams on it too (not to a node of version 2).
    let state = self.state.clone();
    let serve = state.speaks > LEGACY;
    let served = conn.clone();
    tokio::spawn(async move {
      let remote = served.remote_id();
      let st = state.clone();
      let name = tokio::task::spawn_blocking(move || st.allows(&remote))
        .await
        .ok()
        .flatten();
      if let Some(name) = name
        && serve
      {
        serve_conn(state, served, name).await;
      }
    });
    Ok(Link {
      conn,
      fresh: true,
      version: None,
    })
  }

  fn held(&self, id: &EndpointId) -> Option<Link> {
    let mut pool = self.state.pool.lock().expect("pool lock");
    let held = pool.peers.get_mut(id)?;
    if let Some((c, v)) = &held.dialed {
      if alive(c) {
        return Some(Link {
          conn: c.clone(),
          fresh: false,
          version: *v,
        });
      }
      held.dialed = None;
    }
    if let Some((c, v3)) = &held.accepted {
      if alive(c) && *v3 {
        return Some(Link {
          conn: c.clone(),
          fresh: false,
          version: Some(VERSION),
        });
      }
      if !alive(c) {
        held.accepted = None;
      }
    }
    None
  }

  /// Forgets a kept connection that did not answer.
  fn drop_link(&self, id: &EndpointId, conn: &Connection) {
    let mut pool = self.state.pool.lock().expect("pool lock");
    if let Some(held) = pool.peers.get_mut(id) {
      if held
        .dialed
        .as_ref()
        .is_some_and(|(c, _)| c.stable_id() == conn.stable_id())
      {
        held.dialed = None;
      }
      if held
        .accepted
        .as_ref()
        .is_some_and(|(c, _)| c.stable_id() == conn.stable_id())
      {
        held.accepted = None;
      }
    }
    conn.close(0u32.into(), b"no answer");
  }

  /// Notes the version a peer speaks on a connection this node dialed.
  fn learned(&self, id: &EndpointId, conn: &Connection, version: u32) {
    let mut pool = self.state.pool.lock().expect("pool lock");
    if let Some(Held {
      dialed: Some((c, v)),
      ..
    }) = pool.peers.get_mut(id)
      && c.stable_id() == conn.stable_id()
    {
      *v = Some(version);
    }
  }

  /// Runs `op` on the kept connection, and when that one turns out dead
  /// (no first answer in time, or lost), once more on a fresh one. Only
  /// for operations that can be run twice (a sync, a summary, a blob).
  async fn retrying<T, F, Fut>(
    &self,
    peer: &EndpointAddr,
    connect: Option<Duration>,
    op: F,
  ) -> std::result::Result<T, AskError>
  where
    F: Fn(Link) -> Fut,
    Fut: Future<Output = Result<T>>,
  {
    let link = self.link(peer, connect).await?;
    let conn = link.conn.clone();
    if link.fresh {
      return op(link)
        .await
        .map_err(|e| AskError::Failed(refusal(&conn, e)));
    }
    match op(link).await {
      Err(Error::Net(_) | Error::Stale) => {
        self.drop_link(&peer.id, &conn);
        let link = self.link(peer, connect).await?;
        let conn = link.conn.clone();
        op(link)
          .await
          .map_err(|e| AskError::Failed(refusal(&conn, e)))
      }
      other => other.map_err(|e| AskError::Failed(refusal(&conn, e))),
    }
  }

  /// Syncs one repository with a peer.
  pub async fn sync_with(&self, peer: EndpointAddr, repo: &RepoId) -> Result<SessionReport> {
    self
      .sync_repo(&peer, repo, None, true)
      .await
      .map_err(|e| match e {
        AskError::Unreachable(why) => Error::Net(why),
        AskError::Failed(e) => e,
      })
  }

  /// Syncs one repository with a peer, reaching it within `connect` (else
  /// `Unreachable`: an offline peer costs no more than that).
  pub async fn sync_within(
    &self,
    peer: EndpointAddr,
    repo: &RepoId,
    connect: Duration,
  ) -> std::result::Result<SessionReport, AskError> {
    self.sync_repo(&peer, repo, Some(connect), true).await
  }

  /// One repository's session, in the version the peer speaks (version 3
  /// first; one that refuses it gets version 2 from then on).
  async fn sync_repo(
    &self,
    peer: &EndpointAddr,
    repo: &RepoId,
    connect: Option<Duration>,
    seal: bool,
  ) -> std::result::Result<SessionReport, AskError> {
    let root = self.held_root(repo).map_err(AskError::Failed)?;
    let started = Instant::now();
    let mut report = self
      .retrying(peer, connect, |link| {
        let root = root.clone();
        async move {
          let version = link.version.unwrap_or(VERSION).min(self.state.speaks);
          match initiate(&self.state, &link.conn, root.clone(), version, seal).await {
            Err(Error::Refused(why)) if version > LEGACY && why.starts_with("protocol version") => {
              self.learned(&peer.id, &link.conn, LEGACY);
              initiate(&self.state, &link.conn, root, LEGACY, seal).await
            }
            other => {
              if other.is_ok() {
                self.learned(&peer.id, &link.conn, version);
              }
              other
            }
          }
          .map(|r| (r, link))
        }
      })
      .await?;
    // Version 3: the blobs this node wants, from the same connection.
    let (r, link) = &mut report;
    if r.version > LEGACY {
      let (got, unavailable) = fetch_wanted(&self.state, &link.conn, repo).await;
      r.blobs_received += got;
      r.blobs_unavailable += unavailable;
    }
    let (mut r, _) = report;
    r.elapsed = started.elapsed();
    Ok(r)
  }

  fn held_root(&self, repo: &RepoId) -> Result<RecordRoot> {
    self
      .state
      .root(repo)
      .ok_or_else(|| Error::Config(format!("repository {repo} is not held here")))
  }

  /// Compares repositories with a peer in one round trip: what it holds
  /// of each. `None` when the peer speaks version 2 (it knows no
  /// summaries): everything is to be synced then.
  pub async fn summary(
    &self,
    peer: &EndpointAddr,
    repos: &[RepoId],
    seal: bool,
    connect: Duration,
  ) -> std::result::Result<Option<Vec<RepoState>>, AskError> {
    let st = self.state.clone();
    let list = repos.to_vec();
    let mine = tokio::task::spawn_blocking(move || fingerprints(&st, &list, seal))
      .await
      .map_err(|e| AskError::Failed(Error::net(e)))?
      .map_err(AskError::Failed)?;
    let frame = Frame::Summary {
      version: VERSION,
      seal,
      repos: repos
        .iter()
        .zip(&mine)
        .map(|(r, f)| (r.to_string(), *f))
        .collect(),
    };
    let theirs = self
      .retrying(peer, Some(connect), |link| {
        let frame = &frame;
        async move {
          if link.version == Some(LEGACY) || self.state.speaks == LEGACY {
            return Ok(None);
          }
          let (mut send, mut recv) = link.conn.open_bi().await.map_err(Error::net)?;
          write_frame(&mut send, frame).await?;
          send.finish().map_err(Error::net)?;
          match tokio::time::timeout(FIRST_REPLY, read_frame(&mut recv)).await {
            Err(_) => Err(Error::Stale),
            Ok(Ok(Frame::Summaries { version, repos })) => {
              self.learned(&peer.id, &link.conn, version.min(VERSION));
              Ok(Some(repos))
            }
            Ok(Ok(f)) => Err(unexpected(&f, "Summaries")),
            // The stream ended unanswered on a connection that lives: a
            // node of version 2, which does not know the frame.
            Ok(Err(e)) if alive(&link.conn) => {
              let _ = e;
              self.learned(&peer.id, &link.conn, LEGACY);
              Ok(None)
            }
            Ok(Err(e)) => Err(e),
          }
        }
      })
      .await?;
    let Some(theirs) = theirs else {
      return Ok(None);
    };
    if theirs.len() != repos.len() {
      return Err(AskError::Failed(Error::Protocol(format!(
        "{} summaries for {} repositories",
        theirs.len(),
        repos.len()
      ))));
    }
    Ok(Some(
      mine
        .iter()
        .zip(theirs)
        .map(|(m, t)| match t {
          None => RepoState::NotHeld,
          Some(t) if t == *m => RepoState::Same,
          Some(_) => RepoState::Differs,
        })
        .collect(),
    ))
  }

  /// Syncs repositories with one peer: a summary, then a session for each
  /// that differs, `parallel` at a time (the home workspace first, alone:
  /// it lists the projects). A peer of version 2 gets a session for each.
  pub async fn sync_peer(
    &self,
    peer: &EndpointAddr,
    repos: &[RepoId],
    how: PeerSync,
  ) -> std::result::Result<PeerReport, AskError> {
    let started = Instant::now();
    let mut report = PeerReport {
      version: VERSION,
      ..PeerReport::default()
    };
    let states = self.summary(peer, repos, how.seal, how.connect).await?;
    let mut todo = Vec::new();
    match states {
      None => {
        report.version = LEGACY;
        todo = repos.to_vec();
      }
      Some(states) => {
        for (repo, state) in repos.iter().zip(states) {
          match state {
            RepoState::Same => report.same.push(repo.clone()),
            RepoState::NotHeld => report.not_held.push(repo.clone()),
            RepoState::Differs => todo.push(repo.clone()),
          }
        }
      }
    }
    let home = self.state.home().map(|h| h.repo().clone());
    if let Some(h) = home
      && let Some(i) = todo.iter().position(|r| *r == h)
    {
      let repo = todo.remove(i);
      let r = self
        .sync_repo(peer, &repo, Some(how.connect), how.seal)
        .await;
      let moved = matches!(&r, Ok(r) if r.sent + r.received.len() > 0);
      match r {
        Err(AskError::Unreachable(why)) => return Err(AskError::Unreachable(why)),
        Err(AskError::Failed(e)) => report.synced.push((repo, Err(e))),
        Ok(r) => report.synced.push((repo, Ok(r))),
      }
      // The home workspace it now has may list projects it did not hold:
      // asked again (it holds them once listed).
      if moved && !report.not_held.is_empty() {
        let again = std::mem::take(&mut report.not_held);
        match self.summary(peer, &again, how.seal, how.connect).await? {
          None => todo.extend(again),
          Some(states) => {
            for (repo, state) in again.into_iter().zip(states) {
              match state {
                RepoState::Same => report.same.push(repo),
                RepoState::NotHeld => report.not_held.push(repo),
                RepoState::Differs => todo.push(repo),
              }
            }
          }
        }
      }
    }
    let mut running = FuturesUnordered::new();
    let mut next = todo.into_iter();
    loop {
      while running.len() < how.parallel.max(1)
        && let Some(repo) = next.next()
      {
        running.push(async move {
          let r = self
            .sync_repo(peer, &repo, Some(how.connect), how.seal)
            .await;
          (repo, r)
        });
      }
      match running.next().await {
        Some((repo, Ok(r))) => report.synced.push((repo, Ok(r))),
        Some((repo, Err(AskError::Failed(e)))) => report.synced.push((repo, Err(e))),
        Some((repo, Err(AskError::Unreachable(why)))) => {
          report.synced.push((repo, Err(Error::Net(why))))
        }
        None => break,
      }
    }
    report.elapsed = started.elapsed();
    Ok(report)
  }

  /// Fetches one blob from a peer; false if the peer does not hold it.
  pub async fn fetch_blob(&self, peer: EndpointAddr, hash: &BlobHash) -> Result<bool> {
    match self.offer(&peer, hash).await? {
      Some(offer) => self.take(offer, hash).await.map(|()| true),
      None => Ok(false),
    }
  }

  /// Fetches one blob from whichever of `peers` holds it: all are asked
  /// at once (`find` bounds each answer, so a peer that is offline costs
  /// no more), and it comes from the first that has it, the others let
  /// go before they send it. False if none holds it.
  pub async fn fetch_blob_any(
    &self,
    peers: &[EndpointAddr],
    hash: &BlobHash,
    find: Duration,
  ) -> Result<bool> {
    let mut asked: FuturesUnordered<_> = peers
      .iter()
      .map(|p| tokio::time::timeout(find, self.offer(p, hash)))
      .collect();
    while let Some(answer) = asked.next().await {
      if let Ok(Ok(Some(offer))) = answer {
        // The others' streams are stopped as they are dropped.
        drop(asked);
        return self.take(offer, hash).await.map(|()| true);
      }
    }
    Ok(false)
  }

  /// Asks a peer for a blob: its size and the stream it follows on when
  /// the peer holds it.
  async fn offer(&self, peer: &EndpointAddr, hash: &BlobHash) -> Result<Option<Offer>> {
    let hash_bytes = *hash.0.as_bytes();
    self
      .retrying(peer, None, |link| async move {
        let (mut send, mut recv) = link.conn.open_bi().await.map_err(Error::net)?;
        write_frame(&mut send, &Frame::GetBlob { hash: hash_bytes }).await?;
        send.finish().map_err(Error::net)?;
        let first = match link.fresh {
          true => read_frame(&mut recv).await,
          false => tokio::time::timeout(FIRST_REPLY, read_frame(&mut recv))
            .await
            .map_err(|_| Error::Stale)?,
        };
        match first? {
          Frame::Blob { hash: h, size } if h == hash_bytes => Ok(size.map(|size| Offer {
            conn: link.conn.clone(),
            recv,
            size,
          })),
          f => Err(unexpected(&f, "the requested Blob")),
        }
      })
      .await
      .map_err(|e| match e {
        AskError::Unreachable(why) => Error::Net(why),
        AskError::Failed(e) => e,
      })
  }

  /// Receives an offered blob into the store.
  async fn take(&self, mut offer: Offer, hash: &BlobHash) -> Result<()> {
    let writer = self.state.blobs.writer()?;
    receive_blob(writer, &mut offer.recv, hash, offer.size)
      .await
      .map_err(|e| refusal(&offer.conn, e))
  }

  /// Asks a peer a question in the host's encoding; its answer, likewise.
  pub async fn ask(&self, peer: EndpointAddr, question: Vec<u8>) -> Result<Vec<u8>> {
    let link = self.link(&peer, None).await.map_err(|e| match e {
      AskError::Unreachable(why) => Error::Net(why),
      AskError::Failed(e) => e,
    })?;
    self.ask_on(&peer, link, question).await
  }

  /// Asks a peer a question that may take long to answer (a tool call):
  /// reaching it takes at most `connect` (else `Unreachable`), then the
  /// answer is waited for as long as the connection lives, which keeps
  /// itself alive meanwhile.
  pub async fn ask_within(
    &self,
    peer: EndpointAddr,
    question: Vec<u8>,
    connect: Duration,
  ) -> std::result::Result<Vec<u8>, AskError> {
    let link = self.link(&peer, Some(connect)).await?;
    self
      .ask_on(&peer, link, question)
      .await
      .map_err(AskError::Failed)
  }

  /// Asks a peer a question that may be asked twice (its answer only
  /// notes a fact, as a call's outcome) and is answered at once: reaching
  /// it takes at most `connect`; a kept connection that brings no answer
  /// within `reply` is taken for dead (a phone back from suspension keeps
  /// connections its peers already dropped) and the question is asked
  /// again on a fresh one, as syncs are.
  pub async fn ask_again_within(
    &self,
    peer: EndpointAddr,
    question: Vec<u8>,
    connect: Duration,
    reply: Duration,
  ) -> std::result::Result<Vec<u8>, AskError> {
    self
      .retrying(&peer, Some(connect), |link| {
        let question = question.clone();
        async move {
          let (mut send, mut recv) = link.conn.open_bi().await.map_err(Error::net)?;
          write_frame(&mut send, &Frame::Ask(question)).await?;
          send.finish().map_err(Error::net)?;
          match tokio::time::timeout(reply, read_frame(&mut recv))
            .await
            .map_err(|_| Error::Stale)??
          {
            Frame::Answer(bytes) => Ok(bytes),
            Frame::Refused { reason } => Err(Error::Refused(reason)),
            f => Err(unexpected(&f, "Answer")),
          }
        }
      })
      .await
  }

  /// A question is never asked twice (its answer may run a tool): a kept
  /// connection found dead before it was asked is dialed afresh, one that
  /// dies after is the error.
  async fn ask_on(&self, peer: &EndpointAddr, link: Link, question: Vec<u8>) -> Result<Vec<u8>> {
    let conn = link.conn.clone();
    let opened = match conn.open_bi().await {
      Ok(s) => s,
      Err(_) if !link.fresh => {
        self.drop_link(&peer.id, &conn);
        let link = self.link(peer, None).await.map_err(|e| match e {
          AskError::Unreachable(why) => Error::Net(why),
          AskError::Failed(e) => e,
        })?;
        return Box::pin(self.ask_on(peer, link, question)).await;
      }
      Err(e) => return Err(Error::net(e)),
    };
    let result = async {
      let (mut send, mut recv) = opened;
      write_frame(&mut send, &Frame::Ask(question)).await?;
      send.finish().map_err(Error::net)?;
      match read_frame(&mut recv).await? {
        Frame::Answer(bytes) => Ok(bytes),
        Frame::Refused { reason } => Err(Error::Refused(reason)),
        f => Err(unexpected(&f, "Answer")),
      }
    }
    .await;
    if result.is_err() && !alive(&conn) {
      self.drop_link(&peer.id, &conn);
    }
    result.map_err(|e| refusal(&conn, e))
  }

  pub async fn shutdown(self) -> Result<()> {
    self.forget_connections();
    self.router.shutdown().await.map_err(Error::net)
  }
}

/// A peer's answer that it holds a blob: the blob follows on `recv`.
struct Offer {
  conn: Connection,
  recv: RecvStream,
  size: u64,
}

/// Each repository's whole fingerprint here (its open turns sealed first
/// when `seal`).
fn fingerprints(state: &NodeState, repos: &[RepoId], seal: bool) -> Result<Vec<Fingerprint>> {
  let mut index = state.index.lock().expect("index lock");
  let mut out = Vec::with_capacity(repos.len());
  for repo in repos {
    match state.root(repo) {
      Some(root) => {
        if seal {
          root.seal_all()?;
        }
        index.refresh(&root)?;
        out.push(index.fingerprint(repo)?);
      }
      None => out.push(Fingerprint::default()),
    }
  }
  Ok(out)
}

/// When the peer closed the connection with a reason (it does not know this
/// node), that reason is the error.
pub(crate) fn refusal(conn: &Connection, e: Error) -> Error {
  match conn.close_reason() {
    Some(iroh::endpoint::ConnectionError::ApplicationClosed(close))
      if close.error_code != 0u32.into() =>
    {
      Error::Refused(String::from_utf8_lossy(&close.reason).into_owned())
    }
    _ => e,
  }
}

#[derive(Clone)]
struct Handler {
  state: Arc<NodeState>,
}

impl std::fmt::Debug for Handler {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Handler({})", self.state.node)
  }
}

impl ProtocolHandler for Handler {
  async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
    let remote = conn.remote_id();
    let state = self.state.clone();
    let allowed = tokio::task::spawn_blocking(move || state.allows(&remote))
      .await
      .ok()
      .flatten();
    let Some(peer) = allowed else {
      conn.close(1u32.into(), b"not one of this user's nodes");
      return Ok(());
    };
    self.state.note_reached(&remote, &conn);
    {
      let mut pool = self.state.pool.lock().expect("pool lock");
      pool.peers.entry(remote).or_default().accepted = Some((conn.clone(), false));
    }
    serve_conn(self.state.clone(), conn, peer).await;
    Ok(())
  }
}

/// Serves the streams a peer opens on a connection, whichever side dialed.
async fn serve_conn(state: Arc<NodeState>, conn: Connection, peer: String) {
  while let Ok((send, recv)) = conn.accept_bi().await {
    let state = state.clone();
    let peer = peer.clone();
    let conn = conn.clone();
    tokio::spawn(async move {
      let result = match serve_stream(&state, &peer, &conn, send, recv).await {
        Ok(Some(report)) => Ok(report),
        Ok(None) => return,
        Err(e) => Err(e),
      };
      state.emit(Event::Session { peer, result });
    });
  }
}

/// The peer showed it speaks version 3 on a connection this node
/// accepted: this node's own streams may use it.
fn speaks_v3(state: &NodeState, conn: &Connection) {
  let mut pool = state.pool.lock().expect("pool lock");
  if let Some(Held {
    accepted: Some((c, v3)),
    ..
  }) = pool.peers.get_mut(&conn.remote_id())
    && c.stable_id() == conn.stable_id()
  {
    *v3 = true;
  }
}

/// Serves one stream; a sync returns its report, anything else nothing.
async fn serve_stream(
  state: &Arc<NodeState>,
  peer: &str,
  conn: &Connection,
  mut send: SendStream,
  mut recv: RecvStream,
) -> Result<Option<SessionReport>> {
  let frame = read_frame(&mut recv).await?;
  if state.speaks == LEGACY {
    // As a node of version 2: it knows no newer frame, refuses a newer
    // session.
    match &frame {
      Frame::Summary { .. } | Frame::Summaries { .. } | Frame::GetBlobAt { .. } => {
        return Err(Error::Protocol("unknown frame".into()));
      }
      Frame::Hello { version, .. } if *version != LEGACY => {
        write_frame(
          &mut send,
          &Frame::Refused {
            reason: format!("protocol version {version}, this node speaks {LEGACY}"),
          },
        )
        .await?;
        return Err(Error::Protocol(format!("peer speaks version {version}")));
      }
      _ => {}
    }
  }
  match frame {
    Frame::Hello { version, repo } => {
      if version > LEGACY {
        speaks_v3(state, conn);
      }
      let started = Instant::now();
      let mut report = respond(state, peer, send, recv, version, &repo).await?;
      report.elapsed = started.elapsed();
      if version > LEGACY && state.policy.blobs == BlobPolicy::All {
        // The blobs this node wants, from the peer, once the session is
        // over (its stream is not held for them).
        let (state, conn) = (state.clone(), conn.clone());
        if let Ok(repo) = repo.parse::<RepoId>() {
          tokio::spawn(async move {
            fetch_wanted(&state, &conn, &repo).await;
          });
        }
      }
      Ok(Some(report))
    }
    Frame::Summary {
      version: _,
      seal,
      repos,
    } => {
      speaks_v3(state, conn);
      let st = state.clone();
      let mine = tokio::task::spawn_blocking(move || -> Result<Vec<Option<Fingerprint>>> {
        let ids: Vec<Option<RepoId>> = repos.iter().map(|(r, _)| r.parse().ok()).collect();
        // A project this node has not heard of yet may be listed by now.
        if ids.iter().flatten().any(|r| st.root(r).is_none()) {
          let _ = st.hold_projects();
        }
        let held: Vec<RepoId> = ids
          .iter()
          .flatten()
          .filter(|r| st.root(r).is_some())
          .cloned()
          .collect();
        let prints = fingerprints(&st, &held, seal)?;
        Ok(
          ids
            .iter()
            .map(|r| {
              r.as_ref()
                .and_then(|r| held.iter().position(|h| h == r))
                .map(|i| prints[i])
            })
            .collect(),
        )
      })
      .await
      .map_err(Error::net)??;
      write_frame(
        &mut send,
        &Frame::Summaries {
          version: VERSION,
          repos: mine,
        },
      )
      .await?;
      send.finish().map_err(Error::net)?;
      Ok(None)
    }
    Frame::GetBlob { hash } => {
      let hash = BlobHash(blake3::Hash::from_bytes(hash));
      send_blob(&state.blobs, &mut send, &hash, 0).await?;
      send.finish().map_err(Error::net)?;
      Ok(None)
    }
    Frame::GetBlobAt { hash, offset } => {
      speaks_v3(state, conn);
      let hash = BlobHash(blake3::Hash::from_bytes(hash));
      send_blob(&state.blobs, &mut send, &hash, offset).await?;
      send.finish().map_err(Error::net)?;
      Ok(None)
    }
    Frame::Ask(question) => {
      let reply = match state.answerer.get() {
        // An asker gone (its call interrupted, its node stopped) stops the
        // stream: the answer is dropped unfinished, and what it ran with it.
        Some(answer) => tokio::select! {
          answered = answer(peer.to_owned(), question) => Frame::Answer(answered),
          _ = send.stopped() => return Ok(None),
        },
        None => Frame::Refused {
          reason: "this node answers no questions yet".into(),
        },
      };
      write_frame(&mut send, &reply).await?;
      send.finish().map_err(Error::net)?;
      Ok(None)
    }
    f => Err(unexpected(&f, "Hello, Summary, GetBlob or Ask")),
  }
}

async fn initiate(
  state: &Arc<NodeState>,
  conn: &Connection,
  root: RecordRoot,
  version: u32,
  seal: bool,
) -> Result<SessionReport> {
  let peer = conn.remote_id();
  let (mut send, mut recv) = conn.open_bi().await.map_err(Error::net)?;
  write_frame(
    &mut send,
    &Frame::Hello {
      version,
      repo: root.repo().to_string(),
    },
  )
  .await?;
  // A kept connection that brings no answer is dead (the caller dials
  // afresh); a session under way may take as long as it moves.
  let welcome = tokio::time::timeout(FIRST_REPLY, read_frame(&mut recv))
    .await
    .map_err(|_| Error::Stale)??;
  match welcome {
    Frame::Welcome => {}
    Frame::Refused { reason } => return Err(Error::Refused(reason)),
    f => return Err(unexpected(&f, "Welcome")),
  }
  let items = refreshed_items(state, &root, seal || version == LEGACY).await?;
  let mut reconciler = Reconciler::new(&items);
  let mut batch = reconciler.initiate();
  let mut rounds = 0;
  loop {
    write_frame(&mut send, &Frame::Round(batch)).await?;
    rounds += 1;
    let replies = match read_frame_idle(&mut recv).await? {
      Frame::Round(m) => m,
      f => return Err(unexpected(&f, "Round")),
    };
    batch = replies
      .into_iter()
      .flat_map(|m| reconciler.handle(m))
      .collect();
    if batch.is_empty() {
      write_frame(&mut send, &Frame::Finish).await?;
      break;
    }
  }
  let origin = state.cached_name(&peer);
  let mut report = exchange(
    state,
    &root,
    &mut send,
    &mut recv,
    reconciler.finish(),
    version,
    origin.as_deref(),
  )
  .await?;
  report.rounds = rounds;
  match read_frame_idle(&mut recv).await? {
    Frame::Bye => {}
    f => return Err(unexpected(&f, "Bye")),
  }
  send.finish().map_err(Error::net)?;
  Ok(report)
}

async fn respond(
  state: &Arc<NodeState>,
  peer: &str,
  mut send: SendStream,
  mut recv: RecvStream,
  version: u32,
  repo: &str,
) -> Result<SessionReport> {
  let refuse = |reason: String| Frame::Refused { reason };
  if version != VERSION && version != LEGACY {
    write_frame(
      &mut send,
      &refuse(format!(
        "protocol version {version}, this node speaks {LEGACY} and {VERSION}"
      )),
    )
    .await?;
    return Err(Error::Protocol(format!("peer speaks version {version}")));
  }
  // A project this node has not heard of yet may be listed by now.
  let held = |r: &RepoId| {
    state.root(r).or_else(|| {
      let _ = state.hold_projects();
      state.root(r)
    })
  };
  let Some(root) = repo.parse::<RepoId>().ok().and_then(|r| held(&r)) else {
    write_frame(
      &mut send,
      &refuse(format!("repository {repo} is not held here")),
    )
    .await?;
    return Err(Error::Config(format!("peer asked for repository {repo}")));
  };
  write_frame(&mut send, &Frame::Welcome).await?;
  // Version 2 seals here as it always did; in version 3 the initiator's
  // summary asked for it when the sync wants it.
  let items = refreshed_items(state, &root, version == LEGACY).await?;
  let mut reconciler = Reconciler::new(&items);
  let mut rounds = 0;
  loop {
    match read_frame_idle(&mut recv).await? {
      Frame::Round(messages) => {
        let replies = messages
          .into_iter()
          .flat_map(|m| reconciler.handle(m))
          .collect();
        write_frame(&mut send, &Frame::Round(replies)).await?;
        rounds += 1;
      }
      Frame::Finish => break,
      f => return Err(unexpected(&f, "Round or Finish")),
    }
  }
  let mut report = exchange(
    state,
    &root,
    &mut send,
    &mut recv,
    reconciler.finish(),
    version,
    Some(peer),
  )
  .await?;
  report.rounds = rounds;
  write_frame(&mut send, &Frame::Bye).await?;
  send.finish().map_err(Error::net)?;
  Ok(report)
}

/// Steps 3 and 4, the same on both sides: files, then (version 2) blobs.
async fn exchange(
  state: &Arc<NodeState>,
  root: &RecordRoot,
  send: &mut SendStream,
  recv: &mut RecvStream,
  outcome: Outcome,
  version: u32,
  origin: Option<&str>,
) -> Result<SessionReport> {
  let mut report = SessionReport {
    repo: root.repo().to_string(),
    version,
    conflicts: outcome.conflicts.into_iter().collect(),
    ..SessionReport::default()
  };
  let give: Vec<String> = outcome.give.into_iter().collect();
  let (sent, received) = tokio::join!(send_files(send, root, &give), receive_files(recv, root));
  report.sent = sent?;
  let received = received?;
  report.received = received.received;
  report.conflicts.extend(received.conflicts);
  report.rejected = received.rejected;
  // Index what arrived (the change feed hears where it came from).
  let st = state.clone();
  let r = root.clone();
  let origin = origin.map(str::to_owned);
  let received_any = !report.received.is_empty();
  let wanted = tokio::task::spawn_blocking(move || -> Result<_> {
    let mut index = st.index.lock().expect("index lock");
    index.refresh_from(&r, origin.as_deref())?;
    drop(index);
    if received_any && r.config().role == Role::Home {
      // `peer` and `revoke` records may have come: trust is read afresh,
      // and a node revoked loses its connections.
      st.recheck_trust();
    }
    let index = st.index.lock().expect("index lock");
    if version > LEGACY {
      return Ok(Vec::new());
    }
    let wanted: Vec<[u8; 32]> = match st.policy.blobs {
      BlobPolicy::Lazy => Vec::new(),
      BlobPolicy::All => index
        .blob_refs(r.repo())?
        .into_iter()
        .filter(|h| !st.blobs.contains(h))
        .map(|h| *h.0.as_bytes())
        .collect(),
    };
    Ok(wanted)
  })
  .await
  .map_err(Error::net)??;
  if version > LEGACY {
    return Ok(report);
  }

  let (tx, rx) = oneshot::channel();
  let (served, fetched) = tokio::join!(
    serve_wanted(state, send, wanted, rx),
    receive_blobs(state, recv, tx)
  );
  report.blobs_sent = served?;
  let (got, unavailable) = fetched?;
  report.blobs_received = got;
  report.blobs_unavailable = unavailable;
  Ok(report)
}

async fn refreshed_items(state: &Arc<NodeState>, root: &RecordRoot, seal: bool) -> Result<ItemSet> {
  let st = state.clone();
  let r = root.clone();
  tokio::task::spawn_blocking(move || -> Result<ItemSet> {
    let mut index = st.index.lock().expect("index lock");
    // Turns running here are sealed first, under the index's lock, so what
    // syncs has every step written so far.
    if seal {
      r.seal_all()?;
    }
    index.refresh(&r)?;
    Ok(index.items(r.repo())?)
  })
  .await
  .map_err(Error::net)?
}

async fn send_files(send: &mut SendStream, root: &RecordRoot, paths: &[String]) -> Result<usize> {
  for path in paths {
    let item = ItemPath::parse(path).ok_or_else(|| StoreError::InvalidPath(path.clone()))?;
    let bytes = root.read(&item)?;
    write_frame(
      send,
      &Frame::File {
        path: path.clone(),
        bytes,
      },
    )
    .await?;
  }
  write_frame(send, &Frame::FilesDone).await?;
  Ok(paths.len())
}

#[derive(Default)]
struct Received {
  received: Vec<String>,
  conflicts: Vec<String>,
  rejected: Vec<(String, String)>,
}

async fn receive_files(recv: &mut RecvStream, root: &RecordRoot) -> Result<Received> {
  let mut out = Received::default();
  loop {
    match read_frame_idle(recv).await? {
      Frame::File { path, bytes } => match root.insert(&path, &bytes) {
        Ok(Inserted::New) => out.received.push(path),
        // Present, or a deleted thread's (the peer has not purged it yet).
        Ok(Inserted::Present | Inserted::Purged) => {}
        Err(StoreError::Conflict(p)) => out.conflicts.push(p),
        Err(e @ (StoreError::Mismatch { .. } | StoreError::InvalidPath(_))) => {
          out.rejected.push((path, e.to_string()))
        }
        Err(e) => return Err(e.into()),
      },
      Frame::FilesDone => return Ok(out),
      f => return Err(unexpected(&f, "File or FilesDone")),
    }
  }
}

/// Version 2: sends our wants, then serves the peer's.
async fn serve_wanted(
  state: &Arc<NodeState>,
  send: &mut SendStream,
  wanted: Vec<[u8; 32]>,
  theirs: oneshot::Receiver<Vec<[u8; 32]>>,
) -> Result<usize> {
  write_frame(send, &Frame::WantBlobs(wanted)).await?;
  // The receiving half hands over the peer's list; dropped means it failed.
  let theirs = theirs.await.unwrap_or_default();
  let mut sent = 0;
  for hash in theirs {
    if send_blob(
      &state.blobs,
      send,
      &BlobHash(blake3::Hash::from_bytes(hash)),
      0,
    )
    .await?
    {
      sent += 1;
    }
  }
  write_frame(send, &Frame::BlobsDone).await?;
  Ok(sent)
}

/// Version 2: reads the peer's wants, then the blobs it sends:
/// (received, unavailable).
async fn receive_blobs(
  state: &Arc<NodeState>,
  recv: &mut RecvStream,
  theirs: oneshot::Sender<Vec<[u8; 32]>>,
) -> Result<(usize, usize)> {
  match read_frame_idle(recv).await? {
    Frame::WantBlobs(w) => {
      let _ = theirs.send(w);
    }
    f => return Err(unexpected(&f, "WantBlobs")),
  }
  let (mut got, mut unavailable) = (0, 0);
  loop {
    match read_frame_idle(recv).await? {
      Frame::Blob {
        hash,
        size: Some(size),
      } => {
        let writer = state.blobs.writer()?;
        receive_blob(
          writer,
          recv,
          &BlobHash(blake3::Hash::from_bytes(hash)),
          size,
        )
        .await?;
        got += 1;
      }
      Frame::Blob { size: None, .. } => unavailable += 1,
      Frame::BlobsDone => return Ok((got, unavailable)),
      f => return Err(unexpected(&f, "Blob or BlobsDone")),
    }
  }
}

/// Version 3: the blobs a repository's records reference that this node
/// lacks (all of them, unless it fetches lazily), fetched from the peer of
/// `conn`, several at once, each going on from what an earlier transfer
/// cut off left: (received, unavailable).
async fn fetch_wanted(state: &Arc<NodeState>, conn: &Connection, repo: &RepoId) -> (usize, usize) {
  if state.policy.blobs != BlobPolicy::All {
    return (0, 0);
  }
  let peer = conn.remote_id();
  let st = state.clone();
  let r = repo.clone();
  let wanted: Vec<BlobHash> = tokio::task::spawn_blocking(move || {
    let index = st.index.lock().expect("index lock");
    index.blob_refs(&r).unwrap_or_default()
  })
  .await
  .unwrap_or_default()
  .into_iter()
  .filter(|h| !state.blobs.contains(h))
  .collect();
  let wanted: Vec<BlobHash> = {
    let mut pool = state.pool.lock().expect("pool lock");
    pool
      .unavailable
      .retain(|_, at| at.elapsed() < UNAVAILABLE_FOR);
    let mut mine = Vec::new();
    for h in wanted {
      let key = *h.0.as_bytes();
      if pool.unavailable.contains_key(&(peer, key)) || pool.fetching.contains(&key) {
        continue;
      }
      pool.fetching.insert(key);
      mine.push(h);
    }
    mine
  };
  let (mut got, mut unavailable) = (0, 0);
  let mut running = FuturesUnordered::new();
  let mut next = wanted.into_iter();
  loop {
    while running.len() < BLOB_FETCHES
      && let Some(h) = next.next()
    {
      running.push(async move {
        let r = get_blob_at(state, conn, &h).await;
        state
          .pool
          .lock()
          .expect("pool lock")
          .fetching
          .remove(h.0.as_bytes());
        (h, r)
      });
    }
    match running.next().await {
      Some((_, Ok(true))) => got += 1,
      Some((h, Ok(false))) => {
        unavailable += 1;
        state
          .pool
          .lock()
          .expect("pool lock")
          .unavailable
          .insert((peer, *h.0.as_bytes()), Instant::now());
      }
      // Cut off: what came is kept for the next try.
      Some((_, Err(_))) => {}
      None => break,
    }
  }
  (got, unavailable)
}

/// One blob on its own stream, from where a cut-off transfer stopped;
/// false if the peer does not hold it.
async fn get_blob_at(state: &NodeState, conn: &Connection, hash: &BlobHash) -> Result<bool> {
  let writer = state.blobs.resume(hash)?;
  let offset = writer.size();
  let (mut send, mut recv) = conn.open_bi().await.map_err(Error::net)?;
  write_frame(
    &mut send,
    &Frame::GetBlobAt {
      hash: *hash.0.as_bytes(),
      offset,
    },
  )
  .await?;
  send.finish().map_err(Error::net)?;
  match read_frame_idle(&mut recv).await? {
    Frame::Blob { size: None, .. } => Ok(false),
    Frame::Blob {
      hash: h,
      size: Some(total),
    } if h == *hash.0.as_bytes() => {
      if total < offset {
        // Not the blob what was kept came from: start over next time.
        drop(writer);
        state.blobs.drop_partial(hash);
        return Err(Error::Protocol(format!(
          "blob {hash} is shorter than what came"
        )));
      }
      receive_blob(writer, &mut recv, hash, total - offset).await?;
      Ok(true)
    }
    f => Err(unexpected(&f, "the requested Blob")),
  }
}

const CHUNK: usize = 1 << 16;

/// Writes a `Blob` frame (its whole size) and the content from `offset`;
/// false if this node lacks it.
async fn send_blob(
  blobs: &BlobStore,
  send: &mut SendStream,
  hash: &BlobHash,
  offset: u64,
) -> Result<bool> {
  use std::io::{Read, Seek, SeekFrom};
  let hash_bytes = *hash.0.as_bytes();
  let Ok(mut file) = blobs.open_blob(hash) else {
    write_frame(
      send,
      &Frame::Blob {
        hash: hash_bytes,
        size: None,
      },
    )
    .await?;
    return Ok(false);
  };
  let io = |e| Error::Io {
    path: hash.to_string(),
    source: e,
  };
  let size = file.metadata().map_err(io)?.len();
  write_frame(
    send,
    &Frame::Blob {
      hash: hash_bytes,
      size: Some(size),
    },
  )
  .await?;
  if offset >= size {
    return Ok(true);
  }
  file.seek(SeekFrom::Start(offset)).map_err(io)?;
  let mut buf = vec![0u8; CHUNK];
  let mut left = size - offset;
  while left > 0 {
    let n = file
      .read(&mut buf[..CHUNK.min(left as usize)])
      .map_err(io)?;
    if n == 0 {
      return Err(Error::Protocol(format!(
        "blob {hash} shrank while being sent"
      )));
    }
    send.write_all(&buf[..n]).await.map_err(Error::net)?;
    left -= n as u64;
  }
  Ok(true)
}

/// Reads `size` raw bytes into `writer`; the whole content must hash to
/// `hash`. Each chunk has `IDLE` to come.
async fn receive_blob(
  mut writer: stitch_store::BlobWriter,
  recv: &mut RecvStream,
  hash: &BlobHash,
  size: u64,
) -> Result<()> {
  let mut buf = vec![0u8; CHUNK];
  let mut left = size;
  while left > 0 {
    let n = CHUNK.min(left as usize);
    match tokio::time::timeout(crate::proto::IDLE, recv.read_exact(&mut buf[..n])).await {
      Ok(r) => r.map_err(Error::net)?,
      Err(_) => {
        return Err(Error::Protocol(format!(
          "blob {hash}: nothing for {}s",
          crate::proto::IDLE.as_secs()
        )));
      }
    }
    writer.write(&buf[..n])?;
    left -= n as u64;
  }
  writer.finish(Some(hash))?;
  Ok(())
}
