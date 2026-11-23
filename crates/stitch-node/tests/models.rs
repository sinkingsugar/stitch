//! The models the configured providers offer, through the node: listed
//! from each provider's models endpoint (a mock here), with why a
//! provider gave none; another node's, asked of it.

use std::path::Path;

use stitch_net::{Peer, Peers};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig};
use stitch_node::keys::{self, Store};
use stitch_store::{Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Answers every request with an Anthropic-shaped model list.
async fn serve() -> String {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  tokio::spawn(async move {
    loop {
      let (mut sock, _) = listener.accept().await.unwrap();
      let mut buf = [0u8; 8192];
      let n = sock.read(&mut buf).await.unwrap();
      let head = String::from_utf8_lossy(&buf[..n]).to_string();
      assert!(head.starts_with("GET /v1/models"), "{head}");
      let body = r#"{"data": [{"id": "claude-sonnet-5-5"}, {"id": "claude-opus-5-5"}]}"#;
      let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
      );
      sock.write_all(response.as_bytes()).await.unwrap();
      let _ = sock.shutdown().await;
    }
  });
  base
}

/// A node named `name`; with `anthropic`, an Anthropic provider there.
fn setup(data: &Path, home: &Path, name: &str, anthropic: Option<String>, port: Option<u16>) {
  let mut c = Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // No titles asked of the mock model: its answers go in order.
    threads: Some(stitch_node::config::ThreadSettings {
      titles: Some(false),
      ..Default::default()
    }),
    port,
    default_model: anthropic
      .is_some()
      .then(|| "anthropic/claude-sonnet-5-5".into()),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  };
  if let Some(base) = anthropic {
    c.providers.insert(
      "anthropic".into(),
      ProviderConfig {
        kind: "anthropic".into(),
        base_url: Some(base),
        ..ProviderConfig::default()
      },
    );
    keys::set(data, "anthropic", "sk-test", Store::File).unwrap();
  }
  c.save(data).unwrap();
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

