//! Records: Markdown with TOML front matter (docs/records.md §1).

use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use toml::Table;
use toml::value::{Datetime, Value};

use crate::id::validate_name;
use crate::{BlobHash, Error, Kind, NodeName, RecordId, Ref, millis};

pub const FORMAT_VERSION: i64 = 1;

/// Keys the record itself owns; everything else in the front matter is a field.
const RESERVED: [&str; 7] = [
  "v",
  "id",
  "kind",
  "created",
  "author",
  "attachments",
  "refs",
];

/// Who wrote a record: a node, and on it the user or a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
  pub node: NodeName,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub user: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub run: Option<RecordId>,
}

impl Author {
  pub fn user(node: NodeName) -> Self {
    Author {
      node,
      user: true,
      run: None,
    }
  }

  pub fn node(node: NodeName) -> Self {
    Author {
      node,
      user: false,
      run: None,
    }
  }

  pub fn run(node: NodeName, run: RecordId) -> Self {
    Author {
      node,
      user: false,
      run: Some(run),
    }
  }
}

/// A reference to a blob, with what is needed to show it without fetching it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
  pub hash: BlobHash,
  #[serde(rename = "type")]
  pub media_type: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  pub bytes: u64,
}

/// A record's typed, directed edges: relation name to targets, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refs(Vec<(String, Vec<Ref>)>);

impl Refs {
  pub fn add(&mut self, rel: &str, target: Ref) -> Result<(), Error> {
    validate_name(rel)?;
    match self.0.iter_mut().find(|(r, _)| r == rel) {
      Some((_, targets)) => targets.push(target),
      None => self.0.push((rel.to_owned(), vec![target])),
    }
    Ok(())
  }

  pub fn get(&self, rel: &str) -> &[Ref] {
    self
      .0
      .iter()
      .find(|(r, _)| r == rel)
      .map_or(&[], |(_, t)| t.as_slice())
  }

  pub fn first(&self, rel: &str) -> Option<&Ref> {
    self.get(rel).first()
  }

  /// Every edge, as (relation, target).
  pub fn iter(&self) -> impl Iterator<Item = (&str, &Ref)> {
    self
      .0
      .iter()
      .flat_map(|(rel, targets)| targets.iter().map(move |t| (rel.as_str(), t)))
  }

  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  fn from_toml(value: Value) -> Result<Self, Error> {
    let Value::Table(table) = value else {
      return Err(Error::field("refs", "expected a table"));
    };
    let mut refs = Refs::default();
    for (rel, v) in table {
      let parse = |v: &Value| -> Result<Ref, Error> {
        v.as_str()
          .ok_or_else(|| Error::field(&format!("refs.{rel}"), "expected a string"))?
          .parse()
      };
      match &v {
        Value::Array(items) => {
          for item in items {
            refs.add(&rel, parse(item)?)?;
          }
        }
        v => refs.add(&rel, parse(v)?)?,
      }
    }
    Ok(refs)
  }

  fn to_toml(&self) -> Table {
    self
      .0
      .iter()
      .map(|(rel, targets)| {
        let v = match targets.as_slice() {
          [one] => Value::String(one.to_string()),
          many => Value::Array(many.iter().map(|t| Value::String(t.to_string())).collect()),
        };
        (rel.clone(), v)
      })
      .collect()
  }
}

/// A record, parsed. `fields` holds every front matter key the record does
/// not own, kind-specific ones and unknown ones alike, in file order.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
  pub v: i64,
  pub id: RecordId,
  pub kind: Kind,
  pub created: Timestamp,
  pub author: Author,
  pub fields: Table,
  pub attachments: Vec<Attachment>,
  pub refs: Refs,
  pub body: String,
}

fn timestamp_from_toml(field: &str, v: &Value) -> Result<Timestamp, Error> {
  let Value::Datetime(dt) = v else {
    return Err(Error::field(field, "expected a date-time"));
  };
  if dt.offset.is_none() || dt.date.is_none() || dt.time.is_none() {
    return Err(Error::field(field, "expected a date-time with an offset"));
  }
  dt.to_string().parse().map_err(|e| Error::field(field, e))
}

/// A key as written: bare when it can be, else a quoted basic string.
fn toml_key(k: &str) -> String {
  if !k.is_empty()
    && k
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
  {
    k.to_owned()
  } else {
    serde_json::to_string(k).expect("strings serialize")
  }
}

pub(crate) fn timestamp_to_toml(at: Timestamp) -> Value {
  Value::Datetime(Datetime::from_str(&at.to_string()).expect("jiff writes RFC 3339"))
}

