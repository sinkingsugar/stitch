//! `records_shared`: a kind's records read once and shared until the index
//! changes, by this connection or another; and `attention`'s scan of the
//! `verifies` edges goes through an index.

use std::sync::Arc;

use stitch_records::{Author, Draft, NodeName};
use stitch_store::{Index, RecordRoot, Role};

#[test]
fn shared_records_are_kept_until_the_index_changes() {
  let dir = tempfile::tempdir().unwrap();
  let node: NodeName = "gamer".parse().unwrap();
  let root = RecordRoot::init(&dir.path().join("r"), "p".parse().unwrap(), Role::Project).unwrap();
  let db = dir.path().join("index.sqlite");
  let mut index = Index::open(&db).unwrap();
  let publish = |kind: &str, text: &str| {
    root
      .publish(
        &node,
        Draft::new(kind, Author::user(node.clone()))
          .unwrap()
          .body(text),
      )
      .unwrap()
  };

  publish("note", "a");
  index.refresh(&root).unwrap();
  let first = index.records_shared("note").unwrap();
  assert_eq!(first.len(), 1);
  // Nothing changed: the same list, not read again.
  let again = index.records_shared("note").unwrap();
  assert!(Arc::ptr_eq(&first, &again));
  // Another kind is kept beside it.
  assert!(index.records_shared("other").unwrap().is_empty());
  assert!(Arc::ptr_eq(&first, &index.records_shared("note").unwrap()));

  // A record this connection indexes: read anew.
  publish("note", "b");
  index.refresh(&root).unwrap();
  let after_own = index.records_shared("note").unwrap();
  assert_eq!(after_own.len(), 2);
  assert!(!Arc::ptr_eq(&first, &after_own));

  // One another connection (a command) indexes: read anew too.
  publish("note", "c");
  Index::open(&db).unwrap().refresh(&root).unwrap();
  let after_other = index.records_shared("note").unwrap();
  assert_eq!(after_other.len(), 3);
  assert_eq!(
    after_other.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
    index
      .records(Some("note"))
      .unwrap()
      .into_iter()
      .map(|r| r.id)
      .collect::<Vec<_>>()
  );
}

#[test]
fn verifies_edges_are_found_through_an_index() {
  let dir = tempfile::tempdir().unwrap();
  let db = dir.path().join("index.sqlite");
  drop(Index::open(&db).unwrap());
  let conn = rusqlite::Connection::open(&db).unwrap();
  let plan: Vec<String> = conn
    .prepare("EXPLAIN QUERY PLAN SELECT DISTINCT dst, dst_sub FROM edges WHERE rel = 'verifies' AND dst_sub != ''")
    .unwrap()
    .query_map([], |r| r.get::<_, String>(3))
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap();
  assert!(plan.iter().any(|p| p.contains("edges_rel")), "{plan:?}");
}
