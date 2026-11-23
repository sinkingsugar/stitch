//! Deleting a thread across three nodes of one repository (design §2.5,
//! "Deleting a thread"): deleted on the Mac, purged on the NAS when the
//! tombstone arrives, and not brought back by the phone, which was offline
//! with every record of it and syncs afterwards. Nothing else goes: the
//! thread it spawned, its fork and the child's report to it stay.

use std::collections::BTreeSet;
use std::path::Path;

use stitch_records::{Attachment, Author, Draft, Event, Msg, NodeName, Part, RecordId, Ref};
use stitch_records::{Role as Who, kinds};
use stitch_store::sync::sync_roots;
use stitch_store::{BlobStore, Index, Inserted, ItemPath, RecordRoot, Role, purge};

struct Node {
  name: NodeName,
  root: RecordRoot,
  index: Index,
  blobs: BlobStore,
}

impl Node {
  fn new(dir: &Path, name: &str) -> Self {
    Node {
      name: name.parse().unwrap(),
      root: RecordRoot::init(&dir.join(name), "home".parse().unwrap(), Role::Home).unwrap(),
      index: Index::open_in_memory().unwrap(),
      blobs: BlobStore::open(&dir.join(format!("{name}-blobs"))).unwrap(),
    }
  }

  fn refresh(&mut self) {
    self.index.refresh(&self.root).unwrap();
  }

  /// What `after_sync` does: the deletions known, purged.
  fn honour(&mut self) -> purge::Purged {
    self.refresh();
    let deleted = self.index.deleted_threads().unwrap();
    purge::purge(
      &mut self.index,
      std::slice::from_ref(&self.root),
      &self.blobs,
      &deleted,
      &[],
    )
    .unwrap()
  }

  fn has(&self, id: &RecordId) -> bool {
    self.index.record(id).unwrap().is_some()
  }

  fn segments(&self, thread: &RecordId) -> usize {
    self.index.thread_segments(thread).unwrap().len()
  }

  fn publish(&self, draft: Draft) -> RecordId {
    self.root.publish(&self.name, draft).unwrap().id
  }
}

fn sync(a: &mut Node, b: &mut Node) {
  a.refresh();
  b.refresh();
  let sa = a.index.items(a.root.repo()).unwrap();
  let sb = b.index.items(b.root.repo()).unwrap();
  let report = sync_roots(&a.root, &sa, &b.root, &sb).unwrap();
  assert!(report.conflicts.is_empty());
}

fn msg(id: &str, role: Who, text: &str) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: jiff::Timestamp::now(),
    role,
    from: None,
    model: None,
    parts: vec![Part::Text { text: text.into() }],
    native: None,
  })
}

fn draft(kind: &str, node: &Node) -> Draft {
  Draft::new(kind, Author::user(node.name.clone())).unwrap()
}

