//! SSH on a running node (design §2.20): a host record with a password
//! secret; the shell tools given `host` run there (the host's key trusted
//! on first use and kept as a record), what reaches beyond asks, a dropped
//! connection is made again on the next call and the result says so; this
//! device's key, authorized on the host.

#[cfg(any(target_os = "ios", target_os = "visionos"))]
fn main() {}

#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
fn main() {
  let rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap();
  rt.block_on(t::shells_on_a_host());
  println!("ssh: shells on a host passed");
}

#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
mod t {
  use std::path::Path;
  use std::sync::{Arc, Mutex};

  use serde_json::{Value, json};
  use stitch_node::api::{Reply, Request, SecretOp, SshHost, SshOp};
  use stitch_node::client::Client;
  use stitch_node::config::{AppsSettings, Config, ProviderConfig, ShellSettings};
  use stitch_node::keys::{self, Store};
  use stitch_store::{Index, RecordRoot, Role};
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

  fn tool(id: &str, name: &str, input: Value) -> String {
    sse(vec![
      (
        "message_start",
        json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 50}}}),
      ),
      (
        "content_block_start",
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
      ),
      (
        "content_block_delta",
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
      ),
      (
        "content_block_stop",
        json!({"type": "content_block_stop", "index": 0}),
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

  fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
      .unwrap()
      .local_addr()
      .unwrap()
      .port()
  }

  fn config(data: &Path, anthropic: &str) -> Config {
    let mut c = Config {
      node: Some("mbp".into()),
      discovery: Some("local".into()),
      sync_interval_secs: Some(0),
      threads: Some(stitch_node::config::ThreadSettings {
        titles: Some(false),
        ..Default::default()
      }),
      shell: Some(ShellSettings {
        approval: Some("ask".into()),
        ..Default::default()
      }),
      port: Some(free_port()),
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
    c
  }

  fn chat(thread: Option<&str>, text: &str) -> Request {
    Request::Chat {
      thread: thread.map(str::to_owned),
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

  async fn ssh(c: &mut Client, op: SshOp) -> Reply {
    c.request(&Request::Ssh { op }).await.unwrap()
  }

  /// Runs a turn; the tool results, the commands asked about (each
  /// declined), and the thread.
  async fn turn(c: &mut Client, thread: Option<&str>) -> (Vec<String>, Vec<String>, String) {
    let mut results = Vec::new();
    let mut asked = Vec::new();
    let done = c
      .chat(&chat(thread, "Look at box"), |r| match r {
        Reply::ToolResult { output, .. } => {
          results.push(output.clone());
          None
        }
        Reply::Approve { command, .. } => {
          asked.push(command.clone());
          let (answer, decided) = tokio::sync::oneshot::channel();
          answer
            .send(stitch_node::tools::Decision::decline(None))
            .unwrap();
          Some(decided)
        }
        _ => None,
      })
      .await
      .unwrap();
    let Reply::ChatDone { thread, .. } = done else {
      panic!("{done:?}")
    };
    (results, asked, thread)
  }

  pub async fn shells_on_a_host() {
    let server = stitch_ssh::testing::TestServer::start().await;
    let port = server.addr.port();
    let dir = tempfile::tempdir().unwrap();
    let d = std::fs::canonicalize(dir.path()).unwrap();
    let (base, sent) = serve(vec![
      tool(
        "t1",
        "shell_run",
        json!({"host": "box", "command": "echo hi-from-$((40+2)); cd /tmp"}),
      ),
      tool(
        "t2",
        "shell_run",
        json!({"host": "box", "command": "sudo true"}),
      ),
      answer("Done."),
      tool("t3", "shell_run", json!({"host": "box", "command": "pwd"})),
      answer("Done again."),
    ])
    .await;
    let data = d.join("data");
    config(&data, &base);
    let root = RecordRoot::init(&d.join("home"), "home".parse().unwrap(), Role::Home).unwrap();
    Index::open(&data.join("index.sqlite"))
      .unwrap()
      .refresh(&root)
      .unwrap();
    let node = stitch_node::start(&data).await.unwrap();
    let mut c = Client::connect(&data).await.unwrap();

    c.request(&Request::Secret {
      op: SecretOp::Set {
        name: "boxpw".into(),
        scope: "global".into(),
        value: Some("pw".into()),
        device_only: Some(true),
        file: true,
      },
      cwd: None,
    })
    .await
    .unwrap();
    let added = ssh(
      &mut c,
      SshOp::HostSet(SshHost {
        name: "box".into(),
        address: "127.0.0.1".into(),
        port,
        user: "me".into(),
        auth: "secret:boxpw".into(),
        ..SshHost::default()
      }),
    )
    .await;
    assert!(matches!(added, Reply::Ok), "{added:?}");
    let Reply::SshHosts(hosts) = ssh(&mut c, SshOp::Hosts).await else {
      panic!()
    };
    assert!(hosts.iter().any(|h| h.name == "box" && h.from == "record"));

    let (results, asked, thread) = turn(&mut c, None).await;
    // Run there, unasked; what reaches beyond asked (and was declined).
    assert!(results[0].contains("hi-from-42"), "{results:?}");
    assert_eq!(asked, ["sudo true"]);
    assert!(results[1].contains("declined"), "{results:?}");
    // The model was offered `host`, with the hosts.
    {
      let bodies = sent.lock().unwrap();
      let tools = bodies[0]["tools"].as_array().unwrap();
      let run = tools.iter().find(|t| t["name"] == "shell_run").unwrap();
      // With this machine's `~/.ssh/config` hosts, if it has one.
      let offered = run["input_schema"]["properties"]["host"]["enum"]
        .as_array()
        .unwrap()
        .clone();
      assert!(offered.contains(&json!("box")), "{offered:?}");
      assert!(bodies[0].to_string().contains("# SSH hosts"));
    }
    // Its key, trusted on first use, is a record.
    let Reply::SshHostKeys(keys) = ssh(&mut c, SshOp::HostKeys).await else {
      panic!()
    };
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert_eq!(keys[0].host, format!("[127.0.0.1]:{port}"));
    assert_eq!(keys[0].name, "box");

    // Dropped: the next call reconnects, and says so.
    server.drop_connections().await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (results, _, _) = turn(&mut c, Some(&thread)).await;
    assert!(results[0].contains("reconnected"), "{results:?}");
    if results[0].contains("the same tmux session") {
      // Picked up where it was: the directory too.
      assert!(results[0].contains("/tmp"), "{results:?}");
    }

    // This device's key, authorized on the host from here.
    let Reply::SshKey { line, kind, .. } = ssh(&mut c, SshOp::Key).await else {
      panic!()
    };
    assert!(line.starts_with("ssh-ed25519 ") && line.ends_with(" stitch@mbp"));
    assert_eq!(kind, "keychain");
    let Reply::Lines(said) = ssh(
      &mut c,
      SshOp::Authorize {
        host: "box".into(),
        key: None,
        node: None,
      },
    )
    .await
    else {
      panic!()
    };
    assert!(said[0].contains("added to me@box"), "{said:?}");
    let authorized = std::fs::read_to_string(server.home().join(".ssh/authorized_keys")).unwrap();
    assert_eq!(authorized.trim(), line);
    // Twice is once.
    ssh(
      &mut c,
      SshOp::Authorize {
        host: "box".into(),
        key: None,
        node: None,
      },
    )
    .await;
    let again = std::fs::read_to_string(server.home().join(".ssh/authorized_keys")).unwrap();
    assert_eq!(again, authorized);

    drop(c);
    node.stop().await.unwrap();
  }
}
