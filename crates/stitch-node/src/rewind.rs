//! Rewinding a thread (design §2.15, "Rewinding a thread"): the user's
//! `rewind` record names a thread and the message its conversation goes
//! on from (`segment`, `upto`; neither: from the start). The main line is
//! then the branch the latest rewind starts (segments whose header names
//! it, and what continues them), else, while nothing continues it yet, the
//! line up to that message; without a rewind, the longest chain (design
//! §2.4). What came after the message stays stored, off the line: not in
//! the transcript, the model's context or search.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::Timestamp;
use serde_json::Value;
use stitch_records::{Author, Draft, Event, RecordId, Ref, Segment, kinds};
use stitch_store::Index;

use crate::chat::{self, ChatContext};
use crate::node::Shared;
use crate::{Error, Result};

/// A user's rewind of a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewind {
  pub id: RecordId,
  /// The segment holding the message; `None`: from the thread's start.
  pub segment: Option<RecordId>,
  /// The message (`m7`); `None` with a segment: all of it.
  pub upto: Option<String>,
  /// The node it was written on, which takes the turn after it.
  pub node: String,
  pub created: Timestamp,
}

/// Where a thread's main line ends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEnd {
  /// Its last segment (none: an empty line).
  pub head: Option<RecordId>,
  /// It ends at this rewind, nothing continuing it yet: `head` is cut
  /// after `upto`, and the next segment starts the rewind's branch.
  pub open: Option<Rewind>,
  /// The segments the line is chosen among (a rewind's branch); `None`:
  /// all of them.
  pub pool: Option<HashSet<RecordId>>,
}

/// A segment as the line is chosen: its ID, the one before, the rewind it
/// starts a branch from.
pub type Row = (RecordId, Option<RecordId>, Option<RecordId>);

/// The main line's end, from a thread's segments and its rewinds (oldest
/// first): every node holding the same records chooses the same.
pub fn main_line(segments: &[Row], rewinds: &[Rewind]) -> LineEnd {
  let here: HashSet<&RecordId> = segments.iter().map(|(id, _, _)| id).collect();
  // The latest rewind whose message is here.
  let Some(r) = rewinds
    .iter()
    .filter(|r| r.segment.as_ref().is_none_or(|s| here.contains(s)))
    .max_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)))
  else {
    return LineEnd {
      head: chat::main_head(segments.iter().map(|(id, prev, _)| (id, prev.as_ref()))),
      ..LineEnd::default()
    };
  };
  // Its branch: the segments that start from it, and what follows them.
  let mut pool: HashSet<RecordId> = segments
    .iter()
    .filter(|(_, prev, rw)| rw.as_ref() == Some(&r.id) && *prev == r.segment)
    .map(|(id, _, _)| id.clone())
    .collect();
  loop {
    let more: Vec<RecordId> = segments
      .iter()
      .filter(|(id, prev, _)| !pool.contains(id) && prev.as_ref().is_some_and(|p| pool.contains(p)))
      .map(|(id, _, _)| id.clone())
      .collect();
    if more.is_empty() {
      break;
    }
    pool.extend(more);
  }
  if pool.is_empty() {
    return LineEnd {
      head: r.segment.clone(),
      open: Some(r.clone()),
      pool: Some(pool),
    };
  }
  LineEnd {
    head: chat::main_head(
      segments
        .iter()
        .filter(|(id, _, _)| pool.contains(id))
        .map(|(id, prev, _)| (id, prev.as_ref())),
    ),
    open: None,
    pool: Some(pool),
  }
}

/// A thread's rewinds, as the index has them (the user's only).
pub fn rewinds_in(index: &Index, thread: &RecordId) -> Vec<Rewind> {
  index
    .rewinds(thread)
    .unwrap_or_default()
    .into_iter()
    .map(|r| Rewind {
      segment: r
        .fields
        .get("segment")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok()),
      upto: r
        .fields
        .get("upto")
        .and_then(Value::as_str)
        .map(str::to_owned),
      node: r.node.clone(),
      created: r.created,
      id: r.id,
    })
    .collect()
}

