//! Folder sync between two nodes (design §2.16): a folder at a different
//! path on each, linked as one; what one node changes reaches the other,
//! both ways, as files; text edited on both merges, and edits that overlap
//! keep both versions.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use stitch_net::{BlobPolicy, Discovery, Net, NodeState, Peers, Policy, Ticket, bind};
use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;
use stitch_node::config::{AppsSettings, Config, FolderSettings};
use stitch_records::{Author, Draft, kinds};
use stitch_store::{BlobStore, Index, RecordRoot, Role};

fn free_port() -> u16 {
  std::net::UdpSocket::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap()
    .port()
}

fn config(data: &Path, name: &str, port: u16) {
  Config {
    node: Some(name.into()),
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    port: Some(port),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    // Files over 1 MB stay on their node.
    folders: Some(FolderSettings {
      max_file_mb: Some(1),
      ..FolderSettings::default()
    }),
    ..Config::default()
  }
  .save(data)
  .unwrap();
}

/// Joins `data` to the inviter of `ticket`, as `stitch join` does.
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
        blobs: BlobPolicy::All,
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

async fn until(what: &str, ok: impl FnMut() -> bool) {
  until_or(what, ok, String::new).await
}

/// `until`, saying `state()` when it times out.
async fn until_or(what: &str, mut ok: impl FnMut() -> bool, state: impl Fn() -> String) {
  for _ in 0..300 {
    if ok() {
      return;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  panic!("timed out waiting for {what}\n{}", state());
}

async fn folder_sync(c: &mut Client, key: &str) -> (bool, usize, Vec<String>, Option<String>) {
  match c
    .request(&Request::FolderSync { key: key.into() })
    .await
    .unwrap()
  {
    Reply::FolderPass {
      published,
      applied,
      conflicts,
      skipped,
    } => (published, applied, conflicts, skipped),
    other => panic!("{other:?}"),
  }
}

fn read(p: &Path) -> String {
  std::fs::read_to_string(p).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_is_the_same_on_two_nodes() {
  let dir = tempfile::tempdir().unwrap();
  let d = std::fs::canonicalize(dir.path()).unwrap();
  let (mac, linux) = (d.join("mac"), d.join("linux"));
  let (mac_port, linux_port) = (free_port(), free_port());
  config(&mac, "mac", mac_port);
  config(&linux, "linux", linux_port);
  // The folder, at a path of its own on each node (not in a home
  // directory: both nodes run on this machine).
  let (on_mac, on_linux) = (d.join("notes-mac"), d.join("notes-linux"));
  std::fs::create_dir_all(on_mac.join("docs")).unwrap();
  std::fs::create_dir_all(&on_linux).unwrap();
  std::fs::write(on_mac.join("AGENTS.md"), "Be brief.\n").unwrap();
  std::fs::write(on_mac.join("docs/plan.md"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
  std::fs::write(on_mac.join("photo.png"), [0u8, 159, 146, 150]).unwrap();
  std::fs::write(on_mac.join("video.mov"), vec![1u8; 1_100_000]).unwrap();

  let home = RecordRoot::init(&mac.join("home"), "home".parse().unwrap(), Role::Home).unwrap();
  let node: stitch_records::NodeName = "mac".parse().unwrap();
  // A thread in the folder makes it a place; a link makes the two paths
  // one folder.
  let mut at = toml::Table::new();
  at.insert("node".into(), "mac".into());
  at.insert("path".into(), on_mac.display().to_string().into());
  home
    .publish(
      &node,
      Draft::new(kinds::THREAD, Author::user(node.clone()))
        .unwrap()
        .field("folder", toml::Value::Table(at))
        .unwrap()
        .body("# notes\n"),
    )
    .unwrap();
  home
    .publish(
      &node,
      stitch_node::place::link_draft(
        &node,
        &format!("mac:{}", on_mac.display()),
        &format!("linux:{}", on_linux.display()),
      )
      .unwrap(),
    )
    .unwrap();
  Index::open(&mac.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();
  let index = Index::open(&mac.join("index.sqlite")).unwrap();
  let places = stitch_node::place::Places::load(&index);
  let key = places.folders.key("mac", &on_mac.display().to_string());
  assert_eq!(
    key,
    places.folders.key("linux", &on_linux.display().to_string())
  );
  drop(index);

  // A new folder syncs nothing until the user lets it.
  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();
  let (published, _, _, skipped) = folder_sync(&mut m, &key).await;
  assert!(!published);
  assert!(skipped.unwrap().contains("temporary directory"));
  drop(m);
  mac_node.stop().await.unwrap();
  home
    .publish(
      &node,
      stitch_node::place::sync_draft(&node, &key, true).unwrap(),
    )
    .unwrap();
  Index::open(&mac.join("index.sqlite"))
    .unwrap()
    .refresh(&home)
    .unwrap();

  let mac_node = stitch_node::start(&mac).await.unwrap();
  let mut m = Client::connect(&mac).await.unwrap();
  let Reply::Ticket(ticket) = m.request(&Request::Invite { ttl_secs: 60 }).await.unwrap() else {
    panic!()
  };
  join(&linux, "linux", linux_port, &ticket.parse().unwrap()).await;
  let linux_node = stitch_node::start(&linux).await.unwrap();
  let mut l = Client::connect(&linux).await.unwrap();
  l.request(&Request::Sync { peer: None }).await.unwrap();

  // The Mac's files reach Linux, the photo too; not the video, over the cap.
  let (published, ..) = folder_sync(&mut m, &key).await;
  assert!(published);
  until("the folder on linux", || {
    read(&on_linux.join("docs/plan.md")).starts_with("one")
  })
  .await;
  assert_eq!(read(&on_linux.join("AGENTS.md")), "Be brief.\n");
  assert_eq!(
    std::fs::read(on_linux.join("photo.png")).unwrap(),
    [0u8, 159, 146, 150]
  );
  assert!(
    !on_linux.join("video.mov").exists(),
    "files over the cap stay where they are"
  );

  // Linux's edit reaches the Mac.
  std::fs::write(on_linux.join("AGENTS.md"), "Be brief. Be kind.\n").unwrap();
  let (published, ..) = folder_sync(&mut l, &key).await;
  assert!(published);
  until("linux's edit on the mac", || {
    read(&on_mac.join("AGENTS.md")) == "Be brief. Be kind.\n"
  })
  .await;

  // Both edit the plan, apart: merged. Then the same line: both kept.
  // The nodes stop pushing meanwhile by editing before either syncs.
  std::fs::write(on_mac.join("docs/plan.md"), "ONE\ntwo\nthree\nfour\nfive\n").unwrap();
  std::fs::write(
    on_linux.join("docs/plan.md"),
    "one\ntwo\nthree\nfour\nFIVE\n",
  )
  .unwrap();
  folder_sync(&mut m, &key).await;
  folder_sync(&mut l, &key).await;
  folder_sync(&mut m, &key).await;
  until("the merge on both", || {
    read(&on_mac.join("docs/plan.md")) == "ONE\ntwo\nthree\nfour\nFIVE\n"
      && read(&on_linux.join("docs/plan.md")) == "ONE\ntwo\nthree\nfour\nFIVE\n"
  })
  .await;
  // Settled first: a sync still under way runs a pass on the node it
  // reaches, and one between the two edits below would carry the first
  // over before the second is written (an edit after it, not beside it).
  let snaps = |data: &Path| {
    let index = Index::open(&data.join("index.sqlite")).unwrap();
    let mut ids: Vec<String> =
      stitch_node::folders::snapshots(&index, &stitch_node::folders::store_id(&key))
        .unwrap()
        .into_iter()
        .map(|s| s.id.to_string())
        .collect();
    ids.sort();
    ids
  };
  let mut still = 0;
  let mut last = Vec::new();
  for _ in 0..300 {
    let now = snaps(&mac);
    still = if now == last && now == snaps(&linux) {
      still + 1
    } else {
      0
    };
    last = now;
    if still == 10 {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert_eq!(still, 10, "the nodes never settled: {last:?}");

  std::fs::write(on_mac.join("AGENTS.md"), "Be brief (mac).\n").unwrap();
  tokio::time::sleep(Duration::from_millis(20)).await;
  std::fs::write(on_linux.join("AGENTS.md"), "Be brief (linux).\n").unwrap();
  // Whichever node merges (a push starts a pass on the other), the later
  // edit stands and the other is kept beside it, on both.
  folder_sync(&mut m, &key).await;
  folder_sync(&mut l, &key).await;
  folder_sync(&mut m, &key).await;
  let copies = |d: &Path| -> Vec<String> {
    std::fs::read_dir(d)
      .unwrap()
      .filter_map(|e| e.ok()?.file_name().into_string().ok())
      .filter(|n| n.starts_with("AGENTS ("))
      .collect()
  };
  let state = || {
    let index = Index::open(&mac.join("index.sqlite")).unwrap();
    let snaps =
      stitch_node::folders::snapshots(&index, &stitch_node::folders::store_id(&key)).unwrap();
    format!(
      "mac: {:?} {:?}\nlinux: {:?} {:?}\nsnapshots (as the mac has them):\n{}",
      copies(&on_mac),
      read(&on_mac.join("AGENTS.md")),
      copies(&on_linux),
      read(&on_linux.join("AGENTS.md")),
      snaps
        .iter()
        .map(|s| format!(
          "  {} {} prev={:?} changed={:?}",
          s.id, s.at, s.prev, s.changed
        ))
        .collect::<Vec<_>>()
        .join("\n")
    )
  };
  until_or(
    "both versions on both",
    || copies(&on_mac).len() == 1 && copies(&on_linux) == copies(&on_mac),
    state,
  )
  .await;
  let copy = copies(&on_mac)[0].clone();
  until("the same AGENTS.md on both", || {
    read(&on_mac.join("AGENTS.md")) == read(&on_linux.join("AGENTS.md"))
  })
  .await;
  let (kept, copied) = (read(&on_mac.join("AGENTS.md")), read(&on_mac.join(&copy)));
  assert_ne!(kept, copied);
  assert!(
    [kept.as_str(), copied.as_str()].contains(&"Be brief (mac).\n")
      && [kept.as_str(), copied.as_str()].contains(&"Be brief (linux).\n")
  );

  // A deletion travels too.
  std::fs::remove_file(on_linux.join(&copy)).unwrap();
  folder_sync(&mut l, &key).await;
  until("the deletion on the mac", || !on_mac.join(&copy).exists()).await;

  // The history says who changed what.
  let index = Index::open(&mac.join("index.sqlite")).unwrap();
  let log = stitch_node::folders::log(&index, &key, Some("AGENTS.md")).unwrap();
  assert!(log.iter().any(|c| c.node == "linux"));
  assert!(log.iter().any(|c| c.node == "mac"));
  let blobs = BlobStore::open(&mac.join("blobs")).unwrap();
  let first_linux = log.iter().rev().find(|c| c.node == "linux").unwrap();
  let diff = stitch_node::folders::diff(
    &index,
    &blobs,
    &key,
    None,
    Some(&first_linux.id),
    Some("AGENTS.md"),
  )
  .unwrap();
  assert!(diff.contains("+Be brief. Be kind."), "{diff}");
  drop(index);

  // Settled, the nodes have nothing more to say to each other: no merge
  // answers a merge (what the Mac does not carry, its video, does not set
  // their trees apart).
  let count = || {
    let index = Index::open(&mac.join("index.sqlite")).unwrap();
    stitch_node::folders::snapshots(&index, &stitch_node::folders::store_id(&key))
      .unwrap()
      .len()
  };
  tokio::time::sleep(Duration::from_millis(500)).await;
  let before = count();
  for _ in 0..3 {
    assert!(!folder_sync(&mut m, &key).await.0);
    assert!(!folder_sync(&mut l, &key).await.0);
  }
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert_eq!(count(), before);
  assert!(before < 20, "{before} snapshots for a handful of changes");

  // A folder that becomes a git checkout stops syncing.
  std::fs::create_dir_all(on_linux.join(".git")).unwrap();
  std::fs::write(on_linux.join("AGENTS.md"), "now in git\n").unwrap();
  let (published, _, _, skipped) = folder_sync(&mut l, &key).await;
  assert!(!published);
  assert!(skipped.unwrap().contains("git"));

  drop((m, l));
  linux_node.stop().await.unwrap();
  mac_node.stop().await.unwrap();
}
