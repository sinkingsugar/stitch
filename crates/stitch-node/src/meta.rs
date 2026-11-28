//! What the user keeps about threads (design §2.15): titles (generated
//! after a new chat's first exchange, or given), pins, and quoting a
//! thread into a new one. Titles and pins are `set` records
//! (docs/records.md §6): the latest in the supersedes chain wins.

use std::sync::Arc;

use stitch_providers::{Content, Message, Request};
use stitch_records::{Author, Draft, RecordId, Ref, kinds};
use stitch_store::RecordRoot;

use crate::chat::ChatContext;
use crate::node::Shared;
use crate::{Error, Result};

/// Writes a `set` of `field` on `target`, superseding every value standing
/// (a conflict from two nodes ends with it), or the target itself.
pub(crate) fn set(
  s: &Shared,
  target: &RecordId,
  field: &str,
  value: toml::Value,
  author: Author,
) -> Result<()> {
  let (root, _) = s.thread_place(target)?;
  set_in(s, &root, target, field, value, author)
}

/// `set` in `root`: for a thread whose record is not written yet (a new
/// chat's, named before its turn writes it), in the place it is made in.
pub(crate) fn set_in(
  s: &Shared,
  root: &RecordRoot,
  target: &RecordId,
  field: &str,
  value: toml::Value,
  author: Author,
) -> Result<()> {
  let state = s.state();
  let heads: Vec<RecordId> = {
    let index = state.index.lock().expect("index lock");
    let sets: Vec<RecordId> = index
      .sources(kinds::SET, "target", target, None)?
      .into_iter()
      .filter(|r| r.fields.get("field").and_then(serde_json::Value::as_str) == Some(field))
      .map(|r| r.id)
      .collect();
    let mut superseded = std::collections::HashSet::new();
    for s in &sets {
      for e in index.edges_from(s)? {
        if e.rel == "supersedes" {
          superseded.insert(e.dst.id.to_string());
        }
      }
    }
    sets
      .into_iter()
      .filter(|s| !superseded.contains(s.as_str()))
      .collect()
  };
  let supersedes = if heads.is_empty() {
    vec![target.clone()]
  } else {
    heads
  };
  let mut draft = Draft::new(kinds::SET, author)?
    .field("field", field)?
    .field("value", value)?
    .reference("target", Ref::to(target.clone()))?;
  for h in supersedes {
    draft = draft.reference("supersedes", Ref::to(h))?;
  }
  root.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(root)?;
  Ok(())
}

/// The thread's mode as the user last switched it (a `set` of `approval`
/// the user wrote: `read-only` or `default`); `None` when they never did.
/// A `set` written by anything but the user (an agent's run) is ignored.
pub fn read_only_switch(index: &stitch_store::Index, thread: &RecordId) -> Option<bool> {
  let value = index
    .current_user_value(thread, "approval")
    .ok()
    .flatten()?;
  Some(value.value.as_str() == Some("read-only"))
}

/// Whether a thread is read-only (design §2.2): the user's switch, when
/// there is one, decides; else the thread's record as it was created
/// (`approval = "read-only"`).
pub fn read_only(index: &stitch_store::Index, thread: &RecordId) -> bool {
  if let Some(on) = read_only_switch(index, thread) {
    return on;
  }
  index.record(thread).ok().flatten().is_some_and(|r| {
    r.fields.get("approval").and_then(serde_json::Value::as_str) == Some("read-only")
  })
}

/// The model a thread runs on (design §2.7, "A thread's model",
/// `provider/model`): the latest `set` of `model` the user's turns wrote
/// (a turn typed into it on a model its client picked), else its record's
/// `model` (a spawned thread's); `None`: whatever node runs it uses its
/// default.
pub fn thread_model(index: &stitch_store::Index, thread: &RecordId) -> Option<String> {
  if let Some(v) = index.current_user_value(thread, "model").ok().flatten()
    && let Some(m) = v.value.as_str().filter(|m| !m.is_empty())
  {
    return Some(m.to_owned());
  }
  index
    .record(thread)
    .ok()
    .flatten()?
    .fields
    .get("model")
    .and_then(serde_json::Value::as_str)
    .map(str::to_owned)
}

