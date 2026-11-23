//! Names: record IDs, node names, repository IDs, kinds, references and blob
//! hashes (docs/records.md §2).

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use rand::RngExt;

use crate::{Error, string_serde};

/// Alphabet of the random suffix: lowercase base32 without `l`, `o`, `0`, `1`.
pub const SUFFIX_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";
const SUFFIX_LEN: usize = 4;
/// `2026-10-11T143207Z`
const TIME_LEN: usize = 18;

fn is_lower_alnum(b: u8) -> bool {
  b.is_ascii_lowercase() || b.is_ascii_digit()
}

/// A node's short name, `[a-z0-9]{1,12}`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeName(String);

impl NodeName {
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl FromStr for NodeName {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    if (1..=12).contains(&s.len()) && s.bytes().all(is_lower_alnum) {
      Ok(NodeName(s.to_owned()))
    } else {
      Err(Error::InvalidNode(s.to_owned()))
    }
  }
}

impl fmt::Display for NodeName {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

/// A repository's stable ID, `[a-z0-9-]{1,64}`, from its `repo.toml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoId(String);

impl RepoId {
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl FromStr for RepoId {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    if (1..=64).contains(&s.len()) && s.bytes().all(|b| is_lower_alnum(b) || b == b'-') {
      Ok(RepoId(s.to_owned()))
    } else {
      Err(Error::InvalidRepo(s.to_owned()))
    }
  }
}

impl fmt::Display for RepoId {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

/// A record ID: `2026-10-11T143207Z-mbp-k3f9`, UTC time to the second, the
/// writing node's name and a random suffix. Sorts by time.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId(String);

impl RecordId {
  /// A new ID for `at` on `node`. Uniqueness on one node comes from
  /// creating the file exclusively and retrying with a new suffix.
  pub fn generate<R: rand::Rng + ?Sized>(at: Timestamp, node: &NodeName, rng: &mut R) -> Self {
    let mut s = at.strftime("%Y-%m-%dT%H%M%SZ").to_string();
    s.push('-');
    s.push_str(node.as_str());
    s.push('-');
    for _ in 0..SUFFIX_LEN {
      s.push(SUFFIX_ALPHABET[rng.random_range(0..SUFFIX_ALPHABET.len())] as char);
    }
    RecordId(s)
  }

  /// A new ID for `at` on `node`, with the thread-local generator.
  pub fn random(at: Timestamp, node: &NodeName) -> Self {
    RecordId::generate(at, node, &mut rand::rng())
  }

  pub fn as_str(&self) -> &str {
    &self.0
  }

  /// The time in the ID, to the second.
  pub fn timestamp(&self) -> Timestamp {
    let t = &self.0;
    // Validated in `from_str`: `YYYY-MM-DDTHHMMSSZ`.
    format!("{}:{}:{}Z", &t[..13], &t[13..15], &t[15..17])
      .parse()
      .expect("validated when parsed")
  }

  /// `2026-10`: the month directory the record lives in.
  pub fn month(&self) -> &str {
    &self.0[..7]
  }

