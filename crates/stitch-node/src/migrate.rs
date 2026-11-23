//! The one migration of a record root to a segment per turn (design §2.5):
//! a thread's segments written one per step before 2026-10-13 merged, run
//! by run, into one per turn. Run once per root, when a node of this
//! version starts (before anything reads or syncs it); a root made since is
//! marked from its start. Every node does it on the same segments the same
//! way, so they end with the same files.
//!
//! - A run is consecutive segments of one line, cut where a turn starts (a
//!   segment whose first event is a user's message), where the line forks
//!   (a segment with two continuations), after a segment a record names
//!   (`continues_at`), and where the writing node changes.
//! - The merged segment takes the ID, file and creation of the run's last
//!   segment and the `prev` of its first, so whatever names a segment (a
//!   `prev`, a `continues_at`) still finds it.
//! - Only segments written before this format (`BEFORE`) are merged: one a
//!   node of this version wrote (a turn sealed mid-way, for a sync) is
//!   already where peers have it, and is never rewritten.
//! - A thread whose main line would end elsewhere after merging is left as
//!   it is (said in the log).
//! - The files before are copied first to
//!   `<data>/migrated-segments/<repo>/<thread>/` and kept there (remove it
//!   once satisfied); a migration cut short (the root not marked yet) is
//!   undone from there before it runs again.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use stitch_records::{Event, RecordId, Role, Segment};
use stitch_store::{Index, ItemPath, RecordRoot};

use crate::{Error, Result};

/// When segments began to be one per turn: none written since is merged.
const BEFORE: &str = "2026-10-13T12:00:00Z";

/// What a root's migration did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migrated {
  pub threads: usize,
  pub before: usize,
  pub after: usize,
  /// Threads left as they were (their main line would have moved).
  pub kept: Vec<String>,
}

/// Migrates `root` unless it is marked; `referenced` are the segment IDs
/// records name. Returns what it did, `None` when it was marked.
pub fn migrate(
  index: &mut Index,
  root: &RecordRoot,
  referenced: &HashSet<String>,
  backup: &Path,
) -> Result<Option<Migrated>> {
  if root.turns_marked() {
    return Ok(None);
  }
  let backup = backup.join(root.repo().as_str());
  let mut out = Migrated::default();
  let threads_dir = root.dir().join("threads");
  let mut threads: Vec<RecordId> = Vec::new();
  if let Ok(entries) = fs::read_dir(&threads_dir) {
    for e in entries.flatten() {
      if let Some(t) = e.file_name().to_str().and_then(|t| t.parse().ok()) {
        threads.push(t);
      }
    }
  }
  // A migration cut short is undone first.
  if let Ok(entries) = fs::read_dir(&backup) {
    for e in entries.flatten() {
      if let Some(t) = e
        .file_name()
        .to_str()
        .and_then(|t| t.parse::<RecordId>().ok())
      {
        restore(root, &backup.join(t.as_str()), &t)?;
        if !threads.contains(&t) {
          threads.push(t);
        }
      }
    }
  }
  threads.sort();
  let mut changed: Vec<String> = Vec::new();
  for thread in threads {
    let (n_before, n_after, paths, kept) = migrate_thread(root, &thread, referenced, &backup)?;
    out.threads += 1;
    out.before += n_before;
    out.after += n_after;
    if kept {
      out.kept.push(thread.to_string());
    }
    changed.extend(paths);
  }
  index.forget(root.repo(), &changed)?;
  index.refresh(root)?;
  root.mark_turns()?;
  Ok(Some(out))
}

/// Puts a thread's files back as they were before a migration was cut
/// short.
fn restore(root: &RecordRoot, saved: &Path, thread: &RecordId) -> Result<()> {
  let dir = root.dir().join("threads").join(thread.as_str());
  fs::create_dir_all(&dir).map_err(Error::io(dir.display()))?;
  for e in fs::read_dir(saved)
    .map_err(Error::io(saved.display()))?
    .flatten()
  {
    let to = dir.join(e.file_name());
    fs::copy(e.path(), &to).map_err(Error::io(to.display()))?;
  }
  Ok(())
}