/// Keeps `model` (`provider/model`, a turn's, typed or picked) as the
/// thread's, so the turns nobody types (reports, messages) and other nodes
/// continuing it run on it: a `set` of `model`, written only when it
/// changes.
pub(crate) fn keep_model(s: &Shared, thread: &RecordId, model: &str) -> Result<()> {
  let (root, _) = s.thread_place(thread)?;
  keep_model_in(s, &root, thread, model)
}

/// `keep_model` in `root`: also for a new chat's thread, before its turn
/// writes its record (the turn may never end: a restart cuts it).
pub(crate) fn keep_model_in(
  s: &Shared,
  root: &RecordRoot,
  thread: &RecordId,
  model: &str,
) -> Result<()> {
  let current = thread_model(&s.state().index.lock().expect("index lock"), thread);
  if current.as_deref() == Some(model) {
    return Ok(());
  }
  set_in(
    s,
    root,
    thread,
    "model",
    model.into(),
    Author::user(s.state().node.clone()),
  )
}

/// The node the user had a thread run on (design §2.7, "Continue on
/// another node"; a `set` of `runs-on`), when it is set to one; `None`: it
/// runs where it is typed into.
pub fn runs_on(index: &stitch_store::Index, thread: &RecordId) -> Option<String> {
  crate::weave::run_choice(index, thread)?.node
}

/// The node that owns an unpinned thread, when it is not `me`: where the
/// app's chats into it go by default (`weave::owner_elsewhere_in`).
pub fn owner_elsewhere(index: &stitch_store::Index, thread: &RecordId, me: &str) -> Option<String> {
  crate::weave::owner_elsewhere_in(index, thread, me)
}

/// Whether this build can run programs (shells): a phone cannot.
pub fn can_run_programs() -> bool {
  stitch_shell::CAN_SPAWN
}

/// The thread above `thread`: the one that spawned it, else the one it
/// reports to (`weave::row`'s parent), followed to its newest compaction,
/// which is that thread from then on. A fork is not a child. The TUI's
/// `/parent` and the app's thread tree read it.
pub fn parent(index: &stitch_store::Index, thread: &RecordId) -> Option<RecordId> {
  let edges = index.edges_from(thread).ok()?;
  let mut parent = ["spawned-by", "reports-to"]
    .iter()
    .find_map(|rel| edges.iter().find(|e| e.rel == *rel))
    .map(|e| e.dst.id.clone())
    .filter(|p| p != thread)?;
  for _ in 0..64 {
    let next = index
      .sources(kinds::THREAD, "continues", &parent, None)
      .unwrap_or_default()
      .into_iter()
      .filter(|t| t.fields.get("compacted").and_then(|v| v.as_bool()) == Some(true))
      .max_by_key(|t| t.created);
    match next {
      Some(t) if t.id != parent => parent = t.id,
      _ => break,
    }
  }
  Some(parent)
}

/// Names a thread (`/rename`, `stitch rename`): on the newest thread of
/// its compaction chain, as `pin`.
pub(crate) fn rename(s: &Shared, thread: &RecordId, title: &str) -> Result<()> {
  let title = title.trim();
  if title.is_empty() {
    return Err(Error::Config("a title is words".into()));
  }
  set(
    s,
    &crate::fork::current(s, thread),
    "title",
    title.into(),
    Author::user(s.state().node.clone()),
  )
}

/// Pins a thread (first in lists) or unpins it.
/// Written on the newest thread of its compaction chain: lists read a
/// chain's pin as its latest `set` on any of its threads
/// (`Index::lineage_pinned`, design §2.15), so this one wins.
pub(crate) fn pin(s: &Shared, thread: &RecordId, on: bool) -> Result<()> {
  set(
    s,
    &crate::fork::current(s, thread),
    "pinned",
    on.into(),
    Author::user(s.state().node.clone()),
  )
}

