//! The index's `digests` table (design §2.25, step 2): the latest digest
//! of each thread, filled as its `digest` records are indexed, replaced by
//! a newer one (and not by an older one indexed later), counted by kind,
//! and gone with the thread when it is deleted.

use std::collections::BTreeSet;

use jiff::Timestamp;
use stitch_records::{Author, Draft, NodeName, RecordId, Ref, kinds};
use stitch_store::{BlobStore, DIGEST_KIND, Index, RecordRoot, Role, purge};

fn items(kinds: &[&str]) -> toml::Value {
  toml::Value::Array(
    kinds
      .iter()
      .map(|k| {
        let mut t = toml::map::Map::new();
        t.insert("kind".into(), (*k).into());
        t.insert("text".into(), "x".into());
        t.insert("at".into(), "t#m1".into());
        toml::Value::Table(t)
      })
      .collect(),
  )
}

#[test]
fn the_latest_digest_of_a_thread_is_one_row_and_goes_with_the_thread() {
  let dir = tempfile::tempdir().unwrap();
  let name: NodeName = "mac".parse().unwrap();
  let root = RecordRoot::init(&dir.path().join("r"), "home".parse().unwrap(), Role::Home).unwrap();
  let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
  let mut index = Index::open_in_memory().unwrap();
  let thread = root
    .publish(
      &name,
      Draft::new(kinds::THREAD, Author::user(name.clone())).unwrap(),
    )
    .unwrap()
    .id;
  let digest = |headline: &str, kinds: &[&str], at: Timestamp, step: Option<&str>| {
    let mut d = Draft::new(DIGEST_KIND, Author::node(name.clone()))
      .unwrap()
      .field("thread", thread.as_str())
      .unwrap()
      .field("upto", format!("{thread}#m2").as_str())
      .unwrap()
      .field("headline", headline)
      .unwrap()
      .field("items", items(kinds))
      .unwrap()
      .reference("thread", Ref::to(thread.clone()))
      .unwrap();
    if let Some(s) = step {
      d = d.field("step", s).unwrap();
    }
    root.publish_at(&name, d, at).unwrap().id
  };
  let t0: Timestamp = "2026-11-11T10:00:00Z".parse().unwrap();
  let first = digest(
    "first",
    &["goal", "ask", "question", "question"],
    t0,
    Some("2026-11-11T09:59:00Z"),
  );
  index.refresh(&root).unwrap();
  let row = index.digest(&thread).unwrap().unwrap();
  assert_eq!(
    (row.headline.as_str(), row.asks, row.questions),
    ("first", 1, 2)
  );
  assert_eq!(row.record, first);
  assert_eq!(row.upto.as_deref(), Some(format!("{thread}#m2").as_str()));
  assert_eq!(row.step, Some("2026-11-11T09:59:00Z".parse().unwrap()));

  // A newer one replaces it; one table row, whatever the updates.
  let second = digest(
    "second",
    &["ask"],
    t0 + jiff::SignedDuration::from_mins(20),
    None,
  );
  index.refresh(&root).unwrap();
  let all = index.digests().unwrap();
  assert_eq!(all.len(), 1);
  assert_eq!(
    (all[0].headline.as_str(), all[0].asks, all[0].questions),
    ("second", 1, 0)
  );
  assert_eq!(all[0].record, second);
  assert_eq!(all[0].step, None);

  // An older one arriving later (synced from another node) does not.
  digest("older", &[], t0 - jiff::SignedDuration::from_mins(5), None);
  index.refresh(&root).unwrap();
  assert_eq!(index.digest(&thread).unwrap().unwrap().headline, "second");
  // A rebuild from the files gives the same row.
  let mut again = Index::open_in_memory().unwrap();
  again.refresh(&root).unwrap();
  assert_eq!(again.digests().unwrap(), index.digests().unwrap());

  // Deleting the thread takes its digests, and their rows.
  let deleted: BTreeSet<RecordId> = [thread.clone()].into();
  purge::purge(
    &mut index,
    std::slice::from_ref(&root),
    &blobs,
    &deleted,
    &[],
  )
  .unwrap();
  assert!(index.digest(&thread).unwrap().is_none());
  assert!(index.digests().unwrap().is_empty());
}