/// A thread's segments as rows: the index's, and `open` (its open segment
/// here, not indexed).
pub fn rows_in(index: &Index, thread: &RecordId, open: Option<&Segment>) -> Vec<Row> {
  let starts: std::collections::HashMap<RecordId, RecordId> = index
    .segment_rewinds(thread)
    .unwrap_or_default()
    .into_iter()
    .collect();
  let mut rows: Vec<Row> = index
    .thread_segments(thread)
    .unwrap_or_default()
    .into_iter()
    .map(|(id, prev, _)| {
      let rw = starts.get(&id).cloned();
      (id, prev, rw)
    })
    .collect();
  if let Some(o) = open
    && !rows.iter().any(|(id, _, _)| *id == o.header.segment)
  {
    rows.push((
      o.header.segment.clone(),
      o.header.prev.clone(),
      o.header.rewind.clone(),
    ));
  }
  rows
}

/// The main line's end as the index has it (sealed segments only).
pub fn line_in(index: &Index, thread: &RecordId) -> LineEnd {
  main_line(&rows_in(index, thread, None), &rewinds_in(index, thread))
}

/// Keeps `events` up to the message `upto` and the tool calls and results
/// that followed it (what `upto` means everywhere); all when it is not
/// there.
pub fn cut_after(events: &mut Vec<Event>, upto: &str) {
  let Some(at) = events
    .iter()
    .position(|e| matches!(e, Event::Msg(m) if m.id == upto))
  else {
    return;
  };
  let end = events[at + 1..]
    .iter()
    .position(|e| matches!(e, Event::Msg(_)))
    .map_or(events.len(), |n| at + 1 + n);
  events.truncate(end);
}

/// The number of the highest message (`m12`: 12) in `events`.
pub fn highest_msg<'a>(events: impl IntoIterator<Item = &'a Event>) -> usize {
  events
    .into_iter()
    .filter_map(|e| match e {
      Event::Msg(m) => m.id.strip_prefix('m')?.parse::<usize>().ok(),
      _ => None,
    })
    .max()
    .unwrap_or(0)
}

/// `Request::Rewind`: the thread's line goes on from its message `upto`
/// (the start, `None`), on this node; refused while a turn runs on it
/// here. What came after stays stored, off the line.
pub(crate) fn rewind(s: &Arc<Shared>, thread: &RecordId, upto: Option<&str>) -> Result<()> {
  if s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .contains_key(thread.as_str())
  {
    return Err(Error::Config(
      "a turn runs on this thread: stop it first, then rewind".into(),
    ));
  }
  let (root, _) = s.thread_place(thread)?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  // A turn left open (a crash) is sealed first: the branch starts after
  // sealed segments only.
  chat::seal(&ctx, thread)?;
  let segment = match upto {
    None => None,
    Some(m) => Some(
      chat::segment_of_message(&ctx, thread, m)?
        .ok_or_else(|| Error::Config(format!("thread {thread} has no message {m} on its line")))?,
    ),
  };
  let mut draft = Draft::new(kinds::REWIND, Author::user(state.node.clone()))?
    .reference("thread", Ref::to(thread.clone()))?;
  if let Some(seg) = &segment {
    draft = draft.field("segment", seg.as_str())?;
  }
  if let Some(m) = upto {
    draft = draft.field("upto", m)?;
  }
  root.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&root)?;
  Ok(())
}

