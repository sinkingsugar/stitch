//! A fork shows what it continues (design §2.15): the history of the
//! thread it was forked from up to the fork point, through that thread's
//! compactions and its own fork, read-only and named by its source, the
//! fork's model context marked, then a `forked` divider and the fork's own
//! messages; nothing shown twice. On the web client's transcript
//! (`Request::Transcript`) and on `chat::lineage_shown` (the TUI's and the
//! app's).

use std::path::Path;
use std::sync::Mutex;

use jiff::Timestamp;
use serde_json::Value;
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config};
use stitch_records::{
  Author, Draft, Event, Msg, NodeName, Part, RecordId, Ref, Role as Who, kinds,
};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

fn setup(data: &Path, home: &Path) -> RecordRoot {
  let c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  c.save(data).unwrap();
  let root = RecordRoot::open(home)
    .or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home))
    .unwrap();
  refresh(data, &root);
  root
}

fn refresh(data: &Path, root: &RecordRoot) {
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(root)
    .unwrap();
}

fn msg(id: &str, role: Who, t: &str, text: &str) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: format!("2026-11-24T{t}Z").parse().unwrap(),
    role,
    from: None,
    model: (role == Who::Assistant).then(|| "anthropic/claude-opus-5-5".into()),
    parts: vec![Part::Text { text: text.into() }],
    native: None,
  })
}

/// Writes a segment of `thread`; its ID.
fn segment(
  root: &RecordRoot,
  node: &NodeName,
  thread: &RecordId,
  prev: Option<RecordId>,
  events: Vec<Event>,
) -> RecordId {
  root
    .publish_segment(node, thread, prev, None, events)
    .unwrap()
    .header
    .segment
}

async fn fork(c: &mut Client, thread: &RecordId, last: Option<usize>) -> RecordId {
  match c
    .request(&Request::Fork {
      thread: thread.to_string(),
      upto: None,
      last,
      no_attachments: false,
      no_tools: false,
      summarize: false,
    })
    .await
    .unwrap()
  {
    Reply::Forked(id) => id.parse().unwrap(),
    other => panic!("{other:?}"),
  }
}

