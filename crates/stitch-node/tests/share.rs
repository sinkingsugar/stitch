//! Sharing a thread as an archive (design §2.31): a conversation exported
//! at each level, redacted, its leak findings gating the write, its
//! attachments and app versions carried as blobs; the archive previewed,
//! then imported back on the same node as a foreign, tainted thread whose
//! tool calls reach the model as text; a changed archive refused; an
//! effort imported as threads under one another, which a scope follows.

use std::path::Path;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_node::api::{Reply, Request, ScopeTarget, SecretOp};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_node::share::{Imported, Preview};
use stitch_records::{
  Attachment, Author, Draft, Event, Msg, NodeName, Part, RecordId, Ref, Role as Who, ScopeKind,
  ToolCall, ToolResult, kinds,
};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<Value>>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  tokio::spawn(async move {
    for response in responses {
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = Vec::new();
      let mut chunk = [0u8; 65536];
      loop {
        let n = sock.read(&mut chunk).await.unwrap();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
          let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
          let len: usize = head
            .lines()
            .find_map(|l| {
              l.strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap())
            })
            .unwrap_or(0);
          while buf.len() < end + 4 + len {
            let n = sock.read(&mut chunk).await.unwrap();
            buf.extend_from_slice(&chunk[..n]);
          }
          log
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap());
          break;
        }
      }
      sock.write_all(response.as_bytes()).await.unwrap();
      sock.shutdown().await.unwrap();
    }
  });
  (base, seen)
}

fn answer(text: &str) -> String {
  let events = [
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 80}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ];
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

