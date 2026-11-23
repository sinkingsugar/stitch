//! The user's own instructions (design §2.3): for every thread (`global`)
//! and for a place's (a project's, a folder's), told to a thread before
//! the instruction files of its checkout or folder. They are records, so
//! every node has them, the phone included, and any node edits them: an
//! edit is a new record (`place` = the place's key, the text as its
//! body), and the latest counts. They live where the place's threads do:
//! a project's in its own records, the global ones and a folder's in the
//! home workspace.

use stitch_net::NodeState;
use stitch_records::{Author, Draft, RecordId, kinds};
use stitch_store::RecordRoot;

use crate::place::Place;
use crate::{Error, Result};

/// Each one at most, in bytes, as an instruction file.
const MAX: usize = 64 * 1024;

/// The root a place's instructions are written in.
fn root_of(state: &NodeState, place: &Place) -> Option<RecordRoot> {
  match place {
    Place::Project(repo) => state.root(repo),
    Place::Global | Place::Folder(_) => state.home(),
  }
}

/// A place's instructions, as last written; `None` when never written, or
/// emptied.
pub fn current(state: &NodeState, place: &Place) -> Option<String> {
  let root = root_of(state, place)?;
  let key = place.key();
  let latest = state
    .index
    .lock()
    .expect("index lock")
    .records(Some(kinds::INSTRUCTIONS))
    .ok()?
    .into_iter()
    .filter(|r| &r.repo == root.repo())
    .rfind(|r| r.fields.get("place").and_then(|v| v.as_str()) == Some(key.as_str()))?;
  let text = root.read_record(&latest.id).ok()?.body;
  (!text.trim().is_empty()).then_some(text)
}

/// Writes a place's instructions (empty: none); the record's ID.
pub fn set(state: &NodeState, place: &Place, text: &str) -> Result<RecordId> {
  if text.len() > MAX {
    return Err(Error::Config(format!(
      "instructions are {} bytes; {MAX} at most",
      text.len()
    )));
  }
  let root = root_of(state, place).ok_or_else(|| {
    Error::Config(format!(
      "{} is not held here: no instructions can be written for it",
      place.key()
    ))
  })?;
  let draft = Draft::new(kinds::INSTRUCTIONS, Author::user(state.node.clone()))?
    .field("place", place.key().as_str())?
    .body(text.to_owned());
  let id = root.publish(&state.node, draft)?.id;
  state.index.lock().expect("index lock").refresh(&root)?;
  Ok(id)
}

/// A setting of the user's kept as instructions are, under a key of its
/// own that is no place's (`loose-ends`, design §2.10): the latest record
/// written for it, in the home workspace, so it syncs and any node edits
/// it. Its fields and its text; `None` when never written.
pub(crate) fn keyed(state: &NodeState, key: &str) -> Option<(stitch_records::Record, String)> {
  let root = state.home()?;
  let latest = state
    .index
    .lock()
    .expect("index lock")
    .records(Some(kinds::INSTRUCTIONS))
    .ok()?
    .into_iter()
    .filter(|r| &r.repo == root.repo())
    .rfind(|r| r.fields.get("place").and_then(|v| v.as_str()) == Some(key))?;
  let record = root.read_record(&latest.id).ok()?;
  let text = record.body.clone();
  Some((record, text))
}

/// Writes a keyed setting (`keyed`): its fields and its text, the user's.
pub(crate) fn set_keyed(
  state: &NodeState,
  key: &str,
  fields: &[(&str, &str)],
  text: &str,
) -> Result<RecordId> {
  if text.len() > MAX {
    return Err(Error::Config(format!(
      "the text is {} bytes; {MAX} at most",
      text.len()
    )));
  }
  let root = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here to keep it in".into()))?;
  let mut draft =
    Draft::new(kinds::INSTRUCTIONS, Author::user(state.node.clone()))?.field("place", key)?;
  for (k, v) in fields {
    draft = draft.field(k, *v)?;
  }
  let id = root.publish(&state.node, draft.body(text.to_owned()))?.id;
  state.index.lock().expect("index lock").refresh(&root)?;
  Ok(id)
}

/// The instructions a thread in `place` is told, as (label, text): the
/// global ones, then the place's.
pub fn for_place(state: &NodeState, place: &Place, label: &str) -> Vec<(String, String)> {
  let mut out = Vec::new();
  if let Some(text) = current(state, &Place::Global) {
    out.push(("Yours, for every thread".to_owned(), text));
  }
  if *place != Place::Global
    && let Some(text) = current(state, place)
  {
    out.push((format!("Yours, for {label}"), text));
  }
  out
}
