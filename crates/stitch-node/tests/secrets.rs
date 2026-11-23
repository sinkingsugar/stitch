//! Secrets and variables (design §2.19) on running nodes: a secret set on
//! one node reaches a shell as the variable `[env]` maps it to, and its
//! value never reaches the model or a record (`«secret:<name>»` instead);
//! synced, it is sealed for each node, opened by each, re-sealed for a
//! node paired later, removed everywhere, and a removed node's suggests
//! rotating it.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request, SecretInfo, SecretOp};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, ProviderConfig, ShellSettings};
use stitch_node::keys::{self, Store};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn main() {
  // Nothing of these tests goes to the login keychain: each node has a
  // `credentials.toml` (`config`), so its secrets and sealing key go there.
  let rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap();
  rt.block_on(secrets_reach_shells_and_never_the_model());
  println!("secrets: shells and redaction passed");
  rt.block_on(env_written_during_a_turn_reaches_its_next_sessions());
  println!("secrets: [env] written during a turn passed");
  rt.block_on(secrets_sync_sealed_to_every_node());
  println!("secrets: sync passed");
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

fn shell(id: &str, command: &str) -> String {
  tool(id, "shell_run", json!({"command": command}))
}

fn tool(id: &str, name: &str, input: Value) -> String {
  let input = input.to_string();
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
      json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": input}}),
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

fn config(data: &Path, name: &str, port: u16, anthropic: &str) -> Config {
  let mut c = Config {
    node: Some(name.into()),
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
    port: Some(port),
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

fn home(data: &Path, dir: &Path) {
  let root = RecordRoot::init(dir, "home".parse().unwrap(), Role::Home).unwrap();
  Index::open(&data.join("index.sqlite"))
    .unwrap()
    .refresh(&root)
    .unwrap();
}

async fn ask(c: &mut Client, op: SecretOp) -> Reply {
  c.request(&Request::Secret { op, cwd: None }).await.unwrap()
}

async fn set(c: &mut Client, name: &str, value: &str, device_only: Option<bool>) {
  ask(
    c,
    SecretOp::Set {
      name: name.into(),
      scope: "global".into(),
      value: Some(value.into()),
      device_only,
      file: false,
    },
  )
  .await;
}

async fn get(c: &mut Client, name: &str) -> Option<String> {
  match c
    .request(&Request::Secret {
      op: SecretOp::Get {
        name: name.into(),
        scope: "global".into(),
      },
      cwd: None,
    })
    .await
  {
    Ok(Reply::Secret(v)) => Some(v),
    _ => None,
  }
}

async fn list(c: &mut Client) -> Vec<SecretInfo> {
  match ask(c, SecretOp::List).await {
    Reply::Secrets(l) => l,
    other => panic!("{other:?}"),
  }
}

fn chat(text: &str) -> Request {
  Request::Chat {
    thread: None,
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

/// Every file under `dir` that holds `needle`.
fn holding(dir: &Path, needle: &str) -> Vec<String> {
  walk(dir)
    .into_iter()
    .filter(|p| {
      std::fs::read(p)
        .map(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
        .unwrap_or(false)
    })
    .map(|p| p.display().to_string())
    .collect()
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
  let mut out = Vec::new();
  for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
    let p = e.path();
    if p.is_dir() {
      out.extend(walk(&p));
    } else {
      out.push(p);
    }
  }
  out
}

async fn secrets_reach_shells_and_never_the_model() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let value = "sk-proj-QuiteSecret0123456789";
  let (base, sent) = serve(vec![
    shell(
      "t1",
      "echo key=$OPENAI_API_KEY hf=$HF_HOME; printenv OPENAI_API_KEY | cut -c1-12",
    ),
    answer("Done."),
  ])
  .await;
  let data = d.join("data");
  let mut c = config(&data, "mbp", free_port(), &base);
  c.env
    .insert("OPENAI_API_KEY".into(), "secret:openai".into());
  c.save(&data).unwrap();
  home(&data, &d.join("home"));
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();

  set(&mut c, "openai", value, None).await;
  assert!(matches!(
    ask(
      &mut c,
      SecretOp::VarSet {
        name: "HF_HOME".into(),
        scope: "global".into(),
        value: "/data/hf".into(),
      },
    )
    .await,
    Reply::Ok
  ));
  // The key stored before (credentials.toml) is listed too, as this
  // device's; the new one is synced (to nobody else yet).
  let l = list(&mut c).await;
  let names: Vec<_> = l.iter().map(|s| s.name.as_str()).collect();
  assert_eq!(names, ["anthropic", "openai"], "{l:?}");
  assert!(l[0].device_only && l[0].used == ["provider anthropic"]);
  assert!(!l[1].device_only && l[1].here && l[1].nodes == ["mbp"]);
  assert_eq!(
    l[1].used,
    ["provider openai", "env OPENAI_API_KEY (this node)"]
  );
  assert_eq!(get(&mut c, "openai").await.as_deref(), Some(value));

  let mut results = Vec::new();
  c.chat(&chat("Show the key"), |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  // The shell had it; the model, the client and the transcript see its name.
  assert!(
    results[0].contains("key=«secret:openai» hf=/data/hf"),
    "{results:?}"
  );
  // A part of it is not the value: shown as it is.
  assert!(results[0].contains("sk-proj-Quit"), "{results:?}");
  let bodies = sent.lock().unwrap().clone();
  let asked = bodies[1].to_string();
  assert!(asked.contains("key=«secret:openai»") && !asked.contains(value));
  assert!(
    holding(&d.join("home"), value).is_empty(),
    "the value is in no record"
  );
  // Removed: gone from the list and the store.
  ask(
    &mut c,
    SecretOp::Remove {
      name: "openai".into(),
      scope: "global".into(),
    },
  )
  .await;
  assert_eq!(get(&mut c, "openai").await, None);
  assert_eq!(list(&mut c).await.len(), 1);
  drop(c);
  node.stop().await.unwrap();
}

/// A place's `.stitch/settings.toml` `[env]` written during a turn: the
/// sessions the turn starts after have the variable (issue k6ng: they had
/// what the turn began with).
async fn env_written_during_a_turn_reaches_its_next_sessions() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let value = "nexus-QuiteSecret0123456789";
  let place = d.join("games");
  std::fs::create_dir_all(&place).unwrap();
  let settings = place.join(".stitch/settings.toml");
  let (base, _) = serve(vec![
    shell("t1", "echo before=$NEXUS_API_KEY."),
    tool(
      "t2",
      "write_file",
      json!({
        "path": settings.display().to_string(),
        "content": "[env]\nNEXUS_API_KEY = \"secret:nexus\"\n",
      }),
    ),
    tool(
      "t3",
      "shell_run",
      json!({"command": "echo after=$NEXUS_API_KEY.", "session": "b"}),
    ),
    answer("Done."),
  ])
  .await;
  let data = d.join("data");
  config(&data, "mbp", free_port(), &base);
  home(&data, &d.join("home"));
  let node = stitch_node::start(&data).await.unwrap();
  let mut c = Client::connect(&data).await.unwrap();
  set(&mut c, "nexus", value, None).await;

  let mut results = Vec::new();
  let mut req = chat("Map the key");
  if let Request::Chat { cwd, global, .. } = &mut req {
    *cwd = Some(place.display().to_string());
    *global = false;
  }
  c.chat(&req, |r| {
    if let Reply::ToolResult { output, .. } = r {
      results.push(output.clone());
    }
    None
  })
  .await
  .unwrap();
  assert!(settings.is_file(), "{results:?}");
  assert!(results[0].contains("before=."), "{results:?}");
  assert!(results[2].contains("after=«secret:nexus»."), "{results:?}");
  drop(c);
  node.stop().await.unwrap();
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

async fn until(what: &str, mut ok: impl AsyncFnMut() -> bool) {
  for _ in 0..200 {
    if ok().await {
      return;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  panic!("timed out waiting for {what}");
}

async fn sync(c: &mut Client) {
  let _ = c.request(&Request::Sync { peer: None }).await;
}

async fn secrets_sync_sealed_to_every_node() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (base, _) = serve(vec![]).await;
  let (phone, mac, pi) = (d.join("phone"), d.join("mac"), d.join("pi"));
  let (phone_port, mac_port, pi_port) = (free_port(), free_port(), free_port());
  config(&phone, "phone", phone_port, &base);
  config(&mac, "mac", mac_port, &base);
  config(&pi, "pi", pi_port, &base);
  home(&phone, &d.join("phone-home"));
  let phone_node = stitch_node::start(&phone).await.unwrap();
  let mut p = Client::connect(&phone).await.unwrap();
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&mac, "mac", mac_port, &ticket.parse().unwrap()).await;
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();

  // Entered once on the phone; the Mac has made its key by then (it saw a
  // secret, or not yet: then the phone re-seals once it has).
  let value = "ghp_SecretOnThePhone123456";
  set(&mut p, "github", value, None).await;
  // One kept on the phone alone.
  set(&mut p, "bank", "pin-0000-1111", Some(true)).await;
  until("the Mac holds the phone's secret", async || {
    sync(&mut p).await;
    sync(&mut m).await;
    get(&mut m, "github").await.as_deref() == Some(value)
  })
  .await;
  assert_eq!(get(&mut m, "bank").await, None, "this device's only");
  let l = list(&mut m).await;
  let gh = l.iter().find(|s| s.name == "github").unwrap();
  assert!(gh.here && !gh.device_only && gh.from == "phone", "{gh:?}");
  assert!(gh.nodes.contains(&"mac".to_owned()) && gh.nodes.contains(&"phone".to_owned()));
  assert!(!l.iter().any(|s| s.name == "bank"));
  // What the records carry is sealed: no value in any of them, on either.
  assert!(holding(&d.join("phone-home"), value).is_empty());
  assert!(holding(&mac.join("home"), value).is_empty());

  // A new value on the Mac reaches the phone.
  let newer = "ghp_RotatedOnTheMac987654";
  set(&mut m, "github", newer, None).await;
  until("the phone has the Mac's new value", async || {
    sync(&mut m).await;
    get(&mut p, "github").await.as_deref() == Some(newer)
  })
  .await;

  // A node paired later gets it re-sealed by a node that holds it.
  let Reply::Ticket(ticket) = p.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&pi, "pi", pi_port, &ticket.parse().unwrap()).await;
  let pi_node = stitch_node::start(&pi).await.unwrap();
  let mut r = Client::connect(&pi).await.unwrap();
  until("the Pi holds it", async || {
    sync(&mut r).await;
    sync(&mut p).await;
    sync(&mut m).await;
    get(&mut r, "github").await.as_deref() == Some(newer)
  })
  .await;

  // Removing the Pi (as `stitch peer remove pi` does) suggests rotating
  // what it held.
  drop(r);
  pi_node.stop().await.unwrap();
  {
    use stitch_records::{Author, Draft, Ref, kinds};
    let root = RecordRoot::open(&d.join("phone-home")).unwrap();
    let mut index = Index::open(&phone.join("index.sqlite")).unwrap();
    index.refresh(&root).unwrap();
    let pi_peer = index
      .peer_records()
      .unwrap()
      .into_iter()
      .find(|r| r.fields["name"] == "pi")
      .expect("the Pi's peer record");
    let id = pi_peer.fields["endpoint"].as_str().unwrap().to_owned();
    let draft = Draft::new(kinds::REVOKE, Author::user("phone".parse().unwrap()))
      .unwrap()
      .reference("revokes", Ref::to(pi_peer.id.clone()))
      .unwrap();
    root.publish(&"phone".parse().unwrap(), draft).unwrap();
    index.refresh(&root).unwrap();
    let path = phone.join("peers.toml");
    let mut local = Peers::load(&path).unwrap();
    local.peers.retain(|p| p.id != id);
    local.save(&path).unwrap();
  }
  until("rotation suggested on the phone", async || {
    list(&mut p)
      .await
      .iter()
      .any(|s| s.name == "github" && s.rotate == ["pi"])
  })
  .await;

  // Removed on the phone: gone from the Mac too.
  ask(
    &mut p,
    SecretOp::Remove {
      name: "github".into(),
      scope: "global".into(),
    },
  )
  .await;
  until("the Mac dropped it", async || {
    sync(&mut p).await;
    get(&mut m, "github").await.is_none()
  })
  .await;
  drop((p, m));
  mac_node.stop().await.unwrap();
  phone_node.stop().await.unwrap();
}