/// The user has read a conversation up to `upto` (none: its latest activity
/// now): a `set` of `read` (an instant, RFC 3339) on the newest thread of
/// its chain, as `pin`; a chain's cursor is the latest on any of its
/// threads (`read_cursor`). Only written when it moves the cursor forward,
/// since these records sync to every node: whether it was.
pub(crate) fn read(s: &Shared, thread: &RecordId, upto: Option<jiff::Timestamp>) -> Result<bool> {
  let newest = crate::fork::current(s, thread);
  let upto = match upto {
    Some(u) => u,
    None => {
      let (root, _) = s.thread_place(&newest)?;
      let state = s.state();
      let ctx = crate::chat::ChatContext {
        node: &state.node,
        root: &root,
        index: &state.index,
        blobs: &state.blobs,
      };
      match crate::chat::last_step_at(&ctx, &newest)? {
        Some(at) => at,
        None => return Ok(false),
      }
    }
  };
  let key = s.thread_place(&newest).map(|(_, p)| p.key()).ok();
  let current = {
    let index = s.state().index.lock().expect("index lock");
    cursor_of(&index, &newest, key.as_deref(), &Floors::read(&index))
  };
  if current.is_some_and(|c| c >= upto) {
    return Ok(false);
  }
  set(
    s,
    &newest,
    "read",
    upto.to_string().into(),
    Author::user(s.state().node.clone()),
  )?;
  Ok(true)
}

/// The kind of a floor of the read cursors: "everything up to `upto` is
/// read", for every conversation (`place` = `all`) or one place's (`place` =
/// its key), a record of the user's in the home workspace like the other
/// settings (it syncs; the greatest `upto` per key counts, so two nodes
/// writing at once agree and an older one never lowers it).
pub const READ_FLOOR: &str = "read-floor";
const ALL: &str = "all";

/// The floors standing, read once per request.
#[derive(Default)]
pub(crate) struct Floors {
  all: Option<jiff::Timestamp>,
  places: std::collections::HashMap<String, jiff::Timestamp>,
}

impl Floors {
  pub(crate) fn read(index: &stitch_store::Index) -> Floors {
    let mut f = Floors::default();
    for r in index.records(Some(READ_FLOOR)).unwrap_or_default() {
      let (Some(key), Some(at)) = (
        r.fields.get("place").and_then(|v| v.as_str()),
        r.fields
          .get("upto")
          .and_then(|v| v.as_str())
          .and_then(|v| v.parse::<jiff::Timestamp>().ok()),
      ) else {
        continue;
      };
      if key == ALL {
        f.all = f.all.max(Some(at));
      } else {
        let e = f.places.entry(key.to_owned()).or_insert(at);
        *e = (*e).max(at);
      }
    }
    f
  }

  /// The floor over a place (its key): the greater of its own and the global one.
  pub(crate) fn of(&self, place_key: Option<&str>) -> Option<jiff::Timestamp> {
    self
      .all
      .max(place_key.and_then(|k| self.places.get(k).copied()))
  }
}

/// A conversation's effective cursor: its own `read` or the floor over its
/// place, the later.
pub(crate) fn cursor_of(
  index: &stitch_store::Index,
  thread: &RecordId,
  place_key: Option<&str>,
  floors: &Floors,
) -> Option<jiff::Timestamp> {
  read_cursor(index, thread).max(floors.of(place_key))
}

/// Writes a floor (`READ_FLOOR`) when it raises the one standing for `key`.
fn write_floor(s: &Shared, key: &str, upto: jiff::Timestamp, floors: &Floors) -> Result<bool> {
  let standing = if key == ALL {
    floors.all
  } else {
    floors.places.get(key).copied()
  };
  if standing.is_some_and(|f| f >= upto) {
    return Ok(false);
  }
  let state = s.state();
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here to keep it in".into()))?;
  let draft = Draft::new(READ_FLOOR, Author::user(state.node.clone()))?
    .field("place", key)?
    .field("upto", upto.to_string().as_str())?;
  home.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok(true)
}

