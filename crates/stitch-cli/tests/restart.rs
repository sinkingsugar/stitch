//! `stitch node restart` and `stop --when-idle` on a real node process
//! (docs/self-improve.md §2.1): the node starts its successor itself, from
//! the binary it was started from, and says its build; a build that fails
//! or hangs at its start gives way to the healthy one; a stop asked for
//! during a build's trial is not a failure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct Node {
  data: PathBuf,
  exe: PathBuf,
  /// `STITCH_HEALTHY_AFTER_SECS` for the nodes its commands start.
  healthy_after: u64,
}

impl Node {
  /// A data directory and a copy of the binary of its own (what a restart
  /// starts again, kept in `<data>/bin`), set up as node `name`.
  fn new(name: &str) -> Node {
    // On disk beside the build, not in /tmp (a tmpfs on the CI box):
    // every node keeps a copy of its build.
    let dir =
      Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stitch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    let exe = dir.join("bin/stitch");
    std::fs::copy(env!("CARGO_BIN_EXE_stitch"), &exe).unwrap();
    let node = Node {
      data: dir.join("data"),
      exe,
      healthy_after: 2,
    };
    let setup = node.stitch(&["setup", "--name", name, "--local"]);
    assert!(setup.contains(&format!("node {name}")), "{setup}");
    // Apps not served: a test node never takes the fixed port 7177, which
    // a node running on the machine (the user's, a service) holds.
    let mut config = stitch_node::config::Config::load(&node.data).unwrap();
    config.apps = Some(stitch_node::config::AppsSettings { port: Some(0) });
    config.save(&node.data).unwrap();
    node
  }

  fn stitch(&self, args: &[&str]) -> String {
    // On Linux a binary just copied may be "busy" for a moment: another
    // test's fork can still hold the copy's write descriptor (ETXTBSY).
    let mut tries = 0;
    let out = loop {
      match Command::new(&self.exe)
        .arg("--data")
        .arg(&self.data)
        .args(args)
        .env_remove("STITCH_SUPERVISED")
        .env_remove("INVOCATION_ID")
        .env("STITCH_HEALTHY_AFTER_SECS", self.healthy_after.to_string())
        .env("STITCH_TRIAL_GRACE_SECS", "5")
        .output()
      {
        Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
          tries += 1;
          std::thread::sleep(Duration::from_millis(50));
        }
        other => break other.expect("stitch runs"),
      }
    };
    format!(
      "{}{}",
      String::from_utf8_lossy(&out.stdout),
      String::from_utf8_lossy(&out.stderr)
    )
  }

  fn log(&self) -> String {
    std::fs::read_to_string(self.data.join("node.log")).unwrap_or_default()
  }

  fn state(&self) -> String {
    std::fs::read_to_string(self.data.join("bin/state.json")).unwrap_or_default()
  }

  fn until(&self, what: &str, secs: u64, f: impl Fn(&Node) -> bool) {
    let start = Instant::now();
    while !f(self) {
      assert!(
        start.elapsed() < Duration::from_secs(secs),
        "{what}: not within {secs} s\n{}\n{}",
        self.log(),
        self.state()
      );
      std::thread::sleep(Duration::from_millis(200));
    }
  }

  /// Starts the node (any command does) and waits for its build to be
  /// healthy here.
  fn start_healthy(&self) {
    assert_eq!(
      self.stitch(&["jobs"]).trim(),
      "nothing runs in the background"
    );
    self.until("healthy", 30, |n| {
      n.log().contains("upgrade: build ") && n.log().contains(" healthy")
    });
  }

  /// Puts `bin` where the node's binary is, as an install does.
  fn install(&self, bin: &Path) {
    let tmp = self.exe.with_extension("new");
    std::fs::copy(bin, &tmp).unwrap();
    std::fs::rename(&tmp, &self.exe).unwrap();
  }
}

impl Drop for Node {
  fn drop(&mut self) {
    let _ = self.stitch(&["node", "stop"]);
    // A restart's old process may still watch its successor, and write
    // to the data directory: gone for good only once nothing comes back.
    let dir = self.data.parent().unwrap_or(Path::new("/nonexistent"));
    for _ in 0..30 {
      let _ = std::fs::remove_dir_all(dir);
      std::thread::sleep(Duration::from_secs(1));
      if !dir.exists() {
        break;
      }
    }
  }
}

/// Another build of the binary that runs as it does (its hash differs).
fn variant(of: &Path, at: &Path) {
  std::fs::copy(of, at).unwrap();
  if cfg!(target_os = "macos") {
    // Signed again (ad hoc) under another identifier: other bytes.
    let out = Command::new("codesign")
      .args(["-f", "-s", "-", "-i", "dev.stitch.test.variant"])
      .arg(at)
      .output()
      .unwrap();
    assert!(out.status.success(), "{out:?}");
  } else {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(at).unwrap();
    f.write_all(b"\0another build").unwrap();
  }
}