async fn transcript(c: &mut Client, thread: &RecordId) -> Vec<Value> {
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: thread.to_string(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let v: Value = serde_json::from_str(&l[0]).unwrap();
  v["entries"].as_array().unwrap().clone()
}

/// Each entry as a line: `kind text [id] [<- source] [ctx]`.
fn lines(entries: &[Value]) -> Vec<String> {
  entries
    .iter()
    .map(|e| {
      let kind = e["kind"].as_str().unwrap_or("?");
      let text = match kind {
        "forked" => format!(
          "from {} how {}",
          e["from"].as_str().unwrap(),
          e["how"].as_str().unwrap()
        ),
        "compacted" => "compacted".to_owned(),
        _ => e["text"].as_str().unwrap_or_default().to_owned(),
      };
      let mut l = format!("{kind} {text}");
      if let Some(id) = e["id"].as_str() {
        l.push_str(&format!(" [{id}]"));
      }
      if e["ancestor"] == true && kind != "compacted" {
        l.push_str(&format!(
          " <- {}#{}",
          e["source"]["thread"].as_str().unwrap(),
          e["source"]["id"].as_str().unwrap_or("-")
        ));
      }
      if e["inherited"] == true {
        l.push_str(" ctx");
      }
      l
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_shows_the_history_it_continues() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  std::fs::create_dir_all(&home).unwrap();
  let root = setup(&data, &home);
  let node: NodeName = "mbp".parse().unwrap();

  // A conversation compacted once, keeping its last 2 messages: p1, then
  // p2.
  let p1 = root
    .publish(
      &node,
      Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .body("# Plan the trip\n"),
    )
    .unwrap()
    .id;
  let s1 = segment(
    &root,
    &node,
    &p1,
    None,
    vec![
      msg("m1", Who::User, "09:00:00", "where to?"),
      msg("m2", Who::Assistant, "09:00:05", "Kyoto."),
      msg("m3", Who::User, "09:01:00", "when?"),
      msg("m4", Who::Assistant, "09:01:05", "In April."),
    ],
  );
  let p2 = root
    .publish(
      &node,
      Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .field("compacted", true)
        .unwrap()
        .field("continues_at", s1.as_str())
        .unwrap()
        .field("continues_last", 2)
        .unwrap()
        .field("summary", "They chose Kyoto.")
        .unwrap()
        .reference("continues", Ref::to(p1.clone()))
        .unwrap()
        .body("# ✂ Plan the trip\n"),
    )
    .unwrap()
    .id;
  segment(
    &root,
    &node,
    &p2,
    None,
    vec![
      msg("m1", Who::User, "10:00:00", "hotels?"),
      msg("m2", Who::Assistant, "10:00:05", "Near the station."),
    ],
  );
  refresh(&data, &root);
  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // A fork of the compacted conversation, whole; then its own exchange.
  let f1 = fork(&mut c, &p2, None).await;
  segment(
    &root,
    &node,
    &f1,
    None,
    vec![
      msg("m1", Who::User, "11:00:00", "and food?"),
      msg("m2", Who::Assistant, "11:00:05", "Nishiki market."),
      msg("m3", Who::User, "11:01:00", "budget?"),
      msg("m4", Who::Assistant, "11:01:05", "Modest."),
    ],
  );
  refresh(&data, &root);
  let t = lines(&transcript(&mut c, &f1).await);
  assert_eq!(
    t,
    vec![
      format!("user where to? <- {p1}#m1"),
      format!("assistant Kyoto. <- {p1}#m2"),
      // The fork's model reads the compaction's summary, then from here.
      "context ── the fork's model reads from here on ──".to_owned(),
      format!("user when? <- {p1}#m3 ctx"),
      format!("assistant In April. <- {p1}#m4 ctx"),
      "compacted compacted".to_owned(),
      format!("user hotels? <- {p2}#m1 ctx"),
      format!("assistant Near the station. <- {p2}#m2 ctx"),
      format!("forked from {p2} how full"),
      "user and food? [m1]".to_owned(),
      "assistant Nishiki market. [m2]".to_owned(),
      "user budget? [m3]".to_owned(),
      "assistant Modest. [m4]".to_owned(),
    ],
    "{t:#?}"
  );

  // A fork of the fork, of its last 2 messages: the whole history up to
  // it, through the first fork; its model reads the last 2.
  let f2 = fork(&mut c, &f1, Some(2)).await;
  let t = lines(&transcript(&mut c, &f2).await);
  let forked: Vec<&String> = t.iter().filter(|l| l.starts_with("forked")).collect();
  assert_eq!(
    forked,
    vec![
      &format!("forked from {p2} how full"),
      &format!("forked from {f1} how last")
    ],
    "{t:#?}"
  );
  let ctx: Vec<&String> = t.iter().filter(|l| l.ends_with(" ctx")).collect();
  assert_eq!(
    ctx,
    vec![
      &format!("user budget? <- {f1}#m3 ctx"),
      &format!("assistant Modest. <- {f1}#m4 ctx")
    ],
    "{t:#?}"
  );
  let marker = t.iter().position(|l| l.starts_with("context")).unwrap();
  assert!(t[marker + 1].starts_with("user budget?"), "{t:#?}");
  // Nothing twice, and no ID but the fork's own (it has none yet).
  for said in ["where to?", "budget?", "Near the station."] {
    assert_eq!(
      t.iter().filter(|l| l.contains(said)).count(),
      1,
      "{said}: {t:#?}"
    );
  }
  assert!(!t.iter().any(|l| l.contains(" [m")), "{t:#?}");
  assert!(t.last().unwrap().starts_with("forked"), "{t:#?}");

  // The same on `chat::lineage_shown` (the TUI's and the app's).
  {
    let index = Mutex::new(Index::open(&data.join("index.sqlite")).unwrap());
    let blobs = BlobStore::open(&data.join("blobs")).unwrap();
    let ctx = stitch_node::chat::ChatContext {
      node: &node,
      root: &root,
      index: &index,
      blobs: &blobs,
    };
    let shown = stitch_node::chat::lineage_shown(&ctx, &f1, None).unwrap();
    let whos: Vec<&str> = shown.iter().map(|e| e.who.as_str()).collect();
    assert_eq!(
      whos,
      [
        "you",
        "anthropic/claude-opus-5-5",
        "context",
        "you",
        "anthropic/claude-opus-5-5",
        "compacted",
        "you",
        "anthropic/claude-opus-5-5",
        "forked",
        "you",
        "anthropic/claude-opus-5-5",
        "you",
        "anthropic/claude-opus-5-5"
      ],
      "{shown:#?}"
    );
    let first = &shown[0];
    assert!(first.ancestor && first.id.is_none() && !first.context);
    assert_eq!(first.source, Some((p1.clone(), Some("m1".to_owned()))));
    assert!(shown[4].context);
    assert!(shown[5].compaction.is_some());
    let f = shown[8].fork.as_ref().unwrap();
    assert_eq!((f.from.as_str(), f.how.as_str()), (p2.as_str(), "full"));
    assert_eq!(shown[9].id.as_deref(), Some("m1"));
    assert!(!shown[9].ancestor);
    // A thread that is no fork shows as it did.
    let plain = stitch_node::chat::lineage_shown(&ctx, &p1, None).unwrap();
    assert_eq!(plain.len(), 4);
    assert!(plain.iter().all(|e| !e.ancestor));
  }

  drop(c);
  node_handle.stop().await.unwrap();
  let _ = Timestamp::now();
}