#[test]
fn a_deleted_thread_is_purged_everywhere_and_never_comes_back() {
  let dir = tempfile::tempdir().unwrap();
  let mut mac = Node::new(dir.path(), "mac");
  let mut nas = Node::new(dir.path(), "nas");
  let mut phone = Node::new(dir.path(), "phone");

  // The thread, with an attachment of its own and the system prompt
  // another thread shares.
  let own = mac.blobs.put_bytes(b"only the doomed thread's").unwrap();
  let shared = mac.blobs.put_bytes(b"a system prompt").unwrap();
  let att = |hash, name: &str| Attachment {
    hash,
    media_type: "text/plain".into(),
    name: Some(name.into()),
    bytes: 1,
  };
  let thread = mac.publish(
    draft(kinds::THREAD, &mac)
      .body("# doomed\n")
      .attach(att(own, "notes.txt"))
      .attach(att(shared, "system-prompt.md")),
  );
  let other = mac.publish(
    draft(kinds::THREAD, &mac)
      .body("# kept\n")
      .attach(att(shared, "system-prompt.md")),
  );
  let seg = mac
    .root
    .publish_segment(
      &mac.name,
      &thread,
      None,
      None,
      vec![
        msg("m1", Who::User, "a secret plan"),
        msg("m2", Who::Assistant, "noted"),
      ],
    )
    .unwrap();
  // What exists only for it.
  let r = |rel: &str, id: &RecordId| (rel.to_owned(), Ref::to(id.clone()));
  let with = |d: Draft, refs: &[(String, Ref)]| {
    refs
      .iter()
      .fold(d, |d, (rel, t)| d.reference(rel, t.clone()).unwrap())
  };
  let question = mac.publish(with(
    draft(kinds::QUESTION, &mac).body("# may I?\n"),
    &[r("thread", &thread)],
  ));
  let answer = mac.publish(with(draft(kinds::ANSWER, &mac), &[r("answers", &question)]));
  let title = mac.publish(with(
    draft(kinds::SET, &mac)
      .field("field", "title")
      .unwrap()
      .field("value", "doomed")
      .unwrap(),
    &[r("target", &thread), r("supersedes", &thread)],
  ));
  let report = mac.publish(with(
    draft(kinds::REPORT, &mac).body("# done\n"),
    &[r("thread", &thread), r("reports-to", &other)],
  ));
  let wake = mac.publish(with(
    draft(kinds::SCHEDULE, &mac)
      .field("at", "2030-01-01T00:00:00Z")
      .unwrap(),
    &[r("thread", &thread), r("reports-to", &thread)],
  ));
  // What does not: a thread it spawned, its fork, the child's report to it.
  let child = mac.publish(with(
    draft(kinds::THREAD, &mac).body("# child\n"),
    &[r("spawned-by", &thread), r("reports-to", &thread)],
  ));
  mac
    .root
    .publish_segment(
      &mac.name,
      &child,
      None,
      None,
      vec![msg("m1", Who::User, "go")],
    )
    .unwrap();
  let child_report = mac.publish(with(
    draft(kinds::REPORT, &mac).body("# the child's\n"),
    &[r("thread", &child), r("reports-to", &thread)],
  ));
  let fork = mac.publish(with(
    draft(kinds::THREAD, &mac)
      .body("# ⑂ doomed\n")
      .field("continues_at", seg.header.segment.as_str())
      .unwrap(),
    &[r("continues", &thread)],
  ));
  let gone = [&thread, &question, &answer, &title, &report, &wake];
  let kept = [&other, &child, &child_report, &fork];

  // Everyone has everything; then the phone goes offline.
  sync(&mut mac, &mut nas);
  sync(&mut nas, &mut phone);
  phone.refresh();
  nas.refresh();
  assert!(gone.iter().all(|id| phone.has(id) && nas.has(id)));

  // The user deletes it on the Mac.
  let tombstone = mac.publish(with(draft(kinds::DELETION, &mac), &[r("deletes", &thread)]));
  // One written by anything but the user counts for nothing.
  mac.publish(with(
    Draft::new(kinds::DELETION, Author::node(mac.name.clone())).unwrap(),
    &[r("deletes", &other)],
  ));
  let purged = mac.honour();
  assert_eq!(purged.files, gone.len() + 1, "its records and its segment");
  assert_eq!(purged.blobs, 1, "its own blob, not the shared one");
  assert!(!mac.blobs.contains(&own) && mac.blobs.contains(&shared));
  assert!(gone.iter().all(|id| !mac.has(id)), "gone on the Mac");
  assert!(kept.iter().all(|id| mac.has(id)), "nothing else goes");
  assert_eq!(mac.segments(&thread), 0);
  assert_eq!(mac.segments(&child), 1, "the child keeps its transcript");
  assert!(mac.has(&tombstone), "the tombstone is kept");
  assert!(
    !mac
      .root
      .dir()
      .join("threads")
      .join(thread.as_str())
      .exists()
  );

  // The NAS gets the tombstone, and purges.
  sync(&mut mac, &mut nas);
  nas.honour();
  assert!(gone.iter().all(|id| !nas.has(id)), "gone on the NAS");
  assert!(kept.iter().all(|id| nas.has(id)));
  assert_eq!(nas.segments(&thread), 0);

  // The phone comes back with all of it, and something new for it written
  // offline (an answer to its question): the NAS takes none of it.
  let late = phone.publish(with(
    draft(kinds::ANSWER, &phone),
    &[r("answers", &question)],
  ));
  sync(&mut phone, &mut nas);
  nas.refresh();
  assert!(gone.iter().all(|id| !nas.has(id)), "not brought back");
  assert!(!nas.has(&late), "nor what goes with it");
  assert_eq!(nas.segments(&thread), 0);
  // The phone purges too once it has the tombstone.
  phone.honour();
  assert!(gone.iter().all(|id| !phone.has(id)) && !phone.has(&late));
  assert!(kept.iter().all(|id| phone.has(id)));
  // From then on, the three agree.
  sync(&mut phone, &mut nas);
  sync(&mut nas, &mut mac);
  let items = |n: &Node| n.index.items(n.root.repo()).unwrap().items().to_vec();
  mac.refresh();
  nas.refresh();
  phone.refresh();
  assert_eq!(items(&mac), items(&nas));
  assert_eq!(items(&nas), items(&phone));

  // A file of it, offered by hand, is refused; a loop still running here
  // cannot write to it.
  let bytes = std::fs::read(nas.root.path_of(&ItemPath::Record(other.clone()))).unwrap();
  let path = ItemPath::Record(other.clone()).to_string();
  assert_eq!(nas.root.insert(&path, &bytes).unwrap(), Inserted::Present);
  let seg_path = ItemPath::Segment {
    thread: thread.clone(),
    segment: seg.header.segment.clone(),
  };
  assert_eq!(
    nas
      .root
      .insert(&seg_path.to_string(), seg.to_text().as_bytes())
      .unwrap(),
    Inserted::Purged
  );
  assert!(
    nas
      .root
      .append_step(&nas.name, &thread, None, vec![msg("m3", Who::User, "x")])
      .is_err()
  );
  assert!(
    nas
      .root
      .publish_segment(&nas.name, &thread, None, None, vec![])
      .is_err()
  );
  let deleted: BTreeSet<RecordId> = [thread.clone()].into();
  assert_eq!(nas.index.deleted_threads().unwrap(), deleted);
}
