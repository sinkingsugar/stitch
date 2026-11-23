//! A transcript for clients that show attachments (the app): each message
//! with its ID and what it carries, its text without the lines that name
//! them, which the TUI's text keeps.

use std::sync::Mutex;

use stitch_node::chat::{self, ChatContext};
use stitch_records::{Attachment, Author, Draft, Event, Msg, NodeName, Part, Role as Who, kinds};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

#[test]
fn messages_carry_their_ids_and_attachments() {
  let dir = tempfile::tempdir().unwrap();
  let root = RecordRoot::init(
    &dir.path().join("home"),
    "home".parse().unwrap(),
    Role::Home,
  )
  .unwrap();
  let node: NodeName = "mbp".parse().unwrap();
  let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
  let photo = Attachment {
    hash: blobs.put_bytes(b"a cat").unwrap(),
    media_type: "image/png".into(),
    name: Some("cat.png".into()),
    bytes: 5,
  };
  let thread = root
    .publish(
      &node,
      Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body("# a thread\n"),
    )
    .unwrap()
    .id;
  let msg = |id: &str, role: Who, parts: Vec<Part>| {
    Event::Msg(Msg {
      id: id.into(),
      at: jiff::Timestamp::now(),
      role,
      from: None,
      model: (role == Who::Assistant).then(|| "m".into()),
      parts,
      native: None,
    })
  };
  root
    .publish_segment(
      &node,
      &thread,
      None,
      None,
      vec![
        msg(
          "m1",
          Who::User,
          vec![
            Part::Text {
              text: "What is in the photo?".into(),
            },
            Part::Attachment { att: photo.clone() },
          ],
        ),
        msg(
          "m2",
          Who::Assistant,
          vec![Part::Text {
            text: "A cat.".into(),
          }],
        ),
      ],
    )
    .unwrap();
  let mut index = Index::open(&dir.path().join("index.sqlite")).unwrap();
  index.refresh(&root).unwrap();
  let index = Mutex::new(index);
  let ctx = ChatContext {
    node: &node,
    root: &root,
    index: &index,
    blobs: &blobs,
  };

  let shown = chat::transcript_shown(&ctx, &thread, None).unwrap();
  assert_eq!(shown.len(), 2, "{shown:?}");
  assert_eq!(shown[0].who, "you");
  assert_eq!(shown[0].id.as_deref(), Some("m1"));
  assert_eq!(shown[0].said, "What is in the photo?");
  assert_eq!(shown[0].text, "What is in the photo?\n[image/png cat.png]");
  assert_eq!(shown[0].attachments, vec![photo]);
  assert_eq!(shown[1].id.as_deref(), Some("m2"));
  assert!(shown[1].attachments.is_empty());

  // The TUI's transcript reads as it did.
  let t = chat::transcript(&ctx, &thread).unwrap();
  assert_eq!(t[0].1, "What is in the photo?\n[image/png cat.png]");
}
