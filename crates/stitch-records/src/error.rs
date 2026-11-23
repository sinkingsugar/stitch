#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("invalid record ID `{0}`")]
  InvalidId(String),
  #[error("invalid node name `{0}` (1 to 12 of a-z, 0-9)")]
  InvalidNode(String),
  #[error("invalid repository ID `{0}` (1 to 64 of a-z, 0-9, -)")]
  InvalidRepo(String),
  #[error("invalid reference `{0}`")]
  InvalidRef(String),
  #[error("invalid blob hash `{0}`")]
  InvalidBlobHash(String),
  #[error("invalid name `{0}` (a-z first, then a-z, 0-9, -)")]
  InvalidName(String),
  #[error("a record starts with a `+++` line and its front matter ends with one")]
  NoFrontMatter,
  #[error("front matter: {0}")]
  Toml(#[from] toml::de::Error),
  #[error("missing field `{0}`")]
  Missing(&'static str),
  #[error("field `{field}`: {message}")]
  Field { field: String, message: String },
  #[error("`{0}` is set by the record itself, not as a field")]
  Reserved(String),
  #[error("unsupported format version {0}")]
  Version(i64),
  #[error("segment line {line}: {message}")]
  Segment { line: usize, message: String },
}

impl Error {
  pub(crate) fn field(field: &str, message: impl std::fmt::Display) -> Self {
    Error::Field {
      field: field.to_owned(),
      message: message.to_string(),
    }
  }
}
