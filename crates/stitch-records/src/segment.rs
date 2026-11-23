//! Transcript segments: JSON Lines, a header then one event per line
//! (docs/records.md §5).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Attachment, BlobHash, Error, NodeName, RecordId, millis};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SegmentHeader {
  pub v: i64,
  pub segment: RecordId,
  pub thread: RecordId,
  pub created: Timestamp,
  pub node: NodeName,
  /// The previous segment of the thread; two segments with the same `prev`
  /// are a fork.
  pub prev: Option<RecordId>,
  /// An external worker's native stream, kept as a blob.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub raw: Option<RawStream>,
  /// Segments this one compacts.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub replaces: Vec<RecordId>,
  /// The user's `rewind` this segment continues from (design §2.15,
  /// "Rewinding a thread"): the first segment of the branch it starts.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rewind: Option<RecordId>,
  /// With `rewind`: the message of `prev` it follows; what `prev` holds
  /// after it (and its tool calls) is not on this branch. `None` with a
  /// `prev`: all of `prev`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub upto: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawStream {
  pub blob: BlobHash,
  pub format: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
  System,
  User,
  Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
  Text {
    text: String,
  },
  Attachment {
    att: Attachment,
  },
  /// What a thread's instructions and skills became, when they changed
  /// since it started (they are in its system prompt until then).
  Context {
    context: String,
  },
  /// Memories recalled for this message unasked (design §2.9), with their
  /// handles (hex), so a thread never gets the same one twice.
  Recalled {
    recalled: String,
    handles: Vec<String>,
    /// How the note was chosen, kept to recalibrate memory with: the
    /// cue's measures and each memory's distances (`stitch memory notes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    measures: Option<Value>,
  },
  /// The user said this message in a voice call: its reply is read aloud
  /// (the model is told so with the message). An older build reads it as
  /// `Other`, and leaves it out.
  Spoken {
    spoken: bool,
    /// Which note the model was told with it, so a thread's history is
    /// built again with the same bytes (its prompt cache holds) when the
    /// note changes: 0 (absent) the first (2026-11-12), 1 the one that
    /// also says the text is an automatic transcription (2026-11-13). A
    /// build that knows only the first reads any as 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    note: u32,
  },
  /// Threads and efforts the user attached to this message (design §2.27,
  /// docs/rooms.md §3): what they are, for clients to show as chips, and
  /// the briefing the model was given with the message, rendered once when
  /// it was written so the thread's history is built again with the same
  /// bytes (its prompt cache holds) however those threads move on. An
  /// older build reads it as `Other`, and leaves it out.
  Scope {
    scopes: Vec<ScopeItem>,
    briefing: String,
  },
  /// A room's messages relayed to one of its participants (design §2.27,
  /// docs/rooms.md §4): the room, the last room message relayed (the next
  /// relay starts after it), the participant's handle there, the roster it
  /// was told, whether it may pass, and which note it was told with them
  /// (versioned as `Spoken::note`, so its history is built again with the
  /// same bytes when the note's wording changes). An older build reads it
  /// as `Other`, and leaves it out.
  RoomTurn {
    room: RecordId,
    upto: String,
    slug: String,
    roster: String,
    can_pass: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    note: u32,
  },
  /// A part this version does not know, kept as written.
  Other(Value),
}

/// What a scope points at: an effort (a conversation with every thread
/// spawned under it, however deep) or one conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
  Effort,
  Thread,
}

impl ScopeKind {
  pub fn as_str(self) -> &'static str {
    match self {
      ScopeKind::Effort => "effort",
      ScopeKind::Thread => "thread",
    }
  }
}