  pub fn node(&self) -> &str {
    let rest = &self.0[TIME_LEN + 1..];
    &rest[..rest.len() - SUFFIX_LEN - 1]
  }
}

impl FromStr for RecordId {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    let bad = || Error::InvalidId(s.to_owned());
    let b = s.as_bytes();
    if b.len() < TIME_LEN + 1 + 1 + 1 + SUFFIX_LEN || !s.is_ascii() {
      return Err(bad());
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let shape = digits(0..4)
      && b[4] == b'-'
      && digits(5..7)
      && b[7] == b'-'
      && digits(8..10)
      && b[10] == b'T'
      && digits(11..17)
      && b[17] == b'Z'
      && b[18] == b'-';
    if !shape {
      return Err(bad());
    }
    let rest = &s[TIME_LEN + 1..];
    let (node, suffix) = rest.rsplit_once('-').ok_or_else(bad)?;
    node.parse::<NodeName>().map_err(|_| bad())?;
    if suffix.len() != SUFFIX_LEN || !suffix.bytes().all(|c| SUFFIX_ALPHABET.contains(&c)) {
      return Err(bad());
    }
    let time = format!("{}:{}:{}Z", &s[..13], &s[13..15], &s[15..17]);
    time.parse::<Timestamp>().map_err(|_| bad())?;
    Ok(RecordId(s.to_owned()))
  }
}

impl fmt::Display for RecordId {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

fn is_name(s: &str) -> bool {
  let mut b = s.bytes();
  matches!(b.next(), Some(c) if c.is_ascii_lowercase())
    && b.all(|c| is_lower_alnum(c) || c == b'-')
    && s.len() <= 64
}

/// A record kind (`task`, `question`, ...): `[a-z][a-z0-9-]*`. Unknown kinds
/// are valid; the node only derives status for the ones it knows.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Kind(String);

impl Kind {
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl FromStr for Kind {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    if is_name(s) {
      Ok(Kind(s.to_owned()))
    } else {
      Err(Error::InvalidName(s.to_owned()))
    }
  }
}

impl fmt::Display for Kind {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

impl PartialEq<&str> for Kind {
  fn eq(&self, other: &&str) -> bool {
    self.0 == *other
  }
}

/// The kinds of docs/records.md §4.
pub mod kinds {
  pub const THREAD: &str = "thread";
  pub const TASK: &str = "task";
  pub const RUN: &str = "run";
  pub const REPORT: &str = "report";
  pub const REVIEW: &str = "review";
  pub const RESOLUTION: &str = "resolution";
  pub const VERIFICATION: &str = "verification";
  pub const QUESTION: &str = "question";
  pub const ANSWER: &str = "answer";
  pub const HANDOFF: &str = "handoff";
  pub const ASSIGNMENT: &str = "assignment";
  pub const CLAIM: &str = "claim";
  pub const CLOSE: &str = "close";
  pub const SET: &str = "set";
  pub const CAPABILITIES: &str = "capabilities";
  pub const PEER: &str = "peer";
  pub const REVOKE: &str = "revoke";
  pub const PROJECT: &str = "project";
  pub const FOLDER: &str = "folder";
  pub const MEMORY: &str = "memory";
  pub const TOOLSET: &str = "toolset";
  pub const PLACE: &str = "place";
  pub const CALL: &str = "call";
  pub const INSTRUCTIONS: &str = "instructions";
  pub const SCHEDULE: &str = "schedule";
  /// The user deleted threads (`deletes`): a tombstone, kept forever.
  pub const DELETION: &str = "deletion";
  /// The user rewound a thread to a message (`thread`, `segment`, `upto`).
  pub const REWIND: &str = "rewind";
}

pub(crate) fn validate_name(s: &str) -> Result<(), Error> {
  if is_name(s) {
    Ok(())
  } else {
    Err(Error::InvalidName(s.to_owned()))
  }
}

/// A reference: `[repo:]id[#sub]`. `repo` is a hint for a target in another
/// repository; `sub` names an item inside the target (`F1`, `m12`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref {
  pub repo: Option<RepoId>,
  pub id: RecordId,
  pub sub: Option<String>,
}

impl Ref {
  pub fn to(id: RecordId) -> Self {
    Ref {
      repo: None,
      id,
      sub: None,
    }
  }

  pub fn item(id: RecordId, sub: &str) -> Result<Self, Error> {
    Ref::to(id).with_sub(sub)
  }