/// A "build" that says it is one and then does `then` (a shell line).
fn script(at: &Path, label: &str, then: &str) {
  std::fs::write(
    at,
    format!(
      "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 'stitch 0.1.0 ({label})'; exit 0; }}\n{then}\n"
    ),
  )
  .unwrap();
  use std::os::unix::fs::PermissionsExt;
  std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_node_restarts_itself_and_says_its_build() {
  let node = Node::new("restarttest");
  let version = node.stitch(&["--version"]);
  let build = version
    .trim()
    .split_once(" (")
    .unwrap()
    .1
    .trim_end_matches(')');
  assert!(!build.is_empty() && build != "unknown", "{version}");
  assert_eq!(
    node.stitch(&["jobs"]).trim(),
    "nothing runs in the background"
  );
  let status = node.stitch(&["node", "status"]);
  assert!(status.contains(&format!("build: {build}")), "{status}");

  let said = node.stitch(&["node", "restart"]);
  assert!(said.contains("the node restarts now (idle)"), "{said}");
  node.until("the successor started", 30, |n| {
    n.log().contains("): started ")
  });
  node.until("the successor serves", 30, |n| {
    n.log().matches("serving").count() == 2 && n.stitch(&["node", "status"]).contains("build: ")
  });
  // The successor took over: no client was left to start its own.
  assert!(!node.data.join("restarting").exists());

  let said = node.stitch(&["node", "stop", "--when-idle"]);
  assert!(said.contains("the node stops now (idle)"), "{said}");
  node.until("stopped", 30, |n| {
    n.stitch(&["node", "status"]).contains("no node runs")
  });
  // A stop starts nothing again.
  std::thread::sleep(Duration::from_secs(2));
  assert!(node.stitch(&["node", "status"]).contains("no node runs"));
  assert_eq!(node.log().matches("serving").count(), 2, "{}", node.log());
}

/// A build installed that does not run gives way to the healthy one: the
/// old process, watching its successor, installs it back and starts it.
#[test]
fn a_build_that_fails_at_its_start_is_rolled_back() {
  let node = Node::new("rollback");
  node.start_healthy();
  let bad = node.data.parent().unwrap().join("broken");
  script(&bad, "broken", "exit 3");
  node.install(&bad);

  node.until("rolled back", 60, |n| n.log().contains("rolling back to"));
  node.until("the healthy build serves again", 30, |n| {
    n.log().matches("serving").count() == 2 && n.stitch(&["node", "status"]).contains("build: ")
  });
  let version = node.stitch(&["--version"]);
  assert!(!version.contains("broken"), "{version}");
  let state = node.state();
  assert!(state.contains("\"failed\""), "{state}");
  assert!(state.contains("broken-"), "{state}");
}

/// A build that hangs before its node starts (never taking the lock) is
/// stopped at the end of its trial, and the healthy one comes back.
#[test]
fn a_build_that_hangs_at_its_start_is_rolled_back() {
  let node = Node::new("hang");
  node.start_healthy();
  let hangs = node.data.parent().unwrap().join("hangs");
  script(&hangs, "hangs", "sleep 600");
  node.install(&hangs);

  node.until("rolled back", 90, |n| {
    n.log().contains("did not start within its trial") && n.log().contains("rolling back to")
  });
  node.until("the healthy build serves again", 30, |n| {
    n.log().matches("serving").count() == 2
  });
  assert!(node.state().contains("hangs-"), "{}", node.state());
}

/// A build on trial stopped because someone asked (`node stop`, a
/// restart) has not failed: it is neither rolled back nor counted.
#[test]
fn a_stop_during_the_trial_is_not_a_failure() {
  let mut node = Node::new("trialstop");
  node.start_healthy();
  node.stitch(&["node", "stop"]);
  node.until("stopped", 30, |n| {
    n.stitch(&["node", "status"]).contains("no node runs")
  });
  // From now on, a trial long enough to stop the node during it.
  node.healthy_after = 120;
  assert_eq!(
    node.stitch(&["jobs"]).trim(),
    "nothing runs in the background"
  );
  let b = node.data.parent().unwrap().join("variant");
  variant(&node.exe, &b);
  node.install(&b);
  node.until("the new build serves, on trial", 60, |n| {
    n.log().contains(", on trial)")
  });
  let said = node.stitch(&["node", "stop"]);
  assert!(!said.contains("error"), "{said}");
  node.until("stopped", 30, |n| {
    n.stitch(&["node", "status"]).contains("no node runs")
  });
  // Its watcher (the old process) lets it be.
  std::thread::sleep(Duration::from_secs(3));
  assert!(!node.log().contains("rolling back"), "{}", node.log());
  let state = node.state();
  assert!(!state.contains("\"failed\": [\n    \""), "{state}");
  assert!(state.contains("\"attempts\": 0"), "{state}");
  // It starts again as itself, still on trial.
  assert_eq!(
    node.stitch(&["jobs"]).trim(),
    "nothing runs in the background"
  );
  assert_eq!(
    std::fs::read(&node.exe).unwrap(),
    std::fs::read(&b).unwrap(),
    "the build stopped during its trial was replaced"
  );
}

/// A build that hangs inside the node's start, holding the lock (here: its
/// config is a FIFO nobody writes), gives up at the end of its trial (its
/// watchdog runs from before the start), and the healthy one comes back.
#[test]
fn a_build_that_hangs_holding_the_lock_is_rolled_back() {
  let node = Node::new("hanglock");
  node.start_healthy();
  let config = node.data.join("config.toml");
  let text = std::fs::read_to_string(&config).unwrap();
  let b = node.data.parent().unwrap().join("variant");
  variant(&node.exe, &b);
  // From now on, whatever starts blocks reading its config.
  std::fs::remove_file(&config).unwrap();
  let made = Command::new("mkfifo").arg(&config).status().unwrap();
  assert!(made.success());
  node.install(&b);

  node.until("given up", 90, |n| {
    n.log().contains("giving up for the healthy build")
  });
  node.until("rolled back", 60, |n| {
    n.log().contains("restarting (rolled back to")
  });
  // The healthy build is back where the binary is; it hangs on the same
  // config until it is given one.
  assert!(
    std::fs::read(&node.exe).unwrap() != std::fs::read(&b).unwrap(),
    "the new build is still installed\n{}\n{}",
    node.log(),
    node.state()
  );
  std::fs::write(&config, &text).unwrap();
  std::fs::remove_file(&config).unwrap();
  std::fs::write(&config, &text).unwrap();
  node.until("the healthy build serves again", 30, |n| {
    n.log().matches("serving").count() == 2
  });
  let state = node.state();
  assert!(state.contains("\"failed\": [\n    \""), "{state}");
}

/// `node builds --healthy-for` counts the running process's time up, not
/// the build's first health: after a crash and a start again, the trial
/// begins again.
#[test]
#[allow(unsafe_code)]
fn healthy_for_starts_again_after_a_crash() {
  let node = Node::new("healthyfor");
  node.start_healthy();
  let healthy_for = |n: &Node, secs: &str| {
    let out = n.stitch(&["node", "builds", "--healthy-for", secs]);
    !out.contains("not healthy for that long")
  };
  node.until("healthy for 3 s", 30, |n| healthy_for(n, "3"));
  // A crash.
  let pid: i32 = std::fs::read_to_string(node.data.join("node.lock"))
    .unwrap()
    .trim()
    .parse()
    .unwrap();
  // SAFETY: a signal to the test's own node.
  unsafe { libc::kill(pid, libc::SIGKILL) };
  let crashed = Instant::now();
  node.until("gone", 30, |n| {
    n.stitch(&["node", "status"]).contains("no node runs")
  });
  assert!(
    !healthy_for(&node, "0"),
    "{}",
    node.stitch(&["node", "builds"])
  );
  // Started again: the same build, healthy still, but up for less than
  // the trial.
  assert_eq!(
    node.stitch(&["jobs"]).trim(),
    "nothing runs in the background"
  );
  node.until("up", 30, |n| {
    n.stitch(&["node", "builds"]).contains("running: ")
  });
  // The new process is younger than the time since the crash, however
  // slow a loaded machine was to start it; the build's first health is
  // older by the 3 s waited above, or more.
  let limit = (crashed.elapsed().as_secs() + 2).to_string();
  assert!(
    !healthy_for(&node, &limit),
    "{}",
    node.stitch(&["node", "builds"])
  );
  node.until("healthy for 3 s again", 30, |n| healthy_for(n, "3"));
}

/// Starts `stitch node` of `node` by hand, as its service would (`args`
/// after `node`), its output appended to `node.log`.
fn start_node_process(node: &Node, args: &[&str]) -> std::process::Child {
  let log = || {
    std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(node.data.join("node.log"))
      .unwrap()
  };
  let mut tries = 0;
  loop {
    match Command::new(&node.exe)
      .arg("--data")
      .arg(&node.data)
      .arg("node")
      .args(args)
      .env_remove("STITCH_SUPERVISED")
      .env_remove("INVOCATION_ID")
      .env("STITCH_HEALTHY_AFTER_SECS", node.healthy_after.to_string())
      .env("STITCH_TRIAL_GRACE_SECS", "5")
      .stdin(std::process::Stdio::null())
      .stdout(log())
      .stderr(log())
      .spawn()
    {
      Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
        tries += 1;
        std::thread::sleep(Duration::from_millis(50));
      }
      other => break other.expect("stitch runs"),
    }
  }
}

