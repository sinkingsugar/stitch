//! docs/records.md §9 across three nodes of one repository: the phone writes
//! a task, the laptop runs it and asks a question, the phone answers, the
//! laptop reports, a third node reviews. Records move by record sync only.

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use stitch_records::{Attachment, Author, Draft, Event, NodeName, RecordId, Ref, Usage};
use stitch_store::sync::{ItemSet, sync_roots};
use stitch_store::{
  AssignmentStatus, AttentionItem, BlobStore, FindingStatus, Index, ItemPath, RecordRoot, Role,
  TaskStatus,
};

fn node(s: &str) -> NodeName {
  s.parse().unwrap()
}

struct Node {
  name: NodeName,
  root: RecordRoot,
  index: Index,
}

impl Node {
  fn new(dir: &Path, name: &str) -> Self {
    let root = RecordRoot::init(&dir.join(name), "stitch".parse().unwrap(), Role::Project).unwrap();
    Node {
      name: node(name),
      root,
      index: Index::open_in_memory().unwrap(),
    }
  }

  fn refresh(&mut self) {
    let r = self.index.refresh(&self.root).unwrap();
    assert!(r.quarantined.is_empty(), "{:?}", r.quarantined);
  }

  fn user(&self) -> Author {
    Author::user(self.name.clone())
  }

  fn status(&mut self, task: &RecordId) -> TaskStatus {
    self.refresh();
    self.index.task_status(task).unwrap()
  }
}

fn sync(a: &mut Node, b: &mut Node) {
  a.refresh();
  b.refresh();
  let sa = a.index.items(a.root.repo()).unwrap();
  let sb = b.index.items(b.root.repo()).unwrap();
  let report = sync_roots(&a.root, &sa, &b.root, &sb).unwrap();
  assert!(report.conflicts.is_empty());
  a.refresh();
  b.refresh();
  assert_eq!(
    a.index.items(a.root.repo()).unwrap().items(),
    b.index.items(b.root.repo()).unwrap().items()
  );
}

