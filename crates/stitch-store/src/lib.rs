//! The store of a Stitch node (docs/records.md): record roots on disk, the
//! blob store, the local index with derived status, and record sync.

pub mod artefacts;
mod blobs;
pub mod changes;
mod error;
mod index;
mod journal;
pub mod purge;
mod root;
pub mod sync;

pub use blobs::{BlobStore, BlobWriter};
pub use changes::{Change, ChangeFeed};
pub use error::Error;
pub use index::{
  AssignmentStatus, AttentionItem, DIGEST_KIND, DigestRow, Edge, FindingStatus, Index,
  IndexedRecord, RefreshReport, SetValue, TaskStatus, Touch,
};
pub use root::{Inserted, ItemPath, RecordRoot, RepoConfig, Role};

pub type Result<T, E = Error> = std::result::Result<T, E>;
