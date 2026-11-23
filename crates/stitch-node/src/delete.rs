//! Deleting threads (design §2.5, "Deleting a thread"): the user's
//! `deletion` record names them, and every node purges them when it has
//! it (`stitch_store::purge`): their records and segments, what exists
//! only for them, the blobs nothing else references, what memory observed
//! of them. Nothing else goes: the threads they spawned, their forks and
//! the threads that report to them stay, their parent or source reading as
//! deleted. Only the user deletes: there is no tool for it, and a request
//! from a thread's shell is refused.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use stitch_records::{Author, Draft, RecordId, Ref, kinds};
use stitch_store::Index;

use crate::node::Shared;
use crate::{Error, Result};

/// A thread and those of its compaction chain (design §2.15): one
/// conversation, deleted together. Oldest first.
fn chain(index: &Index, thread: &RecordId) -> Vec<RecordId> {
  let compacted = |r: &stitch_store::IndexedRecord| {
    r.fields.get("compacted").and_then(Value::as_bool) == Some(true)
  };
  let mut out = vec![thread.clone()];
  // Back: what it was compacted from.
  for _ in 0..64 {
    let first = out[0].clone();
    let Some(r) = index.record(&first).ok().flatten() else {
      break;
    };
    if !compacted(&r) {
      break;
    }
    let prev = index
      .edges_from(&first)
      .unwrap_or_default()
      .into_iter()
      .find(|e| e.rel == "continues")
      .map(|e| e.dst.id);
    match prev {
      Some(p) if !out.contains(&p) => out.insert(0, p),
      _ => break,
    }
  }
  // On: what it was compacted into.
  for _ in 0..64 {
    let last = out.last().expect("one").clone();
    let next = index
      .sources(kinds::THREAD, "continues", &last, None)
      .unwrap_or_default()
      .into_iter()
      .filter(compacted)
      .max_by_key(|t| t.created)
      .map(|t| t.id);
    match next {
      Some(n) if !out.contains(&n) => out.push(n),
      _ => break,
    }
  }
  out
}

/// A thread a deletion takes, and why: `this` (named, or spawned by the
/// thread `--spawned-by` names), `earlier` (a part of the same
/// conversation it was compacted from) or `later` (a part compacted from
/// it), so a confirmation can say what each line is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Doomed {
  id: RecordId,
  title: String,
  role: &'static str,
}

/// The threads a request names, with their compaction chains, each chain
/// oldest first; those already deleted are left out.
fn resolve(index: &Index, threads: &[String], spawned_by: Option<&str>) -> Result<Vec<Doomed>> {
  let mut named: Vec<RecordId> = Vec::new();
  for t in threads {
    let id: RecordId = t
      .parse()
      .map_err(|_| Error::Config(format!("`{t}` is not a thread ID")))?;
    named.push(id);
  }
  if let Some(p) = spawned_by {
    let p: RecordId = p
      .parse()
      .map_err(|_| Error::Config(format!("`{p}` is not a thread ID")))?;
    named.extend(
      index
        .sources(kinds::THREAD, "spawned-by", &p, None)?
        .into_iter()
        .map(|r| r.id),
    );
  }
  let mut out: Vec<Doomed> = Vec::new();
  for id in &named {
    match index.record(id)? {
      Some(r) if r.kind == kinds::THREAD => {}
      Some(_) => return Err(Error::Config(format!("{id} is not a thread"))),
      None if index.is_deleted(id)? => continue,
      None => return Err(Error::Config(format!("no thread {id} here"))),
    }
    let mut role = "earlier";
    for t in chain(index, id) {
      let this = t == *id || named.contains(&t);
      if t == *id {
        role = "later";
      }
      let role = if this { "this" } else { role };
      if let Some(d) = out.iter_mut().find(|d| d.id == t) {
        // Named itself, after a chain listed it as a part: it is `this`.
        if this {
          d.role = "this";
        }
        continue;
      }
      if !index.is_deleted(&t)? {
        let title = index.title(&t)?.unwrap_or_else(|| "(untitled)".into());
        out.push(Doomed { id: t, title, role });
      }
    }
  }
  Ok(out)
}

/// `Request::DeleteThreads`: the threads named (and those `spawned_by`
/// spawned), each with its compaction chain, as lines
/// `<id>\t<title>\t<role>` (`Doomed`: `this`, `earlier`, `later`);
/// deleted unless `dry_run`. `from_thread` (the thread whose shell runs
/// the command) refuses it: deleting is the user's.
pub(crate) async fn request(
  s: &Arc<Shared>,
  threads: Vec<String>,
  spawned_by: Option<String>,
  dry_run: bool,
  from_thread: Option<String>,
) -> Result<Vec<String>> {
  if !dry_run && from_thread.is_some() {
    return Err(Error::Config(
      "deleting threads is the user's: run `stitch thread rm` in your own terminal, not from a \
       thread's shell"
        .into(),
    ));
  }
  let doomed = {
    let index = s.state().index.lock().expect("index lock");
    resolve(&index, &threads, spawned_by.as_deref())?
  };
  let lines = doomed
    .iter()
    .map(|d| format!("{}\t{}\t{}", d.id, d.title.replace('\t', " "), d.role))
    .collect();
  if dry_run || doomed.is_empty() {
    return Ok(lines);
  }
  delete(s, &doomed.into_iter().map(|d| d.id).collect::<Vec<_>>())?;
  honour(s).await;
  // The others purge them as soon as they hear of it.
  let s = s.clone();
  tokio::spawn(async move {
    crate::node::sync_all(&s, None).await;
  });
  Ok(lines)
}