impl Record {
  /// Parses a record file.
  pub fn parse(text: &str) -> Result<Record, Error> {
    let rest = text.strip_prefix("+++\n").ok_or(Error::NoFrontMatter)?;
    // The front matter ends at the first line that is exactly `+++`.
    let mut start = 0;
    let (front, body) = loop {
      let end = rest[start..].find('\n').ok_or(Error::NoFrontMatter)? + start;
      if &rest[start..end] == "+++" {
        break (&rest[..start], &rest[end + 1..]);
      }
      start = end + 1;
    };
    let body = body.strip_prefix('\n').unwrap_or(body);
    let mut table: Table = toml::from_str(front)?;

    let mut take = |key: &'static str| table.remove(key).ok_or(Error::Missing(key));
    let v = take("v")?
      .as_integer()
      .ok_or_else(|| Error::field("v", "expected an integer"))?;
    if v != FORMAT_VERSION {
      return Err(Error::Version(v));
    }
    let string = |key: &str, v: Value| -> Result<String, Error> {
      match v {
        Value::String(s) => Ok(s),
        _ => Err(Error::field(key, "expected a string")),
      }
    };
    let id = string("id", take("id")?)?.parse()?;
    let kind = string("kind", take("kind")?)?.parse()?;
    let created = timestamp_from_toml("created", &take("created")?)?;
    let author = take("author")?
      .try_into()
      .map_err(|e| Error::field("author", e))?;
    let attachments = match table.remove("attachments") {
      Some(v) => v.try_into().map_err(|e| Error::field("attachments", e))?,
      None => Vec::new(),
    };
    let refs = match table.remove("refs") {
      Some(v) => Refs::from_toml(v)?,
      None => Refs::default(),
    };
    Ok(Record {
      v,
      id,
      kind,
      created,
      author,
      fields: table,
      attachments,
      refs,
      body: body.to_owned(),
    })
  }

  /// The file's text.
  /// The file's text: one `key = value` line per field, tables inline,
  /// then arrays of tables (`[[attachments]]`) and `[refs]` as sections.
  pub fn to_text(&self) -> String {
    let mut out = String::from("+++\n");
    let mut line = |k: &str, v: &Value| out.push_str(&format!("{} = {v}\n", toml_key(k)));
    line("v", &Value::Integer(self.v));
    line("id", &Value::String(self.id.to_string()));
    line("kind", &Value::String(self.kind.to_string()));
    line("created", &timestamp_to_toml(self.created));
    line(
      "author",
      &Value::try_from(&self.author).expect("serializable"),
    );
    let mut sections = Table::new();
    for (k, v) in &self.fields {
      match v {
        Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_table) => {
          sections.insert(k.clone(), v.clone());
        }
        v => line(k, v),
      }
    }
    if !self.attachments.is_empty() {
      sections.insert(
        "attachments".into(),
        Value::try_from(&self.attachments).expect("serializable"),
      );
    }
    if !sections.is_empty() {
      out.push('\n');
      out.push_str(&toml::to_string(&sections).expect("a table serializes"));
    }
    if !self.refs.is_empty() {
      out.push_str("\n[refs]\n");
      for (rel, v) in self.refs.to_toml() {
        out.push_str(&format!("{} = {v}\n", toml_key(&rel)));
      }
    }
    out.push_str("+++\n");
    if !self.body.is_empty() {
      out.push('\n');
      out.push_str(&self.body);
      if !self.body.ends_with('\n') {
        out.push('\n');
      }
    }
    out
  }

  /// The body's first `# ` heading.
  pub fn title(&self) -> Option<&str> {
    self
      .body
      .lines()
      .find_map(|l| l.strip_prefix("# "))
      .map(str::trim)
  }

  pub fn field_str(&self, key: &str) -> Option<&str> {
    self.fields.get(key).and_then(Value::as_str)
  }

  pub fn field_bool(&self, key: &str) -> Option<bool> {
    self.fields.get(key).and_then(Value::as_bool)
  }

  pub fn field_time(&self, key: &str) -> Option<Timestamp> {
    self
      .fields
      .get(key)
      .and_then(|v| timestamp_from_toml(key, v).ok())
  }
}

/// A record before it has an ID and a creation time.
#[derive(Clone, Debug)]
pub struct Draft {
  pub kind: Kind,
  pub author: Author,
  fields: Table,
  pub attachments: Vec<Attachment>,
  pub refs: Refs,
  pub body: String,
}

impl Draft {
  pub fn new(kind: &str, author: Author) -> Result<Self, Error> {
    Ok(Draft {
      kind: kind.parse()?,
      author,
      fields: Table::new(),
      attachments: Vec::new(),
      refs: Refs::default(),
      body: String::new(),
    })
  }

  /// Sets a kind-specific field. The record's own keys are refused.
  pub fn field(mut self, key: &str, value: impl Into<Value>) -> Result<Self, Error> {
    self.set_field(key, value.into())?;
    Ok(self)
  }

  pub fn set_field(&mut self, key: &str, value: Value) -> Result<(), Error> {
    if RESERVED.contains(&key) {
      return Err(Error::Reserved(key.to_owned()));
    }
    self.fields.insert(key.to_owned(), value);
    Ok(())
  }

  /// Sets a date-time field.
  pub fn time_field(self, key: &str, at: Timestamp) -> Result<Self, Error> {
    self.field(key, timestamp_to_toml(millis(at)))
  }

  pub fn reference(mut self, rel: &str, target: Ref) -> Result<Self, Error> {
    self.refs.add(rel, target)?;
    Ok(self)
  }

