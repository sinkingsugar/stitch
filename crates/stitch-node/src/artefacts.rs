//! `Request::Artefacts` and `Request::Touched` (design §2.25, step 2): what a
//! conversation touched, and what touched a path, URL, commit or thread,
//! read from the index's `artefacts` table (`stitch_store::artefacts`).
//! Both give JSON, one line, as the transcript does (payload types in
//! `apps/client/src/payload.ts`).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use stitch_records::RecordId;
use stitch_store::Touch;
use stitch_store::artefacts::Kind;

use crate::node::Shared;
use crate::{Error, Result};

const KINDS: [Kind; 4] = [Kind::Path, Kind::Url, Kind::Commit, Kind::Thread];

/// A message's durable name: `<thread>#<local id>`.
fn durable(thread: &RecordId, msg: &Option<String>) -> Value {
  msg
    .as_ref()
    .map_or(Value::Null, |m| json!(format!("{thread}#{m}")))
}

/// What one or more threads' rows say of one artefact and verb, together:
/// counts add up, the first touch is the earliest, the last the latest.
struct Agg {
  verb: String,
  value: String,
  first_at: jiff::Timestamp,
  last_at: jiff::Timestamp,
  n: u32,
  first_msg: Value,
  last_msg: Value,
  last_thread: RecordId,
  subject: Option<String>,
}

impl Agg {
  fn new(t: &Touch) -> Agg {
    Agg {
      verb: t.verb.clone(),
      value: t.value.clone(),
      first_at: t.first_at,
      last_at: t.last_at,
      n: t.n,
      first_msg: durable(&t.thread, &t.first_msg),
      last_msg: durable(&t.thread, &t.last_msg),
      last_thread: t.thread.clone(),
      subject: t.subject.clone(),
    }
  }

  fn add(&mut self, t: &Touch) {
    self.n += t.n;
    if t.first_at < self.first_at {
      self.first_at = t.first_at;
      self.first_msg = durable(&t.thread, &t.first_msg);
    }
    if t.last_at >= self.last_at {
      self.last_at = t.last_at;
      self.last_msg = durable(&t.thread, &t.last_msg);
      self.last_thread = t.thread.clone();
      if t.subject.is_some() {
        self.subject = t.subject.clone();
      }
    }
  }
}

/// The value as the index keeps it: a URL without its fragment, a path or
/// anything else trimmed.
fn normal(kind: Kind, value: &str) -> Result<String> {
  let v = value.trim();
  match kind {
    Kind::Url => stitch_store::artefacts::clean_url(v)
      .ok_or_else(|| Error::Config(format!("`{value}` is not an http(s) URL"))),
    _ if v.is_empty() => Err(Error::Config("an empty value".into())),
    _ => Ok(v.to_owned()),
  }
}

/// A conversation's artefacts: those of every thread of its compaction
/// chain, one item per `(kind, value, verb)`, grouped by kind, the latest
/// first. `{thread, chain, kinds: {path: […], url: […], commit: […], thread: […]}}`;
/// an item `{value, verb, first_at, last_at, n, msg, first_msg, subject,
/// thread}`: `msg` the message of its latest touch (`<thread>#<id>`, on
/// `thread`, the chain's thread that touched it last), `first_msg` its
/// first's.
pub(crate) fn of_conversation(s: &Arc<Shared>, thread: &str) -> Result<Value> {
  let thread: RecordId = thread.parse()?;
  let (chain, rows) = {
    let index = s.state().index.lock().expect("index lock");
    let chain = index.compaction_lineage(&thread);
    let rows = index.artefacts(&chain, None)?;
    (chain, rows)
  };
  let mut groups = serde_json::Map::new();
  for kind in KINDS {
    // Over the chain's threads: counts add up, the latest touch wins.
    let mut merged: BTreeMap<(String, String), Agg> = BTreeMap::new();
    for t in rows.iter().filter(|t| t.kind == kind.as_str()) {
      match merged.get_mut(&(t.value.clone(), t.verb.clone())) {
        Some(a) => a.add(t),
        None => {
          merged.insert((t.value.clone(), t.verb.clone()), Agg::new(t));
        }
      }
    }
    let mut items: Vec<&Agg> = merged.values().collect();
    items.sort_by(|a, b| {
      b.last_at
        .cmp(&a.last_at)
        .then_with(|| a.value.cmp(&b.value))
    });
    groups.insert(
      kind.as_str().into(),
      Value::Array(
        items
          .into_iter()
          .map(|t| {
            json!({
              "value": t.value,
              "verb": t.verb,
              "first_at": t.first_at.to_string(),
              "last_at": t.last_at.to_string(),
              "n": t.n,
              "msg": t.last_msg,
              "first_msg": t.first_msg,
              "thread": t.last_thread.to_string(),
              "subject": t.subject,
            })
          })
          .collect(),
      ),
    );
  }
  Ok(json!({
    "thread": chain.last().unwrap_or(&thread).to_string(),
    "chain": chain.iter().map(ToString::to_string).collect::<Vec<_>>(),
    "kinds": groups,
  }))
}

/// Every conversation that touched `value` (a chain folded to its newest
/// thread, deleted ones left out), in time order of their first touch:
/// `{kind, value, conversations: [{thread, title, place, verb, first_at,
/// last_at, n, msg, first_msg, subject}]}`, one entry per conversation and
/// verb; `msg` is `<thread>#<id>` of the latest touch (an earlier thread of
/// the chain when it was there).
pub(crate) fn touched(s: &Arc<Shared>, kind: &str, value: &str) -> Result<Value> {
  let kind = Kind::parse(kind)
    .ok_or_else(|| Error::Config(format!("kind `{kind}`: path, url, commit or thread")))?;
  let value = normal(kind, value)?;
  let rows = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .artefacts(&[], Some((kind, &value)))?;
  // Fold chains: by the newest thread, per verb.
  let mut convs: BTreeMap<(RecordId, String), Agg> = BTreeMap::new();
  {
    let index = s.state().index.lock().expect("index lock");
    for t in &rows {
      let newest = index.compaction_newest(&t.thread);
      if index.is_deleted(&t.thread).unwrap_or(false) || index.is_deleted(&newest).unwrap_or(false)
      {
        continue;
      }
      match convs.get_mut(&(newest.clone(), t.verb.clone())) {
        Some(a) => a.add(t),
        None => {
          convs.insert((newest, t.verb.clone()), Agg::new(t));
        }
      }
    }
  }
  let mut convs: Vec<(RecordId, Agg)> = convs.into_iter().map(|((n, _), a)| (n, a)).collect();
  convs.sort_by(|a, b| a.1.first_at.cmp(&b.1.first_at).then_with(|| a.0.cmp(&b.0)));
  let here = s.state().node.to_string();
  let known = s.places();
  let conversations: Vec<Value> = convs
    .iter()
    .map(|(newest, c)| {
      let title = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .lineage_title(newest);
      // Outside the index lock: `thread_place` takes it.
      let place = s
        .thread_place(newest)
        .ok()
        .map_or_else(|| "?".to_owned(), |(_, p)| p.label(&here, &known));
      json!({
        "thread": newest.to_string(),
        "title": title,
        "place": place,
        "verb": c.verb,
        "first_at": c.first_at.to_string(),
        "last_at": c.last_at.to_string(),
        "n": c.n,
        "msg": c.last_msg,
        "first_msg": c.first_msg,
        "subject": c.subject,
      })
    })
    .collect();
  Ok(json!({
    "kind": kind.as_str(),
    "value": value,
    "conversations": conversations,
  }))
}
