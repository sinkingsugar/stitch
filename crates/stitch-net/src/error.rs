#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error(transparent)]
  Store(#[from] stitch_store::Error),
  #[error(transparent)]
  Format(#[from] stitch_records::Error),
  #[error("{path}: {source}")]
  Io {
    path: String,
    source: std::io::Error,
  },
  #[error("network: {0}")]
  Net(String),
  /// The peer broke the protocol.
  #[error("protocol: {0}")]
  Protocol(String),
  #[error("refused by the peer: {0}")]
  Refused(String),
  #[error("{0}")]
  Config(String),
  /// A kept connection brought no answer in time: dead, to be dialed
  /// afresh.
  #[error("no answer on the kept connection")]
  Stale,
}

impl Error {
  pub(crate) fn net(e: impl std::fmt::Display) -> Self {
    Error::Net(e.to_string())
  }
}
