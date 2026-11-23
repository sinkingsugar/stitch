//! The change feed and the kept fingerprints: what comes into a root is
//! told once, with where it came from; another process's indexing is
//! found by polling; the fingerprint follows without a full read.

use std::sync::{Arc, Mutex};

use stitch_records::{Author, Draft, NodeName};
use stitch_store::{Change, Index, RecordRoot, Role};

#[test]
fn changes_are_told_once_and_fingerprints_follow() {
  let dir = tempfile::tempdir().unwrap();
  let node: NodeName = "gamer".parse().unwrap();
  let root = RecordRoot::init(&dir.path().join("r"), "p".parse().unwrap(), Role::Project).unwrap();
  let db = dir.path().join("index.sqlite");
  let mut index = Index::open(&db).unwrap();
  let seen: Arc<Mutex<Vec<Change>>> = Arc::default();
  {
    let seen = seen.clone();
    index
      .changes()
      .subscribe(move |c| seen.lock().unwrap().push(c.clone()));
  }
  let publish = |text: &str| {
    root
      .publish(
        &node,
        Draft::new("note", Author::user(node.clone()))
          .unwrap()
          .body(text),
      )
      .unwrap()
  };

  let empty = index.fingerprint(root.repo()).unwrap();
  assert_eq!(empty.count, 0);
  let a = publish("a");
  index.refresh(&root).unwrap();
  // Nothing new: nothing told.
  index.refresh(&root).unwrap();
  let b = publish("b");
  index.refresh_from(&root, Some("geek")).unwrap();
  {
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].added[0].contains(a.id.as_str()));
    assert_eq!(seen[0].origin, None);
    assert!(seen[1].added[0].contains(b.id.as_str()));
    assert_eq!(seen[1].origin.as_deref(), Some("geek"));
  }
  // Kept up to date as files came, it is what a full read gives.
  let kept = index.fingerprint(root.repo()).unwrap();
  assert_eq!(kept.count, 2);
  assert_eq!(kept, index.items(root.repo()).unwrap().summary());

  // Another process (a command) indexes a record: a poll finds it.
  assert!(index.poll_changes().unwrap().is_empty());
  let c = publish("c");
  {
    let mut other = Index::open(&db).unwrap();
    other.refresh(&root).unwrap();
  }
  // This one's refresh sees nothing new: the index knows the file.
  index.refresh(&root).unwrap();
  assert_eq!(seen.lock().unwrap().len(), 2);
  assert_eq!(index.poll_changes().unwrap(), vec![root.repo().clone()]);
  assert_eq!(seen.lock().unwrap().len(), 3);
  assert_eq!(index.fingerprint(root.repo()).unwrap().count, 3);
  assert!(index.poll_changes().unwrap().is_empty());
  let _ = c;
}
