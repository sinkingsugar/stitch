//! Tools on other nodes (design §2.14): the Mac shares an MCP server
//! (this test binary, started again with `STITCH_REMOTE_TEST_SERVER` set);
//! the phone's thread lists its tools as the Mac's, its calls run on the
//! Mac under the Mac's approvals (answered from the phone), and with the
//! Mac stopped a call fails at once and can be queued, its result coming
//! back as a report once the Mac is there again.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_node::mcp::{McpSettings, ServerSettings};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SERVER: &str = "STITCH_REMOTE_TEST_SERVER";

fn main() {
  if std::env::var_os(SERVER).is_some() {
    serve_mcp();
    return;
  }
  let rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap();
  rt.block_on(tools_run_on_the_node_that_shares_them());
  println!("remote tools: all passed");
}

/// A mail server of two tools: `search` (allowed) and `archive` (asks),
/// and `more`, which adds a tool and says so.
fn serve_mcp() {
  let mut out = std::io::stdout();
  let mut extra = false;
  for line in std::io::stdin().lock().lines() {
    let Ok(line) = line else { break };
    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
      continue;
    };
    let Some(id) = msg.get("id").cloned() else {
      continue;
    };
    let tool = |name: &str, d: &str| {
      json!({"name": name, "description": d,
             "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}})
    };
    let result = match msg["method"].as_str().unwrap_or("") {
      "initialize" => json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {"listChanged": true}},
        "serverInfo": {"name": "mail", "version": "1"}
      }),
      "tools/list" => {
        let mut tools = vec![
          tool("search", "Search the mail."),
          tool("archive", "Archive a message."),
          tool("more", "Adds a tool."),
        ];
        if extra {
          tools.push(tool("extra", "Added later."));
        }
        json!({"tools": tools})
      }
      "tools/call" => {
        let q = msg["params"]["arguments"]["query"].as_str().unwrap_or("");
        let text = match msg["params"]["name"].as_str().unwrap_or("") {
          "search" => format!("found: {q}"),
          "archive" => format!("archived: {q}"),
          "more" => {
            extra = true;
            let changed = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
            writeln!(out, "{changed}").unwrap();
            "added".to_owned()
          }
          other => format!("no tool {other}"),
        };
        json!({"content": [{"type": "text", "text": text}]})
      }
      _ => json!({}),
    };
    writeln!(
      out,
      "{}",
      json!({"jsonrpc": "2.0", "id": id, "result": result})
    )
    .unwrap();
    out.flush().unwrap();
  }
}

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

fn mac_mcp(data: &Path) {
  let mut c = Config::load(data).unwrap();
  let mut env = std::collections::BTreeMap::new();
  env.insert(SERVER.to_owned(), "1".to_owned());
  let mut servers = std::collections::BTreeMap::new();
  servers.insert(
    "mail".to_owned(),
    ServerSettings {
      command: Some(std::env::current_exe().unwrap().display().to_string()),
      env: env.clone(),
      allow: vec!["search".into(), "more".into()],
      share: Some(true),
      ..ServerSettings::default()
    },
  );
  servers.insert(
    "secret".to_owned(),
    ServerSettings {
      command: Some(std::env::current_exe().unwrap().display().to_string()),
      env,
      ..ServerSettings::default()
    },
  );
  // Shared, but it never starts: still shared, with no tools to list.
  servers.insert(
    "broken".to_owned(),
    ServerSettings {
      command: Some("/nonexistent/mcp-server".into()),
      share: Some(true),
      ..ServerSettings::default()
    },
  );
  // Shared, and not there when the node starts: published once it starts.
  servers.insert(
    "late".to_owned(),
    ServerSettings {
      command: Some(data.join("late-server").display().to_string()),
      env: [(SERVER.to_owned(), "1".to_owned())].into_iter().collect(),
      share: Some(true),
      ..ServerSettings::default()
    },
  );
  c.mcp = Some(McpSettings {
    servers,
    ..McpSettings::default()
  });
  c.save(data).unwrap();
}

/// The tools a model request offered, with their descriptions.
fn offered(body: &Value) -> Vec<(String, String)> {
  body["tools"]
    .as_array()
    .map(|t| {
      t.iter()
        .map(|t| {
          (
            t["name"].as_str().unwrap().to_owned(),
            t["description"].as_str().unwrap_or("").to_owned(),
          )
        })
        .collect()
    })
    .unwrap_or_default()
}