fn setup(data: &Path, home: &Path, anthropic: &str) {
  let mut c = Config {
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
  c.providers.insert(
    "anthropic".into(),
    ProviderConfig {
      kind: "anthropic".into(),
      base_url: Some(anthropic.into()),
      ..ProviderConfig::default()
    },
  );
  c.save(data).unwrap();
  keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  let root = home_root(home);
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn home_root(home: &Path) -> RecordRoot {
  RecordRoot::open(home)
    .or_else(|_| RecordRoot::init(home, "home".parse().unwrap(), Role::Home))
    .unwrap()
}

fn at(s: &str) -> Timestamp {
  format!("2026-11-11T{s}Z").parse().unwrap()
}

fn msg(id: &str, role: Who, t: &str, parts: Vec<Part>) -> Event {
  Event::Msg(Msg {
    id: id.into(),
    at: at(t),
    role,
    from: None,
    model: (role == Who::Assistant).then(|| "anthropic/claude-opus-5-5".into()),
    parts,
    native: None,
  })
}

fn text(t: &str) -> Part {
  Part::Text { text: t.into() }
}

fn thread(
  root: &RecordRoot,
  node: &NodeName,
  title: &str,
  compacts: Option<&RecordId>,
  spawned_by: Option<&RecordId>,
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
  if let Some(p) = spawned_by {
    d = d
      .field("detached", true)
      .unwrap()
      .field("goal", title)
      .unwrap()
      .reference("spawned-by", Ref::to(p.clone()))
      .unwrap();
  }
  let d = d.body(format!("# {title}\n"));
  let id = root.publish(node, d).unwrap().id;
  root.publish_segment(node, &id, None, None, events).unwrap();
  id
}

async fn lines(c: &mut Client, r: &Request) -> Result<Vec<String>, String> {
  match c.request(r).await {
    Ok(Reply::Lines(l)) => Ok(l),
    Ok(Reply::Error(e)) => Err(e),
    Ok(other) => panic!("{other:?}"),
    Err(e) => Err(e.to_string()),
  }
}

fn share(
  thread: &RecordId,
  effort: bool,
  level: &str,
  preview: bool,
  confirm: Option<&str>,
) -> Request {
  Request::Share {
    thread: thread.to_string(),
    effort,
    level: level.into(),
    preview,
    confirm: confirm.map(str::to_owned),
  }
}

async fn previewed(c: &mut Client, thread: &RecordId, effort: bool, level: &str) -> Preview {
  let l = lines(c, &share(thread, effort, level, true, None))
    .await
    .unwrap();
  serde_json::from_str(&l[0]).unwrap()
}

async fn exported(
  c: &mut Client,
  thread: &RecordId,
  effort: bool,
  level: &str,
) -> (Preview, Vec<u8>) {
  let p = previewed(c, thread, effort, level).await;
  let l = lines(c, &share(thread, effort, level, false, Some(&p.hash)))
    .await
    .unwrap();
  let p: Preview = serde_json::from_str(&l[0]).unwrap();
  let file = std::fs::read(p.path.as_ref().unwrap()).unwrap();
  (p, file)
}

async fn imported(c: &mut Client, file: &[u8]) -> Result<Imported, String> {
  import(c, file, false).await
}

async fn import(c: &mut Client, file: &[u8], preview: bool) -> Result<Imported, String> {
  lines(
    c,
    &Request::Import {
      bundle: file.to_vec(),
      preview,
    },
  )
  .await
  .map(|l| serde_json::from_str(&l[0]).unwrap())
}

async fn transcript(c: &mut Client, thread: &str) -> Vec<Value> {
  let l = lines(
    c,
    &Request::Transcript {
      thread: thread.into(),
      after: None,
      before: None,
      tail: None,
    },
  )
  .await
  .unwrap();
  let t: Value = serde_json::from_str(&l[0]).unwrap();
  t["entries"].as_array().unwrap().clone()
}

/// An archive's entries.
fn unzip(file: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
  let mut zip = zip::ZipArchive::new(std::io::Cursor::new(file)).unwrap();
  (0..zip.len())
    .map(|i| {
      let mut f = zip.by_index(i).unwrap();
      let mut b = Vec::new();
      std::io::Read::read_to_end(&mut f, &mut b).unwrap();
      (f.name().unwrap().into_owned(), b)
    })
    .collect()
}

/// `threads.json`, as the payload.
fn envelope(file: &[u8]) -> Value {
  json!({"payload": serde_json::from_slice::<Value>(&unzip(file)["threads.json"]).unwrap()})
}

/// Every entry's text, for looking for what must not be there.
fn all_text(file: &[u8]) -> String {
  unzip(file)
    .values()
    .map(|b| String::from_utf8_lossy(b).into_owned())
    .collect::<Vec<_>>()
    .join("\n")
}

fn chat(thread: &str, text: &str) -> Request {
  Request::Chat {
    thread: Some(thread.into()),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: None,
    global: true,
    env: Vec::new(),
    tools: Vec::new(),
    skills: Vec::new(),
    place: None,
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

fn tool_names(body: &Value) -> Vec<String> {
  body["tools"]
    .as_array()
    .map(|t| {
      t.iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
    })
    .unwrap_or_default()
}

async fn fork(c: &mut Client, thread: &str, last: Option<usize>) -> String {
  match c
    .request(&Request::Fork {
      thread: thread.into(),
      upto: None,
      last,
      no_attachments: false,
      no_tools: false,
      summarize: false,
    })
    .await
    .unwrap()
  {
    Reply::Forked(id) => id,
    other => panic!("{other:?}"),
  }
}

const SECRET: &str = "hunter2-SHARE-SECRET-91";

#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_goes_out_as_a_file_and_comes_back_foreign() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home) = (d.join("data"), d.join("home"));
  std::fs::create_dir_all(&home).unwrap();
  let root = home_root(&home);
  let node: NodeName = "mbp".parse().unwrap();
  let blobs = BlobStore::open(&data.join("blobs")).unwrap();
  let png = b"\x89PNG\r\n\x1a\n fake image".to_vec();
  let png_hash = blobs.put_bytes(&png).unwrap();
  // An app version: its page, its tree, its manifest (which names the
  // thread it kept state for: an ID with the node's name).
  let page = b"<!doctype html><p>Demo app</p>".to_vec();
  let page_hash = blobs.put_bytes(&page).unwrap();
  let tree = serde_json::to_vec(&json!({"entries": {"index.html": {"t": "file", "hash": page_hash.to_string(), "size": page.len()}}})).unwrap();
  let tree_hash = blobs.put_bytes(&tree).unwrap();
  let app = serde_json::to_vec(&json!({"app": 1, "name": "Demo", "tree": tree_hash.to_string(), "entry": "index.html", "thread": "2026-11-11T100000Z-mbp-aaaa"})).unwrap();
  let app_hash = blobs.put_bytes(&app).unwrap();

  // A conversation compacted once: a picture, a secret, a home path, the
  // node's name, a long command output; then its compaction goes on.
  let first = thread(
    &root,
    &node,
    "Fix the build",
    None,
    None,
    vec![
      msg(
        "m1",
        Who::User,
        "10:00:00",
        vec![
          text("Why does the build fail on mbp? Here is the screen."),
          Part::Attachment {
            att: Attachment {
              hash: png_hash,
              media_type: "image/png".into(),
              name: Some("screen.png".into()),
              bytes: png.len() as u64,
            },
          },
          Part::Recalled {
            recalled: "the user's private memory".into(),
            handles: vec![],
            measures: None,
          },
        ],
      ),
      Event::Msg(Msg {
        native: Some(json!({"provider": "anthropic", "model": "x", "content": [
          {"type": "thinking", "thinking": "private thoughts", "signature": "SIG-BOUND-TO-ACCOUNT"}
        ]})),
        ..match msg("m2", Who::Assistant, "10:00:05", vec![text("Let me look.")]) {
          Event::Msg(m) => m,
          _ => unreachable!(),
        }
      }),
      Event::ToolCall(ToolCall {
        id: "c1".into(),
        at: at("10:00:06"),
        name: "shell_run".into(),
        input: json!({"command": format!("cat /home/alice/build.log; echo {SECRET}")}),
      }),
      Event::ToolResult(ToolResult {
        id: "c1".into(),
        at: at("10:00:07"),
        ok: true,
        output: json!({"text": format!("error: missing crate\nline two\nline three {SECRET}")}),
      }),
      msg(
        "m3",
        Who::Assistant,
        "10:00:08",
        vec![text("A crate is missing.")],
      ),
    ],
  );
  let second = thread(
    &root,
    &node,
    "✂ Fix the build",
    Some(&first),
    None,
    vec![
      msg("m1", Who::User, "11:00:00", vec![text("Add it then.")]),
      msg(
        "m2",
        Who::Assistant,
        "11:00:05",
        vec![
          text("Added. Here is an app for it."),
          Part::Attachment {
            att: Attachment {
              hash: app_hash,
              media_type: "application/vnd.stitch.app+json".into(),
              name: Some("Demo".into()),
              bytes: app.len() as u64,
            },
          },
        ],
      ),
    ],
  );
  let child = thread(
    &root,
    &node,
    "Check the lockfile",
    None,
    Some(&first),
    vec![
      msg(
        "m1",
        Who::User,
        "10:30:00",
        vec![text("Check the lockfile")],
      ),
      msg("m2", Who::Assistant, "10:30:05", vec![text("It is fine.")]),
    ],
  );
  let (base, sent) = serve(vec![
    answer("I read the shared conversation."),
    answer("Forked, with tools."),
    answer("Forked, the last messages."),
  ])
  .await;
  setup(&data, &home, &base);
  let node_handle = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  assert!(matches!(
    c.request(&Request::Secret {
      op: SecretOp::Set {
        name: "token".into(),
        scope: "global".into(),
        value: Some(SECRET.into()),
        device_only: None,
        file: false,
      },
      cwd: None,
    })
    .await
    .unwrap(),
    Reply::Ok | Reply::Lines(_) | Reply::Secret(_)
  ));

  // The preview: the whole chain (asked by its first thread), redacted,
  // what would leak found, nothing written.
  let p = previewed(&mut c, &first, false, "calls").await;
  assert_eq!(p.kind, "thread");
  assert_eq!(p.threads, 1);
  assert_eq!(p.calls, 1);
  assert_eq!((p.attachments, p.apps), (2, 1));
  assert!(p.redactions >= 2, "{p:?}");
  assert!(p.path.is_none());
  let has =
    |p: &Preview, kind: &str, t: &str| p.findings.iter().any(|f| f.kind == kind && f.text == t);
  assert!(has(&p, "home path", "/home/alice"), "{:?}", p.findings);
  assert!(has(&p, "node name", "mbp"), "{:?}", p.findings);
  assert_eq!(p.file_name, "fix-the-build.stitch");

  // With findings, not written unless confirmed with the preview's hash.
  let refused = lines(&mut c, &share(&first, false, "calls", false, None)).await;
  assert!(
    refused.as_ref().is_err_and(|e| e.contains("leak scan")),
    "{refused:?}"
  );
  let refused = lines(
    &mut c,
    &share(&first, false, "calls", false, Some("blake3:00")),
  )
  .await;
  assert!(refused.is_err(), "{refused:?}");
  let (p, file) = exported(&mut c, &second, false, "calls").await;
  let text = all_text(&file);
  assert!(p.path.as_deref().unwrap().ends_with("fix-the-build.stitch"));
  assert!(!text.contains(SECRET), "redacted");
  assert!(text.contains("«secret:token»"));
  assert!(!text.contains("private thoughts") && !text.contains("SIG-BOUND"));
  assert!(!text.contains("private memory"));
  assert!(!text.contains(&first.to_string()) && !text.contains(&second.to_string()));
  assert!(text.contains("compacted here"));
  // The app version travels whole: its manifest (without the thread it
  // kept state for), tree and page, as blobs.
  assert_eq!(p.apps, 1);
  let entries = unzip(&file);
  assert!(entries.contains_key(&format!("blobs/{}", page_hash.hex())));
  assert!(entries.contains_key(&format!("blobs/{}", tree_hash.hex())));
  assert!(!entries.contains_key(&format!("blobs/{}", app_hash.hex())));
  assert!(entries.contains_key("index.html") && entries.contains_key("manifest.json"));
  let env = envelope(&file);
  let events = env["payload"]["threads"][0]["events"].as_array().unwrap();
  let result = events.iter().find(|e| e["t"] == "tool_result").unwrap();
  assert_eq!(
    result["output"]["text"],
    "error: missing crate [… 2 more lines not shared]"
  );
  let ids: Vec<&str> = events
    .iter()
    .filter(|e| e["t"] == "msg")
    .map(|e| e["id"].as_str().unwrap())
    .collect();
  assert_eq!(
    ids,
    ["m1", "m2", "m3", "m4", "m5"],
    "renumbered over the chain"
  );

  // Levels: messages carry no calls; full carries the output whole.
  let (pm, fm) = exported(&mut c, &first, false, "messages").await;
  assert_eq!(pm.calls, 0);
  assert!(!envelope(&fm).to_string().contains("tool_call"));
  let (_, ff) = exported(&mut c, &first, false, "full").await;
  let full = envelope(&ff).to_string();
  assert!(full.contains("line three «secret:token»"), "{full}");

  // Back in, on the same node: previewed first (nothing written), then a
  // new thread, foreign and tainted, its blobs in the store.
  let pre = import(&mut c, &file, true).await.unwrap();
  assert!(!pre.written && pre.thread.is_empty());
  assert_eq!((pre.attachments, pre.apps), (2, 1));
  let i = imported(&mut c, &file).await.unwrap();
  assert!(i.written);
  assert!(!i.already);
  assert!(i.mine, "signed by this node's share key");
  assert_eq!(i.threads.len(), 1);
  assert_eq!(i.calls, 1);
  let entries = transcript(&mut c, &i.thread).await;
  assert_eq!(entries[0]["kind"], "note");
  assert!(
    entries[0]["text"]
      .as_str()
      .unwrap()
      .starts_with("⇩ Imported from a shared file"),
    "{entries:?}"
  );
  assert!(
    !entries.iter().any(|e| e["text"]
      .as_str()
      .is_some_and(|t| t.contains("another person shared"))),
    "the model's note is not shown"
  );
  assert!(entries.iter().any(|e| e["kind"] == "tool"));
  assert!(
    entries
      .iter()
      .any(|e| !e["images"].as_array().is_none_or(Vec::is_empty))
  );
  // Again: the same threads.
  let again = imported(&mut c, &file).await.unwrap();
  assert!(again.already);
  assert_eq!(again.thread, i.thread);
  // The app version's manifest as shared (a new blob: without the thread)
  // is in the store, with its files.
  let shared_app: stitch_records::BlobHash = envelope(&file)["payload"]["attachments"]
    .as_array()
    .unwrap()
    .iter()
    .find(|a| a["type"] == "application/vnd.stitch.app+json")
    .unwrap()["hash"]
    .as_str()
    .unwrap()
    .parse()
    .unwrap();
  assert_ne!(shared_app, app_hash);
  let m: Value = serde_json::from_slice(&std::fs::read(blobs.path(&shared_app)).unwrap()).unwrap();
  assert_eq!(m["name"], "Demo");
  assert!(m.get("thread").is_none());
  for h in [&page_hash, &tree_hash] {
    assert!(blobs.contains(h));
  }

  // Continued: the model is told it is foreign, gets the calls as text,
  // and no tools.
  let done = c
    .chat(&chat(&i.thread, "What happened there?"), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  {
    let bodies = sent.lock().unwrap();
    let body = bodies[0].to_string();
    assert!(body.contains("another person shared"), "{body}");
    assert!(
      body.contains("[In the shared conversation, not here: called $ cat"),
      "{body}"
    );
    assert!(
      body.contains("[Its output there: error: missing crate"),
      "{body}"
    );
    assert!(
      !body.contains("\"tool_use\"") && !body.contains("\"tool_result\""),
      "{body}"
    );
    // No tools: an untrusted profile's ceiling keeps only `event_reply`.
    let tools: Vec<&str> = bodies[0]["tools"]
      .as_array()
      .map(|t| t.iter().filter_map(|t| t["name"].as_str()).collect())
      .unwrap_or_default();
    assert!(
      tools.iter().all(|t| *t == "event_reply"),
      "no tools: {tools:?}"
    );
  }

  // Continuing means forking (design §2.31): a fork of the imported thread
  // has the user's usual tools, while its history is still fenced: the
  // import's note, the calls as text.
  let f = fork(&mut c, &i.thread, None).await;
  let done = c
    .chat(&chat(&f, "Go on with the fix."), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  // The fork shows the imported history it continues (design §2.15), as
  // shared, read-only, and a divider saying whose it was.
  let shown = transcript(&mut c, &f).await;
  let divider = shown.iter().position(|e| e["kind"] == "forked").unwrap();
  assert_eq!(shown[divider]["shared"]["from"], json!(i.hash), "{shown:?}");
  assert_eq!(shown[divider]["shared"]["signed"], json!(true));
  assert!(
    shown[divider]["said"]
      .as_str()
      .unwrap()
      .contains("a conversation shared with you")
  );
  let past = &shown[..divider];
  assert!(past.iter().any(|e| e["kind"] == "note"));
  let said: Vec<&Value> = past
    .iter()
    .filter(|e| e["kind"] == "user" || e["kind"] == "assistant")
    .collect();
  assert!(!said.is_empty());
  assert!(
    said
      .iter()
      .all(|e| e["ancestor"] == true && e["id"].is_null())
  );
  assert!(
    said
      .iter()
      .filter(|e| e["from"].as_str().is_some_and(|f| f.starts_with("shared:")))
      .all(|e| e["shared"] == true)
  );
  assert!(
    said
      .iter()
      .all(|e| e["source"]["thread"] == json!(i.thread))
  );
  // The rows say which thread was imported, and from where; not its fork.
  for (t, imported) in [(&i.thread, true), (&f, false)] {
    let Reply::Lines(l) = c
      .request(&Request::Threads {
        forest: None,
        status: None,
        since: None,
        limit: None,
        thread: Some(t.clone()),
        place: None,
      })
      .await
      .unwrap()
    else {
      panic!("no rows")
    };
    let v: Value = serde_json::from_str(&l[0]).unwrap();
    let row = v["threads"]
      .as_array()
      .unwrap()
      .iter()
      .find(|r| r["id"] == json!(t))
      .unwrap_or_else(|| panic!("{t}: {v}"))
      .clone();
    if imported {
      assert_eq!(row["imported_from"], json!(i.hash), "{row}");
      assert_eq!(row["imported_source"], json!(i.signer.clone().unwrap()));
      assert_eq!(row["imported_signed"], json!(true));
    } else {
      assert!(row.get("imported_from").is_none(), "{row}");
    }
  }
  // Of its last messages only: the import's note is not among them, so a
  // fence stands before the shared messages.
  let g = fork(&mut c, &i.thread, Some(4)).await;
  c.chat(&chat(&g, "And now?"), |_| None).await.unwrap();
  {
    let bodies = sent.lock().unwrap();
    let body = &bodies[1];
    let tools = tool_names(body);
    assert!(
      tools.iter().any(|t| t == "shell_run"),
      "a fork has tools: {tools:?}"
    );
    let text = body.to_string();
    assert!(text.contains("another person shared"), "{text}");
    assert!(
      text.contains("[In the shared conversation, not here: called $ cat"),
      "{text}"
    );
    assert!(!text.contains("\"tool_use\""), "{text}");
    let body = &bodies[2];
    assert!(tool_names(body).iter().any(|t| t == "shell_run"));
    let text = body.to_string();
    assert!(
      text.contains("comes from a conversation another person shared"),
      "the fence: {text}"
    );
    assert!(!text.contains("another person shared with the user as a file (\\\""));
  }

  // A changed archive is refused, nothing written.
  let mut entries = unzip(&file);
  let threads = String::from_utf8(entries["threads.json"].clone()).unwrap();
  entries.insert(
    "threads.json".into(),
    threads
      .replacen("A crate is missing.", "A crate is mising.", 1)
      .into_bytes(),
  );
  let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
  for (n, b) in &entries {
    zip
      .start_file(n.as_str(), zip::write::SimpleFileOptions::default())
      .unwrap();
    std::io::Write::write_all(&mut zip, b).unwrap();
  }
  let changed = zip.finish().unwrap().into_inner();
  let e = imported(&mut c, &changed).await.unwrap_err();
  assert!(e.contains("threads.json does not match its hash"), "{e}");

  // The effort: the conversation and its spawned thread, imported as two
  // threads, the second under the first, which a scope follows.
  let (pe, fe) = exported(&mut c, &child, true, "calls").await;
  assert_eq!((pe.kind.as_str(), pe.threads), ("effort", 2));
  let env = envelope(&fe);
  assert_eq!(env["payload"]["threads"][1]["parent"], "t1");
  let ie = imported(&mut c, &fe).await.unwrap();
  assert_eq!(ie.threads.len(), 2);
  let briefing = lines(
    &mut c,
    &Request::Scope {
      scopes: vec![ScopeTarget {
        kind: ScopeKind::Effort,
        id: ie.threads[1].clone(),
      }],
      query: None,
    },
  )
  .await
  .unwrap()
  .remove(0);
  assert!(briefing.contains("2 threads"), "{briefing}");
  assert!(briefing.contains(&ie.threads[0]) && briefing.contains(&ie.threads[1]));

  drop(c);
  node_handle.stop().await.unwrap();
}
