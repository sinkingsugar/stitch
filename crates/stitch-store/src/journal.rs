//! A thread's open segment (design §2.5): the steps of a turn appended to
//! one file, `<root>/journal/<thread>/<segment>.jsonl`, as they are done,
//! never synced; sealed into the layout (`threads/<thread>/<segment>.jsonl`,
//! the same name and content) when the turn ends, before the repository
//! syncs, or when the node starts and finds one left. Readers on this node
//! read it with the sealed segments, so a step is seen the moment it is
//! written; other nodes see it once sealed. A segment per turn instead of
//! per step: fewer files, each fuller than a disk block.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use jiff::Timestamp;
use stitch_records::{Event, NodeName, RecordId, Segment};

use crate::error::IoContext;
use crate::root::{ItemPath, RecordRoot};
use crate::{Error, Result};

const JOURNAL_DIR: &str = "journal";

/// One lock per thread journal in this process: appending and sealing
/// never overlap, so nothing is written into a file once it is sealed.
fn lock_of(dir: &Path) -> Arc<Mutex<()>> {
  static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
  LOCKS
    .get_or_init(Default::default)
    .lock()
    .expect("journal locks")
    .entry(dir.to_owned())
    .or_default()
    .clone()
}

impl RecordRoot {
  fn journal_dir(&self, thread: &RecordId) -> PathBuf {
    self.dir().join(JOURNAL_DIR).join(thread.as_str())
  }

  /// The open segment file of a thread, if any: its ID and path.
  fn journal_file(&self, thread: &RecordId) -> Result<Option<(RecordId, PathBuf)>> {
    let dir = self.journal_dir(thread);
    let Ok(entries) = fs::read_dir(&dir) else {
      return Ok(None);
    };
    let mut found = Vec::new();
    for e in entries {
      let e = e.at(&dir)?;
      let name = e.file_name();
      let Some(id) = name
        .to_str()
        .and_then(|n| n.strip_suffix(".jsonl"))
        .and_then(|n| n.parse::<RecordId>().ok())
      else {
        continue;
      };
      found.push((id, e.path()));
    }
    found.sort();
    Ok(found.pop())
  }

  /// Appends a step's events to the thread's open segment, starting one
  /// after `prev` when none is open (the turn's first step, or the open
  /// one was sealed meanwhile: `prev` is then the one sealed). Returns the
  /// open segment's ID, which it keeps once sealed.
  pub fn append_step(
    &self,
    node: &NodeName,
    thread: &RecordId,
    prev: Option<RecordId>,
    events: Vec<Event>,
  ) -> Result<RecordId> {
    self.append_step_from(node, thread, prev, None, events)
  }

  /// `append_step`; a segment it starts begins a branch from `rewind`
  /// (the rewind record, and the message of `prev` it follows: design
  /// §2.15, "Rewinding a thread").
  pub fn append_step_from(
    &self,
    node: &NodeName,
    thread: &RecordId,
    prev: Option<RecordId>,
    rewind: Option<(RecordId, Option<String>)>,
    events: Vec<Event>,
  ) -> Result<RecordId> {
    if self.purged().contains(thread.as_str()) {
      return Err(Error::Mismatch {
        path: format!("journal/{thread}"),
        message: "the thread was deleted".into(),
      });
    }
    let dir = self.journal_dir(thread);
    let lock = lock_of(&dir);
    let _held = lock.lock().expect("journal lock");
    let mut lines = String::new();
    for e in &events {
      lines.push_str(&serde_json::to_string(&e.to_value()).expect("an event serializes"));
      lines.push('\n');
    }
    match self.journal_file(thread)? {
      // The one this turn writes: the step goes on it.
      Some((id, path)) if prev.as_ref() == Some(&id) => {
        let mut f = fs::OpenOptions::new().append(true).open(&path).at(&path)?;
        f.write_all(lines.as_bytes()).at(&path)?;
        f.sync_data().at(&path)?;
        Ok(id)
      }
      open => {
        // One left by a loop before this one: it goes first, whole.
        if open.is_some() {
          self.seal_locked(thread)?;
        }
        let at = Timestamp::now();
        let id = RecordId::generate(at, node, &mut rand::rng());
        let mut segment = Segment::new(id.clone(), thread.clone(), at, node.clone(), prev);
        if let Some((r, upto)) = rewind {
          segment.header.rewind = Some(r);
          segment.header.upto = upto;
        }
        let mut text = segment.to_text();
        text.push_str(&lines);
        fs::create_dir_all(&dir).at(&dir)?;
        let path = dir.join(format!("{id}.jsonl"));
        let mut f = fs::OpenOptions::new()
          .write(true)
          .create_new(true)
          .open(&path)
          .at(&path)?;
        f.write_all(text.as_bytes()).at(&path)?;
        f.sync_all().at(&path)?;
        if let Ok(d) = fs::File::open(&dir) {
          let _ = d.sync_all();
        }
        Ok(id)
      }
    }
  }

  /// The thread's open segment as written so far (a line cut short by a
  /// crash left out), and when its last step was written.
  pub fn open_segment(&self, thread: &RecordId) -> Result<Option<(Segment, Timestamp)>> {
    let Some((_, path)) = self.journal_file(thread)? else {
      return Ok(None);
    };
    let Ok(text) = fs::read_to_string(&path) else {
      // Sealed meanwhile.
      return Ok(None);
    };
    let at = fs::metadata(&path)
      .and_then(|m| m.modified())
      .ok()
      .and_then(|t| Timestamp::try_from(t).ok())
      .unwrap_or_else(Timestamp::now);
    Ok(parse_whole(&text).map(|s| (s, at)))
  }