/// One scope of a `Part::Scope`, as clients show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeItem {
  pub kind: ScopeKind,
  /// The conversation it opens: an effort's root, or the thread (the
  /// newest of its compaction chain).
  pub id: RecordId,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  /// How many threads it covers.
  #[serde(default, skip_serializing_if = "is_zero_usize")]
  pub threads: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Msg {
  /// Local to the thread; a record cites it as `<thread>#<id>`.
  pub id: String,
  pub at: Timestamp,
  pub role: Role,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub from: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  pub parts: Vec<Part>,
  /// The provider's own content for an assistant message
  /// (`{provider, model, content}`), sent back unchanged when the thread
  /// continues on that provider and model (docs/records.md §5.1).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub native: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
  pub id: String,
  pub at: Timestamp,
  pub name: String,
  pub input: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
  /// The call's ID.
  pub id: String,
  pub at: Timestamp,
  pub ok: bool,
  pub output: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
  pub at: Timestamp,
  pub model: String,
  pub input_tokens: u64,
  pub output_tokens: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_read_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cache_write_tokens: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
  Msg(Msg),
  ToolCall(ToolCall),
  ToolResult(ToolResult),
  Usage(Usage),
  /// An event this version does not know, kept as written.
  Other(Value),
}

impl Event {
  fn tag(&self) -> Option<&'static str> {
    Some(match self {
      Event::Msg(_) => "msg",
      Event::ToolCall(_) => "tool_call",
      Event::ToolResult(_) => "tool_result",
      Event::Usage(_) => "usage",
      Event::Other(_) => return None,
    })
  }

  fn from_value(mut v: Value) -> Result<Event, String> {
    let obj = v.as_object_mut().ok_or("an event is an object")?;
    let tag = match obj.get("t") {
      Some(Value::String(t)) => t.clone(),
      _ => return Err("an event has a string `t`".into()),
    };
    fn typed<T: serde::de::DeserializeOwned>(mut obj: Map<String, Value>) -> Result<T, String> {
      obj.remove("t");
      serde_json::from_value(Value::Object(obj)).map_err(|e| e.to_string())
    }
    let obj = std::mem::take(obj);
    Ok(match tag.as_str() {
      "msg" => Event::Msg(typed(obj)?),
      "tool_call" => Event::ToolCall(typed(obj)?),
      "tool_result" => Event::ToolResult(typed(obj)?),
      "usage" => Event::Usage(typed(obj)?),
      _ => Event::Other(Value::Object(obj)),
    })
  }

  pub fn to_value(&self) -> Value {
    let body = match self {
      Event::Msg(e) => serde_json::to_value(e),
      Event::ToolCall(e) => serde_json::to_value(e),
      Event::ToolResult(e) => serde_json::to_value(e),
      Event::Usage(e) => serde_json::to_value(e),
      Event::Other(v) => return v.clone(),
    }
    .expect("events serialize");
    let mut obj = Map::new();
    obj.insert("t".into(), Value::String(self.tag().expect("typed").into()));
    if let Value::Object(fields) = body {
      obj.extend(fields);
    }
    Value::Object(obj)
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
  pub header: SegmentHeader,
  pub events: Vec<Event>,
}

impl Segment {
  pub fn new(
    segment: RecordId,
    thread: RecordId,
    created: Timestamp,
    node: NodeName,
    prev: Option<RecordId>,
  ) -> Self {
    Segment {
      header: SegmentHeader {
        v: crate::FORMAT_VERSION,
        segment,
        thread,
        created: millis(created),
        node,
        prev,
        raw: None,
        replaces: Vec::new(),
        rewind: None,
        upto: None,
      },
      events: Vec::new(),
    }
  }

  pub fn parse(text: &str) -> Result<Segment, Error> {
    let err = |line: usize, message: String| Error::Segment { line, message };
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = body.split('\n').enumerate().map(|(i, l)| (i + 1, l));
    let (_, first) = lines.next().ok_or_else(|| err(1, "empty segment".into()))?;
    let header: SegmentHeader = serde_json::from_str(first).map_err(|e| err(1, e.to_string()))?;
    if header.v != crate::FORMAT_VERSION {
      return Err(Error::Version(header.v));
    }
    let mut events = Vec::new();
    for (n, line) in lines {
      let v: Value = serde_json::from_str(line).map_err(|e| err(n, e.to_string()))?;
      events.push(Event::from_value(v).map_err(|e| err(n, e))?);
    }
    Ok(Segment { header, events })
  }