#[test]
fn the_worked_example() {
  let dir = tempfile::tempdir().unwrap();
  let mut phone = Node::new(dir.path(), "phone");
  let mut mbp = Node::new(dir.path(), "mbp");
  let mut nas = Node::new(dir.path(), "nas");

  // The phone: a task with a photo, and an assignment.
  let blobs = BlobStore::open(&dir.path().join("phone-blobs")).unwrap();
  let photo = b"not really a HEIC";
  let task = phone
    .root
    .publish(
      &phone.name,
      Draft::new("task", phone.user())
        .unwrap()
        .field("budget", toml::toml! { usd = 5.0 })
        .unwrap()
        .attach(Attachment {
          hash: blobs.put_bytes(photo).unwrap(),
          media_type: "image/heic".into(),
          name: Some("IMG_4121.HEIC".into()),
          bytes: photo.len() as u64,
        })
        .body("# Draft the records format\n\nWrite docs/records.md.\n"),
    )
    .unwrap();
  let assignment = phone
    .root
    .publish(
      &phone.name,
      Draft::new("assignment", Author::node(phone.name.clone()))
        .unwrap()
        .reference("task", Ref::to(task.id.clone()))
        .unwrap(),
    )
    .unwrap();
  assert_eq!(phone.status(&task.id), TaskStatus::Open);
  assert_eq!(
    phone.index.attachments(&task.id).unwrap()[0].media_type,
    "image/heic"
  );

  // The laptop gets them, claims the assignment, starts a run.
  sync(&mut phone, &mut mbp);
  let now = Timestamp::now();
  mbp
    .root
    .publish(
      &mbp.name,
      Draft::new("claim", Author::node(mbp.name.clone()))
        .unwrap()
        .time_field("lease_until", now + SignedDuration::from_hours(2))
        .unwrap()
        .reference("claims", Ref::to(assignment.id.clone()))
        .unwrap(),
    )
    .unwrap();
  let run = mbp
    .root
    .publish(
      &mbp.name,
      Draft::new("run", Author::node(mbp.name.clone()))
        .unwrap()
        .field("worker", toml::toml! { kind = "claude-code" })
        .unwrap()
        .reference("task", Ref::to(task.id.clone()))
        .unwrap()
        .reference("assignment", Ref::to(assignment.id.clone()))
        .unwrap(),
    )
    .unwrap();
  mbp.refresh();
  assert!(matches!(
      mbp.index.assignment_status(&assignment.id, now).unwrap(),
      AssignmentStatus::Claimed { ref node, .. } if node == "mbp"
  ));
  assert!(matches!(
    mbp
      .index
      .assignment_status(&assignment.id, now + SignedDuration::from_hours(3))
      .unwrap(),
    AssignmentStatus::Expired { .. }
  ));
  assert_eq!(mbp.status(&task.id), TaskStatus::Running);

  // Its transcript, with usage, and a question.
  let at = Timestamp::now();
  mbp
    .root
    .publish_segment(
      &mbp.name,
      &run.id,
      None,
      None,
      vec![Event::Usage(Usage {
        at,
        model: "claude-opus-5-5".into(),
        input_tokens: 48_211,
        output_tokens: 6_120,
        cache_read_tokens: Some(40_102),
        cache_write_tokens: None,
        usd: Some(0.41),
      })],
    )
    .unwrap();
  let question = mbp
    .root
    .publish(
      &mbp.name,
      Draft::new("question", Author::run(mbp.name.clone(), run.id.clone()))
        .unwrap()
        .field(
          "choices",
          toml::Value::Array(vec!["Keep TOML".into(), "Switch to YAML".into()]),
        )
        .unwrap()
        .reference("task", Ref::to(task.id.clone()))
        .unwrap()
        .body("# Front matter: TOML or YAML?\n"),
    )
    .unwrap();
  assert_eq!(mbp.status(&task.id), TaskStatus::Waiting);
  assert_eq!(mbp.index.task_usage(&task.id).unwrap().0, 0.41);

  // The phone sees the question in its attention view and answers.
  sync(&mut mbp, &mut phone);
  assert_eq!(
    phone.index.attention().unwrap(),
    vec![AttentionItem::Question {
      id: question.id.clone(),
      title: Some("Front matter: TOML or YAML?".into())
    }]
  );
  phone
    .root
    .publish(
      &phone.name,
      Draft::new("answer", phone.user())
        .unwrap()
        .field("choice", "Keep TOML")
        .unwrap()
        .reference("answers", Ref::to(question.id.clone()))
        .unwrap(),
    )
    .unwrap();
  assert_eq!(phone.status(&task.id), TaskStatus::Running);
  assert!(phone.index.attention().unwrap().is_empty());

  // The laptop reports; a third node reviews; a resolution and a failed
  // verification follow.
  sync(&mut phone, &mut mbp);
  let report = mbp
    .root
    .publish(
      &mbp.name,
      Draft::new("report", Author::run(mbp.name.clone(), run.id.clone()))
        .unwrap()
        .field("final", true)
        .unwrap()
        .reference("task", Ref::to(task.id.clone()))
        .unwrap()
        .reference("run", Ref::to(run.id.clone()))
        .unwrap(),
    )
    .unwrap();
  assert_eq!(mbp.status(&task.id), TaskStatus::ToReview);
  sync(&mut mbp, &mut nas);
  let review = nas
    .root
    .publish(
      &nas.name,
      Draft::new("review", Author::node(nas.name.clone()))
        .unwrap()
        .reference("reviews", Ref::to(report.id.clone()))
        .unwrap()
        .body("# Review\n\n## F1 — P2 A `set` on a record the node does not hold\n"),
    )
    .unwrap();
  let f1 = Ref::item(review.id.clone(), "F1").unwrap();
  nas.refresh();
  assert_eq!(nas.index.finding_status(&f1).unwrap(), FindingStatus::Open);
  nas
    .root
    .publish(
      &nas.name,
      Draft::new("resolution", Author::node(nas.name.clone()))
        .unwrap()
        .field("disposition", "fixed")
        .unwrap()
        .reference("addresses", f1.clone())
        .unwrap(),
    )
    .unwrap();
  nas.refresh();
  assert_eq!(
    nas.index.finding_status(&f1).unwrap(),
    FindingStatus::Addressed
  );
  nas
    .root
    .publish(
      &nas.name,
      Draft::new("verification", Author::node(nas.name.clone()))
        .unwrap()
        .field("result", "still-failing")
        .unwrap()
        .reference("verifies", f1.clone())
        .unwrap(),
    )
    .unwrap();
  sync(&mut nas, &mut phone);
  assert_eq!(
    phone.index.finding_status(&f1).unwrap(),
    FindingStatus::StillFailing
  );
  assert_eq!(
    phone.index.attention().unwrap(),
    vec![AttentionItem::Finding {
      finding: f1.clone()
    }]
  );

  // The user closes the task from the phone.
  phone
    .root
    .publish(
      &phone.name,
      Draft::new("close", phone.user())
        .unwrap()
        .field("result", "done")
        .unwrap()
        .reference("closes", Ref::to(task.id.clone()))
        .unwrap(),
    )
    .unwrap();
  assert_eq!(phone.status(&task.id), TaskStatus::Done);

  // Everyone converges.
  sync(&mut phone, &mut mbp);
  sync(&mut mbp, &mut nas);
  sync(&mut nas, &mut phone);
  for n in [&mut phone, &mut mbp, &mut nas] {
    assert_eq!(n.status(&task.id), TaskStatus::Done);
  }
  // The graph: the review points at the report, the resolution at F1.
  let to_report = phone.index.edges_to(&report.id).unwrap();
  assert!(
    to_report
      .iter()
      .any(|e| e.rel == "reviews" && e.src == review.id)
  );
  let to_review = phone.index.edges_to(&review.id).unwrap();
  assert!(
    to_review
      .iter()
      .any(|e| e.rel == "addresses" && e.dst.sub.as_deref() == Some("F1"))
  );
}