  /// When the thread's open segment last had a step written, without
  /// reading it (a reader polling for news: `None` when none is open).
  pub fn open_segment_at(&self, thread: &RecordId) -> Option<Timestamp> {
    let (_, path) = self.journal_file(thread).ok()??;
    fs::metadata(&path)
      .and_then(|m| m.modified())
      .ok()
      .and_then(|t| Timestamp::try_from(t).ok())
  }

  /// Seals the thread's open segment: into the layout, whole and
  /// immutable, under the same ID. Returns it, if one was open.
  pub fn seal(&self, thread: &RecordId) -> Result<Option<RecordId>> {
    let dir = self.journal_dir(thread);
    let lock = lock_of(&dir);
    let _held = lock.lock().expect("journal lock");
    self.seal_locked(thread)
  }

  fn seal_locked(&self, thread: &RecordId) -> Result<Option<RecordId>> {
    let Some((id, path)) = self.journal_file(thread)? else {
      return Ok(None);
    };
    // A deleted thread's turn, stopping: what it wrote goes.
    if self.purged().contains(thread.as_str()) {
      let _ = fs::remove_dir_all(self.journal_dir(thread));
      return Ok(None);
    }
    let text = fs::read_to_string(&path).at(&path)?;
    let Some(segment) = parse_whole(&text) else {
      return Err(Error::Mismatch {
        path: path.display().to_string(),
        message: "an open segment that does not parse".into(),
      });
    };
    let whole = segment.to_text();
    let item = ItemPath::Segment {
      thread: thread.clone(),
      segment: id.clone(),
    };
    if !self.write_new_item(&item, whole.as_bytes())? {
      // Sealed before, the journal left behind (a crash between the two).
      let there = self.read(&item)?;
      if there != whole.as_bytes() {
        return Err(Error::Conflict(item.to_string()));
      }
    }
    fs::remove_file(&path).at(&path)?;
    let _ = fs::remove_dir(self.journal_dir(thread));
    Ok(Some(id))
  }

  /// Seals every open segment of the root (before it syncs, and when the
  /// node starts). Returns how many.
  pub fn seal_all(&self) -> Result<usize> {
    let top = self.dir().join(JOURNAL_DIR);
    let Ok(entries) = fs::read_dir(&top) else {
      return Ok(0);
    };
    let mut n = 0;
    for e in entries {
      let e = e.at(&top)?;
      let Some(thread) = e
        .file_name()
        .to_str()
        .and_then(|t| t.parse::<RecordId>().ok())
      else {
        continue;
      };
      if self.seal(&thread)?.is_some() {
        n += 1;
      }
    }
    Ok(n)
  }
}

/// A segment from an open file's text: a last line cut short (a crash
/// mid-write) is left out.
fn parse_whole(text: &str) -> Option<Segment> {
  if let Ok(s) = Segment::parse(text) {
    return Some(s);
  }
  let cut = text.trim_end_matches('\n').rfind('\n')?;
  Segment::parse(&text[..=cut]).ok()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::root::Role;
  use stitch_records::{Msg, Part, Role as MRole};

  fn msg(id: &str, text: &str) -> Event {
    Event::Msg(Msg {
      id: id.into(),
      at: Timestamp::now(),
      role: MRole::User,
      from: None,
      model: None,
      parts: vec![Part::Text { text: text.into() }],
      native: None,
    })
  }

  #[test]
  fn steps_go_on_one_segment_sealed_whole_under_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(dir.path(), "home".parse().unwrap(), Role::Home).unwrap();
    let node: NodeName = "mbp".parse().unwrap();
    let thread = RecordId::generate(Timestamp::now(), &node, &mut rand::rng());
    let a = root
      .append_step(&node, &thread, None, vec![msg("m1", "one")])
      .unwrap();
    let b = root
      .append_step(&node, &thread, Some(a.clone()), vec![msg("m2", "two")])
      .unwrap();
    assert_eq!(a, b, "one segment for the turn");
    let (open, _) = root.open_segment(&thread).unwrap().unwrap();
    assert_eq!(open.events.len(), 2);
    // Not in the layout (nothing syncs it) until sealed.
    assert!(root.scan().unwrap().is_empty());
    // Sealed meanwhile (a sync): the next step starts the next segment.
    assert_eq!(root.seal(&thread).unwrap(), Some(a.clone()));
    assert_eq!(root.scan().unwrap().len(), 1);
    let c = root
      .append_step(&node, &thread, Some(a.clone()), vec![msg("m3", "three")])
      .unwrap();
    assert_ne!(c, a);
    let (open, _) = root.open_segment(&thread).unwrap().unwrap();
    assert_eq!(open.header.prev, Some(a.clone()));
    // A line cut short by a crash is left out when sealed.
    let path = root.journal_file(&thread).unwrap().unwrap().1;
    fs::OpenOptions::new()
      .append(true)
      .open(&path)
      .unwrap()
      .write_all(b"{\"t\":\"msg\",\"id\":")
      .unwrap();
    assert_eq!(root.seal_all().unwrap(), 1);
    let sealed = Segment::parse(
      &String::from_utf8(
        root
          .read(&ItemPath::Segment {
            thread: thread.clone(),
            segment: c,
          })
          .unwrap(),
      )
      .unwrap(),
    )
    .unwrap();
    assert_eq!(sealed.events.len(), 1);
    assert!(root.open_segment(&thread).unwrap().is_none());
  }
}