/// Stops the turns running here on a thread the user rewound elsewhere
/// since they started: they went on from what is now off the line (their
/// steps stay there, stored and unseen).
pub(crate) fn stop_overtaken(s: &Arc<Shared>) {
  let started: Vec<(String, Timestamp)> = s
    .detached
    .started
    .lock()
    .expect("started lock")
    .iter()
    .map(|(t, at)| (t.clone(), *at))
    .collect();
  for (thread, at) in started {
    let Ok(id) = thread.parse::<RecordId>() else {
      continue;
    };
    let newer = {
      let index = s.state().index.lock().expect("index lock");
      rewinds_in(&index, &id)
        .iter()
        .any(|r| r.created > at && r.node != s.state().node.as_str())
    };
    if newer {
      eprintln!("thread {thread}: rewound on another node, its turn here stops");
      crate::detached::stop_deleted(s, &id);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn id(n: u32) -> RecordId {
    format!(
      "2026-10-22T{:02}{:02}{:02}Z-mac-aaaa",
      n / 3600 % 24,
      n / 60 % 60,
      n % 60
    )
    .parse()
    .unwrap()
  }

  fn rw(n: u32, segment: Option<u32>, upto: &str, at: i64) -> Rewind {
    Rewind {
      id: id(900 + n),
      segment: segment.map(id),
      upto: Some(upto.into()),
      node: "mac".into(),
      created: Timestamp::from_second(at).unwrap(),
    }
  }

  #[test]
  fn the_latest_rewind_decides_the_line() {
    // 1 ← 2 ← 3 ← 4: the longest.
    let mut rows: Vec<Row> = vec![
      (id(1), None, None),
      (id(2), Some(id(1)), None),
      (id(3), Some(id(2)), None),
      (id(4), Some(id(3)), None),
    ];
    assert_eq!(main_line(&rows, &[]).head, Some(id(4)));
    // Rewound to m3 in 2: the line ends there, open.
    let r1 = rw(1, Some(2), "m3", 10);
    let end = main_line(&rows, std::slice::from_ref(&r1));
    assert_eq!(end.head, Some(id(2)));
    assert_eq!(end.open, Some(r1.clone()));
    // A turn continues it: its branch is the line, shorter as it is.
    rows.push((id(5), Some(id(2)), Some(r1.id.clone())));
    let end = main_line(&rows, std::slice::from_ref(&r1));
    assert_eq!((end.head, end.open), (Some(id(5)), None));
    // A loop elsewhere goes on from the old line: still not the line.
    rows.push((id(6), Some(id(4)), None));
    assert_eq!(
      main_line(&rows, std::slice::from_ref(&r1)).head,
      Some(id(5))
    );
    // Rewound again, to the start: an empty line until a turn.
    let r2 = Rewind {
      segment: None,
      upto: None,
      ..rw(2, None, "", 20)
    };
    let both = [r1.clone(), r2.clone()];
    let end = main_line(&rows, &both);
    assert_eq!((end.head, end.open), (None, Some(r2.clone())));
    rows.push((id(7), None, Some(r2.id.clone())));
    assert_eq!(main_line(&rows, &both).head, Some(id(7)));
    // A rewind whose segment is not here yet waits.
    let r3 = rw(3, Some(99), "m1", 30);
    assert_eq!(main_line(&rows, &[r1, r2, r3]).head, Some(id(7)));
  }

  #[test]
  fn a_cut_keeps_the_message_and_its_calls() {
    use stitch_records::{Msg, Part, Role, ToolCall};
    let msg = |id: &str| {
      Event::Msg(Msg {
        id: id.into(),
        at: Timestamp::now(),
        role: Role::Assistant,
        from: None,
        model: None,
        parts: vec![Part::Text { text: id.into() }],
        native: None,
      })
    };
    let call = Event::ToolCall(ToolCall {
      id: "c1".into(),
      at: Timestamp::now(),
      name: "x".into(),
      input: Value::Null,
    });
    let mut events = vec![msg("m1"), msg("m2"), call, msg("m3")];
    cut_after(&mut events, "m2");
    assert_eq!(events.len(), 3);
    assert_eq!(highest_msg(&events), 2);
  }
}