/// The display lookup sees a new checkout's own defaults before it has
/// hosted a chat, and later settings edits, just as chat placement does.
#[tokio::test(flavor = "multi_thread")]
async fn effective_models_use_the_place_even_before_its_first_chat() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (data, home, project) = (d.join("data"), d.join("home"), d.join("project"));
  setup(&data, &home, "mbp", Some(serve().await), None);
  std::fs::create_dir_all(project.join(".stitch")).unwrap();
  for args in [
    vec!["init", "--quiet"],
    vec![
      "-c",
      "user.name=T",
      "-c",
      "user.email=t@example.com",
      "commit",
      "--quiet",
      "--allow-empty",
      "-m",
      "init",
    ],
  ] {
    assert!(
      std::process::Command::new("git")
        .args(args)
        .current_dir(&project)
        .status()
        .unwrap()
        .success()
    );
  }
  let write = |model: &str| {
    std::fs::write(
      project.join(".stitch/settings.toml"),
      format!("[models]\ndefault = '{model}'\nquick = 'anthropic/claude-haiku-5-5'\n"),
    )
    .unwrap()
  };
  write("anthropic/claude-opus-5");
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  let request = |place: Option<&str>, model: Option<&str>| Request::EffectiveModel {
    thread: None,
    place: place.map(str::to_owned),
    cwd: Some(project.display().to_string()),
    model: model.map(str::to_owned),
  };
  let Reply::Lines(lines) = c.request(&request(None, None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-opus-5"]);
  let Reply::Lines(lines) = c.request(&request(None, Some("quick"))).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-haiku-5-5"]);
  write("anthropic/claude-sonnet-5-5");
  let Reply::Lines(lines) = c.request(&request(None, Some("default"))).await.unwrap() else {
    panic!()
  };
  assert_eq!(lines, ["anthropic/claude-sonnet-5-5"]);
  write("anthropic/claude-opus-5");
  let Reply::Lines(lines) = c.request(&request(Some("global"), None)).await.unwrap() else {
    panic!()
  };
  assert_eq!(
    lines,
    ["anthropic/claude-sonnet-5-5"],
    "global ignores the cwd's project override"
  );
  c.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_node_lists_its_providers_models() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  setup(
    &data,
    &dir.path().join("home"),
    "mbp",
    Some(serve().await),
    None,
  );
  let node = stitch_node::start(&data).await.unwrap();
  let mut client = Client::connect(&data).await.unwrap();
  let Reply::Models {
    models,
    problems,
    default,
  } = client
    .request(&Request::Models { node: None })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert_eq!(
    models,
    ["anthropic/claude-opus-5-5", "anthropic/claude-sonnet-5-5"]
  );
  assert_eq!(default, "anthropic/claude-sonnet-5-5");
  // The built-in providers without credentials say so.
  assert!(
    problems.iter().any(|p| p.starts_with("openai:")),
    "{problems:?}"
  );
  client.request(&Request::Stop).await.unwrap();
  node.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_asks_another_for_its_models() {
  let dir = tempfile::tempdir().unwrap();
  let d = dir.path();
  let (data_a, data_b) = (d.join("a"), d.join("b"));
  let (port_a, port_b) = (free_port(), free_port());
  // Only the first holds a provider with credentials.
  setup(
    &data_a,
    &d.join("home-a"),
    "mbp",
    Some(serve().await),
    Some(port_a),
  );
  setup(&data_b, &d.join("home-b"), "nas", None, Some(port_b));
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

  // Its own: none, and its default is the built-in one.
  let Reply::Models {
    models, default, ..
  } = b.request(&Request::Models { node: None }).await.unwrap()
  else {
    panic!()
  };
  assert!(models.is_empty(), "{models:?}");
  assert_eq!(default, stitch_node::config::DEFAULT_MODEL);
  // The other's, with the other's default.
  let reply = b
    .request(&Request::Models {
      node: Some("mbp".into()),
    })
    .await
    .unwrap();
  let Reply::Models {
    models, default, ..
  } = reply
  else {
    panic!("{reply:?}")
  };
  assert_eq!(
    models,
    ["anthropic/claude-opus-5-5", "anthropic/claude-sonnet-5-5"]
  );
  assert_eq!(default, "anthropic/claude-sonnet-5-5");
  // Naming itself is asking itself; a node it does not know, an error.
  let Reply::Models { models, .. } = b
    .request(&Request::Models {
      node: Some("nas".into()),
    })
    .await
    .unwrap()
  else {
    panic!()
  };
  assert!(models.is_empty());
  let err = b
    .request(&Request::Models {
      node: Some("nowhere".into()),
    })
    .await
    .unwrap_err();
  assert!(err.to_string().contains("no peer `nowhere`"), "{err}");

  // Delegating names a model the other node runs, or is refused with its
  // list before anything is written.
  let delegate = |model: &str| Request::Detach {
    goal: "release notes".into(),
    model: Some(model.into()),
    repo: None,
    cwd: None,
    parent: Some("2026-10-12T000000Z-nas-aaaa".into()),
    budget_usd: None,
    budget_tokens: None,
    budget_minutes: None,
    max_steps: None,
    global: true,
    node: Some("mbp".into()),
    context: None,
    no_tools: false,
    tools: Vec::new(),
    skills: Vec::new(),
    keep: false,
    memory: false,
    read_only: false,
    scopes: Vec::new(),
  };
  let err = b
    .request(&delegate("anthropic/claude-nope"))
    .await
    .unwrap_err()
    .to_string();
  assert!(
    err.contains("mbp cannot run anthropic/claude-nope"),
    "{err}"
  );
  assert!(err.contains("anthropic/claude-opus-5-5"), "{err}");
  assert!(
    err.contains("its default: anthropic/claude-sonnet-5-5"),
    "{err}"
  );
  // One it runs gets past the check (to the thread, which is not here).
  let err = b
    .request(&delegate("anthropic/claude-opus-5-5"))
    .await
    .unwrap_err()
    .to_string();
  assert!(!err.contains("cannot run"), "{err}");

  for (c, n) in [(&mut a, node_a), (&mut b, node_b)] {
    c.request(&Request::Stop).await.unwrap();
    n.wait().await.unwrap();
  }
}
