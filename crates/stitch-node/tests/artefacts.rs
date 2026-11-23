//! The artefact index (design §2.25, step 2): what the tool calls of sealed
//! segments touched, read through `Request::Artefacts` (a conversation's,
//! over its compaction chain) and `Request::Touched` (every conversation that
//! touched one thing, chains folded, deleted threads left out).

use std::path::Path;

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config};
use stitch_records::{
  Author, Draft, Event, Msg, NodeName, Part, RecordId, Ref, Role as Who, ToolCall, ToolResult,
  kinds,
};
use stitch_store::{Index, RecordRoot, Role};

fn at(s: &str) -> Timestamp {
  format!("2026-11-11T{s}Z").parse().unwrap()
}

fn msg(id: &str, role: Who, t: &str) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: at(t),
    role,
    from: None,
    model: (role == Who::Assistant).then(|| "m".into()),
    parts: vec![Part::Text { text: "x".into() }],
    native: None,
  })
}

/// A call and its result, one second apart.
fn call(id: &str, t: &str, name: &str, input: Value, ok: bool, text: &str) -> Vec<Event> {
  let end = at(t)
    .checked_add(jiff::SignedDuration::from_secs(1))
    .unwrap();
  vec![
    Event::ToolCall(ToolCall {
      id: id.into(),
      at: at(t),
      name: name.into(),
      input,
    }),
    Event::ToolResult(ToolResult {
      id: id.into(),
      at: end,
      ok,
      output: json!({ "text": text }),
    }),
  ]
}

fn thread(
  root: &RecordRoot,
  node: &NodeName,
  title: &str,
  compacts: Option<&RecordId>,
  events: Vec<Event>,
) -> RecordId {
  let mut d = Draft::new(kinds::THREAD, Author::user(node.clone())).unwrap();
  if let Some(old) = compacts {
    d = d
      .field("compacted", true)
      .unwrap()
      .reference("continues", Ref::to(old.clone()))
      .unwrap();
  }
  let id = root
    .publish(node, d.body(format!("# {title}\n")))
    .unwrap()
    .id;
  root.publish_segment(node, &id, None, None, events).unwrap();
  id
}

async fn lines(c: &mut Client, r: Request) -> Value {
  let Reply::Lines(l) = c.request(&r).await.unwrap() else {
    panic!("not lines")
  };
  serde_json::from_str(&l[0]).unwrap()
}

fn values(group: &Value) -> Vec<(String, String, u64)> {
  group
    .as_array()
    .unwrap()
    .iter()
    .map(|i| {
      (
        i["value"].as_str().unwrap().to_owned(),
        i["verb"].as_str().unwrap().to_owned(),
        i["n"].as_u64().unwrap(),
      )
    })
    .collect()
}

fn s(a: &str, b: &str, n: u64) -> (String, String, u64) {
  (a.to_owned(), b.to_owned(), n)
}