  pub fn attach(mut self, attachment: Attachment) -> Self {
    self.attachments.push(attachment);
    self
  }

  pub fn body(mut self, body: impl Into<String>) -> Self {
    self.body = body.into();
    self
  }

  pub fn into_record(self, id: RecordId, created: Timestamp) -> Record {
    Record {
      v: FORMAT_VERSION,
      id,
      kind: self.kind,
      created: millis(created),
      author: self.author,
      fields: self.fields,
      attachments: self.attachments,
      refs: self.refs,
      body: self.body,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const QUESTION: &str = r#"+++
v = 1
id = "2026-10-11T143207Z-mbp-k3f9"
kind = "question"
created = 2026-10-11T14:32:07.412Z
author = { node = "mbp", run = "2026-10-11T140112Z-mbp-q7cd" }
choices = ["Keep TOML", "Switch to YAML"]

[refs]
task = "2026-10-11T135840Z-phone-a2x8"
+++

# Front matter: TOML or YAML?

Which one should records use?
"#;

  #[test]
  fn the_documented_sample_parses() {
    let r = Record::parse(QUESTION).unwrap();
    assert_eq!(r.id.as_str(), "2026-10-11T143207Z-mbp-k3f9");
    assert_eq!(r.kind, "question");
    assert_eq!(r.created.as_millisecond() % 1000, 412);
    assert_eq!(r.author.node.as_str(), "mbp");
    assert_eq!(
      r.author.run.as_ref().unwrap().as_str(),
      "2026-10-11T140112Z-mbp-q7cd"
    );
    assert_eq!(
      r.refs.first("task").unwrap().id.as_str(),
      "2026-10-11T135840Z-phone-a2x8"
    );
    assert_eq!(r.title(), Some("Front matter: TOML or YAML?"));
    assert!(r.fields.contains_key("choices"));
    assert!(r.body.starts_with("# Front matter"));
  }

  #[test]
  fn the_documented_sample_is_written_back_byte_for_byte() {
    assert_eq!(Record::parse(QUESTION).unwrap().to_text(), QUESTION);
  }

  #[test]
  fn odd_keys_and_arrays_of_tables_round_trip() {
    let text = QUESTION.replace(
      "choices =",
      "\"odd key\" = 1\nsteps = [{ a = 1 }, { a = 2 }]\nnested = { x = { y = [1, 2] } }\nchoices =",
    );
    let r = Record::parse(&text).unwrap();
    let written = r.to_text();
    assert!(written.contains("[[steps]]"), "{written}");
    assert!(
      written.contains("nested = { x = { y = [1, 2] } }"),
      "{written}"
    );
    assert_eq!(Record::parse(&written).unwrap(), r);
  }

  #[test]
  fn written_records_parse_back_equal() {
    let r = Record::parse(QUESTION).unwrap();
    let text = r.to_text();
    assert_eq!(Record::parse(&text).unwrap(), r);
    // Writing is stable.
    assert_eq!(Record::parse(&text).unwrap().to_text(), text);
  }

  #[test]
  fn records_without_body_or_refs_round_trip() {
    let node: NodeName = "nas".parse().unwrap();
    let id = RecordId::generate(Timestamp::now(), &node, &mut rand::rng());
    let r = Draft::new("claim", Author::node(node))
      .unwrap()
      .time_field("lease_until", "2026-10-11T16:00:55Z".parse().unwrap())
      .unwrap()
      .into_record(id, Timestamp::now());
    let text = r.to_text();
    assert!(text.ends_with("+++\n"));
    let back = Record::parse(&text).unwrap();
    assert_eq!(back, r);
    assert_eq!(
      back.field_time("lease_until"),
      Some("2026-10-11T16:00:55Z".parse().unwrap())
    );
  }

  #[test]
  fn invalid_records_are_refused() {
    assert!(matches!(
      Record::parse("no front matter"),
      Err(Error::NoFrontMatter)
    ));
    assert!(matches!(
      Record::parse(&QUESTION.replace("v = 1", "v = 2")),
      Err(Error::Version(2))
    ));
    assert!(matches!(
      Record::parse(&QUESTION.replace("id = ", "ident = ")),
      Err(Error::Missing("id"))
    ));
    assert!(Record::parse(&QUESTION.replace("14:32:07.412Z", "14:32:07.412")).is_err());
    assert!(Record::parse(&QUESTION.replace("phone-a2x8", "phone-a2x")).is_err());
  }

  #[test]
  fn unknown_fields_are_kept() {
    let text = QUESTION.replace("choices =", "from_the_future = 3\nchoices =");
    let r = Record::parse(&text).unwrap();
    assert_eq!(
      r.fields.get("from_the_future").and_then(Value::as_integer),
      Some(3)
    );
    assert!(
      Record::parse(&r.to_text())
        .unwrap()
        .fields
        .contains_key("from_the_future")
    );
  }

  #[test]
  fn reserved_keys_are_not_fields() {
    let node: NodeName = "mbp".parse().unwrap();
    let d = Draft::new("task", Author::user(node)).unwrap();
    assert!(matches!(d.field("refs", 1), Err(Error::Reserved(_))));
  }
}