#[test]
fn concurrent_sets_resolve_to_the_latest_and_say_so() {
  let dir = tempfile::tempdir().unwrap();
  let mut phone = Node::new(dir.path(), "phone");
  let mut mbp = Node::new(dir.path(), "mbp");
  let task = phone
    .root
    .publish(
      &phone.name,
      Draft::new("task", phone.user())
        .unwrap()
        .body("# Old title\n"),
    )
    .unwrap();
  sync(&mut phone, &mut mbp);
  let set = |n: &Node, title: &str, at: Timestamp| {
    n.root
      .publish_at(
        &n.name,
        Draft::new("set", n.user())
          .unwrap()
          .field("field", "title")
          .unwrap()
          .field("value", title)
          .unwrap()
          .reference("target", Ref::to(task.id.clone()))
          .unwrap()
          .reference("supersedes", Ref::to(task.id.clone()))
          .unwrap(),
        at,
      )
      .unwrap()
  };
  let t0 = Timestamp::now();
  // Offline on both: each supersedes the creation record.
  let first = set(&phone, "From the phone", t0);
  let later = set(&mbp, "From the laptop", t0 + SignedDuration::from_secs(5));
  sync(&mut phone, &mut mbp);
  let v = phone
    .index
    .current_value(&task.id, "title")
    .unwrap()
    .unwrap();
  assert_eq!(v.value, "From the laptop");
  assert_eq!(v.record, later.id);
  assert!(v.conflict);
  // An edit that supersedes only the winner leaves the other head.
  let partial = phone
    .root
    .publish(
      &phone.name,
      Draft::new("set", phone.user())
        .unwrap()
        .field("field", "title")
        .unwrap()
        .field("value", "Still split")
        .unwrap()
        .reference("target", Ref::to(task.id.clone()))
        .unwrap()
        .reference("supersedes", Ref::to(later.id.clone()))
        .unwrap(),
    )
    .unwrap();
  phone.refresh();
  let v = phone
    .index
    .current_value(&task.id, "title")
    .unwrap()
    .unwrap();
  assert_eq!(v.record, partial.id);
  assert!(v.conflict);
  // An edit naming every head it replaces ends the conflict.
  let resolved = phone
    .root
    .publish(
      &phone.name,
      Draft::new("set", phone.user())
        .unwrap()
        .field("field", "title")
        .unwrap()
        .field("value", "Agreed")
        .unwrap()
        .reference("target", Ref::to(task.id.clone()))
        .unwrap()
        .reference("supersedes", Ref::to(partial.id.clone()))
        .unwrap()
        .reference("supersedes", Ref::to(first.id.clone()))
        .unwrap(),
    )
    .unwrap();
  phone.refresh();
  let v = phone
    .index
    .current_value(&task.id, "title")
    .unwrap()
    .unwrap();
  assert_eq!(v.record, resolved.id);
  assert!(!v.conflict);
  assert_eq!(
    phone.index.title(&task.id).unwrap().as_deref(),
    Some("Agreed")
  );
}