/// Stops the node and waits (a while) for a process started by hand to end.
fn reap(node: &Node, mut child: std::process::Child) {
  node.stitch(&["node", "stop"]);
  let start = Instant::now();
  while child.try_wait().ok().flatten().is_none() && start.elapsed() < Duration::from_secs(60) {
    std::thread::sleep(Duration::from_millis(200));
  }
}

/// A node whose binary was replaced by another build before it got the
/// lock (here it waits behind another node, as a service's did behind a
/// client's node: incident 2026-11-02, 9 hours on the old build) finds
/// the build installed by itself and restarts into it once idle.
#[test]
fn a_node_started_before_an_install_restarts_into_it() {
  let node = Node::new("replaced");
  node.start_healthy();
  let running = std::fs::read_to_string(node.data.join("bin/running")).unwrap();
  let first = running.split_whitespace().next().unwrap().to_owned();
  // The next node, waiting for the first to let go of the lock.
  let next = start_node_process(&node, &["--after", &first]);
  std::thread::sleep(Duration::from_secs(1));
  // Another build installed meanwhile.
  let b = node.data.parent().unwrap().join("variant");
  variant(&node.exe, &b);
  node.install(&b);
  // The first stops; the next serves, on its old build.
  let said = node.stitch(&["node", "stop"]);
  assert!(!said.contains("error"), "{said}");
  node.until("the next node serves", 60, |n| {
    n.log().matches("serving").count() == 2
  });
  node.until("it restarts into the build installed", 90, |n| {
    n.log()
      .contains(" installed is waiting: stopping once idle")
      && n.log().matches("serving").count() == 3
      && n.log().contains(", on trial)")
  });
  reap(&node, next);
}