/// The user has read every listed conversation (of the place a row names
/// `place`, or all) up to the latest activity now: one floor record, none
/// when nothing is unread; how many conversations that read. `T` is the
/// latest listed activity, not the clock, so a step landing after stays
/// unread.
pub(crate) fn read_all(s: &std::sync::Arc<Shared>, place: Option<&str>) -> Result<usize> {
  use crate::place::Place;
  let state = s.state();
  let home = state.home().map(|h| h.repo().clone());
  let known = s.places();
  let here = state.node.to_string();
  let (floors, rows) = {
    let index = state.index.lock().expect("index lock");
    let floors = Floors::read(&index);
    let mut rows: Vec<(String, bool, jiff::Timestamp)> = Vec::new();
    for (r, at) in index.listed_threads()? {
      if index.is_deleted(&r.id).unwrap_or(false) {
        continue;
      }
      let p = Place::of_thread(&r, home.as_ref(), &known);
      if place.is_some_and(|w| p.label(&here, &known) != w) {
        continue;
      }
      let key = p.key();
      let unread = cursor_of(&index, &r.id, Some(&key), &floors).is_none_or(|c| at > c);
      rows.push((key, unread, at));
    }
    (floors, rows)
  };
  let moved = rows.iter().filter(|r| r.1).count();
  if moved == 0 {
    return Ok(0);
  }
  let t = rows.iter().map(|r| r.2).max().expect("some");
  if place.is_none() {
    write_floor(s, ALL, t, &floors)?;
  } else {
    // The label may name more than one place (a folder and a project of
    // one name): each gets its floor.
    let mut keys: Vec<&String> = rows.iter().map(|r| &r.0).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
      write_floor(s, k, t, &floors)?;
    }
  }
  Ok(moved)
}

/// The user typed into a conversation: they had read what was in it, so the
/// cursor advances to its latest activity *before* their message (written
/// only when that moves it: one record per message at most). The reply that
/// follows is after it, so it shows unread if they are no longer looking.
/// Not an error when it cannot be written (a thread held elsewhere).
pub(crate) fn read_on_send(s: &Shared, thread: &RecordId) {
  if let Err(e) = read(s, thread, None) {
    eprintln!("thread {thread}: not marked read on send: {e}");
  }
}

/// Up to when the user has read a conversation (its chain's latest `read`).
pub(crate) fn read_cursor(
  index: &stitch_store::Index,
  thread: &RecordId,
) -> Option<jiff::Timestamp> {
  index
    .lineage_value(thread, "read")
    .and_then(|v| v.value.as_str().and_then(|s| s.parse().ok()))
}

const TITLE_SYSTEM: &str = "You name conversations. Reply with the title only: 3 to 7 words, \
in the conversation's language, no quotes, no final period.";

/// Names a new thread from its first exchange, in the background (by
/// `[threads] title_model`, else the place's quick model). A title the user gave first
/// stands.
pub(crate) fn title_later(
  s: &Arc<Shared>,
  thread: RecordId,
  question: String,
  answer: String,
  model: String,
) {
  if s.config.threads.as_ref().and_then(|t| t.titles) == Some(false) {
    return;
  }
  let s = s.clone();
  tokio::spawn(async move {
    if let Err(e) = title_now(&s, &thread, &question, &answer, &model).await {
      eprintln!("thread {thread}: no title: {e}");
    }
  });
}

async fn title_now(
  s: &Arc<Shared>,
  thread: &RecordId,
  question: &str,
  answer: &str,
  _model: &str,
) -> Result<()> {
  let place = s.thread_place(thread).ok().map(|(_, p)| p);
  let configured = s
    .config
    .threads
    .as_ref()
    .and_then(|t| t.title_model.as_deref());
  let title_model =
    crate::modelset::named(s, place.as_ref(), Some(configured.unwrap_or("quick"))).await;
  let (provider, _, model) = s
    .config
    .resolve_authenticated(&s.data, Some(&title_model))
    .await?;
  let clip = |t: &str| t.chars().take(2000).collect::<String>();
  let prompt = format!(
    "The conversation starts:\n\nUser: {}\n\nAssistant: {}\n\nIts title:",
    clip(question),
    clip(answer)
  );
  let mut request = Request::new(model, vec![Message::user(vec![Content::Text(prompt)])]);
  request.system = Some(TITLE_SYSTEM.into());
  request.max_tokens = 64;
  let response = provider.stream(&request, &mut |_| {}).await?;
  let title: String = response
    .text()
    .lines()
    .find(|l| !l.trim().is_empty())
    .unwrap_or("")
    .trim()
    .trim_matches(['"', '\'', '*', '#', ' '])
    .trim_end_matches('.')
    .chars()
    .take(80)
    .collect();
  if title.is_empty() {
    return Ok(());
  }
  // Named meanwhile: the user's stands.
  let named = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .current_value(thread, "title")?
    .is_some();
  if named {
    return Ok(());
  }
  set(
    s,
    thread,
    "title",
    title.as_str().into(),
    Author::node(s.state().node.clone()),
  )
}