fn setup(data: &Path, home: &Path, port: u16) {
  let c = Config {
    node: Some("mbp".into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    apps: Some(AppsSettings { port: Some(port) }),
    ..Config::default()
  };
  c.save(data).unwrap();
  let root = RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn artefacts_are_read_per_conversation_and_per_thing() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  let port = std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port();
  setup(&data, &home, port);
  let root = RecordRoot::open(&home).unwrap();
  let node: NodeName = "mbp".parse().unwrap();
  let sub = "2026-10-12T123254Z-mbp-yhj2";

  // The old thread of a chain: writes, reads, a page, a commit (multi-line
  // output), a spawned thread, and a failed write.
  let mut old = vec![
    msg("m1", Who::User, "10:00:00"),
    msg("m2", Who::Assistant, "10:00:01"),
  ];
  old.extend(call(
    "c1",
    "10:00:02",
    "edit_file",
    json!({"path": "src/a.rs"}),
    true,
    "ok",
  ));
  old.extend(call(
    "c2",
    "10:00:03",
    "view_file",
    json!({"path": "docs/x.md"}),
    true,
    "…",
  ));
  old.extend(call(
    "c3",
    "10:00:04",
    "edit_file",
    json!({"path": "src/other.rs"}),
    false,
    "no match",
  ));
  old.push(msg("m3", Who::Assistant, "10:00:10"));
  old.extend(call(
    "c4",
    "10:00:11",
    "browse_web",
    json!({"url": "https://example.com/p#frag"}),
    true,
    "page",
  ));
  old.extend(call(
    "c5",
    "10:00:12",
    "shell_run",
    json!({"command": "git commit -am fix"}),
    true,
    "exit 0\n\n[main abc1234] fix the thing\n 1 file changed, 2 insertions(+)\n$ ",
  ));
  old.extend(call(
    "c6",
    "10:00:13",
    "spawn",
    json!({"goal": "g"}),
    true,
    &format!("Spawned thread {sub} on mbp. Its report comes back here."),
  ));
  let a = thread(&root, &node, "The talk", None, old);

  // Its compaction: writes the same file again, tells the thread.
  let mut new = vec![
    msg("m1", Who::User, "11:00:00"),
    msg("m2", Who::Assistant, "11:00:01"),
  ];
  new.extend(call(
    "c1",
    "11:00:02",
    "edit_file",
    json!({"path": "src/a.rs"}),
    true,
    "ok",
  ));
  new.extend(call(
    "c2",
    "11:00:03",
    "send",
    json!({"thread": sub, "message": "hi"}),
    true,
    "Sent",
  ));
  let b = thread(&root, &node, "The talk", Some(&a), new);

  // Another conversation reads the file and the page (another fragment);
  // a third, deleted one wrote the file.
  let mut other = vec![
    msg("m1", Who::User, "12:00:00"),
    msg("m2", Who::Assistant, "12:00:01"),
  ];
  other.extend(call(
    "c1",
    "12:00:02",
    "view_file",
    json!({"path": "src/a.rs"}),
    true,
    "…",
  ));
  other.extend(call(
    "c2",
    "12:00:03",
    "browse_web",
    json!({"url": "https://example.com/p#other"}),
    true,
    "p",
  ));
  let c = thread(&root, &node, "Something else", None, other);
  let mut gone = vec![
    msg("m1", Who::User, "13:00:00"),
    msg("m2", Who::Assistant, "13:00:01"),
  ];
  gone.extend(call(
    "c1",
    "13:00:02",
    "edit_file",
    json!({"path": "src/a.rs"}),
    true,
    "ok",
  ));
  let deleted = thread(&root, &node, "Deleted", None, gone);

  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut cl = Client::connect(&data).await.unwrap();
  cl.request(&Request::DeleteThreads {
    threads: vec![deleted.to_string()],
    spawned_by: None,
    dry_run: false,
    from_thread: None,
  })
  .await
  .unwrap();

  // A conversation's artefacts, asked by either of its threads: the same.
  let by_old = lines(
    &mut cl,
    Request::Artefacts {
      thread: a.to_string(),
    },
  )
  .await;
  let by_new = lines(
    &mut cl,
    Request::Artefacts {
      thread: b.to_string(),
    },
  )
  .await;
  assert_eq!(by_old, by_new);
  assert_eq!(by_new["thread"], b.as_str());
  assert_eq!(by_new["chain"], json!([a.as_str(), b.as_str()]));
  let k = &by_new["kinds"];
  assert_eq!(
    values(&k["path"]),
    // The latest first: the compaction's write, then the old reads.
    vec![s("src/a.rs", "wrote", 2), s("docs/x.md", "read", 1)],
    "{k}"
  );
  let w = &k["path"][0];
  assert_eq!(
    w["msg"],
    format!("{b}#m2"),
    "the latest touch's message: {w}"
  );
  assert_eq!(w["first_msg"], format!("{a}#m2"));
  assert_eq!(w["thread"], b.as_str());
  assert_eq!(w["first_at"], "2026-11-11T10:00:02Z");
  assert_eq!(w["last_at"], "2026-11-11T11:00:02Z");
  assert_eq!(k["path"][1]["msg"], format!("{a}#m2"));
  // The failed write left nothing; the fragment is gone from the URL.
  assert!(!k["path"].to_string().contains("other.rs"));
  assert_eq!(
    values(&k["url"]),
    vec![s("https://example.com/p", "read", 1)]
  );
  assert_eq!(k["url"][0]["msg"], format!("{a}#m3"));
  assert_eq!(values(&k["commit"]), vec![s("abc1234", "wrote", 1)]);
  assert_eq!(k["commit"][0]["subject"], "fix the thing");
  assert_eq!(
    values(&k["thread"]),
    vec![s(sub, "told", 1), s(sub, "spawned", 1)],
    "{k}"
  );

  // A thing's conversations: the chain once (folded to its newest thread,
  // both threads' touches counted), the other one, not the deleted one.
  let t = lines(
    &mut cl,
    Request::Touched {
      kind: "path".into(),
      value: "src/a.rs".into(),
    },
  )
  .await;
  assert_eq!(t["kind"], "path");
  let rows = t["conversations"].as_array().unwrap();
  let want = |thread: &RecordId| {
    rows
      .iter()
      .filter(|r| r["thread"] == thread.as_str())
      .count()
  };
  assert_eq!(want(&b) + want(&c), 2, "{t}");
  assert_eq!(want(&a) + want(&deleted), 0, "{t}");
  assert_eq!(rows.len(), 2);
  assert_eq!(rows[0]["thread"], b.as_str(), "in time order: {t}");
  assert_eq!(rows[0]["verb"], "wrote");
  assert_eq!(rows[0]["n"], 2);
  assert_eq!(rows[0]["title"], "The talk");
  assert_eq!(rows[0]["first_at"], "2026-11-11T10:00:02Z");
  assert_eq!(rows[0]["last_at"], "2026-11-11T11:00:02Z");
  assert_eq!(rows[0]["first_msg"], format!("{a}#m2"));
  assert_eq!(rows[0]["msg"], format!("{b}#m2"));
  assert!(rows[0]["place"].is_string());
  assert_eq!(rows[1]["thread"], c.as_str());
  assert_eq!(rows[1]["verb"], "read");
  assert_eq!(rows[1]["title"], "Something else");

  // A URL is asked for with or without a fragment.
  let u = lines(
    &mut cl,
    Request::Touched {
      kind: "url".into(),
      value: "https://example.com/p#zzz".into(),
    },
  )
  .await;
  assert_eq!(u["value"], "https://example.com/p");
  let who: Vec<&str> = u["conversations"]
    .as_array()
    .unwrap()
    .iter()
    .map(|r| r["thread"].as_str().unwrap())
    .collect();
  assert_eq!(who, [b.as_str(), c.as_str()]);
  // A commit, and a thread (spawned by one, told by the other verb of the
  // same conversation: one entry per verb).
  let cm = lines(
    &mut cl,
    Request::Touched {
      kind: "commit".into(),
      value: "abc1234".into(),
    },
  )
  .await;
  assert_eq!(cm["conversations"][0]["subject"], "fix the thing");
  assert_eq!(cm["conversations"][0]["thread"], b.as_str());
  let th = lines(
    &mut cl,
    Request::Touched {
      kind: "thread".into(),
      value: sub.into(),
    },
  )
  .await;
  let verbs: Vec<&str> = th["conversations"]
    .as_array()
    .unwrap()
    .iter()
    .map(|r| r["verb"].as_str().unwrap())
    .collect();
  assert_eq!(verbs, ["spawned", "told"]);
  // Unknown things and kinds.
  let none = lines(
    &mut cl,
    Request::Touched {
      kind: "path".into(),
      value: "nope".into(),
    },
  )
  .await;
  assert!(none["conversations"].as_array().unwrap().is_empty());
  assert!(matches!(
    cl.request(&Request::Touched {
      kind: "bogus".into(),
      value: "x".into()
    })
    .await,
    Err(_) | Ok(Reply::Error(_))
  ));
  assert!(matches!(
    cl.request(&Request::Touched {
      kind: "url".into(),
      value: "back".into()
    })
    .await,
    Err(_) | Ok(Reply::Error(_))
  ));

  // Deleting another takes it out too.
  cl.request(&Request::DeleteThreads {
    threads: vec![c.to_string()],
    spawned_by: None,
    dry_run: false,
    from_thread: None,
  })
  .await
  .unwrap();
  let t = lines(
    &mut cl,
    Request::Touched {
      kind: "path".into(),
      value: "src/a.rs".into(),
    },
  )
  .await;
  assert_eq!(t["conversations"].as_array().unwrap().len(), 1, "{t}");
  node_handle.stop().await.unwrap();
}
