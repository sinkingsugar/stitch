//! Where threads go. A git checkout is a project, known by its first
//! commit: its threads live in the project's record store in the data
//! directory, never in the checkout, and every node holds every project's,
//! so a clone made later on another node finds them with nothing to set up.
//! A fork split off has its own. Other folders tie threads to themselves;
//! `global` ones belong nowhere.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_net::{Peer, Peers};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
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

fn sse(events: Vec<(&str, Value)>) -> String {
  let body: String = events
    .iter()
    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
    .collect();
  format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// The model asks for one shell command.
fn tool_use(id: &str, command: &str) -> String {
  call(id, "shell_run", json!({"command": command}))
}

/// The model calls one tool.
fn call(id: &str, name: &str, input: Value) -> String {
  let input = input.to_string();
  let (a, b) = input.split_at(input.len() / 2);
  sse(vec![
    (
      "message_start",
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Let me check."}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 0}),
    ),
    (
      "content_block_start",
      json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": a}}),
    ),
    (
      "content_block_delta",
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": b}}),
    ),
    (
      "content_block_stop",
      json!({"type": "content_block_stop", "index": 1}),
    ),
    (
      "message_delta",
      json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
    ),
    ("message_stop", json!({"type": "message_stop"})),
  ])
}

fn answer(text: &str) -> String {
  sse(vec![
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
  ])
}

fn setup(data: &Path, name: &str, home: &Path, anthropic: &str, port: u16) {
  let mut c = Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    port: Some(port),
    // Apps not served: a test node never takes the fixed port 7177.
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
  let root = RecordRoot::init(home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

fn free_port() -> u16 {
  std::net::UdpSocket::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn git(dir: &Path, args: &[&str]) {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {args:?}: {}",
    String::from_utf8_lossy(&out.stderr)
  );
}

fn chat(thread: Option<&str>, text: &str, cwd: &Path, global: bool) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: Some(cwd.display().to_string()),
    global,
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

/// Sends a chat; the thread and the shell's output, if any.
async fn send(c: &mut Client, req: &Request) -> (String, String) {
  let mut out = String::new();
  let done = c
    .chat(req, |r| {
      if let Reply::ToolResult { output, .. } = r {
        out.push_str(output);
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread, .. } = done else {
    panic!("{done:?}")
  };
  (thread, out)
}

fn record(data: &Path, id: &str) -> Option<stitch_store::IndexedRecord> {
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .record(&id.parse().unwrap())
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_follow_projects_to_every_node_and_clone() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  std::fs::create_dir_all(app.join("src")).unwrap();
  git(&app, &["init", "--quiet"]);
  git(
    &app,
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "first",
    ],
  );
  git(
    &app,
    &["remote", "add", "origin", "git@github.com:me/app.git"],
  );
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();

  let (base_a, _) = serve(vec![
    tool_use("t1", "pwd"),
    answer("in the project"),
    tool_use("t2", "pwd"),
    answer("in the folder"),
    answer("global"),
    tool_use("t4", "pwd"),
    answer("folded"),
  ])
  .await;
  let (base_b, _) = serve(vec![tool_use("t3", "pwd"), answer("in the clone")]).await;
  let (port_a, port_b) = (free_port(), free_port());
  let (data_a, data_b) = (d.join("a"), d.join("b"));
  setup(&data_a, "mbp", &d.join("home-a"), &base_a, port_a);
  setup(&data_b, "nas", &d.join("home-b"), &base_b, port_b);
  let node_a = stitch_node::start(&data_a).await.unwrap();
  let node_b = stitch_node::start(&data_b).await.unwrap();
  for (data, name, id, port) in [
    (&data_a, "nas", &node_b.endpoint, port_b),
    (&data_b, "mbp", &node_a.endpoint, port_a),
  ] {
    let mut peers = Peers::default();
    peers
      .add(Peer {
        name: name.into(),
        id: id.clone(),
        addrs: vec![([127, 0, 0, 1], port).into()],
      })
      .unwrap();
    peers.save(&data.join("peers.toml")).unwrap();
  }
  let mut a = Client::connect(&data_a).await.unwrap();
  let mut b = Client::connect(&data_b).await.unwrap();

  // In the checkout: the project's store, in the data directory; nothing
  // is written into the checkout. The shell starts where the user is.
  let (project_thread, out) = send(&mut a, &chat(None, "where?", &app.join("src"), false)).await;
  let r = record(&data_a, &project_thread).unwrap();
  let repo = r.repo.to_string();
  assert!(repo.starts_with("p-"), "{repo}");
  assert!(data_a.join("projects").join(&repo).is_dir());
  assert!(!app.join(".stitch").exists());
  assert!(
    out.contains(&app.join("src").display().to_string()),
    "{out}"
  );

  // Outside git: home, tied to the folder. Asked to: global.
  let (folder_thread, out) = send(&mut a, &chat(None, "where?", &plain, false)).await;
  let r = record(&data_a, &folder_thread).unwrap();
  assert_eq!(r.repo.as_str(), "home");
  assert_eq!(r.fields["folder"]["path"], plain.display().to_string());
  assert!(out.contains(&plain.display().to_string()), "{out}");
  let (global, _) = send(&mut a, &chat(None, "anywhere", &plain, true)).await;
  assert!(
    record(&data_a, &global)
      .unwrap()
      .fields
      .get("folder")
      .is_none()
  );

  // The folder becomes a checkout: its thread is the project's now, and
  // `last` there continues it, its shell in the checkout.
  git(&plain, &["init", "--quiet"]);
  // A first commit of its own: the same empty commit in the same second
  // as the app's would be the same project.
  git(
    &plain,
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "first of plain",
    ],
  );
  let (t, out) = send(&mut a, &chat(Some("last"), "and now?", &plain, false)).await;
  assert_eq!(t, folder_thread);
  assert!(out.contains(&plain.display().to_string()), "{out}");
  // Folded with no new thread started there, the project is listed all the
  // same: by its folder's name, with its record store.
  let co = stitch_node::place::Checkout::of(&plain).unwrap();
  let plain_repo = stitch_node::place::project_repo(&co.root, None);
  let index = Index::open(&data_a.join("index.sqlite")).unwrap();
  let listed = stitch_node::place::projects(&index);
  let p = listed
    .iter()
    .find(|p| p.repo == plain_repo)
    .expect("listed");
  assert_eq!(p.name, "plain");
  assert!(data_a.join("projects").join(plain_repo.as_str()).is_dir());

  // The other node has no checkout, yet gets the project with the home
  // workspace that lists it.
  let Reply::Synced(lines) = b.request(&Request::Sync { peer: None }).await.unwrap() else {
    panic!()
  };
  assert!(lines.iter().any(|l| l.contains(&repo)), "{lines:?}");
  assert_eq!(
    record(&data_b, &project_thread).unwrap().repo.to_string(),
    repo
  );
  assert!(data_b.join("projects").join(&repo).is_dir());

  // Cloned there later: `last` in the clone is the project's thread, and
  // its shell starts in the clone.
  let clone = d.join("elsewhere/app");
  std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
  git(
    &d,
    &[
      "clone",
      "--quiet",
      app.to_str().unwrap(),
      clone.to_str().unwrap(),
    ],
  );
  git(
    &clone,
    &["remote", "set-url", "origin", "https://github.com/me/app"],
  );
  let (t, out) = send(&mut b, &chat(Some("last"), "and here?", &clone, false)).await;
  assert_eq!(t, project_thread);
  assert!(out.contains(&clone.display().to_string()), "{out}");

  // Split off as a fork, the clone's new threads go to a project of its
  // own; the original's stay.
  git(
    &clone,
    &[
      "remote",
      "set-url",
      "origin",
      "git@github.com:someone/app-fork.git",
    ],
  );
  let Reply::Split(name) = b
    .request(&Request::Split {
      cwd: clone.display().to_string(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(name.starts_with("app-fork ("), "{name}");
  let fork_repo = name
    .trim_end_matches(')')
    .rsplit('(')
    .next()
    .unwrap()
    .to_owned();
  assert_ne!(fork_repo, repo);
  assert!(data_b.join("projects").join(&fork_repo).is_dir());

  for c in [&mut a, &mut b] {
    assert!(matches!(
      c.request(&Request::Stop).await.unwrap(),
      Reply::Ok
    ));
  }
  node_a.wait().await.unwrap();
  node_b.wait().await.unwrap();
}

/// A chat from a client with no directory (the phone), in a place it names.
fn chat_in(place: &str, text: &str) -> Request {
  Request::Chat {
    thread: None,
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: None,
    global: false,
    env: Vec::new(),
    tools: Vec::new(),
    skills: Vec::new(),
    place: Some(place.into()),
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_with_no_directory_names_the_place_and_lists_by_time() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  std::fs::create_dir_all(&app).unwrap();
  git(&app, &["init", "--quiet"]);
  git(
    &app,
    &[
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "first",
    ],
  );
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();
  let (base, _) = serve((0..6).map(|i| answer(&format!("answer {i}"))).collect()).await;
  let data = d.join("mbp");
  setup(&data, "mbp", &d.join("home"), &base, free_port());
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let (in_project, _) = send(&mut c, &chat(None, "here", &app, false)).await;
  let repo = record(&data, &in_project).unwrap().repo.to_string();
  let (in_folder, _) = send(&mut c, &chat(None, "there", &plain, false)).await;

  // From no directory: the project, the folder outside the home directory
  // (one node's: it keeps its node), a `~` folder (every node's), global.
  let (t, _) = send(&mut c, &chat_in(&format!("project:{repo}"), "p")).await;
  assert_eq!(record(&data, &t).unwrap().repo.as_str(), repo);
  let key = format!("folder:mbp:{}", plain.display());
  let (t, _) = send(&mut c, &chat_in(&key, "f")).await;
  let r = record(&data, &t).unwrap();
  assert_eq!(r.repo.as_str(), "home");
  assert_eq!(r.fields["folder"]["node"], "mbp");
  assert_eq!(r.fields["folder"]["path"], plain.display().to_string());
  let (t, _) = send(&mut c, &chat_in("folder:~/notes", "n")).await;
  assert_eq!(
    record(&data, &t).unwrap().fields["folder"]["path"],
    "~/notes"
  );
  let (global, _) = send(&mut c, &chat_in("global", "g")).await;
  assert!(
    record(&data, &global)
      .unwrap()
      .fields
      .get("folder")
      .is_none()
  );
  let refused = c.request(&chat_in("nowhere", "x")).await;
  assert!(
    refused.is_err_and(|e| e.to_string().contains("a place is")),
    "a key that names no place is refused"
  );

  // Listed in the project, the global thread written last comes first;
  // `last` there is still the project's latest.
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let home = Some("home".parse().unwrap());
  let places = stitch_node::place::Places::load(&index);
  let here = stitch_node::place::Place::Project(repo.parse().unwrap());
  let listed: Vec<String> = stitch_node::place::listed(
    index.threads().unwrap(),
    home.as_ref(),
    &here,
    false,
    &places,
  )
  .into_iter()
  .map(|(r, ..)| r.id.to_string())
  .collect();
  assert_eq!(listed.first(), Some(&global));
  assert!(listed.contains(&in_project) && !listed.contains(&in_folder));
  let ordered = stitch_node::place::ordered(
    index.threads().unwrap(),
    home.as_ref(),
    &here,
    false,
    &places,
  );
  assert_eq!(ordered[0].2, here);

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A chat in an existing thread from a client with no directory.
fn chat_on(thread: &str, text: &str) -> Request {
  Request::Chat {
    thread: Some(thread.into()),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: None,
    global: false,
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

/// The last line of a shell's output.
fn last(out: &str) -> &str {
  out.trim().lines().last().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_with_no_place_has_a_working_folder_of_its_own() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();
  let (base, seen) = serve(vec![
    tool_use("t1", "pwd"),
    answer("there"),
    call(
      "t2",
      "write_file",
      json!({"path": "apps/x/index.html", "content": "<p>x</p>"}),
    ),
    answer("written"),
    tool_use("t3", "pwd"),
    answer("the fork's"),
    tool_use("t4", "pwd"),
    answer("where the user is"),
  ])
  .await;
  let data = d.join("mbp");
  setup(&data, "mbp", &d.join("home"), &base, free_port());
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // From no directory (the phone), global: its own folder in the data
  // directory, made for it, never the home directory.
  let (global, out) = send(&mut c, &chat_in("global", "where?")).await;
  let folder = data.join("workdirs").join(&global);
  assert!(folder.is_dir());
  assert_eq!(last(&out), folder.display().to_string());
  // The turn is told where it is.
  let told = format!("This thread's working folder: {}", folder.display());
  assert!(seen.lock().unwrap()[0].to_string().contains(&told));

  // Relative paths are written there, without asking.
  let (_, out) = send(&mut c, &chat_on(&global, "an app")).await;
  assert!(out.contains("created"), "{out}");
  assert_eq!(
    std::fs::read_to_string(folder.join("apps/x/index.html")).unwrap(),
    "<p>x</p>"
  );

  // A fork reads its conversation and keeps its folder.
  let Reply::Forked(fork) = c
    .request(&Request::Fork {
      thread: global.clone(),
      upto: None,
      last: None,
      no_attachments: false,
      no_tools: false,
      summarize: false,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let (_, out) = send(&mut c, &chat_on(&fork, "and here?")).await;
  assert_eq!(last(&out), folder.display().to_string());
  assert!(!data.join("workdirs").join(&fork).exists());

  // A client in a directory keeps it.
  let (other, out) = send(&mut c, &chat(None, "here?", &plain, true)).await;
  assert_eq!(last(&out), plain.display().to_string());
  assert!(!data.join("workdirs").join(&other).exists());

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A global thread's working folder syncs as a folder place does (design
/// §2.3, §2.16): written on one node, continued on another, its files are
/// there, with no question asked; its app exports on the other node.
#[tokio::test(flavor = "multi_thread")]
async fn a_global_threads_working_folder_follows_it_to_another_node() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let plain = d.join("plain");
  std::fs::create_dir_all(&plain).unwrap();
  let (base_a, _) = serve(vec![
    call(
      "t1",
      "write_file",
      json!({"path": "apps/heel/index.html", "content": "<p>heel</p>"}),
    ),
    call(
      "t2",
      "show_app",
      json!({"path": "apps/heel", "name": "Heel Drop"}),
    ),
    answer("shown"),
  ])
  .await;
  let (base_b, seen_b) = serve(vec![
    tool_use("t3", "cat apps/heel/index.html"),
    answer("read"),
    answer("from the terminal"),
  ])
  .await;
  let (port_a, port_b) = (free_port(), free_port());
  let (data_a, data_b) = (d.join("a"), d.join("b"));
  setup(&data_a, "iphone", &d.join("home-a"), &base_a, port_a);
  setup(&data_b, "mbp", &d.join("home-b"), &base_b, port_b);
  let node_a = stitch_node::start(&data_a).await.unwrap();
  let node_b = stitch_node::start(&data_b).await.unwrap();
  for (data, name, id, port) in [
    (&data_a, "mbp", &node_b.endpoint, port_b),
    (&data_b, "iphone", &node_a.endpoint, port_a),
  ] {
    let mut peers = Peers::default();
    peers
      .add(Peer {
        name: name.into(),
        id: id.clone(),
        addrs: vec![([127, 0, 0, 1], port).into()],
      })
      .unwrap();
    peers.save(&data.join("peers.toml")).unwrap();
  }
  let mut a = Client::connect(&data_a).await.unwrap();
  let mut b = Client::connect(&data_b).await.unwrap();

  // Written on one node, in its working folder (in a temporary directory,
  // which a folder place would never sync unless allowed: no guard here).
  let (thread, _) = send(&mut a, &chat_in("global", "an app")).await;
  let on_a = data_a.join("workdirs").join(&thread);
  assert_eq!(
    std::fs::read_to_string(on_a.join("apps/heel/index.html")).unwrap(),
    "<p>heel</p>"
  );
  // The other node learns of it and takes its changes before any turn
  // there; the folder itself is made there only by a turn.
  let key = stitch_node::folders::workdir_key(&thread.parse().unwrap());
  let store = stitch_node::folders::store_id(&key);
  let snaps = |data: &Path| {
    Index::open(&data.join("index.sqlite"))
      .ok()
      .and_then(|i| stitch_node::folders::snapshots(&i, &store).ok())
      .map_or(0, |s| s.len())
  };
  for _ in 0..300 {
    if snaps(&data_b) > 0 && record(&data_b, &thread).is_some() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  }
  assert!(snaps(&data_b) > 0, "the working folder's changes on mbp");
  let on_b = data_b.join("workdirs").join(&thread);
  assert!(!on_b.exists(), "made only by a turn");

  // Continued there, the turn finds the file, and is told the folder syncs.
  let (_, out) = send(&mut b, &chat_on(&thread, "show me")).await;
  assert!(out.contains("<p>heel</p>"), "{out}");
  let told = format!("This thread's working folder: {}", on_b.display());
  let first = seen_b.lock().unwrap()[0].to_string();
  assert!(first.contains(&told), "{first}");
  assert!(first.contains("It syncs with the user's other nodes"));
  // Nothing was asked of the user.
  for data in [&data_a, &data_b] {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let asked = index
      .records(Some(stitch_records::kinds::QUESTION))
      .unwrap();
    assert!(asked.is_empty(), "{asked:?}");
  }

  // From a terminal in another directory, the start is the client's and
  // the working folder is named.
  send(&mut b, &chat(Some(&thread), "and now?", &plain, true)).await;
  let last_req = seen_b.lock().unwrap().last().unwrap().to_string();
  let named = format!("This thread's working folder is {} here", on_b.display());
  assert!(last_req.contains(&named), "{last_req}");

  // Its app, exported on the other node from the thread that showed it.
  let Reply::AppFiles { name, files, .. } = b
    .request(&Request::AppExport {
      what: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(name, "Heel Drop");
  let blobs = stitch_store::BlobStore::open(&data_b.join("blobs")).unwrap();
  let out_dir = d.join("export").join(stitch_node::apps::slug(&name));
  assert!(out_dir.ends_with("heel-drop"));
  assert_eq!(
    stitch_node::apps::export_files(&blobs, &files, &out_dir, false).unwrap(),
    1
  );
  assert_eq!(
    std::fs::read_to_string(out_dir.join("index.html")).unwrap(),
    "<p>heel</p>"
  );
  // Not over what is there, unless forced.
  assert!(stitch_node::apps::export_files(&blobs, &files, &out_dir, false).is_err());
  assert!(stitch_node::apps::export_files(&blobs, &files, &out_dir, true).is_ok());
  let bad = vec![("../x".to_owned(), files[0].1.clone())];
  assert!(stitch_node::apps::export_files(&blobs, &bad, &d.join("bad"), false).is_err());

  for c in [&mut a, &mut b] {
    assert!(matches!(
      c.request(&Request::Stop).await.unwrap(),
      Reply::Ok
    ));
  }
  node_a.wait().await.unwrap();
  node_b.wait().await.unwrap();
}

/// A project with no checkout this node knows (design §2.3, "A project
/// with no checkout here"; the phone put a chat of paragon on beast, which
/// had never run in its checkout, and the turn started in the home
/// directory and took another repository for it): an unseen checkout
/// beside the known ones is found and named to the turn; with none, the
/// user is asked to clone it from its origin, and the clone is the place;
/// with nothing to clone from, the chat fails saying so.
#[tokio::test(flavor = "multi_thread")]
async fn a_project_with_no_checkout_here_is_found_or_cloned_never_skipped() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  let commit = |dir: &Path, msg: &str| {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet"]);
    git(
      dir,
      &[
        "-c",
        "user.name=T",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        msg,
      ],
    );
  };
  let app = work.join("app");
  commit(&app, "app");
  // `other` has an origin to clone from (a local bare repository).
  let bare = d.join("origin").join("other");
  commit(&d.join("seed"), "other");
  let out = Command::new("git")
    .args(["clone", "--quiet", "--bare"])
    .arg(d.join("seed"))
    .arg(&bare)
    .output()
    .unwrap();
  assert!(out.status.success());
  let other = work.join("other");
  let out = Command::new("git")
    .args(["clone", "--quiet"])
    .arg(&bare)
    .arg(&other)
    .output()
    .unwrap();
  assert!(out.status.success());
  // `lone` has none.
  let lone = work.join("lone");
  commit(&lone, "lone");

  let (base, seen) = serve((0..10).map(|i| answer(&format!("answer {i}"))).collect()).await;
  let data = d.join("mbp");
  setup(&data, "mbp", &d.join("home"), &base, free_port());
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let repo_of = |t: &str| record(&data, t).unwrap().repo.to_string();
  let (t, _) = send(&mut c, &chat(None, "app", &app, false)).await;
  assert_ne!(repo_of(&t), "home");
  let (in_other, _) = send(&mut c, &chat(None, "other", &other, false)).await;
  let other_repo = repo_of(&in_other);
  let (t, _) = send(&mut c, &chat(None, "lone", &lone, false)).await;
  let lone_repo = repo_of(&t);

  // Moved where this node has not seen it: found beside `app`, and named.
  let moved = work.join("other-moved");
  std::fs::rename(&other, &moved).unwrap();
  let n = seen.lock().unwrap().len();
  send(&mut c, &chat_in(&format!("project:{other_repo}"), "where?")).await;
  let told = seen.lock().unwrap()[n].to_string();
  assert!(
    told.contains(&format!("is at {} here", moved.display())),
    "{told}"
  );

  // Its remembered directory now holds another repository: not taken for
  // it (forgotten), the checkout found where it went instead.
  let real = work.join("other-real");
  std::fs::rename(&moved, &real).unwrap();
  commit(&moved, "an impostor");
  let n = seen.lock().unwrap().len();
  send(
    &mut c,
    &chat_in(&format!("project:{other_repo}"), "and now?"),
  )
  .await;
  let told = seen.lock().unwrap()[n].to_string();
  assert!(
    told.contains(&format!("is at {} here", real.display())),
    "{told}"
  );

  // Gone (the impostor still where it was): a new chat (no thread to ask
  // for) fails saying so…
  std::fs::remove_dir_all(&real).unwrap();
  let refused = c
    .request(&chat_in(&format!("project:{other_repo}"), "x"))
    .await;
  let e = format!("{refused:?}");
  assert!(
    e.contains("has no checkout on mbp") && e.contains("git clone"),
    "{e}"
  );
  // …and lone, with no origin, says it has nothing to clone from.
  std::fs::remove_dir_all(&lone).unwrap();
  let refused = c
    .request(&chat_in(&format!("project:{lone_repo}"), "x"))
    .await;
  let e = format!("{refused:?}");
  assert!(e.contains("no origin to clone from"), "{e}");

  // A chat in its thread asks to clone it; allowed, the clone is the place.
  let chatting = tokio::spawn({
    let (data, t) = (data.clone(), in_other.clone());
    async move {
      let mut c = Client::connect(&data).await.unwrap();
      let mut out = String::new();
      let done = c
        .chat(&chat_on(&t, "now?"), |r| {
          if let Reply::Text(x) = r {
            out.push_str(x);
          }
          None
        })
        .await;
      (done, out)
    }
  });
  let mut question = None;
  for _ in 0..200 {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    question = index
      .records(Some("question"))
      .unwrap()
      .into_iter()
      .find(|q| q.fields.get("session").and_then(Value::as_str) == Some("clone"));
    if question.is_some() {
      break;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  let q = question.expect("a clone question");
  let command = q.fields["command"].as_str().unwrap().to_owned();
  let dest = work.join("other");
  assert_eq!(
    command,
    format!("git clone -- {} {}", bare.display(), dest.display())
  );
  for _ in 0..200 {
    match c
      .request(&Request::Answer {
        question: q.id.to_string(),
        allow: true,
        reason: None,
        always: false,
      })
      .await
    {
      Ok(_) => break,
      Err(e) if e.to_string().contains("no thread") => {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await
      }
      Err(e) => panic!("{e}"),
    }
  }
  let (done, _) = chatting.await.unwrap();
  done.unwrap();
  assert!(dest.join(".git").exists());
  let last = seen.lock().unwrap().last().unwrap().to_string();
  assert!(
    last.contains(&format!("is at {} here", dest.display())),
    "{last}"
  );

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}

/// A repository re-created upstream from a new, unrelated first commit (the
/// old one archived, a new one at the same origin, the checkout reset to
/// it) is a new project until the user says otherwise: the old threads'
/// turns fail naming `stitch project rebind`, and a new chat there is in
/// another project. Rebound, the old project follows it: its threads find
/// the checkout, new chats there are its own. Rebinding refuses another
/// origin, a backward step (to an older project's root), and picks among
/// several projects of the origin only when named. Another repository with
/// another origin where the checkout was is still not taken for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_project_follows_its_repository_recreated_only_once_rebound() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  let commit = |dir: &Path, msg: &str| {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet"]);
    git(
      dir,
      &[
        "-c",
        "user.name=T",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        msg,
      ],
    );
  };
  let origin = d.join("origin").join("lib");
  let lib = work.join("lib");
  commit(&lib, "the first history");
  git(&lib, &["remote", "add", "origin", origin.to_str().unwrap()]);

  let (base, seen) = serve((0..10).map(|i| answer(&format!("answer {i}"))).collect()).await;
  let data = d.join("mbp");
  setup(&data, "mbp", &d.join("home"), &base, free_port());
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let repo_of = |t: &str| record(&data, t).unwrap().repo.to_string();
  // Another project beside it: where this node keeps its checkouts.
  let app = work.join("app");
  commit(&app, "app");
  send(&mut c, &chat(None, "app", &app, false)).await;
  let (old, _) = send(&mut c, &chat(None, "lib", &lib, false)).await;
  let project = repo_of(&old);
  assert_ne!(project, "home");

  // A copy of the old history, kept outside where this node looks for
  // checkouts (for the backward case below).
  let archive = d.join("archive");
  let first = archive.join("lib-first");
  git(
    &d,
    &[
      "clone",
      "--quiet",
      lib.to_str().unwrap(),
      first.to_str().unwrap(),
    ],
  );
  git(
    &first,
    &["remote", "set-url", "origin", origin.to_str().unwrap()],
  );

  // Re-created: a new first commit, the same origin.
  std::fs::remove_dir_all(&lib).unwrap();
  commit(&lib, "the history begun again");
  git(&lib, &["remote", "add", "origin", origin.to_str().unwrap()]);
  // Not followed by itself: the old thread's turn fails, naming the way…
  let done = c.chat(&chat_on(&old, "still there?"), |_| None).await;
  let e = format!("{done:?}");
  assert!(
    e.contains("has no checkout on mbp")
      && e.contains(&format!(
        "{} holds a repository re-created from a new first commit",
        lib.display()
      ))
      && e.contains("run `stitch project rebind` there"),
    "{e}"
  );
  // …and a new chat there is another project, listed after the old one.
  let (t, _) = send(&mut c, &chat(None, "a new one", &lib, false)).await;
  let newer = repo_of(&t);
  assert_ne!(newer, project);

  let rebind = |cwd: &Path, project: Option<&str>| Request::Rebind {
    cwd: cwd.display().to_string(),
    project: project.map(str::to_owned),
  };
  // Backward (in a checkout of the old history, to the newer project):
  // refused.
  let refused = c.request(&rebind(&first, None)).await;
  let e = format!("{refused:?}");
  assert!(e.contains("only forward"), "{e}");
  // A third history at the same origin, listed too: the choice is named.
  let third = archive.join("lib-third");
  commit(&third, "a third history");
  git(
    &third,
    &["remote", "add", "origin", origin.to_str().unwrap()],
  );
  let (t, _) = send(&mut c, &chat(None, "a third", &third, false)).await;
  let third_project = repo_of(&t);
  let refused = c.request(&rebind(&lib, None)).await;
  let e = format!("{refused:?}");
  assert!(
    e.contains("several projects have origin")
      && e.contains(&project)
      && e.contains(&third_project)
      && e.contains("--project"),
    "{e}"
  );
  let refused = c.request(&rebind(&lib, Some("lib"))).await;
  assert!(
    format!("{refused:?}").contains("several projects"),
    "{refused:?}"
  );
  // Nor backward from the newer project's root to the third, listed later.
  let refused = c.request(&rebind(&lib, Some(&third_project))).await;
  assert!(
    format!("{refused:?}").contains("only forward"),
    "{refused:?}"
  );

  // Rebound: the old project follows it.
  let Reply::Rebind(said) = c.request(&rebind(&lib, Some(&project))).await.unwrap() else {
    panic!()
  };
  assert!(
    said.contains(&project) && said.contains("follows this checkout"),
    "{said}"
  );
  let n = seen.lock().unwrap().len();
  let done = c.chat(&chat_on(&old, "there now?"), |_| None).await;
  assert!(done.is_ok(), "{done:?}");
  let told = seen.lock().unwrap()[n].to_string();
  assert!(
    told.contains(&format!("is at {} here", lib.display())),
    "{told}"
  );
  let (t, _) = send(&mut c, &chat(None, "and a new one", &lib, false)).await;
  assert_eq!(repo_of(&t), project, "a new chat there is the old project");
  let Reply::Rebind(said) = c.request(&rebind(&lib, None)).await.unwrap() else {
    panic!()
  };
  assert!(said.contains("already follows"), "{said}");

  // Another origin, with no project of its own listed: nothing to rebind.
  let stray = archive.join("stray");
  commit(&stray, "stray");
  git(
    &stray,
    &[
      "remote",
      "add",
      "origin",
      d.join("stray-origin").to_str().unwrap(),
    ],
  );
  let refused = c.request(&rebind(&stray, None)).await;
  assert!(
    format!("{refused:?}").contains("nothing to rebind"),
    "{refused:?}"
  );
  // No origin at all.
  let refused = c.request(&rebind(&app, None)).await;
  assert!(
    format!("{refused:?}").contains("no `origin`"),
    "{refused:?}"
  );

  // An agent's `stitch project rebind` asks the user: on no allow list.
  let rules = stitch_node::tools::Rules::default();
  for line in [
    "stitch project rebind",
    "stitch project rebind --project lib",
  ] {
    assert!(!rules.allows(line), "{line}");
    assert!(!rules.allows_sandboxed(line), "{line}");
  }

  // Another repository with another origin there: not taken for it (the
  // other histories of its origin gone, not to be found instead).
  std::fs::remove_dir_all(&first).unwrap();
  std::fs::remove_dir_all(&third).unwrap();
  std::fs::remove_dir_all(&lib).unwrap();
  commit(&lib, "another project");
  git(
    &lib,
    &[
      "remote",
      "add",
      "origin",
      d.join("elsewhere").to_str().unwrap(),
    ],
  );
  let refused = c
    .request(&chat_in(&format!("project:{project}"), "x"))
    .await;
  let e = format!("{refused:?}");
  assert!(
    e.contains("has no checkout on mbp") && e.contains("is not a checkout of it"),
    "{e}"
  );

  assert!(matches!(
    c.request(&Request::Stop).await.unwrap(),
    Reply::Ok
  ));
  node.wait().await.unwrap();
}