/// A client that finds no node while the node's service may bring it up
/// waits for the service's node instead of starting one of its own (a
/// node outside the service held the lock and took the notice of a new
/// build meant for the service's: incident 2026-11-02).
#[test]
fn a_client_waits_for_the_service_instead_of_starting_a_node() {
  let node = Node::new("servicewait");
  // A service that cannot be started from here: what starts the node is
  // this test, a few seconds later, as a restarting service would.
  std::fs::write(
    node.data.join("service"),
    format!("systemd\nstitch-absent-test-{}\n", std::process::id()),
  )
  .unwrap();
  let client = {
    let (exe, data) = (node.exe.clone(), node.data.clone());
    std::thread::spawn(move || {
      Command::new(exe)
        .arg("--data")
        .arg(data)
        .arg("jobs")
        .env_remove("STITCH_SUPERVISED")
        .env_remove("INVOCATION_ID")
        .env("STITCH_SERVICE_WAIT_SECS", "60")
        .output()
        .unwrap()
    })
  };
  std::thread::sleep(Duration::from_secs(4));
  assert!(
    !node.log().contains("serving"),
    "the client started a node\n{}",
    node.log()
  );
  let service = start_node_process(&node, &[]);
  let out = client.join().unwrap();
  let said = String::from_utf8_lossy(&out.stdout);
  assert!(
    said.contains("nothing runs in the background"),
    "{said}{}\n{}",
    String::from_utf8_lossy(&out.stderr),
    node.log()
  );
  assert_eq!(node.log().matches("serving").count(), 1, "{}", node.log());
  assert!(!node.log().contains("starting a node outside it"));
  reap(&node, service);
}

/// A service that stays down past the window: the client starts a node
/// of its own, and says so in `node.log`.
#[test]
fn a_client_starts_a_node_when_the_service_stays_down() {
  let node = Node::new("servicedown");
  std::fs::write(
    node.data.join("service"),
    format!("systemd\nstitch-absent-test-{}\n", std::process::id()),
  )
  .unwrap();
  let out = Command::new(&node.exe)
    .arg("--data")
    .arg(&node.data)
    .arg("jobs")
    .env_remove("STITCH_SUPERVISED")
    .env_remove("INVOCATION_ID")
    .env("STITCH_SERVICE_WAIT_SECS", "2")
    .output()
    .unwrap();
  let said = String::from_utf8_lossy(&out.stdout);
  assert!(said.contains("nothing runs in the background"), "{said}");
  assert!(
    node
      .log()
      .contains("no node within 2 s: starting a node outside it"),
    "{}",
    node.log()
  );
}
