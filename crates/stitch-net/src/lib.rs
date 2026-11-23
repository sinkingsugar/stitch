//! The node protocol over iroh (design §2.6): record sync by range-based
//! reconciliation, file transfer, and blob fetch, between the user's own
//! nodes, on `stitch/node/1`, where each operation is a bidirectional stream
//! whose first frame says what it is (questions the host answers among
//! them); and pairing of new nodes, on
//! `stitch/pair/1`.

mod error;
mod identity;
mod link;
mod node;
mod pair;
mod proto;
mod seen;

pub use error::Error;
pub use identity::{Peer, Peers, load_or_create_key};
pub use link::{Net, PeerReport, PeerSync, RepoState};
pub use node::{
  Answerer, AskError, BlobPolicy, Discovery, Event, NodeState, Policy, SessionReport, bind,
};
pub use pair::{Joined, PAIR_ALPN, Ticket};
pub use proto::ALPN;
pub use seen::{PathKind, Seen, SeenPeers, ago, is_local_ip};

pub type Result<T, E = Error> = std::result::Result<T, E>;
