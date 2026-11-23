//! Tool and skill sets per thread (design §2.14): changes made for a
//! thread apply from its next turn, as records that follow it to another
//! node; a call to what is off says so.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request, SetItem};
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

/// A free UDP port on localhost, for a node to bind again.
fn free_port() -> u16 {
  std::net::UdpSocket::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
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

/// The tools a model request offered.
fn offered(body: &Value) -> Vec<String> {
  body["tools"]
    .as_array()
    .map(|t| {
      t.iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
    })
    .unwrap_or_default()
}

fn chat(thread: Option<&str>, text: &str, tools: &[&str], skills: &[&str]) -> Request {
  Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: None,
    global: true,
    env: Vec::new(),
    tools: tools.iter().map(|s| (*s).to_owned()).collect(),
    skills: skills.iter().map(|s| (*s).to_owned()).collect(),
    place: None,
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

fn toolset(thread: Option<&str>, tools: &[&str], skills: &[&str]) -> Request {
  Request::Toolset {
    thread: thread.map(str::to_owned),
    cwd: None,
    global: false,
    tools: tools.iter().map(|s| (*s).to_owned()).collect(),
    skills: skills.iter().map(|s| (*s).to_owned()).collect(),
  }
}

fn item<'a>(items: &'a [SetItem], name: &str) -> &'a SetItem {
  items
    .iter()
    .find(|i| i.name == name)
    .unwrap_or_else(|| panic!("no {name} in {items:?}"))
}

fn skill(home: &Path, name: &str, description: &str) {
  std::fs::create_dir_all(home.join("skills").join(name)).unwrap();
  std::fs::write(
    home.join("skills").join(name).join("SKILL.md"),
    format!("---\nname: {name}\ndescription: {description}\n---\nSteps.\n"),
  )
  .unwrap();
}

