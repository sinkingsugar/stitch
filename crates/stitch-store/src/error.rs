use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("{}: {source}", path.display())]
  Io {
    path: PathBuf,
    source: std::io::Error,
  },
  #[error("{}: {source}", path.display())]
  Record {
    path: PathBuf,
    source: stitch_records::Error,
  },
  #[error(transparent)]
  Format(#[from] stitch_records::Error),
  #[error("index: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("{} is not a record root (no repo.toml)", .0.display())]
  NotARoot(PathBuf),
  #[error("`{0}` is not a record path")]
  InvalidPath(String),
  #[error("`{path}`: {message}")]
  Mismatch { path: String, message: String },
  /// The same path with different content: files are immutable, so one of
  /// the copies is corrupt. Neither is overwritten.
  #[error("`{0}` exists with different content")]
  Conflict(String),
  #[error("no free record ID after {0} attempts")]
  Collisions(usize),
  #[error("git: {0}")]
  Git(String),
}

pub(crate) trait IoContext<T> {
  fn at(self, path: &Path) -> Result<T, Error>;
}

impl<T> IoContext<T> for std::io::Result<T> {
  fn at(self, path: &Path) -> Result<T, Error> {
    self.map_err(|source| Error::Io {
      path: path.to_owned(),
      source,
    })
  }
}
