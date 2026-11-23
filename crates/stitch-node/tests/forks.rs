//! A thread continued on two nodes from different steps (design §2.4):
//! its history follows the continuation written first, whichever was
//! written last, and is told what the other one did.

use std::sync::Mutex;

use stitch_node::chat::{self, ChatContext};
use stitch_records::{Author, Draft, Event, Msg, NodeName, Part, RecordId, Role as Who, kinds};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

fn msg(id: &str, role: Who, text: &str) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: jiff::Timestamp::now(),
    role,
    from: None,
    model: (role == Who::Assistant).then(|| "m".into()),
    parts: vec![Part::Text { text: text.into() }],
    native: None,
  })
}

#[test]
fn a_stale_continuation_does_not_take_a_thread_over() {
  let dir = tempfile::tempdir().unwrap();
  let root = RecordRoot::init(
    &dir.path().join("home"),
    "home".parse().unwrap(),
    Role::Home,
  )
  .unwrap();
  let (linux, mbp): (NodeName, NodeName) = ("linux".parse().unwrap(), "mbp".parse().unwrap());
  let thread: RecordId = root
    .publish(
      &linux,
      Draft::new(kinds::THREAD, Author::user(linux.clone()))
        .unwrap()
        .body("# a thread\n"),
    )
    .unwrap()
    .id;
  let step = |node: &NodeName, prev: Option<&RecordId>, events: Vec<Event>| {
    std::thread::sleep(std::time::Duration::from_millis(5));
    root
      .publish_segment(node, &thread, prev.cloned(), None, events)
      .unwrap()
      .header
      .segment
  };
  // Started on the Linux box, then used on the Mac all evening.
  let s0 = step(
    &linux,
    None,
    vec![
      msg("m1", Who::User, "start"),
      msg("m2", Who::Assistant, "started"),
    ],
  );
  let s1 = step(
    &mbp,
    Some(&s0),
    vec![
      msg("m3", Who::User, "evening work"),
      msg("m4", Who::Assistant, "did the evening work"),
    ],
  );
  let _s2 = step(
    &mbp,
    Some(&s1),
    vec![
      msg("m5", Who::User, "more"),
      msg("m6", Who::Assistant, "did more on the Mac"),
    ],
  );
  // Next morning the Linux box, its copy behind, is woken and continues from s0
  // (a later second: IDs alone would put it last).
  std::thread::sleep(std::time::Duration::from_millis(1100));
  let _t1 = step(
    &linux,
    Some(&s0),
    vec![
      msg("m3", Who::User, "[Report r from mbp] late"),
      msg("m4", Who::Assistant, "ran a docs pass from a stale copy"),
    ],
  );
  let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
  index.refresh(&root).unwrap();
  let index = Mutex::new(index);
  let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
  let ctx = ChatContext {
    node: &mbp,
    root: &root,
    index: &index,
    blobs: &blobs,
  };
  let h = chat::history(&ctx, &thread).unwrap();
  let shown = format!("{:?}", h.messages);
  assert!(
    shown.contains("did more on the Mac"),
    "the line in use: {shown}"
  );
  assert!(!shown.contains("stale copy"), "not the stale one: {shown}");
  assert!(h.forked);
  // What the other continuation did is said, with where and when.
  let note = h.forks.expect("a note on the fork");
  assert!(note.contains("on linux"), "{note}");
  assert!(note.contains("ran a docs pass from a stale copy"), "{note}");
  assert!(note.contains(&format!("after step {s0}")), "{note}");
  // Every node finds the same line: the transcript too.
  let t = chat::transcript(&ctx, &thread).unwrap();
  assert!(
    t.iter().any(|(_, text)| text == "did more on the Mac"),
    "{t:?}"
  );
}

#[test]
fn a_continuation_that_died_off_is_left_behind() {
  let ids: Vec<RecordId> = (0..6)
    .map(|i| format!("2026-10-12T13350{i}Z-mbp-aaaa").parse().unwrap())
    .collect();
  // 0 ← 1 ← 2 (written first, then nothing more) and 1 ← 3 ← 4 ← 5.
  let segments = [
    (&ids[0], None),
    (&ids[1], Some(&ids[0])),
    (&ids[2], Some(&ids[1])),
    (&ids[3], Some(&ids[1])),
    (&ids[4], Some(&ids[3])),
    (&ids[5], Some(&ids[4])),
  ];
  assert_eq!(chat::main_head(segments), Some(ids[5].clone()));
  // As long: the first written.
  let even = [
    (&ids[0], None),
    (&ids[1], Some(&ids[0])),
    (&ids[2], Some(&ids[0])),
  ];
  assert_eq!(chat::main_head(even), Some(ids[1].clone()));
  // A segment whose predecessor is not here yet starts a line.
  let gap = [(&ids[3], Some(&ids[1])), (&ids[4], Some(&ids[3]))];
  assert_eq!(chat::main_head(gap), Some(ids[4].clone()));
}
