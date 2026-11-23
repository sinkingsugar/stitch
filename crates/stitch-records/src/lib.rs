//! Stitch's record format (docs/records.md): record IDs and references,
//! records (Markdown with TOML front matter) and transcript segments
//! (JSON Lines). Pure data: no file system, no clock beyond what callers pass.

mod error;
mod id;
mod record;
mod segment;

pub use error::Error;
pub use id::{BlobHash, Kind, NodeName, RecordId, Ref, RepoId, kinds};
pub use record::{Attachment, Author, Draft, FORMAT_VERSION, Record, Refs};
pub use segment::{
  Event, Msg, Part, RawStream, Role, ScopeItem, ScopeKind, Segment, SegmentHeader, ToolCall,
  ToolResult, Usage,
};

/// Truncates a timestamp to whole milliseconds, the precision records store.
pub fn millis(at: jiff::Timestamp) -> jiff::Timestamp {
  jiff::Timestamp::from_millisecond(at.as_millisecond()).expect("in range: came from a timestamp")
}

/// Implements `Serialize` and `Deserialize` through `Display` and `FromStr`.
macro_rules! string_serde {
    ($($t:ty),*) => {$(
        impl serde::Serialize for $t {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> serde::Deserialize<'de> for $t {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    )*};
}
pub(crate) use string_serde;