/// One thread: (segments before, after, paths rewritten or removed,
/// whether it was left as it was).
fn migrate_thread(
  root: &RecordRoot,
  thread: &RecordId,
  referenced: &HashSet<String>,
  backup: &Path,
) -> Result<(usize, usize, Vec<String>, bool)> {
  let dir = root.dir().join("threads").join(thread.as_str());
  let mut segments: HashMap<RecordId, Segment> = HashMap::new();
  for e in fs::read_dir(&dir)
    .map_err(Error::io(dir.display()))?
    .flatten()
  {
    let Some(id) = e
      .file_name()
      .to_str()
      .and_then(|n| n.strip_suffix(".jsonl"))
      .and_then(|n| n.parse::<RecordId>().ok())
    else {
      continue;
    };
    let Ok(text) = fs::read_to_string(e.path()) else {
      continue;
    };
    if let Ok(s) = Segment::parse(&text)
      && s.header.segment == id
    {
      segments.insert(id, s);
    }
  }
  let before = segments.len();
  let mut children: HashMap<RecordId, Vec<RecordId>> = HashMap::new();
  for s in segments.values() {
    if let Some(p) = &s.header.prev {
      children
        .entry(p.clone())
        .or_default()
        .push(s.header.segment.clone());
    }
  }
  let before_format: jiff::Timestamp = BEFORE.parse().expect("a time");
  let starts_run = |s: &Segment| -> bool {
    let Some(prev) = s.header.prev.as_ref().and_then(|p| segments.get(p)) else {
      return true;
    };
    if s.header.created >= before_format || prev.header.created >= before_format {
      return true;
    }
    let turn = matches!(s.events.first(), Some(Event::Msg(m)) if m.role == Role::User);
    turn
      || children
        .get(&prev.header.segment)
        .is_some_and(|c| c.len() > 1)
      || referenced.contains(prev.header.segment.as_str())
      || prev.header.node != s.header.node
      || prev.header.raw.is_some()
      || s.header.raw.is_some()
  };
  // Runs, from each start along its single line.
  let mut runs: Vec<Vec<RecordId>> = Vec::new();
  let mut starts: Vec<&Segment> = segments.values().filter(|s| starts_run(s)).collect();
  starts.sort_by(|a, b| a.header.segment.cmp(&b.header.segment));
  for s in starts {
    let mut run = vec![s.header.segment.clone()];
    let mut cur = s.header.segment.clone();
    while let Some([next]) = children.get(&cur).map(Vec::as_slice) {
      if starts_run(&segments[next]) {
        break;
      }
      run.push(next.clone());
      cur = next.clone();
    }
    runs.push(run);
  }
  let after = runs.len();
  if after == before {
    return Ok((before, after, Vec::new(), false));
  }
  // The main line must end where it did.
  let was = crate::chat::main_head(
    segments
      .values()
      .map(|s| (&s.header.segment, s.header.prev.as_ref())),
  );
  let merged: Vec<(RecordId, Option<RecordId>)> = runs
    .iter()
    .map(|r| {
      (
        r.last().expect("a run").clone(),
        segments[&r[0]].header.prev.clone(),
      )
    })
    .collect();
  let will = crate::chat::main_head(merged.iter().map(|(id, prev)| (id, prev.as_ref())));
  if was != will {
    eprintln!(
      "migrate: thread {thread} left as it was: merged, its main line would end at {will:?}, not {was:?}"
    );
    return Ok((before, before, Vec::new(), true));
  }
  // The files as they were, first.
  let saved = backup.join(thread.as_str());
  fs::create_dir_all(&saved).map_err(Error::io(saved.display()))?;
  for e in fs::read_dir(&dir)
    .map_err(Error::io(dir.display()))?
    .flatten()
  {
    let to = saved.join(e.file_name());
    fs::copy(e.path(), &to).map_err(Error::io(to.display()))?;
  }
  let mut paths = Vec::new();
  for run in runs.iter().filter(|r| r.len() > 1) {
    let last = &segments[run.last().expect("a run")];
    let mut whole = last.clone();
    whole.header.prev = segments[&run[0]].header.prev.clone();
    whole.events = run
      .iter()
      .flat_map(|id| segments[id].events.iter().cloned())
      .collect();
    let item = |id: &RecordId| ItemPath::Segment {
      thread: thread.clone(),
      segment: id.clone(),
    };
    // The last one's file holds the run; the others go.
    let target = root.path_of(&item(&whole.header.segment));
    let tmp = target.with_extension("jsonl.merging");
    fs::write(&tmp, whole.to_text()).map_err(Error::io(tmp.display()))?;
    fs::rename(&tmp, &target).map_err(Error::io(target.display()))?;
    paths.push(item(&whole.header.segment).to_string());
    for id in &run[..run.len() - 1] {
      let p = root.path_of(&item(id));
      fs::remove_file(&p).map_err(Error::io(p.display()))?;
      paths.push(item(id).to_string());
    }
  }
  if let Ok(d) = fs::File::open(&dir) {
    let _ = d.sync_all();
  }
  Ok((before, after, paths, false))
}

