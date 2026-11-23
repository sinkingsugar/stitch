//! Weaving threads between two nodes over iroh on localhost (design §2.7):
//! the phone's model spawns a thread on the Mac that continues the phone's
//! conversation; its report comes back and wakes the phone's thread on
//! the phone; the phone sends the Mac's thread a follow-up, which it
//! answers with all it did, reporting back again; a thread spawned on the
//! Mac and cancelled from the phone stops there.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers model requests in order, recording each.
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
      // A slow model: answers after a while (unless the loop is stopped).
      let response = match response.strip_prefix("SLOW ") {
        Some(r) => {
          tokio::time::sleep(Duration::from_secs(4)).await;
          r.to_owned()
        }
        None => response,
      };
      let _ = sock.write_all(response.as_bytes()).await;
      let _ = sock.shutdown().await;
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

fn tool_call(id: &str, name: &str, input: Value) -> String {
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
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Handing this to the Mac."}}),
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
      json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
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

/// Starts a node again on the data dir of one just stopped, on the same
/// configured port. The old socket may not be released yet when `stop()`
/// returns, or a child process forked by a parallel test may still hold the
/// inherited fd (macOS has no atomic SOCK_CLOEXEC): iroh's "Failed to bind
/// sockets", which stitch-net passes on as the string in `Net(String)`
/// (there is no finer type to match). Only that is retried, for 10 s, on the
/// same port (the peers know the node by it); any other error panics at once.
async fn restart(data: &Path) -> stitch_node::NodeHandle {
  let deadline = std::time::Instant::now() + Duration::from_secs(10);
  let mut retries = 0;
  loop {
    match stitch_node::start(data).await {
      Ok(node) => return node,
      Err(e @ stitch_node::Error::Net(stitch_net::Error::Net(_)))
        if e.to_string().contains("Failed to bind sockets") =>
      {
        if std::time::Instant::now() >= deadline {
          panic!("the port stayed taken after {retries} retries: {e:?}");
        }
        retries += 1;
        eprintln!("restart: port still taken ({e:?}); retry {retries}");
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
      Err(e) => panic!("the node would not start again: {e:?}"),
    }
  }
}

/// A free UDP port on localhost, for a node to bind again.
/// A UDP port no other test, here or in a parallel test process, is about
/// to use. The old "bind port 0, read it, close it" handed the same
/// ephemeral port to two nodes, or to another process's outgoing socket,
/// before the node bound it ("Failed to bind sockets"). These come from
/// below the ephemeral range, each process starting at its own offset and
/// every port checked free on all addresses a node binds, and reserved
/// for this process (`reserve`).
fn free_port() -> u16 {
  use std::sync::atomic::{AtomicU16, Ordering};
  static NEXT: AtomicU16 = AtomicU16::new(0);
  const BASE: u32 = 20_000;
  const SPAN: u32 = 12_000;
  let start = std::process::id().wrapping_mul(2_654_435_761) % SPAN;
  loop {
    let n = NEXT.fetch_add(1, Ordering::SeqCst) as u32;
    let port = (BASE + (start + n) % SPAN) as u16;
    if !reserve(port) {
      continue;
    }
    // One after the other: a dual-stack `::` socket conflicts with the
    // `0.0.0.0` one on the same port.
    let v4 = std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok();
    let v6 = std::net::UdpSocket::bind(("::", port)).is_ok();
    // No IPv6 here is no reason to skip a port.
    if v4 && (v6 || std::net::UdpSocket::bind(("::1", 0)).is_err()) {
      return port;
    }
  }
}

/// Reserves `port` for this test process until it exits: a lock on a file
/// named for it, which another process's `free_port` skips. A port is
/// checked free seconds before its node binds it (a pair's second node
/// binds after the first started), and another process checking it in
/// that time took it too: "Failed to bind sockets", under parallel test
/// runs. The lock goes with the process, however it ends.
fn reserve(port: u16) -> bool {
  static HELD: std::sync::Mutex<Vec<std::fs::File>> = std::sync::Mutex::new(Vec::new());
  let path = std::env::temp_dir().join(format!("stitch-test-udp-{port}.lock"));
  let Ok(file) = std::fs::OpenOptions::new()
    .create(true)
    .truncate(false)
    .write(true)
    .open(&path)
  else {
    // Nowhere to note it: checked free, as before.
    return true;
  };
  if file.try_lock().is_err() {
    return false;
  }
  HELD.lock().unwrap().push(file);
  true
}

fn config(data: &Path, name: &str, port: u16, anthropic: &str) {
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
}

/// Joins `data` (a node named `name` on `port`) to the inviter of
/// `ticket`, as `stitch join` does: trust, then the home workspace.
async fn join(data: &Path, name: &str, port: u16, ticket: &Ticket) {
  let key = stitch_net::load_or_create_key(&data.join("node.key")).unwrap();
  let index = || Index::open(&data.join("index.sqlite")).unwrap();
  let state = |roots| {
    let peers_path = data.join("peers.toml");
    NodeState::new(
      name.parse().unwrap(),
      roots,
      index(),
      BlobStore::open(&data.join("blobs")).unwrap(),
      Peers::load(&peers_path).unwrap(),
      Some(peers_path),
      Policy {
        blobs: BlobPolicy::Lazy,
      },
    )
  };
  let endpoint = bind(key.clone(), Discovery::Local, Some(port))
    .await
    .unwrap();
  let net = Net::spawn(endpoint, Arc::new(state(Default::default())));
  let joined = net.join(ticket).await.unwrap();
  net.shutdown().await.unwrap();
  let repo = joined.home.expect("the inviter holds the home workspace");
  let root = RecordRoot::init(&data.join("home"), repo.parse().unwrap(), Role::Home).unwrap();
  index().refresh(&root).unwrap();
  let roots = [(root.repo().clone(), root.clone())].into_iter().collect();
  let endpoint = bind(key, Discovery::Local, Some(port)).await.unwrap();
  let net = Net::spawn(endpoint, Arc::new(state(roots)));
  net
    .sync_with(joined.inviter.addr().unwrap(), root.repo())
    .await
    .unwrap();
  net.shutdown().await.unwrap();
}

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
  for _ in 0..300 {
    if ok() {
      return;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  panic!("timed out waiting for {what}");
}

/// Every text of every message in a request.
fn texts(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| match &m["content"] {
      Value::String(s) => vec![s.clone()],
      Value::Array(c) => c
        .iter()
        .filter_map(|b| b["text"].as_str().map(str::to_owned))
        .collect(),
      _ => Vec::new(),
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_weave_between_the_phone_and_the_mac() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, phone_sent) = serve(vec![
    answer("Sure: v2 adds offline sync and fixes the export crash."),
    tool_call(
      "d1",
      "spawn",
      json!({"node": "mac", "goal": "Write the release notes for v2 into RELEASE.md.", "context": "this"}),
    ),
    answer("The Mac is on it."),
    answer("The Mac wrote the release notes."),
    answer("And it added the changelog link."),
    answer("Noted: the benchmarks were cancelled."),
  ])
  .await;
  let (mac_api, mac_sent) = serve(vec![
    answer("Wrote RELEASE.md: offline sync, and the export crash fix."),
    answer("Added the changelog link to RELEASE.md."),
    format!("SLOW {}", answer("Benchmarks done.")),
  ])
  .await;

  // The phone holds the home workspace; the Mac joins it.
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();

  // A conversation on the phone, then a thread for the Mac.
  let chat = |thread: Option<String>, text: &str| Request::Chat {
    thread,
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
  };
  let Reply::ChatDone { thread, .. } = p
    .chat(&chat(None, "What goes into the v2 release?"), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  let mut results = Vec::new();
  p.chat(
    &chat(
      Some(thread.clone()),
      "Write the release notes for that on the Mac, please.",
    ),
    |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    },
  )
  .await
  .unwrap();
  let spawned = results
    .iter()
    .find_map(|r| r.strip_prefix("Spawned thread "))
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{results:?}"))
    .to_owned();
  assert!(spawned.contains("-mac-"), "named for the Mac: {spawned}");
  let tools: Vec<String> = phone_sent.lock().unwrap()[1]["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|t| t["name"].as_str().unwrap().to_owned())
    .collect();
  for name in ["spawn", "send", "wait", "models"] {
    assert!(tools.iter().any(|t| t == name), "{name}: {tools:?}");
  }
  assert!(!tools.iter().any(|t| t == "ask_model" || t == "delegate"));

  // The Mac, nudged, continues the conversation up to the spawning call.
  until("the Mac to start", || !mac_sent.lock().unwrap().is_empty()).await;
  {
    let body = mac_sent.lock().unwrap()[0].clone();
    let all = texts(&body);
    assert!(
      all
        .iter()
        .any(|t| t.contains("What goes into the v2 release?")),
      "{all:?}"
    );
    assert!(all.iter().any(|t| t.contains("offline sync")), "{all:?}");
    assert!(
      all
        .iter()
        .any(|t| t.contains("Write the release notes for that on the Mac")),
      "the request that led to it: {all:?}"
    );
    assert!(
      all
        .iter()
        .any(|t| t.starts_with("Write the release notes for v2 into RELEASE.md.")),
      "{all:?}"
    );
    assert!(
      !body["messages"].to_string().contains("\"tool_use\""),
      "not the spawning call itself: {}",
      body["messages"]
    );
    // What the phone said with the call stands as a line, and the goal
    // comes with who the thread is: the conversation is the phone's
    // thread's, and the thread it spawned is this one (2026-10-19).
    assert!(
      all
        .iter()
        .any(|t| t.starts_with("[Stitch: here this thread called `spawn`")),
      "{all:?}"
    );
    let last = all.last().unwrap();
    assert!(
      last.starts_with(&format!(
        "[Stitch: you are thread {spawned}, running on node mac, spawned by thread {thread}"
      )) && last.contains("The conversation before it is your spawner's")
        && last.contains("the thread it started for this goal is you"),
      "{last}"
    );
    assert!(
      body["system"]
        .to_string()
        .contains(&format!("This thread is {spawned}")),
      "its prompt names it"
    );
  }

  // Its report comes back and wakes the phone's thread, on the phone.
  until("the report to wake the phone's thread", || {
    phone_sent.lock().unwrap().len() >= 4
  })
  .await;
  assert!(
    texts(&phone_sent.lock().unwrap()[3])
      .iter()
      .any(|t| t.contains("[Report ") && t.contains("Wrote RELEASE.md")),
    "the report wakes the spawning thread"
  );
  assert_eq!(mac_sent.lock().unwrap().len(), 1, "the Mac woke nothing");

  // A follow-up to the Mac's thread: it answers with all it did, on the
  // Mac, and its reply comes back to the phone's thread.
  let Reply::Lines(lines) = p
    .request(&Request::Send {
      thread: spawned.clone(),
      text: "Also add a link to the changelog.".into(),
      from: Some(thread.clone()),
      keep: false,
      interject: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("on mac"), "{lines:?}");
  until("the Mac to answer the follow-up", || {
    mac_sent.lock().unwrap().len() >= 2
  })
  .await;
  {
    let all = texts(&mac_sent.lock().unwrap()[1]);
    assert!(
      all.iter().any(|t| t.contains("Wrote RELEASE.md")),
      "its own history: {all:?}"
    );
    assert!(
      all
        .iter()
        .any(|t| t.contains("Also add a link to the changelog.")),
      "{all:?}"
    );
  }
  until("the follow-up's report to reach the phone", || {
    phone_sent.lock().unwrap().len() >= 5
  })
  .await;
  assert!(
    texts(&phone_sent.lock().unwrap()[4])
      .iter()
      .any(|t| t.contains("Added the changelog link")),
  );

  // Spawned on the Mac by command, and cancelled from the phone while it
  // runs there: it stops.
  p.request(&Request::Sync { peer: None }).await.unwrap();
  let Reply::Detached(slow) = p
    .request(&Request::Detach {
      goal: "Run the benchmarks.".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: Some(thread.clone()),
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: None,
      global: true,
      node: Some("mac".into()),
      context: Some("this".into()),
      no_tools: false,
      tools: Vec::new(),
      skills: Vec::new(),
      keep: false,
      memory: false,
      read_only: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the Mac to start the benchmarks", || {
    mac_sent.lock().unwrap().len() >= 3
  })
  .await;
  p.request(&Request::Sync { peer: None }).await.unwrap();
  assert!(matches!(
    p.request(&Request::Cancel {
      thread: slow.clone()
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
  tokio::time::sleep(Duration::from_secs(6)).await;
  let slow: stitch_records::RecordId = slow.parse().unwrap();
  let mac_index = Index::open(&mac.join("index.sqlite")).unwrap();
  let reports = mac_index.sources("report", "thread", &slow, None).unwrap();
  assert_eq!(reports.len(), 1, "only the cancellation: {reports:?}");
  assert_eq!(reports[0].fields["outcome"], json!("cancelled"));

  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}

/// An assignment can be withdrawn before the remote thread has synced back.
/// The withdrawal survives restart and prevents a late claim, without a model call.
#[tokio::test(flavor = "multi_thread")]
async fn pending_remote_assignment_is_reported_and_cancelled_before_claim() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, _phone_sent, mac_sent) = keyed_pair(
    dir.path(),
    vec![(
      "BOSS",
      vec![
        tool_call(
          "p1",
          "spawn",
          json!({"node": "mac", "goal": "CHILD: do not run"}),
        ),
        tool_call("p2", "wait", json!({"minutes": 0.0001})),
        tool_call(
          "p3",
          "cancel",
          json!({"thread": "@SPAWNED@", "reason": "withdrawn before start"}),
        ),
        tool_call("p4", "threads", json!({"all": true, "status": "ended"})),
        tool_call(
          "p5",
          "wait",
          json!({"threads": ["@SPAWNED@"], "minutes": 0.0001}),
        ),
        answer("BOSS done."),
      ],
    )],
    vec![("CHILD", vec![answer("This must never run.")])],
    false,
  )
  .await;
  mac_node.stop().await.unwrap();
  let mut results = Vec::new();
  p.chat(&global_chat(None, "BOSS: assign and withdraw"), |r| {
    never_asked(r);
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let child = spawned_in(&results, 0);
  assert!(results[1].contains("start unconfirmed"), "{results:?}");
  assert!(!results[1].contains("still working"), "{results:?}");
  assert!(results[2].contains("assignment withdrawn"), "{results:?}");
  assert!(results[3].contains("ended: cancelled"), "{results:?}");
  assert!(results[4].contains("cancelled"), "{results:?}");
  let phone = dir.path().join("phone");
  let mac = dir.path().join("mac");
  let id = child.parse().unwrap();
  let index = Index::open(&phone.join("index.sqlite")).unwrap();
  assert!(index.record(&id).unwrap().is_none());
  assert_eq!(
    index.sources("report", "thread", &id, None).unwrap().len(),
    1
  );
  // Idempotent even without the thread record.
  assert!(matches!(
    p.request(&Request::Cancel {
      thread: child.clone()
    })
    .await
    .unwrap(),
    Reply::Ok
  ));
  assert_eq!(
    index.sources("report", "thread", &id, None).unwrap().len(),
    1
  );
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  // Restart loses discovery's in-memory addresses. Pin the fixture's known
  // localhost port rather than depending on multicast rediscovery timing.
  let mut peers = Peers::load(&phone.join("peers.toml")).unwrap();
  peers
    .add(stitch_net::Peer {
      name: "mac".into(),
      id: stitch_net::load_or_create_key(&mac.join("node.key"))
        .unwrap()
        .public()
        .to_string(),
      addrs: vec![
        format!("127.0.0.1:{}", Config::load(&mac).unwrap().port.unwrap())
          .parse()
          .unwrap(),
      ],
    })
    .unwrap();
  peers.save(&phone.join("peers.toml")).unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let remote = Index::open(&mac.join("index.sqlite")).unwrap();
  for _ in 0..20 {
    let reply = p.request(&Request::Sync { peer: None }).await.unwrap();
    if !remote
      .sources("report", "thread", &id, None)
      .unwrap()
      .is_empty()
    {
      break;
    }
    eprintln!("waiting for restarted peer: {reply:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
  assert!(
    !remote
      .sources("report", "thread", &id, None)
      .unwrap()
      .is_empty(),
    "withdrawal did not sync"
  );
  assert!(remote.record(&id).unwrap().is_none());
  assert!(
    mac_sent.lock().unwrap().is_empty(),
    "withdrawn work reached the model"
  );
  mac_node.stop().await.unwrap();
  phone_node.stop().await.unwrap();
}

/// A spawn to a node not reached is queued all the same, and its result
/// says so at once (design §2.7, "A spawn to a node not reached"), with
/// how to withdraw it and the nodes reached now that could run it: here,
/// the phone itself, which holds the place asked for.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_to_a_node_not_reached_says_so_with_who_could_run_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let otter = d.join("otter");
  checkout(&otter, "OTTER RULES\n");
  let (phone_node, mac_node, mut p, _phone_sent, _mac_sent) = keyed_pair(
    &d,
    vec![(
      "BOSS",
      vec![
        tool_call(
          "p1",
          "spawn",
          json!({"node": "mac", "goal": "CHILD: look at otter", "place": otter.display().to_string()}),
        ),
        answer("BOSS done."),
      ],
    )],
    vec![("CHILD", vec![answer("Looked.")])],
    false,
  )
  .await;
  // Reached once (the join), then gone.
  p.request(&Request::Sync { peer: None }).await.unwrap();
  mac_node.stop().await.unwrap();
  let mut results = Vec::new();
  let started = std::time::Instant::now();
  p.chat(&global_chat(None, "BOSS: have the Mac look"), |r| {
    never_asked(r);
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  // Said at once: the reach and the alternatives' asks only, not the
  // model and place questions' own timeouts.
  assert!(
    started.elapsed() < Duration::from_secs(25),
    "{:?}",
    started.elapsed()
  );
  let said = &results[0];
  let child = spawned_in(&results, 0);
  assert!(said.starts_with("mac not reached ("), "{said}");
  assert!(
    said.contains("is queued there and starts when mac is back"),
    "{said}"
  );
  assert!(
    said.contains(&format!("`cancel {child}` withdraws it")),
    "{said}"
  );
  let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
  assert!(
    said.contains(&format!(
      "Reached now and able to run it: phone (this node, {platform}, holds otter"
    )),
    "{said}"
  );
  // Queued, not refused: the assignment is written for the Mac.
  let index = Index::open(&d.join("phone").join("index.sqlite")).unwrap();
  let assigned: Vec<_> = index
    .records(Some("assignment"))
    .unwrap()
    .into_iter()
    .filter(|a| a.fields.get("thread") == Some(&json!(child)))
    .collect();
  assert_eq!(assigned.len(), 1, "{assigned:?}");
  assert_eq!(assigned[0].fields["node"], json!("mac"));
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}

/// An unconfirmed start may already be running remotely. The same withdrawal
/// must stop that loop, not just suppress future claims.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_an_unconfirmed_remote_start_stops_its_running_turn() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    dir.path(),
    vec![answer("Parent ready."), answer("Cancellation noted.")],
    vec![format!("SLOW {}", answer("Must not finish."))],
  )
  .await;
  let parent = chat_thread(&mut p, "Prepare work").await;
  let mut request = detach_global("Do the work", &parent);
  if let Request::Detach { node, .. } = &mut request {
    *node = Some("mac".into());
  }
  let Reply::Detached(child) = p.request(&request).await.unwrap() else {
    panic!()
  };
  until("remote provider call", || {
    !mac_sent.lock().unwrap().is_empty()
  })
  .await;
  let id = child.parse().unwrap();
  let local = Index::open(&dir.path().join("phone/index.sqlite")).unwrap();
  assert!(
    local.record(&id).unwrap().is_none(),
    "test requires an unconfirmed start"
  );
  assert!(matches!(
    p.request(&Request::Cancel { thread: child }).await.unwrap(),
    Reply::Ok
  ));
  let remote = Index::open(&dir.path().join("mac/index.sqlite")).unwrap();
  until("remote cancellation", || {
    remote
      .sources("report", "thread", &id, None)
      .unwrap()
      .iter()
      .any(|r| r.fields["outcome"] == "cancelled")
  })
  .await;
  tokio::time::sleep(Duration::from_secs(5)).await;
  let reports = remote.sources("report", "thread", &id, None).unwrap();
  assert_eq!(
    reports.len(),
    1,
    "cancelled turn must not later finish: {reports:?}"
  );
  assert_eq!(reports[0].fields["outcome"], "cancelled");
  mac_node.stop().await.unwrap();
  phone_node.stop().await.unwrap();
}

/// Two nodes, the phone holding the home workspace and the Mac joined to
/// it, both running; each node's model answers from its own list.
async fn pair(
  d: &Path,
  phone_says: Vec<String>,
  mac_says: Vec<String>,
) -> (
  stitch_node::NodeHandle,
  stitch_node::NodeHandle,
  Client,
  Client,
  Arc<Mutex<Vec<Value>>>,
  Arc<Mutex<Vec<Value>>>,
) {
  let (phone_api, phone_sent) = serve(phone_says).await;
  let (mac_api, mac_sent) = serve(mac_says).await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let m = Client::connect(&mac).await.unwrap();
  (phone_node, mac_node, p, m, phone_sent, mac_sent)
}

fn global_chat(thread: Option<String>, text: &str) -> Request {
  Request::Chat {
    thread,
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

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_spawned_on_another_node_is_reachable_at_once() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    dir.path(),
    vec![
      tool_call(
        "s1",
        "spawn",
        json!({"node": "mac", "goal": "Count the open issues."}),
      ),
      answer("The Mac counts them."),
      answer("Noted."),
      answer("Noted again."),
    ],
    vec![
      format!("SLOW {}", answer("There are 12 open issues.")),
      answer("Of those, 3 are bugs."),
    ],
  )
  .await;
  let mut results = Vec::new();
  let Reply::ChatDone { thread, .. } = p
    .chat(&global_chat(None, "Have the Mac count the issues."), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let spawned = results
    .iter()
    .find_map(|r| r.strip_prefix("Spawned thread "))
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{results:?}"))
    .to_owned();
  // Not on the phone yet: the Mac has it, and is still at its first turn.
  let index = Index::open(&dir.path().join("phone").join("index.sqlite")).unwrap();
  assert!(
    index.record(&spawned.parse().unwrap()).unwrap().is_none(),
    "already synced back: the test proves nothing"
  );
  // A message for it is taken all the same, by the node it runs on.
  let Reply::Lines(lines) = p
    .request(&Request::Send {
      thread: spawned.clone(),
      text: "And how many are bugs?".into(),
      from: Some(thread.clone()),
      keep: false,
      interject: false,
      scopes: Vec::new(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("on mac"), "{lines:?}");
  until("the Mac to take the follow-up", || {
    mac_sent.lock().unwrap().len() >= 2
  })
  .await;
  let all = texts(&mac_sent.lock().unwrap()[1]);
  assert!(
    all.iter().any(|t| t.contains("And how many are bugs?")),
    "{all:?}"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A model answering each conversation (its first message starts with a
/// key) from its own script, in order; "SLOW " before a response delays
/// it 2.5 seconds. Records every request.
async fn keyed(scripts: Vec<(&str, Vec<String>)>) -> (String, Arc<Mutex<Vec<Value>>>) {
  type Scripts = Arc<Mutex<Vec<(String, std::collections::VecDeque<String>)>>>;
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  let scripts: Scripts = Arc::new(Mutex::new(
    scripts
      .into_iter()
      .map(|(k, v)| (k.to_owned(), v.into()))
      .collect(),
  ));
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let (log, scripts) = (log.clone(), scripts.clone());
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        let body: Value = loop {
          let n = sock.read(&mut chunk).await.unwrap();
          if n == 0 {
            return;
          }
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
            break serde_json::from_slice(&buf[end + 4..end + 4 + len]).unwrap();
          }
        };
        let first = texts(&body).into_iter().next().unwrap_or_default();
        // The thread a spawn earlier in the conversation started.
        let raw = body.to_string();
        // (Or, to a node not reached: "… not reached (…): thread <id> is
        // queued there".)
        let spawned = raw
          .split("Spawned thread ")
          .nth(1)
          .or_else(|| {
            raw
              .split_once(" not reached (")
              .and_then(|(_, r)| r.split_once("): thread "))
              .map(|(_, r)| r)
          })
          .and_then(|r| r.split_whitespace().next())
          .map(str::to_owned);
        log.lock().unwrap().push(body);
        let response = scripts
          .lock()
          .unwrap()
          .iter_mut()
          .find(|(k, _)| first.starts_with(k.as_str()))
          .and_then(|(_, q)| q.pop_front())
          .unwrap_or_else(|| answer("ok"));
        let response = match &spawned {
          Some(id) => response.replace("@SPAWNED@", id),
          None => response,
        };
        // The thread that spawned this one (its first message says).
        let parent = raw
          .split("spawned by thread ")
          .nth(1)
          .and_then(|r| r.split([',', ' ']).next())
          .map(str::to_owned);
        let response = match &parent {
          Some(id) => response.replace("@PARENT@", id),
          None => response,
        };
        let response = match response.strip_prefix("SLOW ") {
          Some(r) => {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            r.to_owned()
          }
          None => response,
        };
        let _ = sock.write_all(response.as_bytes()).await;
        let _ = sock.shutdown().await;
      });
    }
  });
  (base, seen)
}

/// The requests of the conversation whose first message starts with `key`.
fn of(sent: &Arc<Mutex<Vec<Value>>>, key: &str) -> Vec<Value> {
  sent
    .lock()
    .unwrap()
    .iter()
    .filter(|b| {
      texts(b)
        .into_iter()
        .next()
        .is_some_and(|t| t.starts_with(key))
    })
    .cloned()
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_runs_where_it_was_continued_last() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, phone_sent) = keyed(vec![
    (
      "TALK",
      vec![
        tool_call("t1", "spawn", json!({"goal": "CHILD: look into it"})),
        answer("Spawned; I'll hear back."),
        answer("Woken on the phone: wrong."),
      ],
    ),
    (
      "CHILD",
      vec![format!("SLOW {}", answer("The child's finding."))],
    ),
  ])
  .await;
  let (mac_api, mac_sent) = keyed(vec![(
    "TALK",
    vec![
      answer("On the Mac now."),
      answer("Took the child's report on the Mac."),
    ],
  )])
  .await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();

  // Started on the phone, with a child at work there.
  let Reply::ChatDone { thread, .. } = p
    .chat(
      &global_chat(None, "TALK: look into it with a child"),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  // Continued on the Mac while the child works.
  p.request(&Request::Sync { peer: None }).await.unwrap();
  m.chat(
    &global_chat(Some(thread.clone()), "TALK more, on the Mac"),
    |_| None,
  )
  .await
  .unwrap();
  let on_mac = texts(&of(&mac_sent, "TALK")[0]).join("\n");
  assert!(
    on_mac.contains("On node mac"),
    "said where it runs: {on_mac}"
  );
  // The child's report goes to the node that continued the thread last.
  until("the Mac to take the report", || {
    of(&mac_sent, "TALK").len() >= 2
  })
  .await;
  let wake = texts(&of(&mac_sent, "TALK")[1]).join("\n");
  assert!(wake.contains("The child's finding."), "{wake}");
  tokio::time::sleep(Duration::from_secs(2)).await;
  assert_eq!(of(&phone_sent, "TALK").len(), 2, "the phone woke nothing");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// One node, holding the home workspace, its model answering `scripts`
/// (see `keyed`).
async fn alone(
  d: &Path,
  scripts: Vec<(&str, Vec<String>)>,
) -> (stitch_node::NodeHandle, Client, Arc<Mutex<Vec<Value>>>) {
  let (api, sent) = keyed(scripts).await;
  let data = d.join("node");
  config(&data, "mbp", free_port(), &api);
  let home = RecordRoot::init(&d.join("home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let c = Client::connect(&data).await.unwrap();
  (node, c, sent)
}

fn detach_global(goal: &str, parent: &str) -> Request {
  Request::Detach {
    goal: goal.into(),
    model: None,
    repo: None,
    cwd: None,
    parent: Some(parent.into()),
    budget_usd: None,
    budget_tokens: None,
    budget_minutes: None,
    max_steps: None,
    global: true,
    node: None,
    context: None,
    no_tools: false,
    tools: Vec::new(),
    skills: Vec::new(),
    keep: false,
    memory: false,
    read_only: false,
    scopes: Vec::new(),
  }
}

fn send_from(thread: &str, text: &str, from: &str, interject: bool) -> Request {
  Request::Send {
    thread: thread.into(),
    text: text.into(),
    from: Some(from.into()),
    keep: false,
    interject,
    scopes: Vec::new(),
  }
}

async fn chat_thread(c: &mut Client, text: &str) -> String {
  match c.chat(&global_chat(None, text), |_| None).await.unwrap() {
    Reply::ChatDone { thread, .. } => thread,
    other => panic!("{other:?}"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_sent_to_interject_is_woven_into_the_running_turn() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "WORK",
        vec![
          format!("SLOW {}", tool_call("w1", "wait", json!({}))),
          format!("SLOW {}", tool_call("w2", "wait", json!({}))),
          answer("WORK done, and moved to Friday."),
          answer("WORK again, as a turn of its own."),
        ],
      ),
      ("BOSS", vec![answer("Boss here.")]),
      ("OTHER", vec![answer("Other here.")]),
    ],
  )
  .await;
  let boss = chat_thread(&mut c, "BOSS: plan the release").await;
  let other = chat_thread(&mut c, "OTHER: watch the calendar").await;
  let Reply::Detached(work) = c
    .request(&detach_global("WORK: prepare the release", &boss))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the work to start", || !of(&sent, "WORK").is_empty()).await;

  // Another thread interjects while the turn runs: woven in, not queued.
  let Reply::Lines(lines) = c
    .request(&send_from(&work, "DEADLINE moved to Friday.", &other, true))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(
    lines[0].contains("woven into its running turn"),
    "{lines:?}"
  );
  until("the turn to end", || of(&sent, "WORK").len() >= 3).await;
  let last = texts(&of(&sent, "WORK")[2]).join("\n");
  assert!(last.contains("DEADLINE moved to Friday."), "{last}");
  assert!(
    last.contains(&format!("[Message from thread {other} ")),
    "says who it is from: {last}"
  );

  // Its last reply goes back to the sender, which it wakes, as well as to
  // whoever started it; no turn of its own followed.
  until("the reply to wake the sender", || {
    of(&sent, "OTHER").iter().any(|b| {
      let t = texts(b).join("\n");
      t.contains("[Report ") && t.contains("WORK done, and moved to Friday.")
    })
  })
  .await;
  let index = Index::open(&dir.path().join("node").join("index.sqlite")).unwrap();
  let work_id: stitch_records::RecordId = work.parse().unwrap();
  let reports = index.sources("report", "thread", &work_id, None).unwrap();
  let to = |r: &stitch_store::IndexedRecord| {
    index
      .edges_from(&r.id)
      .unwrap()
      .into_iter()
      .find(|e| e.rel == "reports-to")
      .map(|e| e.dst.id.to_string())
  };
  assert_eq!(reports.len(), 2, "{reports:?}");
  let woven: Vec<_> = reports
    .iter()
    .filter(|r| r.fields.get("woven") == Some(&json!(true)))
    .collect();
  assert_eq!(woven.len(), 1, "{reports:?}");
  assert_eq!(to(woven[0]).as_deref(), Some(other.as_str()));
  assert!(
    reports
      .iter()
      .any(|r| r.fields.get("woven").is_none() && to(r).as_deref() == Some(boss.as_str())),
    "its own report still goes to whoever started it: {reports:?}"
  );
  tokio::time::sleep(Duration::from_secs(1)).await;
  assert_eq!(of(&sent, "WORK").len(), 3, "no turn of its own followed");

  // With no turn running, it starts one.
  let Reply::Lines(lines) = c
    .request(&send_from(&work, "And the changelog?", &other, true))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("starts a turn"), "{lines:?}");
  until("the turn it starts", || of(&sent, "WORK").len() >= 4).await;
  assert!(
    texts(&of(&sent, "WORK")[3])
      .join("\n")
      .contains("And the changelog?")
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Issue …-umbm: a thread spawned in a new chat's first turn sent its
/// parent a message, which ran as a second loop beside that turn (a new
/// chat took no lock on the thread it made): two turns on one thread, its
/// shell sessions and checkout shared. The message waits for the turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_a_new_chat_at_work_waits_for_its_first_turn() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "BOSS",
        vec![
          tool_call("b1", "spawn", json!({"goal": "HELPER: audit the docs"})),
          format!("SLOW {}", tool_call("b2", "threads", json!({}))),
          format!("SLOW {}", answer("Boss done.")),
        ],
      ),
      (
        "HELPER",
        vec![
          tool_call(
            "h1",
            "send",
            json!({"thread": "@PARENT@", "message": "FINDINGS: setup is stale", "wake": false}),
          ),
          answer("Helper done."),
        ],
      ),
    ],
  )
  .await;
  let done = c
    .chat(&global_chat(None, "BOSS: review the docs"), |_| None)
    .await
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(
    !of(&sent, "HELPER").is_empty(),
    "the helper ran during the turn"
  );
  let during = of(&sent, "BOSS");
  assert!(
    during
      .iter()
      .all(|b| !b.to_string().contains("FINDINGS: setup is stale")),
    "no second loop ran on the thread during its turn: {during:#?}"
  );
  // The message is a turn of its own after it, continuing from its end.
  until("the message's turn", || {
    of(&sent, "BOSS")
      .iter()
      .any(|b| b.to_string().contains("FINDINGS: setup is stale"))
  })
  .await;
  let after = of(&sent, "BOSS");
  let first = after
    .iter()
    .find(|b| b.to_string().contains("FINDINGS: setup is stale"))
    .unwrap();
  assert!(first.to_string().contains("Boss done."), "{first}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interjection_for_another_node_goes_into_its_running_turn() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, _phone_sent) = keyed(vec![(
    "TALK",
    vec![
      tool_call(
        "t1",
        "spawn",
        json!({"node": "mac", "goal": "JOB: count the open issues"}),
      ),
      answer("The Mac counts them."),
    ],
  )])
  .await;
  let (mac_api, mac_sent) = keyed(vec![(
    "JOB",
    vec![
      format!("SLOW {}", tool_call("j1", "wait", json!({}))),
      format!("SLOW {}", tool_call("j2", "wait", json!({}))),
      answer("JOB: 12 open, 3 of them bugs."),
    ],
  )])
  .await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();

  let mut results = Vec::new();
  let Reply::ChatDone { thread, .. } = p
    .chat(&global_chat(None, "TALK: have the Mac count"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let spawned = results
    .iter()
    .find_map(|r| r.strip_prefix("Spawned thread "))
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{results:?}"))
    .to_owned();
  until("the Mac to start", || !of(&mac_sent, "JOB").is_empty()).await;

  let Reply::Lines(lines) = p
    .request(&send_from(
      &spawned,
      "And how many are bugs?",
      &thread,
      true,
    ))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(lines[0].contains("on mac: passed on"), "{lines:?}");
  until("the Mac's turn to end", || of(&mac_sent, "JOB").len() >= 3).await;
  let last = texts(&of(&mac_sent, "JOB")[2]).join("\n");
  assert!(
    last.contains("And how many are bugs?"),
    "woven into the running turn there: {last}"
  );
  tokio::time::sleep(Duration::from_secs(1)).await;
  assert_eq!(of(&mac_sent, "JOB").len(), 3, "no turn of its own followed");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// The threads a node's index holds out of memory: those whose latest
/// `{ thread, remember }` fact says `remember = false`.
fn out_of_memory(data: &Path) -> Vec<String> {
  memory_facts(data)
    .into_iter()
    .filter(|(_, recall)| *recall != Some(true))
    .map(|(t, _)| t)
    .chain(recalling_out_of_memory(data))
    .collect()
}

/// The threads out of memory that may still recall (`recall = true`).
fn recalling_out_of_memory(data: &Path) -> Vec<String> {
  memory_facts(data)
    .into_iter()
    .filter(|(_, recall)| *recall == Some(true))
    .map(|(t, _)| t)
    .collect()
}

/// The threads whose latest `{ thread, remember }` fact says `remember =
/// false`, with its `recall`.
fn memory_facts(data: &Path) -> Vec<(String, Option<bool>)> {
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let mut off: Vec<(String, Option<bool>)> = Vec::new();
  for r in index.records(Some("memory")).unwrap() {
    for f in r.fields["facts"].as_array().cloned().unwrap_or_default() {
      let Some(t) = f.get("thread").and_then(Value::as_str) else {
        continue;
      };
      off.retain(|(o, _)| o != t);
      if f.get("remember") == Some(&json!(false)) {
        off.push((t.to_owned(), f.get("recall").and_then(Value::as_bool)));
      }
    }
  }
  off
}

/// Whether shells can be sandboxed here; not inside another sandbox (an
/// agent's, on macOS), where commands would ask: said when not. (Linux
/// finds the shell helper later, so it is not asked here.)
fn sandboxes() -> bool {
  let ok = !cfg!(target_os = "macos") || stitch_shell::Sandbox::available(None).is_ok();
  if !ok {
    eprintln!("no sandbox for shells here (inside another?): skipped");
  }
  ok
}

/// Fails a test whose commands should run in the sandbox, unasked, when
/// one asks instead: unanswered, it would wait forever.
fn never_asked(r: &Reply) {
  if let Reply::Approve { command, .. } = r {
    panic!(
      "`{command}` asked for approval: no sandbox here (is stitch-shell-helper built? \
       `cargo test --workspace` builds it; else set STITCH_SHELL_HELPER)"
    );
  }
}

/// The thread a `spawn` result names.
fn spawned_in(results: &[String], nth: usize) -> String {
  results
    .iter()
    .filter_map(|r| {
      r.strip_prefix("Spawned thread ").or_else(|| {
        // A spawn to a node not reached: "<node> not reached (…): thread
        // <id> is queued there …".
        r.split_once("): thread ")
          .filter(|(head, _)| head.contains(" not reached ("))
          .map(|(_, rest)| rest)
      })
    })
    .filter_map(|r| r.split_whitespace().next())
    .nth(nth)
    .unwrap_or_else(|| panic!("{results:?}"))
    .to_owned()
}

/// A spawned thread blocked on an approval: its spawner, at work, is told
/// the exact command, the question, the node and place; `threads` lists
/// it blocked with the same; `cancel` stops it, its report saying what it
/// did, and the question is withdrawn.
#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_thread_is_told_listed_and_cancelled() {
  if !sandboxes() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("node");
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "BOSS",
        vec![
          tool_call("b1", "spawn", json!({"goal": "CHILD: publish it"})),
          format!(
            "SLOW {}",
            tool_call("b2", "shell_run", json!({"command": "sleep 2"}))
          ),
          tool_call("b3", "threads", json!({})),
          tool_call("b4", "threads", json!({"status": "idle"})),
          tool_call(
            "b5",
            "cancel",
            json!({"thread": "@SPAWNED@", "reason": "wrong approach"}),
          ),
          tool_call("b6", "threads", json!({})),
          answer("BOSS done."),
        ],
      ),
      (
        "CHILD",
        vec![tool_call(
          "c1",
          "shell_run",
          json!({"command": "git push origin HEAD:refs/heads/never"}),
        )],
      ),
    ],
  )
  .await;
  let mut results = Vec::new();
  c.chat(&global_chat(None, "BOSS: have it published"), |r| {
    never_asked(r);
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let child = spawned_in(&results, 0);

  // Told while at work, woven in after the step: exactly what it asks.
  let told = of(&sent, "BOSS")
    .iter()
    .map(|b| texts(b).join("\n"))
    .find(|t| t.contains("is blocked"))
    .unwrap_or_else(|| panic!("never told: {results:?}"));
  assert!(
    told.contains(&format!("[Thread {child} "))
      && told.contains("`git push origin HEAD:refs/heads/never`")
      && told.contains("asked on mbp, in global; in session \"main\")"),
    "{told}"
  );
  assert_eq!(told.matches("is blocked").count(), 1, "told once: {told}");
  // Listed blocked, with the question and the command.
  let listed = &results[2];
  assert!(
    listed.starts_with(&format!(
      "{child}: runs on mbp, in global, writable: blocked: approval "
    )) && listed.contains("`git push origin HEAD:refs/heads/never`")
      && listed.contains("goal: CHILD: publish it"),
    "{listed}"
  );
  assert_eq!(results[3], "No threads idle.");
  // Cancelled: stopped here, with what it did.
  let cancelled = &results[4];
  assert!(
    cancelled.starts_with(&format!("Cancelled thread {child} "))
      && cancelled.contains("its running turn is stopped")
      && cancelled.contains("(wrong approach)")
      && cancelled.contains(
        "It was waiting for the user's approval to run `git push origin HEAD:refs/heads/never`"
      ),
    "{cancelled}"
  );
  assert!(results[5].contains(": ended: cancelled"), "{results:?}");
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let report = index
    .records(Some("report"))
    .unwrap()
    .into_iter()
    .find(|r| r.fields.get("outcome").and_then(Value::as_str) == Some("cancelled"))
    .expect("its report");
  assert!(report.title.unwrap().contains("cancelled"));
  assert!(
    index.attention().unwrap().is_empty(),
    "its question is withdrawn"
  );
  // The cancellation's report is not delivered on top of the result.
  tokio::time::sleep(Duration::from_secs(7)).await;
  assert_eq!(of(&sent, "BOSS").len(), 7, "nothing woke it");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// `cancel` of a thread at work with a background command: the command's
/// processes end with it, and its job tells nothing.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_a_thread_with_its_running_command() {
  if !sandboxes() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let pid = dir.path().join("pid");
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "BOSS",
        vec![
          tool_call("b1", "spawn", json!({"goal": "CHILD: build it"})),
          format!(
            "SLOW {}",
            tool_call("b2", "shell_run", json!({"command": "sleep 1"}))
          ),
          tool_call("b3", "cancel", json!({"thread": "@SPAWNED@"})),
          answer("BOSS done."),
        ],
      ),
      (
        "CHILD",
        vec![
          tool_call(
            "c1",
            "shell_run",
            json!({"command": format!("bash -c 'trap \"\" HUP INT TERM; sleep 300 & echo $! > {}; wait'", pid.display()), "background": true}),
          ),
          format!("SLOW SLOW {}", answer("never")),
        ],
      ),
    ],
  )
  .await;
  let mut results = Vec::new();
  c.chat(&global_chat(None, "BOSS: have it built"), |r| {
    never_asked(r);
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(
    results[2].contains("its running turn is stopped") && results[2].contains("sleep 300 &"),
    "{results:?}"
  );
  let pid = std::fs::read_to_string(&pid).unwrap().trim().to_owned();
  let mut alive = true;
  for _ in 0..40 {
    let o = std::process::Command::new("ps")
      .args(["-o", "stat=", "-p", &pid])
      .output()
      .unwrap();
    let stat = String::from_utf8_lossy(&o.stdout).trim().to_owned();
    alive = !stat.is_empty() && !stat.starts_with('Z');
    if !alive {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert!(!alive, "its command's process {pid} ended");
  tokio::time::sleep(Duration::from_secs(6)).await;
  assert_eq!(of(&sent, "CHILD").len(), 2, "its job woke nothing");
  assert_eq!(of(&sent, "BOSS").len(), 4, "nothing woke the boss");
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn spawned_threads_are_out_of_memory_from_their_start_unless_asked() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("node");
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "TALK",
        vec![
          tool_call("s1", "spawn", json!({"goal": "QUIET: look into it"})),
          tool_call(
            "s2",
            "spawn",
            json!({"goal": "KEPT: look into it", "remember": true}),
          ),
          tool_call(
            "s3",
            "spawn",
            json!({"goal": "BLANK: look into it", "recall": false}),
          ),
          tool_call(
            "s4",
            "spawn",
            json!({"goal": "OLD KEPT: look into it", "memory": true}),
          ),
          tool_call(
            "s5",
            "spawn",
            json!({"goal": "OLD QUIET: look into it", "memory": false}),
          ),
          answer("All are on it."),
        ],
      ),
      (
        "DETACHED",
        vec![format!("SLOW {}", answer("Detached, done."))],
      ),
    ],
  )
  .await;

  // The spawn tool: out of memory unless asked, recalling unless asked
  // not to; the old `memory` still means both or neither.
  let mut results = Vec::new();
  let Reply::ChatDone { thread: talk, .. } = c
    .chat(&global_chat(None, "TALK: spawn five"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let (quiet, kept) = (spawned_in(&results, 0), spawned_in(&results, 1));
  let (blank, old_kept, old_quiet) = (
    spawned_in(&results, 2),
    spawned_in(&results, 3),
    spawned_in(&results, 4),
  );
  let off = out_of_memory(&data);
  let recalling = recalling_out_of_memory(&data);
  assert!(off.contains(&quiet), "{off:?}");
  assert!(
    recalling.contains(&quiet),
    "recalls by default: {recalling:?}"
  );
  assert!(!off.contains(&kept), "asked to be remembered: {off:?}");
  assert!(
    off.contains(&blank) && !recalling.contains(&blank),
    "{off:?}"
  );
  assert!(!off.contains(&old_kept), "`memory: true`: {off:?}");
  assert!(
    off.contains(&old_quiet) && !recalling.contains(&old_quiet),
    "`memory: false`, as before: {recalling:?}"
  );
  assert!(
    !off.contains(&talk),
    "the user's chat is unchanged: {off:?}"
  );

  // `stitch chat --detach`: out of memory before its first step is written.
  let Reply::Detached(detached) = c
    .request(&detach_global("DETACHED: look into it", &talk))
    .await
    .unwrap()
  else {
    panic!()
  };
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let id: stitch_records::RecordId = detached.parse().unwrap();
  assert!(
    index.thread_segments(&id).unwrap().is_empty(),
    "its first step is written already: the test proves nothing"
  );
  let off = out_of_memory(&data);
  assert!(off.contains(&detached), "{off:?}");
  assert!(recalling_out_of_memory(&data).contains(&detached));
  let mut asked = detach_global("REMEMBERED: look into it", &talk);
  if let Request::Detach { memory, .. } = &mut asked {
    *memory = true;
  }
  let Reply::Detached(remembered) = c.request(&asked).await.unwrap() else {
    panic!()
  };
  assert!(!out_of_memory(&data).contains(&remembered));
  until("the detached thread to answer", || {
    !of(&sent, "DETACHED").is_empty()
  })
  .await;

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A place whose settings say `memory = false` wins over the spawn tool's
/// `remember: true` (design §2.9): the thread starts out of memory,
/// recalling; the spawn's result says so, and the thread is told. Asked
/// elsewhere, it is remembered.
#[tokio::test(flavor = "multi_thread")]
async fn a_place_out_of_memory_wins_over_a_spawns_remember() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let app = d.join("app");
  checkout(&app, "APP RULES\n");
  std::fs::create_dir_all(app.join(".stitch")).unwrap();
  std::fs::write(app.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  let (node, mut c, sent) = alone(
    &d,
    vec![(
      "TALK",
      vec![
        tool_call(
          "s1",
          "spawn",
          json!({"goal": "KEPT: look into it", "remember": true, "place": app.display().to_string()}),
        ),
        tool_call(
          "s2",
          "spawn",
          json!({"goal": "ELSEWHERE: look into it", "remember": true}),
        ),
        answer("Both are on it."),
      ],
    )],
  )
  .await;
  let mut results = Vec::new();
  c.chat(&global_chat(None, "TALK: spawn two"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let (kept, elsewhere) = (spawned_in(&results, 0), spawned_in(&results, 1));
  assert!(
    results[0].contains("Not remembered: its place's settings say `memory = false`"),
    "{results:?}"
  );
  assert!(!results[1].contains("Not remembered"), "{results:?}");
  let data = d.join("node");
  assert!(
    recalling_out_of_memory(&data).contains(&kept),
    "kept out by its place, recalling: {:?}",
    memory_facts(&data)
  );
  assert!(!out_of_memory(&data).contains(&elsewhere));
  until("the kept thread to run", || !of(&sent, "KEPT").is_empty()).await;
  let told = of(&sent, "KEPT")[0].to_string();
  let note = "this thread was asked to be remembered, but its place's settings say";
  assert!(told.contains(note), "{told}");
  let elsewhere_told = {
    until("the other to run", || !of(&sent, "ELSEWHERE").is_empty()).await;
    of(&sent, "ELSEWHERE")[0].to_string()
  };
  assert!(!elsewhere_told.contains(note), "{elsewhere_told}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The user's search ranks what agent-to-agent threads said (spawned,
/// nobody's messages but threads') below the user's own conversations,
/// however much better the words match; it still finds them.
#[tokio::test(flavor = "multi_thread")]
async fn the_users_search_ranks_agent_to_agent_threads_below_their_own() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      ("TALK", vec![answer("Every hour, I think.")]),
      (
        "AGENTS",
        vec![answer(
          "Terrarium misting: terrarium misting checked, terrarium misting logged.",
        )],
      ),
    ],
  )
  .await;
  let Reply::ChatDone { thread: talk, .. } = c
    .chat(
      &global_chat(
        None,
        "TALK: what is the schedule of the terrarium misting, again?",
      ),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  let mut spawn = detach_global("AGENTS: terrarium misting, terrarium misting", &talk);
  if let Request::Detach { keep, .. } = &mut spawn {
    *keep = true;
  }
  let Reply::Detached(agents) = c.request(&spawn).await.unwrap() else {
    panic!()
  };
  until("the agents' thread to answer", || {
    !of(&sent, "AGENTS").is_empty()
  })
  .await;
  let search = Request::Search {
    query: "terrarium misting".into(),
    here: false,
    cwd: None,
    limit: 10,
  };
  let mut hits = Vec::new();
  for _ in 0..100 {
    let Reply::Hits(h) = c.request(&search).await.unwrap() else {
      panic!()
    };
    hits = h;
    if hits.iter().filter(|h| h.thread == agents).count() >= 2 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  let threads: Vec<&str> = hits.iter().map(|h| h.thread.as_str()).collect();
  let first_agents = threads.iter().position(|t| *t == agents);
  let last_talk = threads.iter().rposition(|t| *t == talk);
  assert!(
    first_agents.is_some() && last_talk.is_some(),
    "both found: {hits:?}"
  );
  assert!(
    last_talk < first_agents,
    "the user's own conversation first: {hits:?}"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_spawned_on_another_node_is_out_of_memory_there_unless_asked() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    d,
    vec![
      tool_call(
        "s1",
        "spawn",
        json!({"node": "mac", "goal": "QUIET: count the issues"}),
      ),
      tool_call(
        "s2",
        "spawn",
        json!({"node": "mac", "goal": "KEPT: count the bugs", "remember": true}),
      ),
      answer("The Mac counts them."),
      answer("Noted."),
      answer("Noted again."),
    ],
    vec![answer("12 issues."), answer("3 bugs.")],
  )
  .await;
  let mut results = Vec::new();
  p.chat(&global_chat(None, "Have the Mac count."), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let (quiet, kept) = (spawned_in(&results, 0), spawned_in(&results, 1));
  until("the Mac to start both", || {
    mac_sent.lock().unwrap().len() >= 2
  })
  .await;
  let mac = d.join("mac");
  let index = Index::open(&mac.join("index.sqlite")).unwrap();
  for t in [&quiet, &kept] {
    assert!(index.record(&t.parse().unwrap()).unwrap().is_some(), "{t}");
  }
  let off = out_of_memory(&mac);
  assert!(off.contains(&quiet), "{off:?}");
  assert!(
    recalling_out_of_memory(&mac).contains(&quiet),
    "recalls there by default: {off:?}"
  );
  assert!(!off.contains(&kept), "asked to be remembered: {off:?}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// An assignment asking to be remembered (`memory = true`), claimed by
/// the node it is for: its place's `memory = false` there wins (design
/// §2.9); the spawner, which cannot read that place, is told it might.
#[tokio::test(flavor = "multi_thread")]
async fn an_assignment_to_a_place_out_of_memory_is_out_of_memory() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let quiet = d.join("quiet");
  checkout(&quiet, "QUIET RULES\n");
  std::fs::create_dir_all(quiet.join(".stitch")).unwrap();
  std::fs::write(quiet.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    &d,
    vec![
      tool_call(
        "s1",
        "spawn",
        json!({"node": "mac", "goal": "KEPT: count the bugs", "remember": true,
          "place": quiet.display().to_string()}),
      ),
      answer("The Mac counts them."),
      answer("Noted."),
    ],
    vec![answer("3 bugs.")],
  )
  .await;
  let mut results = Vec::new();
  p.chat(&global_chat(None, "Have the Mac count."), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let kept = spawned_in(&results, 0);
  assert!(
    results[0].contains("unless its place there says `memory = false`"),
    "{results:?}"
  );
  until("the Mac to start it", || {
    !mac_sent.lock().unwrap().is_empty()
  })
  .await;
  let mac = d.join("mac");
  assert!(
    recalling_out_of_memory(&mac).contains(&kept),
    "kept out by its place, recalling: {:?}",
    memory_facts(&mac)
  );
  let told = mac_sent.lock().unwrap()[0].to_string();
  assert!(
    told.contains("this thread was asked to be remembered, but its place's settings say"),
    "{told}"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A git checkout with one commit, holding `AGENTS.md` with `rules`.
fn checkout(dir: &Path, rules: &str) {
  std::fs::create_dir_all(dir).unwrap();
  std::fs::write(dir.join("AGENTS.md"), rules).unwrap();
  for args in [
    &["init", "-q"][..],
    &["add", "."],
    &[
      "-c",
      "user.name=t",
      "-c",
      "user.email=t@t",
      "commit",
      "-qm",
      "first",
    ],
  ] {
    let ok = std::process::Command::new("git")
      .arg("-C")
      .arg(dir)
      .args(args)
      .status()
      .unwrap()
      .success();
    assert!(ok, "git {args:?}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_spawned_to_a_place_on_another_node_works_in_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let otter = d.join("otter");
  checkout(&otter, "OTTER RULES: work in place.\n");
  let missing = d.join("nowhere");
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    &d,
    vec![
      tool_call(
        "s1",
        "spawn",
        json!({"node": "mac", "goal": "Look at the repo.", "place": otter.display().to_string()}),
      ),
      tool_call(
        "s2",
        "spawn",
        json!({"node": "mac", "goal": "Look elsewhere.", "place": missing.display().to_string()}),
      ),
      tool_call(
        "s3",
        "spawn",
        json!({"node": "mac", "goal": "Go on.", "place": otter.display().to_string(), "context": "this"}),
      ),
      answer("The Mac looks."),
      answer("Noted."),
    ],
    vec![answer("A tidy repo.")],
  )
  .await;
  let mut results = Vec::new();
  p.chat(&global_chat(None, "Have the Mac look at otter."), |r| {
    if let Reply::ToolResult { ok, output } = r {
      results.push((*ok, output.clone()));
    }
    None
  })
  .await
  .unwrap();
  // Asked of the Mac: the checkout is its project, the other path is not there.
  assert!(results[0].0, "{results:?}");
  assert!(results[0].1.contains("on mac, in otter."), "{results:?}");
  assert!(!results[1].0, "{results:?}");
  assert!(
    results[1].1.contains("is not a directory on mac"),
    "{results:?}"
  );
  // Its conversation is this place's records, not the checkout's.
  assert!(!results[2].0, "{results:?}");
  assert!(
    results[2].1.contains("put what it needs in the goal"),
    "{results:?}"
  );
  until("the Mac to start it", || {
    !mac_sent.lock().unwrap().is_empty()
  })
  .await;
  // The thread works in the checkout: its instructions are there.
  let body = mac_sent.lock().unwrap()[0].clone();
  assert!(body["system"].to_string().contains("OTTER RULES"), "{body}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A pinned app served by the phone, of a thread the Mac runs: its report
/// goes to the Mac as a message, and wakes the thread there (the phone
/// runs nothing of it); the page hears where it went.
#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_app_on_the_phone_reports_to_its_thread_on_the_mac() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let page = d.join("dash.html");
  std::fs::write(&page, "<p>dash</p>").unwrap();
  let (phone_node, mac_node, mut p, mut m, phone_sent, mac_sent) = pair(
    &d,
    vec![],
    vec![
      tool_call(
        "a1",
        "show_app",
        json!({"path": page.display().to_string(), "name": "Dash"}),
      ),
      answer("Shown."),
      answer("Refreshed on the Mac."),
    ],
  )
  .await;
  let thread = chat_thread(&mut m, "Make me a dashboard").await;
  p.request(&Request::Sync { peer: None }).await.unwrap();
  let Reply::Apps { apps, .. } = p
    .request(&Request::AppPin {
      what: thread.clone(),
      name: None,
      icon: None,
      fixed: Some(false),
    })
    .await
    .unwrap()
  else {
    panic!("not pinned on the phone")
  };
  let host = stitch_node::apps::pinned_host(&apps[0].id.parse().unwrap());
  let r = phone_node
    .serve_app(stitch_node::apps::serve::Request {
      host,
      path: "/_stitch/report".into(),
      method: "POST".into(),
      body: b"{\"refresh\":true}".to_vec(),
    })
    .await;
  let said = String::from_utf8_lossy(&r.body).into_owned();
  assert_eq!(r.status, 200, "{said}");
  assert!(said.contains("passed on to mac"), "{said}");
  until("the Mac to take the report", || {
    mac_sent.lock().unwrap().len() >= 3
  })
  .await;
  let all = texts(&mac_sent.lock().unwrap()[2]).join("\n");
  assert!(all.contains("[Pinned app \"Dash\""), "{all}");
  assert!(
    phone_sent.lock().unwrap().is_empty(),
    "the phone ran nothing"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A thread that reported, sent a message: it reads as working, from its
/// records (as every node and the app read it), while the turn the message
/// started runs, and as its new outcome once that turn reports; a `claim`
/// of the thread says the turn started, for the other nodes.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_sent_a_message_after_it_reported_reads_as_working() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "WORK",
        vec![
          answer("Not done: the first pass."),
          format!("SLOW {}", tool_call("w1", "wait", json!({}))),
          format!("SLOW {}", answer("WORK done, second pass.")),
        ],
      ),
      ("BOSS", vec![answer("Boss here.")]),
    ],
  )
  .await;
  let boss = chat_thread(&mut c, "BOSS: plan the release").await;
  let Reply::Detached(work) = c
    .request(&detach_global("WORK: prepare the release", &boss))
    .await
    .unwrap()
  else {
    panic!()
  };
  let work_id: stitch_records::RecordId = work.parse().unwrap();
  let index_path = dir.path().join("node").join("index.sqlite");
  let state = || {
    let index = Index::open(&index_path).unwrap();
    let t = index.record(&work_id).unwrap().unwrap();
    stitch_node::chat::detached_state(&index, &t).unwrap()
  };
  until("its first report", || {
    state().as_deref() == Some("not-done")
  })
  .await;

  let Reply::Lines(lines) = c
    .request(&send_from(&work, "And the changelog?", &boss, false))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(!lines.is_empty());
  until("the turn the message starts", || {
    of(&sent, "WORK").len() >= 2
  })
  .await;
  assert_eq!(state().as_deref(), Some("running"), "working while it runs");
  let index = Index::open(&index_path).unwrap();
  assert_eq!(
    index
      .sources("claim", "thread", &work_id, None)
      .unwrap()
      .len(),
    1,
    "its turn said started, for every node"
  );
  drop(index);
  until("its second report", || state().as_deref() == Some("done")).await;

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A spawned thread sent a message when its provider no longer resolves:
/// its turn fails before its model call, and says so as a failed model
/// call does, the message in the thread with why, and a `failed` report
/// to its spawner; not only a line in the node's log, the thread working
/// forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_whose_turn_fails_before_its_model_call_reports_it_failed() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, _sent) = alone(
    dir.path(),
    vec![
      ("WORK", vec![answer("Done: the first pass.")]),
      ("BOSS", vec![answer("Boss here."), answer("Noted.")]),
    ],
  )
  .await;
  let boss = chat_thread(&mut c, "BOSS: plan the release").await;
  let Reply::Detached(work) = c
    .request(&detach_global("WORK: prepare the release", &boss))
    .await
    .unwrap()
  else {
    panic!()
  };
  let work_id: stitch_records::RecordId = work.parse().unwrap();
  let data = dir.path().join("node");
  let index_path = data.join("index.sqlite");
  let state = || {
    let index = Index::open(&index_path).unwrap();
    let t = index.record(&work_id).unwrap().unwrap();
    stitch_node::chat::detached_state(&index, &t).unwrap()
  };
  until("its first report", || state().as_deref() == Some("done")).await;
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
  let mut config = Config::load(&data).unwrap();
  config.providers.get_mut("anthropic").unwrap().cache_ttl = Some("never".into());
  // Its old port may not be free again yet.
  config.port = Some(free_port());
  config.save(&data).unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  c.request(&send_from(&work, "And the changelog?", &boss, false))
    .await
    .unwrap();
  until("its failed report", || state().as_deref() == Some("failed")).await;
  let Reply::Lines(l) = c
    .request(&Request::Transcript {
      thread: work.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let read: Value = serde_json::from_str(&l[0]).unwrap();
  let entries = read["entries"].as_array().unwrap();
  let failed = entries.last().unwrap();
  assert_eq!(failed["kind"], "failed", "{read}");
  assert!(
    failed["text"].as_str().unwrap().contains("cache_ttl"),
    "{read}"
  );
  assert!(
    entries.iter().any(|e| e["text"]
      .as_str()
      .is_some_and(|t| t.contains("And the changelog?"))),
    "{read}"
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

async fn needs(c: &mut Client) -> Vec<String> {
  let Reply::Lines(l) = c
    .request(&Request::Live {
      what: "attention".into(),
      query: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let v: Value = serde_json::from_str(&l[0]).unwrap();
  v["threads"]
    .as_array()
    .unwrap()
    .iter()
    .map(|t| t["id"].as_str().unwrap().to_owned())
    .collect()
}

/// "Needs you" counts a blocked thread only when nobody else has its
/// report: a spawned thread's blocked report went to its parent (not
/// counted) unless it was kept for the user; a thread with no parent counts;
/// a dismissal hides it until it moves again.
#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_report_needs_the_user_only_when_nobody_else_has_it() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, _sent) = alone(
    dir.path(),
    vec![
      ("BOSS", vec![answer("Boss here.")]),
      ("KID", vec![answer("Blocked: no access.")]),
      ("KEPT", vec![answer("Blocked: no access either.")]),
      ("ORPHAN", vec![answer("Blocked: nobody to ask.")]),
    ],
  )
  .await;
  let boss = chat_thread(&mut c, "BOSS: plan").await;
  let spawn = |goal: &str, parent: Option<&str>, keep: bool| {
    let mut r = detach_global(goal, parent.unwrap_or(""));
    if let Request::Detach {
      parent: p, keep: k, ..
    } = &mut r
    {
      *p = parent.map(str::to_owned);
      *k = keep;
    }
    r
  };
  let mut ids = Vec::new();
  for (goal, parent, keep) in [
    ("KID: try", Some(boss.as_str()), false),
    ("KEPT: try", Some(boss.as_str()), true),
    ("ORPHAN: try", None, false),
  ] {
    let Reply::Detached(id) = c.request(&spawn(goal, parent, keep)).await.unwrap() else {
      panic!()
    };
    ids.push(id);
  }
  let index_path = dir.path().join("node").join("index.sqlite");
  for id in &ids {
    let rid: stitch_records::RecordId = id.parse().unwrap();
    until("its blocked report", || {
      let index = Index::open(&index_path).unwrap();
      let t = index.record(&rid).unwrap().unwrap();
      stitch_node::chat::detached_state(&index, &t)
        .unwrap()
        .as_deref()
        == Some("blocked")
    })
    .await;
  }
  let mut got = needs(&mut c).await;
  got.sort();
  let mut want = vec![ids[1].clone(), ids[2].clone()];
  want.sort();
  assert_eq!(
    got, want,
    "the delivered report is not the user's to answer"
  );

  c.request(&Request::LooseEnds {
    check: false,
    dismiss: Some(ids[2].clone()),
  })
  .await
  .unwrap();
  assert_eq!(needs(&mut c).await, vec![ids[1].clone()], "dismissed");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn ws_open(port: u16, host: &str, cookie: &str) -> Ws {
  use tokio_tungstenite::tungstenite::client::IntoClientRequest;
  let mut req = format!("ws://127.0.0.1:{port}/_stitch/ws")
    .into_client_request()
    .unwrap();
  let h = req.headers_mut();
  h.insert("host", host.parse().unwrap());
  h.insert("origin", format!("http://{host}").parse().unwrap());
  h.insert("cookie", cookie.parse().unwrap());
  let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
    .await
    .unwrap();
  tokio_tungstenite::client_async(req, tcp).await.unwrap().0
}

async fn ws_send(ws: &mut Ws, v: Value) {
  use futures_util::SinkExt;
  ws.send(tokio_tungstenite::tungstenite::Message::text(v.to_string()))
    .await
    .unwrap();
}

async fn ws_next(ws: &mut Ws) -> Value {
  use futures_util::StreamExt;
  loop {
    match tokio::time::timeout(Duration::from_secs(30), ws.next())
      .await
      .expect("a frame in time")
    {
      Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => {
        return serde_json::from_str(t.as_str()).unwrap();
      }
      Some(Ok(_)) => {}
      other => panic!("{other:?}"),
    }
  }
}

/// The web client (design §2.24) and the in-process app (the phone's) typing
/// into a thread another node owns: by default the turn runs on its owner and
/// streams back on the request; a socket process (the TUI) runs it where it is
/// typed; after "Run here" (`RunOn` none) the web client runs it on this node.
#[tokio::test(flavor = "multi_thread")]
async fn the_web_client_continues_a_foreign_thread_on_its_owner_by_default() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let web_port = free_port();
  let (phone_node, mac_node, mut p, phone_sent, mac_sent) = keyed_pair_web(
    d,
    vec![(
      "TALK",
      vec![
        answer("Ran for the TUI."),
        answer("Ran here."),
        answer("Asked here."),
      ],
    )],
    vec![(
      "TALK",
      vec![
        answer("Started on the Mac."),
        answer("Ran on the Mac."),
        answer("Ran on the Mac for the app."),
      ],
    )],
    false,
    web_port,
  )
  .await;
  // A thread the Mac owns, synced to the phone.
  let mut m = Client::connect(&d.join("mac")).await.unwrap();
  let thread = chat_thread(&mut m, "TALK: hello").await;
  m.request(&Request::Sync { peer: None }).await.unwrap();
  p.request(&Request::Sync { peer: None }).await.unwrap();
  let tid: stitch_records::RecordId = thread.parse().unwrap();
  let phone_index = d.join("phone").join("index.sqlite");
  until("the thread on the phone", || {
    Index::open(&phone_index)
      .unwrap()
      .record(&tid)
      .unwrap()
      .is_some()
  })
  .await;

  // The web client of the phone.
  let Reply::App { url, .. } = p
    .request(&Request::AppOpen {
      what: "Stitch".into(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let client_host = url
    .unwrap()
    .strip_prefix("http://")
    .unwrap()
    .split('.')
    .next()
    .unwrap()
    .to_owned();
  let token = std::fs::read_to_string(d.join("phone").join("apps.token")).unwrap();
  let cookie = format!("stitch={}", token.trim());
  let host = format!("{client_host}.localhost:{web_port}");
  let mut ws = ws_open(web_port, &host, &cookie).await;
  ws_send(
    &mut ws,
    json!({"id": 1, "request": {"Hello": {"version": stitch_node::VERSION}}}),
  )
  .await;
  ws_next(&mut ws).await;

  let chat_into = async |ws: &mut Ws, id: u64, text: &str| -> String {
    let req = serde_json::to_value(global_chat(Some(thread.clone()), text)).unwrap();
    ws_send(ws, json!({"id": id, "request": req})).await;
    let mut said = String::new();
    loop {
      let m = ws_next(ws).await;
      if m["id"] != id {
        continue;
      }
      assert!(m["reply"].get("Error").is_none(), "{m}");
      if let Some(t) = m["reply"]["Text"].as_str() {
        said.push_str(t);
      }
      if m["reply"].get("ChatDone").is_some() {
        return said;
      }
    }
  };
  // Unpinned: the owner runs it, streamed back here.
  let said = chat_into(&mut ws, 2, "TALK: go on").await;
  assert!(said.contains("Ran on the Mac."), "{said}");
  assert_eq!(of(&phone_sent, "TALK").len(), 0, "nothing ran on the phone");
  assert_eq!(of(&mac_sent, "TALK").len(), 2);

  // The in-process app peer (the phone's own app): the same default.
  let mut app = phone_node.client().await.unwrap();
  let mut app_said = String::new();
  app
    .chat(
      &global_chat(Some(thread.clone()), "TALK: from the app"),
      |r| {
        if let Reply::Text(t) = r {
          app_said.push_str(t);
        }
        None
      },
    )
    .await
    .unwrap();
  assert!(
    app_said.contains("Ran on the Mac for the app."),
    "{app_said}"
  );
  assert_eq!(of(&phone_sent, "TALK").len(), 0, "nothing ran on the phone");
  assert_eq!(of(&mac_sent, "TALK").len(), 3);

  // A socket process (the TUI) still runs it where it is typed.
  let mut tui_said = String::new();
  p.chat(
    &global_chat(Some(thread.clone()), "TALK: from the TUI"),
    |r| {
      if let Reply::Text(t) = r {
        tui_said.push_str(t);
      }
      None
    },
  )
  .await
  .unwrap();
  assert!(tui_said.contains("Ran for the TUI."), "{tui_said}");
  assert_eq!(of(&mac_sent, "TALK").len(), 3, "nothing more on the Mac");

  // "Run here": an explicit choice, it runs on the phone.
  p.request(&run_on(&thread, None)).await.unwrap();
  let said = chat_into(&mut ws, 3, "TALK: now here").await;
  assert!(said.contains("Ran here."), "{said}");
  assert_eq!(of(&mac_sent, "TALK").len(), 3, "nothing more on the Mac");

  // After "Run here" (the phone's "Ask here (no shells)"), the app's own
  // chat runs on the phone too.
  let mut here = String::new();
  app
    .chat(&global_chat(Some(thread.clone()), "TALK: ask here"), |r| {
      if let Reply::Text(t) = r {
        here.push_str(t);
      }
      None
    })
    .await
    .unwrap();
  assert!(here.contains("Asked here."), "{here}");
  assert_eq!(of(&mac_sent, "TALK").len(), 3, "nothing more on the Mac");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A writable thread spawned into a checkout where another writable
/// thread works (same node, same path) is started, its result naming that
/// thread (2026-10-17: two writers in one checkout); a read-only one, or
/// one elsewhere, says nothing of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_spawn_into_a_busy_place_names_the_thread_at_work_there() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let work = d.join("work");
  checkout(&work, "WORK RULES\n");
  let other = d.join("other");
  checkout(&other, "OTHER RULES\n");
  let place = |p: &Path| p.display().to_string();
  let (node, mut c, _sent) = alone(
    &d,
    vec![
      (
        "TALK",
        vec![
          tool_call(
            "s1",
            "spawn",
            json!({"goal": "BUSY: change things", "place": place(&work)}),
          ),
          tool_call(
            "s2",
            "spawn",
            json!({"goal": "SECOND: change more", "place": place(&work)}),
          ),
          tool_call(
            "s3",
            "spawn",
            json!({"goal": "LOOK: only look", "place": place(&work), "read_only": true}),
          ),
          tool_call(
            "s4",
            "spawn",
            json!({"goal": "ELSE: change another", "place": place(&other)}),
          ),
          answer("Four on it."),
        ],
      ),
      // At work while the others are spawned.
      ("BUSY", vec![format!("SLOW {}", answer("Busy, done."))]),
    ],
  )
  .await;
  let mut results = Vec::new();
  c.chat(&global_chat(None, "TALK: spawn them"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let busy = spawned_in(&results, 0);
  assert!(!results[0].contains("Note:"), "{results:?}");
  assert!(
    results[1].starts_with("Spawned thread ") && results[1].contains("another writable thread"),
    "said, not refused: {results:?}"
  );
  assert!(results[1].contains(&busy), "names it: {results:?}");
  assert!(!results[2].contains("Note:"), "read-only: {results:?}");
  assert!(!results[3].contains("Note:"), "elsewhere: {results:?}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A spawned thread is told who it is (its ID, in its prompt and with its
/// goal), `threads` marks it "(you)", and it cannot hand its own goal
/// straight back down; a part of it, it can (2026-10-19: a spawned thread
/// spawned its goal again, and that one again, and the first then stopped
/// for "a duplicate" that was itself).
#[tokio::test(flavor = "multi_thread")]
async fn a_spawned_thread_knows_itself_and_does_its_goal_itself() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (node, mut c, sent) = alone(
    &d,
    vec![
      (
        "TALK",
        vec![
          tool_call(
            "s1",
            "spawn",
            json!({"goal": "CHILD: redesign the gate adapter"}),
          ),
          answer("On it."),
        ],
      ),
      (
        "CHILD",
        vec![
          tool_call(
            "c1",
            "spawn",
            json!({"goal": "CHILD: redesign the gate adapter, please"}),
          ),
          tool_call("c2", "threads", json!({"all": true})),
          tool_call("c3", "spawn", json!({"goal": "PART: check the screws"})),
          answer("Did it myself."),
        ],
      ),
      ("PART", vec![answer("Screws fine.")]),
    ],
  )
  .await;
  let mut results = Vec::new();
  c.chat(&global_chat(None, "TALK: start it"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let child = spawned_in(&results, 0);
  assert!(results[0].contains(" to run on mbp, in "), "{results:?}");
  until("the child's four steps", || of(&sent, "CHILD").len() >= 4).await;
  let steps = of(&sent, "CHILD");
  // Its prompt and its first message say who it is.
  let system = steps[0]["system"].to_string();
  assert!(
    system.contains(&format!("This thread is {child}: that ID is you")),
    "{system}"
  );
  let first = texts(&steps[0]);
  assert!(
    first.iter().any(|t| t.starts_with(&format!(
      "[Stitch: you are thread {child}, running on node mbp, spawned by thread "
    )) && t.contains("Do it yourself")),
    "{first:?}"
  );
  // Its own goal, reworded, is refused.
  let after_c1 = steps[1]["messages"].to_string();
  assert!(
    after_c1.contains("refused: this goal is your own goal"),
    "{after_c1}"
  );
  // It finds itself in the list.
  let after_c2 = steps[2]["messages"].to_string();
  assert!(
    after_c2.contains(&format!("{child} (you): runs on mbp")),
    "{after_c2}"
  );
  // A part of it is spawned, with a word about its own place.
  let after_c3 = steps[3]["messages"].to_string();
  assert!(
    after_c3.contains("Spawned thread ")
      && after_c3.contains("you were spawned to work in this place"),
    "{after_c3}"
  );
  until("the part to run", || !of(&sent, "PART").is_empty()).await;

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// The phone and the Mac, paired and running, each node's model answering
/// from its own scripts (`keyed`).
async fn keyed_pair(
  d: &Path,
  phone_says: Vec<(&str, Vec<String>)>,
  mac_says: Vec<(&str, Vec<String>)>,
  mac_shell_off: bool,
) -> (
  stitch_node::NodeHandle,
  stitch_node::NodeHandle,
  Client,
  Arc<Mutex<Vec<Value>>>,
  Arc<Mutex<Vec<Value>>>,
) {
  keyed_pair_web(d, phone_says, mac_says, mac_shell_off, 0).await
}

/// `keyed_pair`, the phone also serving its apps (the web client) on `web_port` (0: not).
async fn keyed_pair_web(
  d: &Path,
  phone_says: Vec<(&str, Vec<String>)>,
  mac_says: Vec<(&str, Vec<String>)>,
  mac_shell_off: bool,
  web_port: u16,
) -> (
  stitch_node::NodeHandle,
  stitch_node::NodeHandle,
  Client,
  Arc<Mutex<Vec<Value>>>,
  Arc<Mutex<Vec<Value>>>,
) {
  keyed_pair_cfg(d, phone_says, mac_says, mac_shell_off, web_port, |_| {}).await
}

/// `keyed_pair_web`, the Mac's config also changed by `mac_config`.
async fn keyed_pair_cfg(
  d: &Path,
  phone_says: Vec<(&str, Vec<String>)>,
  mac_says: Vec<(&str, Vec<String>)>,
  mac_shell_off: bool,
  web_port: u16,
  mac_config: impl FnOnce(&mut Config),
) -> (
  stitch_node::NodeHandle,
  stitch_node::NodeHandle,
  Client,
  Arc<Mutex<Vec<Value>>>,
  Arc<Mutex<Vec<Value>>>,
) {
  let (phone_api, phone_sent) = keyed(phone_says).await;
  let (mac_api, mac_sent) = keyed(mac_says).await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  if web_port != 0 {
    let mut c = Config::load(&phone).unwrap();
    c.apps = Some(AppsSettings {
      port: Some(web_port),
    });
    c.save(&phone).unwrap();
  }
  if mac_shell_off {
    let mut c = Config::load(&mac).unwrap();
    c.shell = Some(stitch_node::config::ShellSettings {
      approval: Some("off".into()),
      ..Default::default()
    });
    c.save(&mac).unwrap();
  }
  let mut c = Config::load(&mac).unwrap();
  mac_config(&mut c);
  c.save(&mac).unwrap();
  let home = RecordRoot::init(&d.join("phone-home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&phone.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  (phone_node, mac_node, p, phone_sent, mac_sent)
}

fn run_on(thread: &str, node: Option<&str>) -> Request {
  Request::RunOn {
    thread: thread.into(),
    node: node.map(str::to_owned),
  }
}

/// A chat typed on the phone into a thread the Mac runs (passed on as an
/// assignment with `chat = true`) takes the Mac's `[chat] max_steps` and,
/// at that limit, asks the user for more steps, as a local chat does
/// (issue 2026-11-14T085125Z-gamer-63f7: it ran to the 50 of a sent turn
/// and wrapped up without asking).
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_typed_for_a_thread_on_another_node_asks_for_more_steps_at_the_limit() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, _phone_sent, mac_sent) = keyed_pair_cfg(
    d,
    vec![("TALK", vec![answer("Hello from the phone.")])],
    vec![(
      "TALK",
      (1..=6)
        .map(|i| tool_call(&format!("m{i}"), "threads", json!({})))
        .chain([answer("Done on the Mac.")])
        .collect(),
    )],
    false,
    0,
    |c| c.chat = Some(stitch_node::config::ChatSettings { max_steps: Some(2) }),
  )
  .await;
  let thread = chat_thread(&mut p, "TALK: hello").await;
  p.request(&run_on(&thread, Some("mac"))).await.unwrap();

  // Typed on the phone, from a client of its own: the turn waits for an
  // answer at the limit, so this does not return meanwhile.
  let mut typing = Client::connect(&d.join("phone")).await.unwrap();
  let typed = tokio::spawn({
    let thread = thread.clone();
    async move {
      typing
        .chat(&global_chat(Some(thread), "TALK: work a while"), |_| None)
        .await
    }
  });
  let phone_index = Index::open(&d.join("phone").join("index.sqlite")).unwrap();
  let mut question = None;
  for _ in 0..300 {
    let _ = p.request(&Request::Sync { peer: None }).await;
    question = phone_index
      .records(Some("question"))
      .unwrap()
      .into_iter()
      .find(|q| format!("{q:?}").contains("Continue for 2 more steps"));
    if question.is_some() || typed.is_finished() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  let question = question.unwrap_or_else(|| {
    panic!(
      "no question to continue at the chat's step limit; the turn ended: {:?}, the Mac took {} model calls",
      typed.is_finished(),
      of(&mac_sent, "TALK").len()
    )
  });
  assert!(
    format!("{question:?}").contains("This turn has taken 2 and is not done"),
    "{question:?}"
  );
  // Declined: it wraps up and the chat ends.
  p.request(&Request::Answer {
    question: question.id.to_string(),
    allow: false,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  let done = tokio::time::timeout(Duration::from_secs(30), typed)
    .await
    .expect("the chat ends once the question is answered")
    .unwrap()
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  let mut m = Client::connect(&d.join("mac")).await.unwrap();
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_continued_on_another_node_runs_there_and_streams_back() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, phone_sent, mac_sent) = keyed_pair(
    dir.path(),
    vec![(
      "TALK",
      vec![
        answer("Hello from the phone."),
        answer("Back on the phone."),
      ],
    )],
    vec![(
      "TALK",
      vec![
        format!("SLOW {}", tool_call("m1", "threads", json!({}))),
        answer("Done on the Mac."),
      ],
    )],
    false,
  )
  .await;
  // Started on the phone.
  let thread = chat_thread(&mut p, "TALK: hello").await;

  // Continue on the Mac.
  let Reply::Lines(said) = p.request(&run_on(&thread, Some("mac"))).await.unwrap() else {
    panic!()
  };
  assert!(said[0].contains("now runs on mac"), "{said:?}");
  let Reply::Lines(on) = p
    .request(&Request::RunsOn {
      thread: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(on, vec!["mac".to_owned()]);

  // Typed on the phone: it runs on the Mac, and streams here as it runs.
  let mut seen: Vec<Reply> = Vec::new();
  let done = p
    .chat(&global_chat(Some(thread.clone()), "TALK: list them"), |r| {
      seen.push(r.clone());
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = &done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "end_turn", "{seen:?}");
  assert!(
    seen.iter().any(|r| matches!(r, Reply::ToolCall(_))),
    "the Mac's tool call streamed here: {seen:?}"
  );
  let text: String = seen
    .iter()
    .filter_map(|r| match r {
      Reply::Text(t) => Some(t.as_str()),
      _ => None,
    })
    .collect();
  assert!(text.contains("Done on the Mac."), "{seen:?}");
  assert_eq!(of(&phone_sent, "TALK").len(), 1, "nothing ran on the phone");
  let on_mac = of(&mac_sent, "TALK");
  assert_eq!(on_mac.len(), 2);
  let asked = texts(&on_mac[0]).join("\n");
  assert!(asked.contains("TALK: list them"), "{asked}");
  assert!(asked.contains("Hello from the phone."), "{asked}");
  assert!(asked.contains("On node mac"), "{asked}");

  // Continue here: the phone runs it again, from all the Mac did.
  let Reply::Lines(said) = p.request(&run_on(&thread, None)).await.unwrap() else {
    panic!()
  };
  assert!(said[0].contains("runs here again"), "{said:?}");
  p.chat(&global_chat(Some(thread.clone()), "TALK: and now?"), |_| {
    None
  })
  .await
  .unwrap();
  let on_phone = of(&phone_sent, "TALK");
  assert_eq!(on_phone.len(), 2);
  let asked = texts(&on_phone[1]).join("\n");
  assert!(asked.contains("Done on the Mac."), "{asked}");
  assert_eq!(
    of(&mac_sent, "TALK").len(),
    2,
    "nothing more ran on the Mac"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A chat typed on the phone into a thread that runs on the Mac, whose
/// turn fails there before its model call (its provider does not resolve:
/// on 2026-11-10 an OAuth refresh that failed): the Mac writes the
/// message with a `failed` entry saying why, as a failed model call does,
/// and the phone, following the turn, ends it and shows both. Before,
/// the Mac only logged it: nothing was written, and the phone showed
/// nothing, ever.
#[tokio::test(flavor = "multi_thread")]
async fn a_chat_that_fails_on_another_node_before_its_model_call_says_why() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, _phone_sent, mac_sent) = keyed_pair(
    dir.path(),
    vec![("TALK", vec![answer("Hello from the phone.")])],
    vec![("TALK", vec![answer("Never asked.")])],
    false,
  )
  .await;
  let thread = chat_thread(&mut p, "TALK: hello").await;
  let Reply::Lines(said) = p.request(&run_on(&thread, Some("mac"))).await.unwrap() else {
    panic!()
  };
  assert!(said[0].contains("now runs on mac"), "{said:?}");
  // The Mac's provider no longer resolves.
  mac_node.stop().await.unwrap();
  let mac = dir.path().join("mac");
  let mut c = Config::load(&mac).unwrap();
  c.providers.get_mut("anthropic").unwrap().cache_ttl = Some("never".into());
  c.save(&mac).unwrap();
  let mac_node = restart(&mac).await;

  let done = tokio::time::timeout(
    Duration::from_secs(60),
    p.chat(
      &global_chat(Some(thread.clone()), "TALK: still there?"),
      |_| None,
    ),
  )
  .await
  .expect("the phone hears the turn ended")
  .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(of(&mac_sent, "TALK").is_empty(), "no model call on the Mac");
  let Reply::Lines(l) = p
    .request(&Request::Transcript {
      thread: thread.clone(),
      after: None,
      before: None,
      tail: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let read: Value = serde_json::from_str(&l[0]).unwrap();
  let entries = read["entries"].as_array().unwrap();
  let last = &entries[entries.len() - 2..];
  assert_eq!(last[0]["kind"], "user", "{read}");
  assert_eq!(last[0]["text"], "TALK: still there?", "{read}");
  assert_eq!(last[1]["kind"], "failed", "{read}");
  let why = last[1]["text"].as_str().unwrap();
  assert!(
    why.starts_with("The turn did not start: ") && why.contains("cache_ttl"),
    "{why}"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A new chat put on another node (`Chat { node }`, design §2.7, "A new
/// chat on another node"): the thread is written on the phone, set to run
/// on the Mac, and its first turn runs there and streams back; the next
/// message goes there too. A node that is not the user's is refused before
/// anything is written.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_chat_put_on_another_node_runs_its_first_turn_there() {
  const ON_MAC: &str = "[Stitch: the instructions and skills";
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, phone_sent, mac_sent) = keyed_pair(
    dir.path(),
    vec![("TALK", vec![answer("Wrongly on the phone.")])],
    // Its first message there: the Mac's instructions, then the user's.
    vec![(
      ON_MAC,
      vec![answer("First, on the Mac."), answer("Second, on the Mac.")],
    )],
    false,
  )
  .await;
  let on = |node: &str, text: &str| {
    let mut r = global_chat(None, text);
    if let Request::Chat { node: n, .. } = &mut r {
      *n = Some(node.to_owned());
    }
    r
  };

  // Not one of the user's nodes: refused (before anything is written).
  let mut named = Vec::new();
  let refused = p
    .chat(&on("nowhere", "TALK: hi"), |r| {
      named.push(r.clone());
      None
    })
    .await;
  assert!(
    format!("{refused:?}").contains("no node `nowhere`"),
    "{refused:?}"
  );
  assert!(
    !named.iter().any(|r| matches!(r, Reply::Thread(_))),
    "{named:?}"
  );

  let mut seen: Vec<Reply> = Vec::new();
  let done = p
    .chat(&on("mac", "TALK: start on the Mac"), |r| {
      seen.push(r.clone());
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { stop, thread, .. } = &done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "end_turn", "{seen:?}");
  assert!(
    seen
      .iter()
      .any(|r| matches!(r, Reply::Thread(t) if t == thread)),
    "the thread named first: {seen:?}"
  );
  assert!(thread.contains("-phone-"), "written on the phone: {thread}");
  let text: String = seen
    .iter()
    .filter_map(|r| match r {
      Reply::Text(t) => Some(t.as_str()),
      _ => None,
    })
    .collect();
  assert!(text.contains("First, on the Mac."), "{seen:?}");
  assert!(
    of(&phone_sent, "TALK").is_empty(),
    "nothing ran on the phone"
  );
  let on_mac = of(&mac_sent, ON_MAC);
  assert_eq!(on_mac.len(), 1);
  let asked = texts(&on_mac[0]).join("\n");
  assert!(asked.contains("TALK: start on the Mac"), "{asked}");
  assert!(asked.contains("On node mac"), "{asked}");
  let Reply::Lines(runs) = p
    .request(&Request::RunsOn {
      thread: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(runs, vec!["mac".to_owned()]);

  // Its next message goes there too.
  p.chat(
    &global_chat(Some(thread.clone()), "TALK: and then?"),
    |_| None,
  )
  .await
  .unwrap();
  assert!(
    of(&phone_sent, "TALK").is_empty(),
    "nothing ran on the phone"
  );
  let on_mac = of(&mac_sent, ON_MAC);
  assert_eq!(on_mac.len(), 2);
  let asked = texts(&on_mac[1]).join("\n");
  assert!(asked.contains("First, on the Mac."), "{asked}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_for_a_thread_on_an_offline_node_waits_for_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, phone_sent, mac_sent) = keyed_pair(
    d,
    vec![("TALK", vec![answer("Hello from the phone.")])],
    vec![("TALK", vec![answer("The Mac, back.")])],
    false,
  )
  .await;
  let thread = chat_thread(&mut p, "TALK: hello").await;
  p.request(&run_on(&thread, Some("mac"))).await.unwrap();
  p.request(&Request::Sync { peer: None }).await.unwrap();
  mac_node.stop().await.unwrap();

  // The Mac is off: the message waits for it, said so; nothing runs here.
  let mut notes = Vec::new();
  let done = p
    .chat(
      &global_chat(Some(thread.clone()), "TALK: when you are back"),
      |r| {
        if let Reply::Note(n) = r {
          notes.push(n.clone());
        }
        None
      },
    )
    .await
    .unwrap();
  let Reply::ChatDone { stop, .. } = &done else {
    panic!("{done:?}")
  };
  assert_eq!(stop, "queued");
  assert!(
    notes.iter().any(|n| n.contains("mac is not reachable now")),
    "{notes:?}"
  );
  assert_eq!(of(&phone_sent, "TALK").len(), 1);

  // Back: it takes the message and runs it.
  let mac_node = restart(&d.join("mac")).await;
  let mut m = Client::connect(&d.join("mac")).await.unwrap();
  m.request(&Request::Sync { peer: None }).await.unwrap();
  until("the Mac to run it", || !of(&mac_sent, "TALK").is_empty()).await;
  let asked = texts(&of(&mac_sent, "TALK")[0]).join("\n");
  assert!(asked.contains("TALK: when you are back"), "{asked}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
}

/// A record written while a peer is down (an answer to its question, in
/// the incident that made this test: the user answered while the asking
/// node rebooted) reaches it within seconds of its coming back, by the
/// push being tried again: not at the next interval's sync (none here),
/// nor only when that node happens to sync itself.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_pushed_while_a_peer_is_down_reaches_it_when_it_is_back() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, mut m, _, _) =
    pair(d, vec![answer("Hello from the phone.")], vec![]).await;
  let thread = chat_thread(&mut p, "hello").await;
  p.request(&Request::Sync { peer: None }).await.unwrap();
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();

  // Written while the Mac is off: its push does not reach it.
  p.request(&Request::Rename {
    thread: thread.clone(),
    title: "Named while the Mac was off".into(),
  })
  .await
  .unwrap();
  // Longer than a push's dial: it fails.
  tokio::time::sleep(Duration::from_secs(14)).await;

  // Back, syncing nothing itself: the phone's push, tried again, brings it.
  let mac_node = stitch_node::start(&d.join("mac")).await.unwrap();
  let mut m = Client::connect(&d.join("mac")).await.unwrap();
  let id: stitch_records::RecordId = thread.parse().unwrap();
  let index = Index::open(&d.join("mac").join("index.sqlite")).unwrap();
  let mut arrived = false;
  for _ in 0..90 {
    if index.title(&id).ok().flatten().as_deref() == Some("Named while the Mac was off") {
      arrived = true;
      break;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
  }
  assert!(arrived, "the rename never reached the Mac");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
}

/// An approval answered here for a thread on a node that is down (its
/// machine rebooting): `threads` says the thread is blocked on it, the
/// answer not delivered yet, not "working".
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_not_delivered_yet_lists_its_thread_as_blocked() {
  if !sandboxes() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, _, _) = keyed_pair(
    d,
    vec![(
      "BOSS",
      vec![
        tool_call(
          "b1",
          "spawn",
          json!({"node": "mac", "goal": "CHILD: publish it"}),
        ),
        answer("Spawned."),
        tool_call("b2", "threads", json!({})),
        answer("Listed."),
      ],
    )],
    vec![(
      "CHILD",
      vec![tool_call(
        "c1",
        "shell_run",
        json!({"command": "git push origin HEAD:refs/heads/never"}),
      )],
    )],
    false,
  )
  .await;
  let mut results = Vec::new();
  let done = p
    .chat(&global_chat(None, "BOSS: have the Mac publish it"), |r| {
      if let Reply::ToolResult { output, .. } = r {
        results.push(output.clone());
      }
      None
    })
    .await
    .unwrap();
  let Reply::ChatDone { thread: boss, .. } = done else {
    panic!("{done:?}")
  };
  let child = spawned_in(&results, 0);
  let phone_index = Index::open(&d.join("phone").join("index.sqlite")).unwrap();
  let mut question = None;
  for _ in 0..300 {
    let _ = p.request(&Request::Sync { peer: None }).await;
    question = phone_index
      .records(Some("question"))
      .unwrap()
      .into_iter()
      .find(|q| q.node == "mac");
    if question.is_some() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  let question = question.expect("the Mac's question here");
  let mut m = Client::connect(&d.join("mac")).await.unwrap();
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  tokio::time::sleep(Duration::from_secs(2)).await;

  p.request(&Request::Answer {
    question: question.id.to_string(),
    allow: true,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  let mut listed = Vec::new();
  p.chat(&global_chat(Some(boss), "And now?"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      listed.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let listed = listed.join("\n");
  assert!(
    listed.contains(&format!(
      "{child}: runs on mac, in global, writable: blocked: approval {} answered (allow, on phone,",
      question.id
    )) && listed.contains("not delivered to mac yet"),
    "{listed}"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_for_another_node_carries_its_files_and_the_model_picked() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_node, mac_node, mut p, _phone_sent, mac_sent) = keyed_pair(
    d,
    vec![("TALK", vec![answer("Hello from the phone.")])],
    vec![(
      "TALK",
      vec![
        answer("Read the notes on the Mac."),
        answer("Default model."),
      ],
    )],
    false,
  )
  .await;
  let thread = chat_thread(&mut p, "TALK: hello").await;
  p.request(&run_on(&thread, Some("mac"))).await.unwrap();

  // A file, and a model the Mac runs: both go there.
  let file = d.join("notes.txt");
  std::fs::write(&file, "PLUM-42 is the code word.").unwrap();
  let mut req = global_chat(Some(thread.clone()), "TALK: read this");
  if let Request::Chat {
    attachments, model, ..
  } = &mut req
  {
    *attachments = vec![file.display().to_string()];
    *model = Some("anthropic/claude-sonnet-9".into());
  }
  let done = p.chat(&req, |_| None).await.unwrap();
  assert!(
    matches!(&done, Reply::ChatDone { stop, .. } if stop == "end_turn"),
    "{done:?}"
  );
  let asked = &of(&mac_sent, "TALK")[0];
  assert_eq!(asked["model"], "claude-sonnet-9", "the model picked");
  assert!(
    asked.to_string().contains("PLUM-42 is the code word."),
    "the file reached the Mac: {asked}"
  );

  // A model the Mac does not run: said, and its own used instead.
  let mut notes = Vec::new();
  let mut req = global_chat(Some(thread.clone()), "TALK: and again");
  if let Request::Chat { model, .. } = &mut req {
    *model = Some("nosuch/model-x".into());
  }
  p.chat(&req, |r| {
    if let Reply::Note(n) = r {
      notes.push(n.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(
    notes
      .iter()
      .any(|n| n.contains("mac does not run nosuch/model-x")),
    "{notes:?}"
  );
  let asked = &of(&mac_sent, "TALK")[1];
  assert_ne!(asked["model"], "model-x");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// The `spawn` tool's description in a request: what it says the thread
/// runs on.
fn spawn_says(body: &Value) -> String {
  body["tools"]
    .as_array()
    .and_then(|ts| ts.iter().find(|t| t["name"] == "spawn"))
    .and_then(|t| t["description"].as_str())
    .unwrap_or_default()
    .to_owned()
}

/// 2026-10-26: a chat on `gpt-6-astra` (the app's picker) was woken by a
/// report and answered on the node's default, and a thread spawned on
/// `gpt-6-astra` was told by its `spawn` tool that it ran on the default.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_keeps_the_model_it_was_given() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      (
        "TALK",
        vec![
          answer("On the model picked."),
          answer("Woken, on the same model."),
          answer("Typed again, no model named."),
        ],
      ),
      ("OTHER", vec![answer("Other here.")]),
      ("CHILD", vec![answer("The child's finding.")]),
    ],
  )
  .await;
  let other = chat_thread(&mut c, "OTHER: watch the calendar").await;
  // A chat whose client picked a model, as the app's picker does.
  let mut req = global_chat(None, "TALK: hello");
  if let Request::Chat { model, .. } = &mut req {
    *model = Some("anthropic/claude-sonnet-9".into());
  }
  let Reply::ChatDone { thread, .. } = c.chat(&req, |_| None).await.unwrap() else {
    panic!()
  };
  let first = &of(&sent, "TALK")[0];
  assert_eq!(first["model"], "claude-sonnet-9");
  assert!(
    spawn_says(first).contains("This thread runs on anthropic/claude-sonnet-9."),
    "{}",
    spawn_says(first)
  );

  // A turn nobody typed (a message from another thread, as a report is)
  // runs on the thread's model, not the node's default.
  c.request(&send_from(&thread, "NEWS for you.", &other, false))
    .await
    .unwrap();
  until("the message's turn", || of(&sent, "TALK").len() >= 2).await;
  let woken = &of(&sent, "TALK")[1];
  assert_eq!(woken["model"], "claude-sonnet-9", "the thread's model");
  assert!(
    spawn_says(woken).contains("This thread runs on anthropic/claude-sonnet-9."),
    "{}",
    spawn_says(woken)
  );

  // A chat that names no model goes on with the thread's too.
  c.chat(&global_chat(Some(thread.clone()), "TALK: again"), |_| None)
    .await
    .unwrap();
  until("the third turn", || of(&sent, "TALK").len() >= 3).await;
  assert_eq!(of(&sent, "TALK")[2]["model"], "claude-sonnet-9");

  // A spawned thread is told the model it runs on, not the default.
  let mut req = detach_global("CHILD: look into it", &other);
  if let Request::Detach { model, .. } = &mut req {
    *model = Some("anthropic/claude-sonnet-9".into());
  }
  c.request(&req).await.unwrap();
  until("the child to start", || !of(&sent, "CHILD").is_empty()).await;
  let child = &of(&sent, "CHILD")[0];
  assert_eq!(child["model"], "claude-sonnet-9");
  assert!(
    spawn_says(child).contains("This thread runs on anthropic/claude-sonnet-9."),
    "{}",
    spawn_says(child)
  );

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_node_that_runs_programs_can_run_a_thread() {
  let dir = tempfile::tempdir().unwrap();
  let (phone_node, mac_node, mut p, _phone_sent, _mac_sent) = keyed_pair(
    dir.path(),
    vec![("TALK", vec![answer("Hello from the phone.")])],
    Vec::new(),
    true,
  )
  .await;
  let thread = chat_thread(&mut p, "TALK: hello").await;
  // The Mac, its shell off, says it runs no programs: not offered, refused.
  let Reply::Lines(runners) = p.request(&Request::Runners).await.unwrap() else {
    panic!()
  };
  assert_eq!(runners, vec!["mac\tno".to_owned()]);
  let e = p
    .request(&run_on(&thread, Some("mac")))
    .await
    .unwrap_err()
    .to_string();
  assert!(
    e.contains("cannot run on mac") && e.contains("shell is off"),
    "{e}"
  );
  let Reply::Lines(on) = p
    .request(&Request::RunsOn {
      thread: thread.clone(),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(on.is_empty(), "nothing was set: {on:?}");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

async fn model_settings(c: &mut Client, input: Value) -> String {
  match c
    .request(&Request::ModelSettings {
      input: input.to_string(),
    })
    .await
    .unwrap()
  {
    Reply::Lines(l) => l.join("\n"),
    other => panic!("{other:?}"),
  }
}

/// The user's default and quick models (synced `model-choice` records):
/// a chat naming no model runs on the default, a spawn with `model:
/// "quick"` on the quick one, one naming none on the default.
#[tokio::test(flavor = "multi_thread")]
async fn the_users_default_and_quick_models_are_used_by_every_thread() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (node, mut c, sent) = alone(
    &d,
    vec![
      ("PLAIN", vec![answer("Hi.")]),
      (
        "GO",
        vec![
          tool_call("m1", "models", json!({})),
          tool_call(
            "s1",
            "spawn",
            json!({"goal": "QUICK: count the files", "model": "quick", "tools": false}),
          ),
          tool_call(
            "s2",
            "spawn",
            json!({"goal": "DEF: design the thing", "tools": false}),
          ),
          answer("Both spawned."),
        ],
      ),
      ("QUICK", vec![answer("Seven.")]),
      ("DEF", vec![answer("Designed.")]),
    ],
  )
  .await;
  let said = model_settings(&mut c, json!({"action": "quick"})).await;
  assert!(said.starts_with("quick: none set ("), "{said}");
  let said = model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-opus-5"}),
  )
  .await;
  assert_eq!(said, "default: anthropic/claude-opus-5 (every node)");
  let said = model_settings(
    &mut c,
    json!({"action": "quick", "model": "anthropic/claude-haiku-5-5"}),
  )
  .await;
  assert_eq!(said, "quick: anthropic/claude-haiku-5-5 (every node)");

  chat_thread(&mut c, "PLAIN: hello").await;
  assert_eq!(of(&sent, "PLAIN")[0]["model"], "claude-opus-5");

  let mut results = Vec::new();
  c.chat(&global_chat(None, "GO: split it up"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(
    results[0].starts_with(
      "default: anthropic/claude-opus-5\nquick: anthropic/claude-haiku-5-5 (spawn with model \
       \"quick\")\n"
    ),
    "{results:?}"
  );
  assert!(results[1].starts_with("Spawned thread "), "{results:?}");
  until("both children to run", || {
    !of(&sent, "QUICK").is_empty() && !of(&sent, "DEF").is_empty()
  })
  .await;
  assert_eq!(of(&sent, "QUICK")[0]["model"], "claude-haiku-5-5");
  assert_eq!(of(&sent, "DEF")[0]["model"], "claude-opus-5");

  let said = model_settings(&mut c, json!({"action": "quick", "unset": true})).await;
  assert_eq!(said, "quick: unset (now anthropic/claude-opus-5)");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A node's own `default_model` and `quick_model` (its `config.toml`) win
/// on it over the synced ones, and `stitch model` says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_nodes_own_models_win_over_the_synced_ones() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (api, sent) = keyed(vec![
    ("PLAIN", vec![answer("Hi.")]),
    (
      "GO",
      vec![
        tool_call(
          "s1",
          "spawn",
          json!({"goal": "QUICK: count the files", "model": "quick", "tools": false}),
        ),
        answer("Spawned."),
      ],
    ),
    ("QUICK", vec![answer("Seven.")]),
  ])
  .await;
  let data = d.join("node");
  config(&data, "mbp", free_port(), &api);
  let mut cfg = Config::load(&data).unwrap();
  cfg.default_model = Some("anthropic/claude-node-default".into());
  cfg.quick_model = Some("anthropic/claude-node-quick".into());
  cfg.save(&data).unwrap();
  let home = RecordRoot::init(&d.join("home"), "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  let said = model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-opus-5"}),
  )
  .await;
  assert_eq!(
    said,
    "default: anthropic/claude-opus-5 (every node)\nthis node keeps its own: \
     anthropic/claude-node-default (default_model in config.toml)"
  );
  model_settings(
    &mut c,
    json!({"action": "quick", "model": "anthropic/claude-haiku-5-5"}),
  )
  .await;
  let said = model_settings(&mut c, json!({"action": "default"})).await;
  assert_eq!(
    said,
    "default: anthropic/claude-node-default (this node's config.toml; every node: \
     anthropic/claude-opus-5)"
  );

  chat_thread(&mut c, "PLAIN: hello").await;
  assert_eq!(of(&sent, "PLAIN")[0]["model"], "claude-node-default");
  chat_thread(&mut c, "GO: split it up").await;
  until("the quick child to run", || !of(&sent, "QUICK").is_empty()).await;
  assert_eq!(of(&sent, "QUICK")[0]["model"], "claude-node-quick");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// Clients display the exact same resolution that a turn uses, without
/// converting a displayed default into a permanent explicit thread pick.
#[tokio::test(flavor = "multi_thread")]
async fn effective_models_follow_roles_but_preserve_deliberate_picks() {
  let dir = tempfile::tempdir().unwrap();
  let (node, mut c, sent) = alone(dir.path(), vec![("PLAIN", vec![answer("Hi."); 5])]).await;
  let effective = |thread: Option<&str>, model: Option<&str>| Request::EffectiveModel {
    thread: thread.map(str::to_owned),
    model: model.map(str::to_owned),
    place: Some("global".into()),
    cwd: None,
  };
  model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-opus-5"}),
  )
  .await;
  let Reply::Lines(lines) = c.request(&effective(None, None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-opus-5"]);
  let thread = chat_thread(&mut c, "PLAIN hello").await;
  model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-sonnet-5-5"}),
  )
  .await;
  let Reply::Lines(lines) = c.request(&effective(Some(&thread), None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(
    lines,
    ["anthropic/claude-sonnet-5-5"],
    "an implicit turn never pins the default"
  );
  // Even picking today's default concretely is an intentional fixed pick.
  let mut picked = global_chat(Some(thread.clone()), "PLAIN concrete");
  if let Request::Chat { model, .. } = &mut picked {
    *model = Some("anthropic/claude-sonnet-5-5".into());
  }
  c.chat(&picked, |_| None).await.unwrap();
  model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-opus-5"}),
  )
  .await;
  let Reply::Lines(lines) = c.request(&effective(Some(&thread), None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-sonnet-5-5"]);
  // Follow default deliberately replaces that fixed choice, durably.
  if let Request::Chat { model, .. } = &mut picked {
    *model = Some("default".into());
  }
  c.chat(&picked, |_| None).await.unwrap();
  model_settings(
    &mut c,
    json!({"action": "default", "model": "anthropic/claude-haiku-5-5"}),
  )
  .await;
  let Reply::Lines(lines) = c.request(&effective(Some(&thread), None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-haiku-5-5"]);
  c.chat(&global_chat(Some(thread.clone()), "PLAIN follow"), |_| None)
    .await
    .unwrap();
  assert_eq!(of(&sent, "PLAIN")[3]["model"], "claude-haiku-5-5");
  // Quick falls back to default until set, and a local explicit override wins.
  let Reply::Lines(lines) = c
    .request(&effective(Some(&thread), Some("quick")))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-haiku-5-5"]);
  model_settings(
    &mut c,
    json!({"action": "quick", "model": "anthropic/claude-sonnet-5-5"}),
  )
  .await;
  if let Request::Chat { model, .. } = &mut picked {
    *model = Some("quick".into());
  }
  c.chat(&picked, |_| None).await.unwrap();
  let Reply::Lines(lines) = c.request(&effective(Some(&thread), None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-sonnet-5-5"]);
  let Reply::Lines(lines) = c
    .request(&effective(Some(&thread), Some("anthropic/claude-opus-5")))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-opus-5"]);
  // An unavailable synced default is displayed as the same node-local
  // fallback a chat would use, never as a model it cannot run.
  model_settings(
    &mut c,
    json!({"action": "default", "model": "missing/not-here"}),
  )
  .await;
  let Reply::Lines(lines) = c.request(&effective(None, None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, [stitch_node::config::DEFAULT_MODEL]);
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A model the user reserved (a synced `model-setting` record) is listed
/// apart, and an agent's spawn on it is refused until the user names it.
#[tokio::test(flavor = "multi_thread")]
async fn a_reserved_model_is_spawned_only_when_the_user_asks_for_it() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let spawn = |id: &str| {
    tool_call(
      id,
      "spawn",
      json!({"goal": "CHILD: look it over", "model": "anthropic/claude-fable-5-1", "tools": false}),
    )
  };
  let (node, mut c, sent) = alone(
    &d,
    vec![
      (
        "TALK",
        vec![
          tool_call("m1", "models", json!({})),
          spawn("s1"),
          answer("Refused."),
        ],
      ),
      ("ASK", vec![spawn("s2"), answer("Spawned.")]),
      ("CHILD", vec![answer("Looked.")]),
    ],
  )
  .await;
  let said = model_settings(
    &mut c,
    json!({"action": "set", "pattern": "anthropic/claude-fable-*", "reserved": true}),
  )
  .await;
  assert_eq!(said, "anthropic/claude-fable-*: reserved");
  let said = model_settings(
    &mut c,
    json!({"action": "set", "pattern": "anthropic/claude-fable-*", "note": "the big one"}),
  )
  .await;
  assert_eq!(
    said,
    "anthropic/claude-fable-*: reserved, note \"the big one\""
  );

  let mut results = Vec::new();
  c.chat(&global_chat(None, "TALK: have a light model look"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(
    results[1].contains("anthropic/claude-fable-5-1 is reserved"),
    "{results:?}"
  );
  let mut results = Vec::new();
  c.chat(&global_chat(None, "ASK: have Fable look it over"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(results[0].starts_with("Spawned thread "), "{results:?}");
  until("the child to run", || !of(&sent, "CHILD").is_empty()).await;
  let child = &of(&sent, "CHILD")[0];
  assert_eq!(child["model"], "claude-fable-5-1");

  let said = model_settings(
    &mut c,
    json!({"action": "rm", "pattern": "anthropic/claude-fable-*"}),
  )
  .await;
  assert_eq!(said, "anthropic/claude-fable-*: removed");
  let said = model_settings(&mut c, json!({"action": "ls"})).await;
  assert!(said.contains("\nnothing set: "), "{said}");

  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

/// A fixer asked of another node (the fixer node, as the synced settings
/// say) is admitted there, which reserves its budget and starts it; its
/// goal quotes the issue as data.
#[tokio::test(flavor = "multi_thread")]
async fn a_fixer_runs_on_the_fixer_node() {
  let dir = tempfile::tempdir().unwrap();
  let checkout = dir.path().join("stitch");
  std::fs::create_dir_all(&checkout).unwrap();
  let (phone_node, mac_node, mut p, _m, _phone_sent, mac_sent) = pair(
    dir.path(),
    vec![
      tool_call(
        "f1",
        "self_improve",
        json!({"issue": "the TUI forgets the scroll position on resize", "rollout": "none"}),
      ),
      answer("Asked the Mac."),
      answer("Noted."),
    ],
    vec![answer("Fixed it.")],
  )
  .await;
  let Reply::Lines(set) = p
    .request(&Request::Issue {
      input: json!({"action": "settings", "node": "mac", "place": checkout.display().to_string(), "on": true, "budget_usd": 2.0})
        .to_string(),
      thread: None,
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(set.iter().any(|l| l == "fixer node: mac"), "{set:?}");
  let mut results = Vec::new();
  let mut fix = global_chat(None, "Fix the scroll bug.");
  if let Request::Chat { tools, .. } = &mut fix {
    tools.push("+self_improve".into());
  }
  p.chat(&fix, |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  let fixer = results
    .iter()
    .find_map(|r| r.strip_prefix("Fixer "))
    .and_then(|r| r.split_whitespace().next())
    .unwrap_or_else(|| panic!("{results:?}"))
    .to_owned();
  assert!(
    results.iter().any(|r| r.contains("asked of mac")),
    "{results:?}"
  );
  until("the Mac to start the fixer", || {
    !mac_sent.lock().unwrap().is_empty()
  })
  .await;
  let goal = texts(&mac_sent.lock().unwrap()[0]).join("\n");
  assert!(goal.contains("=====ISSUE-AS-FILED====="), "{goal}");
  assert!(
    goal.contains("the TUI forgets the scroll position"),
    "{goal}"
  );
  // The Mac's ledger has it, and the issue is its fixer's.
  let index = Index::open(&dir.path().join("mac").join("index.sqlite")).unwrap();
  let reservations = index.records(Some("fix-reservation")).unwrap();
  assert_eq!(reservations.len(), 1);
  assert_eq!(reservations[0].fields["fixer"], fixer.as_str());
  assert_eq!(reservations[0].node, "mac");

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}

/// A peer on the local network that never answers (a phone asleep: its
/// packets go nowhere) holds a turn's start a second at most, not the five
/// it used to.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_asleep_on_the_local_network_holds_a_turn_a_second_at_most() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("node");
  let (node, mut c, sent) = alone(
    dir.path(),
    vec![
      ("BOSS", vec![answer("Boss here.")]),
      ("WORK", vec![answer("WORK done."), answer("WORK again.")]),
    ],
  )
  .await;
  let boss = chat_thread(&mut c, "BOSS: plan the release").await;
  let Reply::Detached(work) = c
    .request(&detach_global("WORK: prepare the release", &boss))
    .await
    .unwrap()
  else {
    panic!()
  };
  until("the work to start", || !of(&sent, "WORK").is_empty()).await;
  until("its turn to end", || {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    !index.records(Some("report")).unwrap().is_empty()
  })
  .await;
  // Where the phone was: a UDP socket that reads nothing.
  let hole = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
  let key = stitch_net::load_or_create_key(&dir.path().join("phone.key")).unwrap();
  let peers_path = data.join("peers.toml");
  let mut peers = Peers::load(&peers_path).unwrap();
  peers
    .add(stitch_net::Peer {
      name: "phone".into(),
      id: key.public().to_string(),
      addrs: vec![hole.local_addr().unwrap()],
    })
    .unwrap();
  peers.save(&peers_path).unwrap();

  // A message wakes it: its turn catches up with the peers first.
  let started = std::time::Instant::now();
  c.request(&send_from(&work, "MORE to do.", &boss, false))
    .await
    .unwrap();
  until("the work to go on", || of(&sent, "WORK").len() >= 2).await;
  let took = started.elapsed();
  // A second's wait on the phone (none once it is known to have failed),
  // and the turn's own start around it (a tenth of a second here).
  assert!(took < Duration::from_millis(1500), "{took:?}");
  drop(hole);
  node.stop().await.unwrap();
}

/// A new chat of a project put on a node with no checkout of it (design
/// §2.3, "A project with no checkout here"; the phone put a chat of
/// paragon on beast, whose turn started in the home directory, and the
/// agent answered about another repository): the turn there does not
/// start, and the phone's transcript says why, naming the project and the
/// node. Here the project has no origin, so nothing is asked.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_chat_of_a_project_on_a_node_without_its_checkout_fails_saying_so() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (phone_node, mac_node, mut p, _phone_sent, mac_sent) = keyed_pair(
    &d,
    vec![("TALK", vec![answer("On the phone.")])],
    vec![("TALK", vec![answer("Wrongly on the Mac.")])],
    false,
  )
  .await;
  let app = d.join("work").join("paragon");
  std::fs::create_dir_all(&app).unwrap();
  for args in [
    &["init", "--quiet"][..],
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
    ][..],
  ] {
    let out = std::process::Command::new("git")
      .arg("-C")
      .arg(&app)
      .args(args)
      .output()
      .unwrap();
    assert!(out.status.success());
  }
  // The phone has the checkout: the project is listed.
  let mut r = global_chat(None, "TALK: here");
  if let Request::Chat { cwd, global, .. } = &mut r {
    *cwd = Some(app.display().to_string());
    *global = false;
  }
  let Reply::ChatDone { thread: first, .. } = p.chat(&r, |_| None).await.unwrap() else {
    panic!()
  };
  let index = Index::open(&d.join("phone").join("index.sqlite")).unwrap();
  let repo = index
    .record(&first.parse().unwrap())
    .unwrap()
    .unwrap()
    .repo
    .to_string();
  drop(index);

  let mut r = global_chat(None, "TALK: is the repo there?");
  if let Request::Chat {
    place,
    node,
    global,
    ..
  } = &mut r
  {
    *place = Some(format!("project:{repo}"));
    *node = Some("mac".into());
    *global = false;
  }
  let done = tokio::time::timeout(Duration::from_secs(60), p.chat(&r, |_| None))
    .await
    .expect("the phone hears the turn ended")
    .unwrap();
  let Reply::ChatDone { thread, .. } = &done else {
    panic!("{done:?}")
  };
  assert!(of(&mac_sent, "TALK").is_empty(), "no model call on the Mac");
  let mut why = String::new();
  for _ in 0..100 {
    let Reply::Lines(l) = p
      .request(&Request::Transcript {
        thread: thread.clone(),
        after: None,
        before: None,
        tail: None,
      })
      .await
      .unwrap()
    else {
      panic!()
    };
    let read: Value = serde_json::from_str(&l[0]).unwrap();
    if let Some(f) = read["entries"]
      .as_array()
      .unwrap()
      .iter()
      .find(|e| e["kind"] == "failed")
    {
      why = f["text"].as_str().unwrap().to_owned();
      break;
    }
    p.request(&Request::Sync { peer: None }).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert!(
    why.starts_with("The turn did not start: project paragon has no checkout on mac")
      && why.contains("no origin to clone from"),
    "{why}"
  );

  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
  mac_node.stop().await.unwrap();
}