/// A thread's transcript as a file to attach to another conversation
/// (edge talk's quote): framed as material from another conversation, not
/// one to continue. Returns its path, under `<data>/downloads/`.
pub(crate) fn quote(s: &Shared, thread: &RecordId) -> Result<std::path::PathBuf> {
  let (root, _) = s.thread_place(thread)?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let title = state
    .index
    .lock()
    .expect("index lock")
    .title(thread)?
    .unwrap_or_else(|| "Chat".into());
  let mut text = format!(
    "# ❝ {title}\n\nThe user is sharing an earlier conversation (thread {thread}) with you. You \
     were not part of it. Treat it strictly as quoted reference material: do not continue it, \
     and answer what the user asks about it.\n\n---\n\n"
  );
  for (who, said) in crate::chat::transcript_to(&ctx, thread, None)? {
    text.push_str(&format!("**{who}:**\n\n{said}\n\n"));
  }
  let dir = s.data.join("downloads");
  std::fs::create_dir_all(&dir).map_err(Error::io(dir.display()))?;
  let path = dir.join(format!("quote-{thread}.md"));
  std::fs::write(&path, text).map_err(Error::io(path.display()))?;
  Ok(path)
}

#[cfg(test)]
mod tests {
  use stitch_records::{Author, Draft, NodeName, Ref};
  use stitch_store::{Index, RecordRoot, Role};

  /// Only the user switches a thread's mode: a `set` an agent's run or a
  /// node wrote is ignored, whatever it supersedes.
  #[test]
  fn only_the_users_switch_counts() {
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let root = RecordRoot::init(dir.path(), "home".parse().unwrap(), Role::Home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    let thread = root
      .publish(
        &node,
        Draft::new("thread", Author::user(node.clone()))
          .unwrap()
          .field("approval", "read-only")
          .unwrap(),
      )
      .unwrap()
      .id;
    let mut last = thread.clone();
    let mut set = |author: Author, value: &str, index: &mut Index| {
      last = root
        .publish(
          &node,
          Draft::new("set", author)
            .unwrap()
            .field("field", "approval")
            .unwrap()
            .field("value", value)
            .unwrap()
            .reference("target", Ref::to(thread.clone()))
            .unwrap()
            .reference("supersedes", Ref::to(last.clone()))
            .unwrap(),
        )
        .unwrap()
        .id;
      index.refresh(&root).unwrap();
    };
    index.refresh(&root).unwrap();
    assert!(super::read_only(&index, &thread));
    assert_eq!(super::read_only_switch(&index, &thread), None);
    set(
      Author::run(node.clone(), thread.clone()),
      "default",
      &mut index,
    );
    set(Author::node(node.clone()), "default", &mut index);
    assert!(
      super::read_only(&index, &thread),
      "an agent cannot switch it"
    );
    set(Author::user(node.clone()), "default", &mut index);
    assert!(!super::read_only(&index, &thread));
    assert_eq!(super::read_only_switch(&index, &thread), Some(false));
    set(
      Author::run(node.clone(), thread.clone()),
      "read-only",
      &mut index,
    );
    assert!(!super::read_only(&index, &thread));
    set(Author::user(node.clone()), "read-only", &mut index);
    assert!(super::read_only(&index, &thread));
  }
}