#[cfg(test)]
mod tests {
  use super::*;
  use stitch_records::{Msg, NodeName, Part, ToolCall};
  use stitch_store::Role as RootRole;

  fn user(id: &str) -> Event {
    Event::Msg(Msg {
      id: id.into(),
      at: "2026-10-12T10:00:00Z".parse().unwrap(),
      role: Role::User,
      from: None,
      model: None,
      parts: vec![Part::Text { text: id.into() }],
      native: None,
    })
  }

  fn call(id: &str) -> Event {
    Event::ToolCall(ToolCall {
      id: id.into(),
      at: "2026-10-12T10:00:00Z".parse().unwrap(),
      name: "shell_run".into(),
      input: serde_json::json!({"command": "ls"}),
    })
  }

  /// Writes segments of one thread, a minute apart on 2026-10-12, as the
  /// node wrote them one per step: (events, prev index).
  fn old_thread(root: &RecordRoot, steps: Vec<Vec<Event>>) -> (RecordId, Vec<RecordId>) {
    let node: NodeName = "mbp".parse().unwrap();
    let base: jiff::Timestamp = "2026-10-12T10:00:00Z".parse().unwrap();
    let thread = RecordId::random(base, &node);
    let mut ids: Vec<RecordId> = Vec::new();
    for (i, events) in steps.into_iter().enumerate() {
      let at = base + jiff::SignedDuration::from_mins(i as i64 + 1);
      let id = RecordId::random(at, &node);
      let mut s = Segment::new(
        id.clone(),
        thread.clone(),
        at,
        node.clone(),
        ids.last().cloned(),
      );
      s.events = events;
      let item = ItemPath::Segment {
        thread: thread.clone(),
        segment: id.clone(),
      };
      root
        .insert(&item.to_string(), s.to_text().as_bytes())
        .unwrap();
      ids.push(id);
    }
    (thread, ids)
  }

