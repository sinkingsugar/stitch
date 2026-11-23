//! The node as the app opens it: set up on first start, served in process.

use stitch_ffi::Stitch;
use stitch_node::config::{AppsSettings, Config};

/// Runs one of the node's async calls as Swift would, from a thread that
/// is not the node's runtime.
fn wait<T>(f: impl std::future::Future<Output = T>) -> T {
  tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap()
    .block_on(f)
}

#[test]
fn the_app_sets_up_its_node_and_reaches_it_in_process() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  // Local discovery, no periodic sync: nothing leaves the machine.
  Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  }
  .save(&data)
  .unwrap();

  let stitch = Stitch::open(data.display().to_string(), "phone".into()).unwrap();
  assert_eq!(stitch.node_name(), "phone");
  let status = wait(stitch.status()).unwrap();
  assert_eq!(status.node, "phone");
  assert_eq!(status.repos, vec!["home".to_string()]);
  assert!(wait(stitch.clone().threads()).unwrap().is_empty());
  // The revision the app polls: steady while nothing is written.
  let first = wait(stitch.clone().revision());
  assert!(first > 0);
  assert_eq!(wait(stitch.clone().revision()), first);
  // The node's own refusals come back as errors.
  let none = "not-a-thread".to_string();
  assert!(wait(stitch.rename(none.clone(), "A name".into())).is_err());
  assert!(wait(stitch.pin(none.clone(), true)).is_err());
  assert!(wait(stitch.quote(none)).is_err());
  let config = Config::load(&data).unwrap();
  assert_eq!(config.blobs.as_deref(), Some("lazy"));
  wait(stitch.stop()).unwrap();
  drop(stitch);

  // Opened again: its name stays, whatever the app passes.
  let stitch = Stitch::open(data.display().to_string(), "other".into()).unwrap();
  assert_eq!(stitch.node_name(), "phone");
  wait(stitch.stop()).unwrap();
}

#[test]
fn a_node_whose_data_directory_moved_finds_its_home_again() {
  let dir = tempfile::tempdir().unwrap();
  let before = dir.path().join("container-a");
  Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  }
  .save(&before)
  .unwrap();
  let stitch = Stitch::open(before.display().to_string(), "phone".into()).unwrap();
  wait(stitch.stop()).unwrap();
  drop(stitch);

  // iOS moves an app's container when the app is updated.
  let after = dir.path().join("container-b");
  std::fs::rename(&before, &after).unwrap();
  let stitch = Stitch::open(after.display().to_string(), "phone".into()).unwrap();
  assert_eq!(
    wait(stitch.status()).unwrap().repos,
    vec!["home".to_string()]
  );
  let roots = stitch_store::Index::open(&after.join("index.sqlite"))
    .unwrap()
    .roots()
    .unwrap();
  assert_eq!(roots["home"], after.join("home").display().to_string());
  wait(stitch.stop()).unwrap();
}

/// Memory turned on and off from the app: the node starts again in place,
/// and runs memory as its models directory says. (The encoder loads on
/// first use, so a stand-in file serves: nothing here encodes.)
#[test]
fn the_app_turns_memory_on_and_off_with_its_node_running() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  }
  .save(&data)
  .unwrap();
  let stitch = Stitch::open(data.display().to_string(), "phone".into()).unwrap();
  let m = wait(stitch.memory()).unwrap();
  assert!(!m.installed && !m.images && !m.running);

  let models = data.join("models");
  std::fs::create_dir_all(&models).unwrap();
  std::fs::write(models.join(stitch_memory::ModelFiles::MODEL), b"stand-in").unwrap();
  std::fs::write(
    models.join(stitch_memory::ModelFiles::PROJECTOR),
    b"stand-in",
  )
  .unwrap();
  // Images only removed: the node starts again, with memory on.
  wait(stitch.remove_memory(true)).unwrap();
  let m = wait(stitch.memory()).unwrap();
  assert!(m.installed && !m.images && m.running);
  wait(stitch.set_foreground(false));
  wait(stitch.set_foreground(true));
  assert_eq!(wait(stitch.status()).unwrap().node, "phone");

  wait(stitch.remove_memory(false)).unwrap();
  let m = wait(stitch.memory()).unwrap();
  assert!(!m.installed && !m.running);
  assert!(wait(stitch.remember("nope".into(), false)).is_err());
  wait(stitch.stop()).unwrap();
}

/// Records the page a sign-in shows.
struct Browser(std::sync::mpsc::Sender<String>);

impl stitch_ffi::SignInBrowser for Browser {
  fn show(&self, url: String) {
    let _ = self.0.send(url);
  }
}

#[test]
fn a_sign_in_shows_its_page_and_is_cancelled_when_closed() {
  let dir = tempfile::tempdir().unwrap();
  let data = dir.path().join("data");
  Config {
    discovery: Some("local".into()),
    sync_interval_secs: Some(0),
    // Apps not served: a test node never takes the fixed port 7177.
    apps: Some(AppsSettings { port: Some(0) }),
    ..Config::default()
  }
  .save(&data)
  .unwrap();
  let stitch = Stitch::open(data.display().to_string(), "phone".into()).unwrap();
  let (tx, rx) = std::sync::mpsc::channel();
  let signing = {
    let stitch = stitch.clone();
    std::thread::spawn(move || {
      wait(stitch.sign_in("grok".into(), std::sync::Arc::new(Browser(tx))))
    })
  };
  let url = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
  assert!(url.starts_with("https://"), "{url}");
  // The page closed before its redirect came.
  stitch.cancel_sign_in();
  let e = signing.join().unwrap().unwrap_err();
  assert!(e.to_string().contains("cancelled"), "{e}");
  assert_eq!(
    wait(stitch.sign_in_status("grok".into())).unwrap(),
    "signed out"
  );
  wait(stitch.stop()).unwrap();
}