#[test]
fn a_reset_index_is_rebuilt_in_place_while_another_connection_reads_it() {
  let dir = tempfile::tempdir().unwrap();
  let mut a = Node::new(dir.path(), "a");
  let rec = a
    .root
    .publish(
      &a.name,
      Draft::new("task", a.user()).unwrap().body("# Body\n"),
    )
    .unwrap();
  let file = dir.path().join("index.sqlite");
  // `stitch reindex` with the node (or a TUI) holding the file open.
  let mut reindex = Index::open(&file).unwrap();
  let reader = Index::open(&file).unwrap();
  reindex.refresh(&a.root).unwrap();
  assert!(reader.record(&rec.id).unwrap().is_some());
  reindex.reset().unwrap();
  assert!(reader.record(&rec.id).unwrap().is_none());
  reindex.refresh(&a.root).unwrap();
  assert!(reader.record(&rec.id).unwrap().is_some());
  // No file was replaced: the reader's connection is the same database.
  assert!(file.exists());
  a.refresh();
}

#[test]
fn a_corrupt_file_is_quarantined_and_fetched_again() {
  let dir = tempfile::tempdir().unwrap();
  let mut a = Node::new(dir.path(), "a");
  let mut b = Node::new(dir.path(), "b");
  let rec = a
    .root
    .publish(
      &a.name,
      Draft::new("task", a.user()).unwrap().body("# Body\n"),
    )
    .unwrap();
  sync(&mut a, &mut b);
  // A crash left b's copy truncated, before b indexed it again.
  let mut fresh = Node {
    name: b.name.clone(),
    root: b.root.clone(),
    index: Index::open_in_memory().unwrap(),
  };
  let path = b.root.path_of(&ItemPath::Record(rec.id.clone()));
  std::fs::write(&path, "+++\nv = 1\n").unwrap();
  let report = fresh.index.refresh(&fresh.root).unwrap();
  assert_eq!(report.quarantined.len(), 1);
  assert!(!path.exists());
  sync(&mut a, &mut fresh);
  assert!(fresh.root.read_record(&rec.id).is_ok());
}

#[test]
fn mentions_in_bodies_become_derived_edges() {
  let dir = tempfile::tempdir().unwrap();
  let mut a = Node::new(dir.path(), "a");
  let t = a
    .root
    .publish(&a.name, Draft::new("task", a.user()).unwrap())
    .unwrap();
  let note = a
    .root
    .publish(
      &a.name,
      Draft::new("note", a.user())
        .unwrap()
        .body(format!("See stitch:{}, and stitch:not-an-id.\n", t.id)),
    )
    .unwrap();
  a.refresh();
  let edges = a.index.edges_from(&note.id).unwrap();
  assert_eq!(edges.len(), 1);
  assert_eq!(edges[0].rel, "mentions");
  assert!(edges[0].derived);
  assert_eq!(edges[0].dst.id, t.id);
}

#[test]
fn the_index_follows_files_removed_from_the_root() {
  let dir = tempfile::tempdir().unwrap();
  let mut a = Node::new(dir.path(), "a");
  let t = a
    .root
    .publish(&a.name, Draft::new("task", a.user()).unwrap())
    .unwrap();
  a.refresh();
  assert!(a.index.record(&t.id).unwrap().is_some());
  std::fs::remove_file(a.root.path_of(&ItemPath::Record(t.id.clone()))).unwrap();
  let r = a.index.refresh(&a.root).unwrap();
  assert_eq!(r.removed, 1);
  assert!(a.index.record(&t.id).unwrap().is_none());
  assert!(a.index.items(a.root.repo()).unwrap().is_empty());
  // The index and a full scan agree on the item set.
  a.root
    .publish(&a.name, Draft::new("task", a.user()).unwrap())
    .unwrap();
  a.refresh();
  assert_eq!(
    a.index.items(a.root.repo()).unwrap().items(),
    ItemSet::from_root(&a.root).unwrap().items()
  );
}