  #[test]
  fn steps_are_merged_by_turn_keeping_what_names_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      RootRole::Home,
    )
    .unwrap();
    // A root from before the format: not marked.
    fs::remove_file(dir.path().join("home").join(".segments-by-turn")).unwrap();
    assert!(!root.turns_marked());
    let (thread, ids) = old_thread(
      &root,
      vec![
        vec![user("m1"), call("c1")],
        vec![call("c2")],
        vec![user("m3")],
        vec![call("c3")],
        vec![call("c4")],
      ],
    );
    let mut index = Index::open_in_memory().unwrap();
    index.refresh(&root).unwrap();
    let backup = dir.path().join("migrated");
    // The second turn's first step is named by a record (a spawn from it).
    let referenced = HashSet::from([ids[3].to_string()]);
    let m = migrate(&mut index, &root, &referenced, &backup)
      .unwrap()
      .unwrap();
    assert_eq!((m.threads, m.before, m.after), (1, 5, 3));
    assert!(root.turns_marked());
    // The runs: [0, 1] as 1, [2, 3] as 3 (named: it ends a run), [4].
    let segs = index.thread_segments(&thread).unwrap();
    let kept: Vec<&RecordId> = segs.iter().map(|(id, _, _)| id).collect();
    assert_eq!(kept, vec![&ids[1], &ids[3], &ids[4]]);
    assert_eq!(segs[0].1, None);
    assert_eq!(segs[1].1.as_ref(), Some(&ids[1]));
    assert_eq!(segs[2].1.as_ref(), Some(&ids[3]));
    let read = |id: &RecordId| {
      Segment::parse(
        &String::from_utf8(
          root
            .read(&ItemPath::Segment {
              thread: thread.clone(),
              segment: id.clone(),
            })
            .unwrap(),
        )
        .unwrap(),
      )
      .unwrap()
    };
    assert_eq!(read(&ids[1]).events.len(), 3);
    assert_eq!(read(&ids[3]).events.len(), 2);
    // The files before, kept.
    assert_eq!(
      fs::read_dir(backup.join("home").join(thread.as_str()))
        .unwrap()
        .count(),
      5
    );
    // Once only.
    assert_eq!(
      migrate(&mut index, &root, &referenced, &backup).unwrap(),
      None
    );
  }

  #[test]
  fn a_migration_cut_short_is_undone_first_and_new_segments_are_never_merged() {
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      RootRole::Home,
    )
    .unwrap();
    fs::remove_file(dir.path().join("home").join(".segments-by-turn")).unwrap();
    let (thread, ids) = old_thread(&root, vec![vec![user("m1")], vec![call("c1")]]);
    let backup = dir.path().join("migrated");
    // Cut short: the files saved, one merged, the other not removed yet.
    let saved = backup.join("home").join(thread.as_str());
    fs::create_dir_all(&saved).unwrap();
    for id in &ids {
      let p = root.path_of(&ItemPath::Segment {
        thread: thread.clone(),
        segment: id.clone(),
      });
      fs::copy(&p, saved.join(p.file_name().unwrap())).unwrap();
    }
    fs::write(
      root.path_of(&ItemPath::Segment {
        thread: thread.clone(),
        segment: ids[1].clone(),
      }),
      "garbage\n",
    )
    .unwrap();
    let mut index = Index::open_in_memory().unwrap();
    let m = migrate(&mut index, &root, &HashSet::new(), &backup)
      .unwrap()
      .unwrap();
    assert_eq!((m.before, m.after), (2, 1));
    // A segment written in this format (a turn sealed mid-way) stays.
    let node: NodeName = "mbp".parse().unwrap();
    let now = jiff::Timestamp::now();
    let id = RecordId::random(now, &node);
    let mut s = Segment::new(id.clone(), thread.clone(), now, node, Some(ids[1].clone()));
    s.events = vec![call("c9")];
    root
      .insert(
        &ItemPath::Segment {
          thread: thread.clone(),
          segment: id,
        }
        .to_string(),
        s.to_text().as_bytes(),
      )
      .unwrap();
    fs::remove_file(dir.path().join("home").join(".segments-by-turn")).unwrap();
    let m = migrate(
      &mut index,
      &root,
      &HashSet::new(),
      &dir.path().join("again"),
    )
    .unwrap()
    .unwrap();
    assert_eq!((m.before, m.after), (2, 2));
  }
}