/// The text of the last message sent.
fn last_user(body: &Value) -> Vec<String> {
  body["messages"].as_array().unwrap().last().unwrap()["content"]
    .as_array()
    .unwrap()
    .iter()
    .filter_map(|c| {
      c["text"]
        .as_str()
        .or_else(|| c["content"].as_str())
        .map(str::to_owned)
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_threads_sets_change_from_its_next_turn() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let home = d.join("home");
  skill(&home, "release", "Cut a release.");
  skill(&home, "notes", "Keep meeting notes.");
  let (base, sent) = serve(vec![
    answer("one"),
    tool_call("c1", "shell_run", json!({"command": "ls"})),
    answer("the shell is off"),
    answer("three"),
    answer("the detached one"),
  ])
  .await;
  let data = d.join("data");
  config(&data, "mbp", free_port(), &base);
  let root = RecordRoot::init(&home, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  // A new thread's sets, before it exists: the changes applied, not kept.
  let Reply::Toolset {
    thread: None,
    tools,
    skills,
  } = c
    .request(&toolset(None, &["-viewer", "+fetch"], &[]))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(!item(&tools, "viewer").on);
  assert!(item(&tools, "shell").on);
  let fetch = item(&tools, "fetch");
  assert!(
    fetch.on && !fetch.available,
    "no such server here: {fetch:?}"
  );
  assert!(item(&skills, "release").on && item(&skills, "notes").on);

  // Started with changes: they are its first turn's.
  let Reply::ChatDone { thread, .. } = c
    .chat(&chat(None, "hello", &["-viewer"], &["-notes"]), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  {
    let bodies = sent.lock().unwrap();
    let tools = offered(&bodies[0]);
    assert!(tools.iter().any(|t| t == "shell_run"), "{tools:?}");
    assert!(!tools.iter().any(|t| t == "view_file"), "{tools:?}");
    let system = bodies[0]["system"].to_string();
    assert!(system.contains("- release: Cut a release."), "{system}");
    assert!(!system.contains("- notes:"), "{system}");
  }

  // The shell turned off: gone from the next turn, and a call to it
  // (from the model's memory of it) says so.
  let Reply::Toolset { tools, .. } = c
    .request(&toolset(Some(&thread), &["-shell"], &[]))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(!item(&tools, "shell").on);
  let mut results = Vec::new();
  c.chat(&chat(Some(&thread), "list the files", &[], &[]), |r| {
    if let Reply::ToolResult { ok, output } = r {
      results.push((*ok, output.clone()));
    }
    None
  })
  .await
  .unwrap();
  assert!(
    !offered(&sent.lock().unwrap()[1])
      .iter()
      .any(|t| t == "shell_run")
  );
  assert_eq!(results.len(), 1, "{results:?}");
  assert!(!results[0].0);
  assert!(
    results[0].1.contains("shell_run is off for this thread")
      && results[0]
        .1
        .contains(&format!("stitch thread tools {thread} +shell")),
    "{results:?}"
  );

  // A skill turned back on reaches the model with the next message.
  c.request(&toolset(Some(&thread), &[], &["+notes"]))
    .await
    .unwrap();
  c.chat(&chat(Some(&thread), "again", &[], &[]), |_| None)
    .await
    .unwrap();
  {
    let bodies = sent.lock().unwrap();
    let said = last_user(&bodies[3]);
    assert!(
      said[0].starts_with("[Stitch: the instructions and skills for this thread changed")
        && said[0].contains("- notes: Keep meeting notes."),
      "{said:?}"
    );
  }

  // A thread spawned with a set has it from its first step.
  let Reply::Detached(_) = c
    .request(&Request::Detach {
      goal: "Summarize.".into(),
      model: None,
      repo: None,
      cwd: None,
      parent: Some(thread.clone()),
      budget_usd: None,
      budget_tokens: None,
      budget_minutes: None,
      max_steps: None,
      global: true,
      node: None,
      context: None,
      no_tools: false,
      tools: vec!["-weave".into()],
      skills: vec![],
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
  until("the detached thread's first step", || {
    sent.lock().unwrap().len() >= 5
  })
  .await;
  {
    let tools = offered(&sent.lock().unwrap()[4]);
    assert!(tools.iter().any(|t| t == "shell_run"), "{tools:?}");
    assert!(!tools.iter().any(|t| t == "spawn"), "{tools:?}");
  }
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_threads_set_follows_it_to_another_node() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, _) = serve(vec![]).await;
  let (mac_api, mac_sent) = serve(vec![answer("first"), answer("second")]).await;
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

  // A thread on the Mac; the phone turns its viewer off.
  let Reply::ChatDone { thread, .. } = m
    .chat(&chat(None, "hello", &[], &[]), |_| None)
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(
    offered(&mac_sent.lock().unwrap()[0])
      .iter()
      .any(|t| t == "view_file")
  );
  m.request(&Request::Sync { peer: None }).await.unwrap();
  until("the thread on the phone", || {
    Index::open(&phone.join("index.sqlite"))
      .unwrap()
      .record(&thread.parse().unwrap())
      .unwrap()
      .is_some()
  })
  .await;
  p.request(&toolset(Some(&thread), &["-viewer"], &[]))
    .await
    .unwrap();
  p.request(&Request::Sync { peer: None }).await.unwrap();

  // The Mac's next turn has it.
  m.chat(&chat(Some(&thread), "again", &[], &[]), |_| None)
    .await
    .unwrap();
  let tools = offered(&mac_sent.lock().unwrap()[1]);
  assert!(!tools.iter().any(|t| t == "view_file"), "{tools:?}");
  assert!(tools.iter().any(|t| t == "shell_run"), "{tools:?}");
  let Reply::Toolset { tools, .. } = m.request(&toolset(Some(&thread), &[], &[])).await.unwrap()
  else {
    panic!()
  };
  assert!(!item(&tools, "viewer").on);

  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}

fn git(dir: &Path, args: &[&str]) {
  let ok = std::process::Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .unwrap()
    .status
    .success();
  assert!(ok, "git {args:?}");
}

fn place_records(data: &Path) -> Vec<stitch_store::IndexedRecord> {
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .records(Some("place"))
    .unwrap()
}

/// The threads a node's index holds out of memory: those whose latest
/// `{ thread, remember }` fact says `remember = false`.
fn out_of_memory(data: &Path) -> Vec<String> {
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  let mut off: Vec<String> = Vec::new();
  for r in index.records(Some("memory")).unwrap() {
    for f in r.fields["facts"].as_array().cloned().unwrap_or_default() {
      let Some(t) = f.get("thread").and_then(Value::as_str) else {
        continue;
      };
      off.retain(|o| o != t);
      if f.get("remember") == Some(&json!(false)) {
        off.push(t.to_owned());
      }
    }
  }
  off
}

fn index_memory_records(data: &Path) -> usize {
  let index = Index::open(&data.join("index.sqlite")).unwrap();
  index.records(Some("memory")).unwrap().len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_places_settings_and_skills_reach_a_node_without_its_checkout() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  // A checkout on the Mac only, with settings and a skill of its own.
  let app = d.join("app");
  std::fs::create_dir_all(app.join(".stitch/skills/mail")).unwrap();
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
  std::fs::write(
    app.join(".stitch/settings.toml"),
    "disable = [\"viewer\"]\n[skills]\ndisable = [\"release\"]\n",
  )
  .unwrap();
  std::fs::write(
    app.join(".stitch/skills/mail/SKILL.md"),
    "---\nname: mail\ndescription: Handle the mail.\n---\nArchive what was read.\n",
  )
  .unwrap();
  let (phone_api, phone_sent) = serve(vec![
    tool_call("r1", "read_skill", json!({"name": "mail"})),
    answer("read it"),
  ])
  .await;
  let (mac_api, mac_sent) = serve(vec![
    answer("first"),
    answer("second"),
    answer("third"),
    answer("fourth"),
    answer("fifth"),
  ])
  .await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  // A phone runs no programs: its shell off stands for that here (a node
  // that runs them looks for, or clones, a project it has no checkout of).
  let mut c = Config::load(&phone).unwrap();
  c.shell = Some(stitch_node::config::ShellSettings {
    approval: Some("off".into()),
    ..Default::default()
  });
  c.save(&phone).unwrap();
  config(&mac, "mac", mac_port, &mac_api);
  let home_dir = d.join("phone-home");
  let home = RecordRoot::init(&home_dir, "home".parse().unwrap(), Role::Home).unwrap();
  skill(&home_dir, "release", "Cut a release.");
  skill(&home_dir, "notes", "Keep notes.");
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
  let in_app = |thread: Option<&str>, text: &str| Request::Chat {
    thread: thread.map(str::to_owned),
    text: text.into(),
    attachments: vec![],
    model: None,
    repo: None,
    cwd: Some(app.display().to_string()),
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
  };

  // On the Mac, in the checkout: its settings and its skill.
  let Reply::ChatDone { thread, .. } = m.chat(&in_app(None, "hello"), |_| None).await.unwrap()
  else {
    panic!()
  };
  {
    let body = &mac_sent.lock().unwrap()[0];
    let tools = offered(body);
    assert!(!tools.iter().any(|t| t == "view_file"), "{tools:?}");
    assert!(tools.iter().any(|t| t == "shell_run"), "{tools:?}");
    // `read_skill` reads Stitch's own skills; this one's files are here.
    let read_skill = body["tools"]
      .as_array()
      .unwrap()
      .iter()
      .find(|t| t["name"] == "read_skill")
      .map(|t| t["description"].as_str().unwrap_or("").to_owned())
      .unwrap_or_default();
    assert!(
      !read_skill.contains("mail"),
      "its files are here: {read_skill}"
    );
    let system = body["system"].to_string();
    let mail = app.join(".stitch/skills/mail/SKILL.md");
    assert!(
      system.contains(&format!("- mail: Handle the mail. ({})", mail.display())),
      "{system}"
    );
  }
  // Published once, not again while the files stay as they are.
  m.chat(&in_app(Some(&thread), "again"), |_| None)
    .await
    .unwrap();
  let records = place_records(&mac);
  assert_eq!(records.len(), 1, "{records:?}");
  let repo = Index::open(&mac.join("index.sqlite"))
    .unwrap()
    .record(&thread.parse().unwrap())
    .unwrap()
    .unwrap()
    .repo
    .to_string();
  assert_eq!(records[0].fields["place"], json!(format!("project:{repo}")));

  // The phone has no checkout: the record tells it, and the skill is read
  // from it.
  m.request(&Request::Sync { peer: None }).await.unwrap();
  until("the place's record on the phone", || {
    place_records(&phone).len() == 1
  })
  .await;
  let mut results = Vec::new();
  let Reply::ChatDone {
    thread: phone_thread,
    ..
  } = p
    .chat(
      &Request::Chat {
        thread: None,
        text: "what does the mail skill say?".into(),
        attachments: vec![],
        model: None,
        repo: Some(repo.clone()),
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
      },
      |r| {
        if let Reply::ToolResult { output, .. } = r {
          results.push(output.clone());
        }
        None
      },
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  {
    let body = &phone_sent.lock().unwrap()[0];
    let tools = offered(body);
    assert!(!tools.iter().any(|t| t == "view_file"), "{tools:?}");
    assert!(tools.iter().any(|t| t == "read_skill"), "{tools:?}");
    let system = body["system"].to_string();
    assert!(
      system.contains("- mail: Handle the mail. (on mac: read_skill)"),
      "{system}"
    );
    assert!(system.contains("- notes: Keep notes."), "{system}");
    assert!(
      !system.contains("release"),
      "turned off by the place: {system}"
    );
  }
  assert!(results[0].contains("Archive what was read."), "{results:?}");
  let Reply::Toolset { skills, .. } = p
    .request(&toolset(Some(&phone_thread), &[], &[]))
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(item(&skills, "mail").node.as_deref(), Some("mac"));
  assert!(!item(&skills, "release").on);

  // The files change on the Mac: a new record at its next turn there.
  std::fs::write(app.join(".stitch/settings.toml"), "enable = [\"github\"]\n").unwrap();
  m.chat(&in_app(Some(&thread), "and now"), |_| None)
    .await
    .unwrap();
  assert!(
    offered(&mac_sent.lock().unwrap()[2])
      .iter()
      .any(|t| t == "view_file")
  );
  assert_eq!(place_records(&mac).len(), 2);

  // `memory = false`: the place's thread is out of memory from its next
  // turn, and `/memory on` brings it back for good.
  std::fs::write(app.join(".stitch/settings.toml"), "memory = false\n").unwrap();
  m.chat(&in_app(Some(&thread), "quietly"), |_| None)
    .await
    .unwrap();
  assert_eq!(
    place_records(&mac).last().unwrap().fields["memory"],
    json!(false)
  );
  assert_eq!(out_of_memory(&mac), std::slice::from_ref(&thread));
  // Taken out once: a thread switched since (`/memory on`, which this node
  // without the model refuses) keeps its own setting.
  m.chat(&in_app(Some(&thread), "again"), |_| None)
    .await
    .unwrap();
  assert_eq!(index_memory_records(&mac), 1);

  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}
