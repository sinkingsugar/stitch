use stitch_records::BlobHash;

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error(transparent)]
  Store(#[from] stitch_store::Error),
  #[error(transparent)]
  Format(#[from] stitch_records::Error),
  #[error(transparent)]
  Net(#[from] stitch_net::Error),
  #[error(transparent)]
  Provider(#[from] stitch_providers::Error),
  #[error("{path}: {source}")]
  Io {
    path: String,
    source: std::io::Error,
  },
  #[error("{0}")]
  Config(String),
  /// Blobs a chat needs that this node does not hold.
  #[error("{} attachment(s) not on this node", .0.len())]
  MissingBlobs(Vec<BlobHash>),
  #[error("a node already runs for this data directory (pid {0})")]
  AlreadyRunning(String),
  #[error("no node runs for this data directory")]
  NotRunning,
  #[error("the running node is version {node}, this command is {client}: {what}")]
  VersionMismatch {
    node: String,
    client: String,
    /// What happens about it.
    what: String,
  },
  /// The node stops to be started at the client's version.
  #[error("the node is restarting for a newer version: try again")]
  Restarting,
  #[error("local protocol: {0}")]
  Protocol(String),
  /// A turn's model call failed after what the step had to write was
  /// written with the failure (the user's message, messages woven in):
  /// nothing is to be sent again, and the turn can be resumed. `said` is
  /// what the user is told (`crate::failed`).
  #[error("{said}")]
  Kept { said: String, source: Box<Error> },
  /// An error the node reported to a client.
  #[error("{0}")]
  Remote(String),
}

impl Error {
  pub(crate) fn io(path: impl std::fmt::Display) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_string();
    move |source| Error::Io { path, source }
  }
}