/// Writes the tombstones: a `deletion` record by the user in each
/// thread's repository, naming the threads deleted there.
fn delete(s: &Shared, threads: &[RecordId]) -> Result<()> {
  let mut by_root: Vec<(stitch_store::RecordRoot, Vec<RecordId>)> = Vec::new();
  for t in threads {
    let (root, _) = s.thread_place(t)?;
    match by_root.iter_mut().find(|(r, _)| r.repo() == root.repo()) {
      Some((_, ts)) => ts.push(t.clone()),
      None => by_root.push((root, vec![t.clone()])),
    }
  }
  let state = s.state();
  for (root, ts) in by_root {
    let mut draft = Draft::new(kinds::DELETION, Author::user(state.node.clone()))?;
    for t in ts {
      draft = draft.reference("deletes", Ref::to(t))?;
    }
    root.publish(&state.node, draft)?;
    state.index.lock().expect("index lock").refresh(&root)?;
  }
  Ok(())
}

/// Honours the deletions this node knows of and has not carried out yet
/// (after a sync, at the start, after the user deleted here): what runs on
/// them stops, then they are purged, memory forgets them, their schedules
/// end.
pub(crate) async fn honour(s: &Arc<Shared>) {
  let state = s.state().clone();
  let pending: BTreeSet<RecordId> = {
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
      let index = state.index.lock().expect("index lock");
      let roots = state.roots();
      index
        .deleted_threads()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
          index.record(t).ok().flatten().is_some()
            || !index.thread_segments(t).unwrap_or_default().is_empty()
            || roots.iter().any(|r| {
              !r.purged().contains(t.as_str()) || r.dir().join("journal").join(t.as_str()).exists()
            })
        })
        .collect()
    })
    .await
    .unwrap_or_default()
  };
  if pending.is_empty() {
    return;
  }
  let mut ran = false;
  for t in &pending {
    ran |= crate::detached::stop_deleted(s, t);
  }
  if ran {
    // Its loops unwind; what they write meanwhile is refused or purged.
    tokio::time::sleep(Duration::from_millis(500)).await;
  }
  let more = s.memory.records_only_of(&state, &pending);
  let purged = {
    let (state, pending, more) = (state.clone(), pending.clone(), more.clone());
    tokio::task::spawn_blocking(move || {
      let mut index = state.index.lock().expect("index lock");
      stitch_store::purge::purge(&mut index, &state.roots(), &state.blobs, &pending, &more)
    })
    .await
  };
  match purged {
    Ok(Ok(p)) => eprintln!(
      "deleted {} thread(s): {} file(s), {} blob(s) removed",
      pending.len(),
      p.files,
      p.blobs
    ),
    Ok(Err(e)) => eprintln!("deleting threads: {e}"),
    Err(e) => eprintln!("deleting threads: {e}"),
  }
  s.memory.forget(&pending, &more);
  s.schedules.changed();
}

#[cfg(test)]
mod tests {
  use super::*;
  use stitch_records::NodeName;
  use stitch_store::RecordRoot;

  fn thread(root: &RecordRoot, node: &NodeName, title: &str, from: Option<&RecordId>) -> RecordId {
    let mut d = Draft::new(kinds::THREAD, Author::user(node.clone())).unwrap();
    if let Some(old) = from {
      d = d
        .field("compacted", true)
        .unwrap()
        .reference("continues", Ref::to(old.clone()))
        .unwrap();
    }
    root
      .publish(node, d.body(format!("# {title}\n")))
      .unwrap()
      .id
  }

  fn roles(index: &Index, named: &[&RecordId]) -> Vec<(RecordId, &'static str)> {
    let named: Vec<String> = named.iter().map(|t| t.to_string()).collect();
    resolve(index, &named, None)
      .unwrap()
      .into_iter()
      .map(|d| (d.id, d.role))
      .collect()
  }

  #[test]
  fn a_deletion_says_which_parts_of_the_conversation_go_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = RecordRoot::init(
      dir.path(),
      "home".parse().unwrap(),
      stitch_store::Role::Home,
    )
    .unwrap();
    let node: NodeName = "gamer".parse().unwrap();
    // A conversation compacted twice, a fork of it (not compacted), and
    // another conversation.
    let a = thread(&root, &node, "Talk", None);
    let b = thread(&root, &node, "Talk", Some(&a));
    let c = thread(&root, &node, "Talk", Some(&b));
    let mut fork = Draft::new(kinds::THREAD, Author::user(node.clone())).unwrap();
    fork = fork.reference("continues", Ref::to(c.clone())).unwrap();
    let fork = root.publish(&node, fork.body("# Fork\n")).unwrap().id;
    let other = thread(&root, &node, "Other", None);
    let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();

    // The middle: what it came from is earlier, what it went on in later;
    // the fork is not taken.
    assert_eq!(
      roles(&index, &[&b]),
      vec![
        (a.clone(), "earlier"),
        (b.clone(), "this"),
        (c.clone(), "later")
      ]
    );
    assert_eq!(
      roles(&index, &[&c]),
      vec![
        (a.clone(), "earlier"),
        (b.clone(), "earlier"),
        (c.clone(), "this")
      ]
    );
    assert_eq!(roles(&index, &[&fork]), vec![(fork.clone(), "this")]);
    // Two named in one chain: each is `this`, whatever the order.
    assert_eq!(
      roles(&index, &[&a, &c]),
      vec![
        (a.clone(), "this"),
        (b.clone(), "later"),
        (c.clone(), "this")
      ]
    );
    assert_eq!(
      roles(&index, &[&c, &a]),
      vec![
        (a.clone(), "this"),
        (b.clone(), "earlier"),
        (c.clone(), "this")
      ]
    );
    assert_eq!(roles(&index, &[&other]), vec![(other, "this")]);
    // Titles come with them.
    let d = resolve(&index, &[b.to_string()], None).unwrap();
    assert_eq!(d[0].title, "Talk");
  }
}