  pub fn to_text(&self) -> String {
    let mut out = serde_json::to_string(&self.header).expect("a header serializes");
    out.push('\n');
    for e in &self.events {
      out.push_str(&serde_json::to_string(&e.to_value()).expect("an event serializes"));
      out.push('\n');
    }
    out
  }

  /// Every blob the segment references: the raw stream, attachments in
  /// messages, and `{"blob": "blake3:…"}` objects anywhere in events (large
  /// tool outputs).
  pub fn blob_refs(&self) -> Vec<BlobHash> {
    fn walk(v: &Value, out: &mut Vec<BlobHash>) {
      match v {
        Value::Object(o) => {
          if let Some(h) = o
            .get("blob")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
          {
            out.push(h);
          }
          if let Some(h) = o
            .get("att")
            .and_then(|a| a.get("hash"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
          {
            out.push(h);
          }
          o.values().for_each(|v| walk(v, out));
        }
        Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
        _ => {}
      }
    }
    let mut out = Vec::new();
    if let Some(raw) = &self.header.raw {
      out.push(raw.blob);
    }
    for e in &self.events {
      walk(&e.to_value(), &mut out);
    }
    out.sort_by_key(BlobHash::hex);
    out.dedup();
    out
  }

  pub fn usage(&self) -> impl Iterator<Item = &Usage> {
    self.events.iter().filter_map(|e| match e {
      Event::Usage(u) => Some(u),
      _ => None,
    })
  }
}

fn is_zero(n: &u32) -> bool {
  *n == 0
}

fn is_zero_usize(n: &usize) -> bool {
  *n == 0
}

#[cfg(test)]
mod tests {
  use super::*;

  const SAMPLE: &str = r##"{"v":1,"segment":"2026-10-11T141530Z-mbp-77ha","thread":"2026-10-11T140112Z-mbp-q7cd","created":"2026-10-11T14:15:30.020Z","node":"mbp","prev":null}
{"t":"msg","id":"m1","at":"2026-10-11T14:01:12.300Z","role":"user","from":"user@phone","parts":[{"text":"Draft records.md for the store."},{"att":{"hash":"blake3:0000000000000000000000000000000000000000000000000000000000000000","type":"image/heic","name":"IMG_4121.HEIC","bytes":2481133}}]}
{"t":"tool_call","id":"c1","at":"2026-10-11T14:01:21.004Z","name":"Read","input":{"file_path":"records.md"}}
{"t":"tool_result","id":"c1","at":"2026-10-11T14:01:21.090Z","ok":true,"output":{"text":"# Record format"}}
{"t":"usage","at":"2026-10-11T14:15:29.700Z","model":"claude-opus-5-5","input_tokens":48211,"output_tokens":6120,"cache_read_tokens":40102,"usd":0.41}
{"t":"from_the_future","x":1}
"##;

  #[test]
  fn the_documented_sample_parses_and_round_trips() {
    let s = Segment::parse(SAMPLE).unwrap();
    assert_eq!(s.header.prev, None);
    assert_eq!(s.events.len(), 5);
    let Event::Msg(m) = &s.events[0] else {
      panic!()
    };
    assert!(matches!(&m.parts[1], Part::Attachment { att } if att.bytes == 2481133));
    assert_eq!(s.usage().map(|u| u.usd.unwrap()).sum::<f64>(), 0.41);
    assert!(matches!(&s.events[4], Event::Other(_)));
    assert_eq!(s.blob_refs().len(), 1);
    let again = Segment::parse(&s.to_text()).unwrap();
    assert_eq!(again, s);
  }

  /// A scope part round-trips, reads as written, is not mistaken for any
  /// other part; a build that does not know it keeps it as `Other`.
  #[test]
  fn a_scope_part_round_trips_and_is_other_to_an_older_build() {
    let part = Part::Scope {
      scopes: vec![
        ScopeItem {
          kind: ScopeKind::Effort,
          id: "2026-11-13T100000Z-gamer-aaaa".parse().unwrap(),
          title: Some("Rooms".into()),
          threads: 3,
        },
        ScopeItem {
          kind: ScopeKind::Thread,
          id: "2026-11-13T100001Z-gamer-bbbb".parse().unwrap(),
          title: None,
          threads: 0,
        },
      ],
      briefing: "<scope>…</scope>".into(),
    };
    let text = serde_json::to_string(&part).unwrap();
    assert_eq!(
      text,
      r#"{"scopes":[{"kind":"effort","id":"2026-11-13T100000Z-gamer-aaaa","title":"Rooms","threads":3},{"kind":"thread","id":"2026-11-13T100001Z-gamer-bbbb"}],"briefing":"<scope>…</scope>"}"#
    );
    let back: Part = serde_json::from_str(&text).unwrap();
    assert_eq!(back, part);
    // An older build: the same part with no `Scope` variant to match.
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    #[allow(dead_code)]
    enum Older {
      Text {
        text: String,
      },
      Context {
        context: String,
      },
      Recalled {
        recalled: String,
        handles: Vec<String>,
      },
      Spoken {
        spoken: bool,
      },
      Other(Value),
    }
    let old: Older = serde_json::from_str(&text).unwrap();
    assert!(matches!(old, Older::Other(_)), "{old:?}");
    // In a message, among others, kept through a segment's round trip.
    let line = format!(
      r#"{{"t":"msg","id":"m1","at":"2026-11-13T10:00:00Z","role":"user","parts":[{{"text":"look at this"}},{text}]}}"#
    );
    let header = SAMPLE.lines().next().unwrap();
    let s = Segment::parse(&format!("{header}\n{line}\n")).unwrap();
    let Event::Msg(m) = &s.events[0] else {
      panic!()
    };
    assert_eq!(m.parts[1], part);
    assert_eq!(Segment::parse(&s.to_text()).unwrap(), s);
  }

  /// A room turn round-trips, is not mistaken for another part, and an
  /// older build (with `Scope` but no `RoomTurn`) keeps it as `Other`.
  #[test]
  fn a_room_turn_round_trips_and_is_other_to_an_older_build() {
    let part = Part::RoomTurn {
      room: "2026-11-13T100000Z-gamer-aaaa".parse().unwrap(),
      upto: "m4".into(),
      slug: "opus".into(),
      roster: "opus (Opus), gemini (Gemini)".into(),
      can_pass: true,
      note: 0,
    };
    let text = serde_json::to_string(&part).unwrap();
    assert_eq!(
      text,
      r#"{"room":"2026-11-13T100000Z-gamer-aaaa","upto":"m4","slug":"opus","roster":"opus (Opus), gemini (Gemini)","can_pass":true}"#
    );
    let back: Part = serde_json::from_str(&text).unwrap();
    assert_eq!(back, part);
    let with_note: Part = serde_json::from_str(&text.replace("true}", "true,\"note\":3}")).unwrap();
    assert!(matches!(with_note, Part::RoomTurn { note: 3, .. }));
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    #[allow(dead_code)]
    enum Older {
      Text {
        text: String,
      },
      Spoken {
        spoken: bool,
      },
      Scope {
        scopes: Vec<ScopeItem>,
        briefing: String,
      },
      Other(Value),
    }
    let old: Older = serde_json::from_str(&text).unwrap();
    assert!(matches!(old, Older::Other(_)), "{old:?}");
  }

  #[test]
  fn broken_lines_are_reported_with_their_number() {
    let text = SAMPLE.replace(r#"{"t":"tool_call""#, r#"{"t":"tool_call"x"#);
    assert!(matches!(
      Segment::parse(&text),
      Err(Error::Segment { line: 3, .. })
    ));
    let text = SAMPLE.replace("\n{\"t\":\"usage\"", "\n\n{\"t\":\"usage\"");
    assert!(matches!(
      Segment::parse(&text),
      Err(Error::Segment { line: 5, .. })
    ));
  }
}