fn records(data: &Path, kind: &str) -> Vec<stitch_store::IndexedRecord> {
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .records(Some(kind))
    .unwrap()
}

/// Every tool result text in a request.
fn results(body: &Value) -> Vec<String> {
  body["messages"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|m| match &m["content"] {
      Value::Array(c) => c
        .iter()
        .filter(|b| b["type"] == "tool_result")
        .map(|b| b["content"].to_string())
        .collect(),
      _ => Vec::new(),
    })
    .collect()
}

fn chat(thread: Option<&str>, text: &str, tools: &[&str]) -> Request {
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
    skills: Vec::new(),
    place: None,
    memory: None,
    end_user: None,
    node: None,
    spoken: false,
    scopes: Vec::new(),
  }
}

async fn tools_run_on_the_node_that_shares_them() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (phone_api, phone_sent) = serve(vec![
    tool_call("s1", "mail__search", json!({"query": "invoice"})),
    answer("Found the invoice."),
    tool_call("a1", "mail__archive", json!({"query": "invoice"})),
    answer("Archived."),
    tool_call("a2", "mail__archive", json!({"query": "invoice"})),
    tool_call("a3", "mail__archive", json!({"query": "invoice"})),
    tool_call("a4", "mail__archive", json!({"query": "invoice"})),
    answer("Archived again."),
    tool_call("s2", "mail__search", json!({"query": "later"})),
    tool_call(
      "q1",
      "queue_call",
      json!({"tool": "mail__search", "arguments": {"query": "later"}}),
    ),
    answer("Queued for the Mac."),
    answer("The Mac found it."),
    tool_call("m1", "mail__more", json!({})),
    answer("One more."),
  ])
  .await;
  let (mac_api, _) = serve(vec![answer("Hello.")]).await;
  let (phone, mac) = (d.join("phone"), d.join("mac"));
  let (phone_port, mac_port) = (free_port(), free_port());
  config(&phone, "phone", phone_port, &phone_api);
  config(&mac, "mac", mac_port, &mac_api);
  mac_mcp(&mac);
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

  // The Mac publishes what it shares (its shared server started for it).
  until("the Mac's capabilities", || {
    !records(&mac, "capabilities").is_empty()
  })
  .await;
  let caps = &records(&mac, "capabilities")[0];
  assert_eq!(
    caps.fields["servers"],
    json!(["broken", "late", "mail", "secret"])
  );
  assert_eq!(caps.fields["shared"][0]["server"], json!("broken"));
  assert_eq!(caps.fields["shared"][0]["tools"], json!([]));
  assert_eq!(caps.fields["shared"][1]["server"], json!("late"));
  assert_eq!(caps.fields["shared"][1]["tools"], json!([]));
  assert_eq!(caps.fields["shared"][2]["server"], json!("mail"));
  let names = |caps: &stitch_store::IndexedRecord, i: usize| -> Vec<String> {
    caps.fields["shared"][i]["tools"]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| t["name"].as_str().unwrap().to_owned())
      .collect()
  };
  assert_eq!(names(caps, 2), ["archive", "more", "search"], "sorted");
  // The late server's command appears: it is tried again after its
  // backoff, and published with its tools.
  std::os::unix::fs::symlink(std::env::current_exe().unwrap(), mac.join("late-server")).unwrap();
  until("the late server published", || {
    records(&mac, "capabilities")
      .iter()
      .any(|c| !names(c, 1).is_empty())
  })
  .await;
  assert_eq!(records(&mac, "capabilities").len(), 2);
  m.request(&Request::Sync { peer: None }).await.unwrap();
  until("the capabilities on the phone", || {
    !records(&phone, "capabilities").is_empty()
  })
  .await;

  // The phone's thread lists the Mac's tools; a call runs there.
  let Reply::ChatDone { thread, .. } = p
    .chat(
      &chat(None, "find the invoice", &["+mail", "+secret"]),
      |_| None,
    )
    .await
    .unwrap()
  else {
    panic!()
  };
  {
    let bodies = phone_sent.lock().unwrap();
    let tools = offered(&bodies[0]);
    let search = tools
      .iter()
      .find(|(n, _)| n == "mail__search")
      .unwrap_or_else(|| panic!("{tools:?}"));
    assert_eq!(search.1, "MCP server mail on mac: Search the mail.");
    assert!(tools.iter().any(|(n, _)| n == "queue_call"), "{tools:?}");
    assert!(
      !tools.iter().any(|(n, _)| n.starts_with("secret__")),
      "not shared: {tools:?}"
    );
    let system = bodies[0]["system"].to_string();
    assert!(
      system.contains("mail__*: MCP server mail on mac"),
      "{system}"
    );
    assert!(system.contains("secret: turned on for this thread, but MCP server secret is on mac, which does not share it"), "{system}");
    assert!(
      results(&bodies[1])
        .iter()
        .any(|r| r.contains("found: invoice")),
      "{:?}",
      results(&bodies[1])
    );
  }
  let Reply::Toolset { tools, .. } = p
    .request(&Request::Toolset {
      thread: Some(thread.clone()),
      cwd: None,
      global: false,
      tools: vec![],
      skills: vec![],
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  let mail = tools.iter().find(|i| i.name == "mail").unwrap();
  assert!(
    mail.on && mail.available && mail.node.as_deref() == Some("mac"),
    "{mail:?}"
  );
  let secret = tools.iter().find(|i| i.name == "secret").unwrap();
  assert!(!secret.available, "{secret:?}");

  // A call the Mac's rules do not allow: the approval is asked on the Mac,
  // synced to the phone, and answered there.
  let mut chatting = Client::connect(&phone).await.unwrap();
  let t = thread.clone();
  let archived = tokio::spawn(async move {
    chatting
      .chat(&chat(Some(&t), "archive it", &[]), |_| None)
      .await
      .unwrap()
  });
  until("the Mac's question on the phone", || {
    records(&phone, "question").iter().any(|q| {
      q.fields["command"]
        .as_str()
        .is_some_and(|c| c.starts_with("mail__archive"))
    })
  })
  .await;
  let q = records(&phone, "question").pop().unwrap();
  assert_eq!(q.node, "mac");
  assert_eq!(q.fields["session"], json!("mcp:mail"));
  p.request(&Request::Answer {
    question: q.id.to_string(),
    allow: true,
    reason: None,
    always: false,
  })
  .await
  .unwrap();
  let done = tokio::time::timeout(Duration::from_secs(30), archived)
    .await
    .unwrap()
    .unwrap();
  assert!(matches!(done, Reply::ChatDone { .. }), "{done:?}");
  assert!(
    results(&phone_sent.lock().unwrap()[3])
      .iter()
      .any(|r| r.contains("archived: invoice") && r.contains("approved by the user")),
    "{:?}",
    results(&phone_sent.lock().unwrap()[3])
  );

  // The same call again: the earlier answer does not approve it (the Mac
  // asks anew), and with the phone's turn interrupted while it asks, the
  // Mac drops the call and withdraws its question.
  let archive_questions = || {
    records(&phone, "question")
      .into_iter()
      .filter(|q| {
        q.fields["command"]
          .as_str()
          .is_some_and(|c| c.starts_with("mail__archive"))
      })
      .count()
  };
  let mut chatting = Client::connect(&phone).await.unwrap();
  let (interrupt, outgoing) = tokio::sync::mpsc::unbounded_channel();
  let t = thread.clone();
  let again = tokio::spawn(async move {
    chatting
      .chat_with(
        &chat(Some(&t), "archive it again", &[]),
        Some(outgoing),
        |_| None,
      )
      .await
      .unwrap()
  });
  until("the Mac's second question on the phone", || {
    archive_questions() == 2
  })
  .await;
  interrupt.send(Request::Interrupt).unwrap();
  tokio::time::timeout(Duration::from_secs(30), again)
    .await
    .unwrap()
    .unwrap();
  until("the Mac to withdraw its question", || {
    records(&mac, "answer")
      .iter()
      .any(|a| a.fields["choice"] == json!("decline") && a.node == "mac")
  })
  .await;
  let withdrawn = records(&mac, "answer")
    .into_iter()
    .find(|a| a.fields["choice"] == json!("decline"))
    .unwrap();
  let body = RecordRoot::open(&mac.join("home"))
    .unwrap()
    .read_record(&withdrawn.id)
    .unwrap()
    .body;
  assert!(body.contains("withdrawn: the caller is gone"), "{body}");
  // Once more: asked anew, neither the allow before nor the withdrawal
  // answering it; answered "always", the call after it is not asked.
  let before: Vec<String> = records(&phone, "question")
    .iter()
    .map(|q| q.id.to_string())
    .collect();
  let mut chatting = Client::connect(&phone).await.unwrap();
  let t = thread.clone();
  let once_more = tokio::spawn(async move {
    chatting
      .chat(&chat(Some(&t), "archive it once more", &[]), |_| None)
      .await
      .unwrap()
  });
  until("the Mac's third question on the phone", || {
    archive_questions() == 3
  })
  .await;
  let q = records(&phone, "question")
    .into_iter()
    .find(|q| !before.contains(&q.id.to_string()))
    .unwrap();
  assert_eq!(q.fields["grant"], json!(["archive"]));
  assert_eq!(q.fields["grant_server"], json!("mail"));
  assert!(q.fields["choices"].to_string().contains("always"), "{q:?}");
  p.request(&Request::Answer {
    question: q.id.to_string(),
    allow: true,
    reason: None,
    always: true,
  })
  .await
  .unwrap();
  tokio::time::timeout(Duration::from_secs(30), once_more)
    .await
    .unwrap()
    .unwrap();
  assert!(
    results(&phone_sent.lock().unwrap()[6])
      .iter()
      .any(|r| r.contains("archived: invoice") && r.contains("approved by the user")),
    "{:?}",
    results(&phone_sent.lock().unwrap()[6])
  );
  assert!(
    results(&phone_sent.lock().unwrap()[7])
      .iter()
      .any(|r| r.contains("archived: invoice") && !r.contains("approved by the user")),
    "{:?}",
    results(&phone_sent.lock().unwrap()[7])
  );
  assert_eq!(archive_questions(), 3, "granted: not asked");
  // Saved on the Mac, the node that asked, for the thread's place.
  let saved = Config::load(&mac).unwrap().mcp.unwrap();
  assert_eq!(
    saved
      .places
      .values()
      .map(|p| p.allow.clone())
      .collect::<Vec<_>>(),
    [[("mail".to_owned(), vec!["archive".to_owned()])]
      .into_iter()
      .collect()],
  );

  // The Mac stops: the call fails at once and is queued.
  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  let started = std::time::Instant::now();
  p.chat(
    &chat(Some(&thread), "search for the later one", &[]),
    |_| None,
  )
  .await
  .unwrap();
  assert!(
    started.elapsed() < Duration::from_secs(20),
    "{:?}",
    started.elapsed()
  );
  {
    let bodies = phone_sent.lock().unwrap();
    let failed = results(&bodies[9]);
    assert!(
      failed.iter().any(|r| r.contains("runs on mac, which cannot be reached now") && r.contains("queue_call")),
      "{failed:?}"
    );
    assert!(
      results(&bodies[10])
        .iter()
        .any(|r| r.contains("Queued as ")),
      "{:?}",
      results(&bodies[10])
    );
  }
  let calls = records(&phone, "call");
  assert_eq!(calls.len(), 1);
  assert_eq!(calls[0].fields["node"], json!("mac"));

  // Back: it takes the call after a sync, and the result wakes the thread.
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();
  p.request(&Request::Sync { peer: None }).await.unwrap();
  until("the result to wake the phone's thread", || {
    phone_sent.lock().unwrap().len() >= 12
  })
  .await;
  {
    let bodies = phone_sent.lock().unwrap();
    let all = bodies[11]["messages"].to_string();
    assert!(
      all.contains("queued call mail__search") && all.contains("found: later"),
      "{all}"
    );
  }
  let reports = records(&mac, "report");
  assert!(
    reports.iter().any(|r| r.fields["outcome"] == json!("done")),
    "{reports:?}"
  );

  // The same capabilities are never published twice: not after the
  // Mac's restart, nor at a turn of its own.
  assert_eq!(records(&mac, "capabilities").len(), 2);
  m.chat(&chat(None, "hello", &["+mail"]), |_| None)
    .await
    .unwrap();
  tokio::time::sleep(Duration::from_secs(1)).await;
  assert_eq!(records(&mac, "capabilities").len(), 2);
  // A shared server's tools change (a call from the phone, so no turn
  // on the Mac): published again.
  p.chat(&chat(Some(&thread), "one more", &[]), |_| None)
    .await
    .unwrap();
  assert!(
    results(&phone_sent.lock().unwrap()[13])
      .iter()
      .any(|r| r.contains("added")),
    "{:?}",
    results(&phone_sent.lock().unwrap()[13])
  );
  until("the new tool published", || {
    records(&mac, "capabilities")
      .iter()
      .any(|c| names(c, 2).contains(&"extra".to_owned()))
  })
  .await;
  assert_eq!(records(&mac, "capabilities").len(), 3);

  m.request(&Request::Stop).await.unwrap();
  mac_node.wait().await.unwrap();
  p.request(&Request::Stop).await.unwrap();
  phone_node.wait().await.unwrap();
}