  fn with_sub(mut self, sub: &str) -> Result<Self, Error> {
    if sub.is_empty()
      || sub.len() > 32
      || !sub.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
      return Err(Error::InvalidRef(format!("{}#{sub}", self.id)));
    }
    self.sub = Some(sub.to_owned());
    Ok(self)
  }
}

impl FromStr for Ref {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    let bad = || Error::InvalidRef(s.to_owned());
    let (repo, rest) = match s.split_once(':') {
      Some((repo, rest)) => (Some(repo.parse().map_err(|_| bad())?), rest),
      None => (None, s),
    };
    let (id, sub) = match rest.split_once('#') {
      Some((id, sub)) => (id, Some(sub)),
      None => (rest, None),
    };
    let r = Ref {
      repo,
      id: id.parse().map_err(|_| bad())?,
      sub: None,
    };
    match sub {
      Some(sub) => r.with_sub(sub).map_err(|_| bad()),
      None => Ok(r),
    }
  }
}

impl fmt::Display for Ref {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(repo) = &self.repo {
      write!(f, "{repo}:")?;
    }
    write!(f, "{}", self.id)?;
    if let Some(sub) = &self.sub {
      write!(f, "#{sub}")?;
    }
    Ok(())
  }
}

/// A blob's content hash: `blake3:<64 hex digits>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlobHash(pub blake3::Hash);

impl BlobHash {
  pub fn of(bytes: &[u8]) -> Self {
    BlobHash(blake3::hash(bytes))
  }

  pub fn hex(&self) -> String {
    self.0.to_hex().to_string()
  }
}

impl FromStr for BlobHash {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self, Error> {
    let bad = || Error::InvalidBlobHash(s.to_owned());
    let hex = s.strip_prefix("blake3:").ok_or_else(bad)?;
    if hex.len() != 64 || hex.bytes().any(|b| b.is_ascii_uppercase()) {
      return Err(bad());
    }
    blake3::Hash::from_hex(hex).map(BlobHash).map_err(|_| bad())
  }
}

impl fmt::Display for BlobHash {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "blake3:{}", self.0.to_hex())
  }
}

string_serde!(NodeName, RepoId, RecordId, Kind, Ref, BlobHash);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn generated_ids_parse_back_with_their_parts() {
    let at: Timestamp = "2026-10-11T14:32:07.412Z".parse().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let id = RecordId::generate(at, &node, &mut rand::rng());
    assert!(id.as_str().starts_with("2026-10-11T143207Z-mbp-"));
    let back: RecordId = id.as_str().parse().unwrap();
    assert_eq!(back, id);
    assert_eq!(id.node(), "mbp");
    assert_eq!(id.month(), "2026-10");
    assert_eq!(
      id.timestamp(),
      "2026-10-11T14:32:07Z".parse::<Timestamp>().unwrap()
    );
  }

  #[test]
  fn malformed_ids_are_refused() {
    for s in [
      "",
      "2026-10-11T143207Z-mbp-k3f",
      "2026-10-11T143207Z-mbp-k3fl",
      "2026-10-11T143207Z--k3f9",
      "2026-10-11T14:32:07Z-mbp-k3f9",
      "2026-13-11T143207Z-mbp-k3f9",
      "2026-10-11T143207Z-MBP-k3f9",
      "2026-10-11T143207Z-mbp-k3f9/../x",
      "../2026-10-11T143207Z-mbp-k3f9",
    ] {
      assert!(s.parse::<RecordId>().is_err(), "{s}");
    }
  }

  #[test]
  fn references_round_trip() {
    for s in [
      "2026-10-11T143207Z-mbp-k3f9",
      "2026-10-11T143207Z-mbp-k3f9#F1",
      "stitch-home:2026-10-11T135840Z-phone-a2x8",
      "stitch-home:2026-10-11T135840Z-phone-a2x8#m12",
    ] {
      assert_eq!(s.parse::<Ref>().unwrap().to_string(), s);
    }
    for s in [
      "x#F1",
      "2026-10-11T143207Z-mbp-k3f9#",
      "Home:2026-10-11T143207Z-mbp-k3f9",
    ] {
      assert!(s.parse::<Ref>().is_err(), "{s}");
    }
  }

  #[test]
  fn blob_hashes_round_trip() {
    let h = BlobHash::of(b"hello");
    assert_eq!(h.to_string().parse::<BlobHash>().unwrap(), h);
    assert!(h.to_string().to_uppercase().parse::<BlobHash>().is_err());
    assert!("sha256:00".parse::<BlobHash>().is_err());
  }
}
