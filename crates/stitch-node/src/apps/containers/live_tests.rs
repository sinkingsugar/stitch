//! Escape tests against the real engine (design §2.23): a small image
//! with a probe (`tests/containers/probe.c`, built statically) is built
//! with the build's own arguments, and run with the run's own invocation
//! (`podman_args`, `execute`, `collect`); each probe must be blocked.
//! Skipped where there is no rootless podman with its controllers, or no
//! C compiler to build the probe.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::build::{L_NODE, build_args};
use super::engine::{self, Podman};
use super::run::{self, Outcome};
use super::spec::Command;

const ALPINE: &str = "docker.io/library/alpine@sha256:c64c687cbea9300178b30c95835354e34c4e4febc4badfe27102879de0483b5e";
const NODE: &str = "live-tests";

struct Env {
  podman: Podman,
  image: String,
  dir: tempfile::TempDir,
  seccomp: PathBuf,
}

/// The engine and the probe's image, or why the tests are skipped.
async fn env() -> Option<Env> {
  let (podman, _) = match engine::engine().await {
    Ok(e) => e,
    Err(why) => {
      eprintln!("skipped: {why}");
      return None;
    }
  };
  let dir = tempfile::Builder::new()
    .prefix("stitch-live")
    .tempdir()
    .unwrap();
  let ctx = dir.path().join("ctx");
  std::fs::create_dir_all(&ctx).unwrap();
  let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/containers/probe.c");
  let built = std::process::Command::new("cc")
    .args(["-static", "-O1", "-o"])
    .arg(ctx.join("probe"))
    .arg(&src)
    .status();
  if !built.is_ok_and(|s| s.success()) {
    eprintln!("skipped: no static C compiler for the probe");
    return None;
  }
  std::fs::write(
    ctx.join("Containerfile"),
    format!("FROM {ALPINE}\nCOPY probe /usr/local/bin/probe\n"),
  )
  .unwrap();
  let tag = "localhost/stitch-live-tests:probe";
  // Built once a run, the tests waiting for it: built by each, the tags
  // moved and left untagged images behind. Those of earlier runs pruned.
  static BUILT: tokio::sync::Mutex<bool> = tokio::sync::Mutex::const_new(false);
  let mut built = BUILT.lock().await;
  if !*built {
    let _ = podman
      .output(
        &[
          "image",
          "prune",
          "-f",
          "--filter",
          &format!("label={L_NODE}={NODE}"),
        ],
        Duration::from_secs(60),
      )
      .await;
    let args = build_args(&ctx, tag, &[format!("{L_NODE}={NODE}")], "512m", "none");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, _, err) = podman
      .output(&refs, Duration::from_secs(600))
      .await
      .unwrap();
    assert_eq!(code, 0, "the probe's image: {err}");
    *built = true;
  }
  drop(built);
  let seccomp = engine::write_profile(dir.path()).unwrap();
  Some(Env {
    podman,
    image: tag.into(),
    dir,
    seccomp,
  })
}

impl Env {
  /// Runs `argv` in a container as a command runs: its outcome, and its
  /// output folder (taken back by the node).
  async fn run(&self, argv: &[&str], tweak: impl FnOnce(&mut Command)) -> (Outcome, PathBuf) {
    let id = rand_suffix();
    let root = self.dir.path().join(&id);
    let (input, output) = (root.join("in"), root.join("out"));
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let out_s = output.display().to_string();
    self
      .podman
      .output(
        &["unshare", "chown", "1000:1000", &out_s],
        Duration::from_secs(30),
      )
      .await
      .unwrap();
    let mut cmd = Command {
      run: argv.iter().map(|s| s.to_string()).collect(),
      timeout: Some(60),
      memory_mb: Some(256),
      cpus: Some(1.0),
      pids: Some(64),
      max_files: Some(16),
      max_mb: Some(16),
      ..Default::default()
    };
    tweak(&mut cmd);
    let name = format!("stitch-live-{id}");
    let labels = vec![format!("{L_NODE}={NODE}"), format!("stitch.run={id}")];
    let args = run::podman_args(
      &cmd,
      &self.image,
      &cmd.run,
      &name,
      &labels,
      &input,
      &output,
      &self.seccomp,
      None,
      None,
    )
    .unwrap();
    let o = run::execute(&self.podman, &cmd, &args, &output)
      .await
      .unwrap();
    run::remove(&self.podman, &name, &id).await.unwrap();
    self
      .podman
      .output(
        &[
          "unshare",
          "chown",
          "-R",
          "-h",
          "0:0",
          &root.display().to_string(),
        ],
        Duration::from_secs(30),
      )
      .await
      .unwrap();
    (o, output)
  }
}

fn rand_suffix() -> String {
  blake3::hash(format!("{:?}", std::time::SystemTime::now()).as_bytes()).to_hex()[..6].to_owned()
}

fn reached(o: &Outcome) -> Vec<&str> {
  o.stdout
    .lines()
    .filter(|l| l.starts_with("REACHED"))
    .collect()
}

/// A listener on the host, for the probes to try.
fn listen(addr: &str) -> (std::net::TcpListener, u16) {
  let l = std::net::TcpListener::bind(addr).unwrap();
  let port = l.local_addr().unwrap().port();
  (l, port)
}

/// The host's own LAN address, when it has one.
fn lan_ip() -> Option<String> {
  let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
  s.connect("192.0.2.1:9").ok()?;
  Some(s.local_addr().ok()?.ip().to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn escapes_are_blocked() {
  let Some(env) = env().await else { return };
  let home = std::env::var("HOME").unwrap_or_default();
  let (_l1, local) = listen("127.0.0.1:0");
  let (_l2, any) = listen("0.0.0.0:0");
  let uid = std::process::Command::new("id").arg("-u").output().unwrap();
  let uid = String::from_utf8_lossy(&uid.stdout).trim().to_owned();
  let mut probes: Vec<String> = vec![
    "id".into(),
    // The node's socket, D-Bus, ssh-agent, Docker's, Wayland, X11.
    format!("unix:{home}/.stitch/node.sock"),
    format!("unix:/run/user/{uid}/bus"),
    "unix:/var/run/docker.sock".into(),
    format!("unix:/run/user/{uid}/wayland-1"),
    "unix:/tmp/.X11-unix/X0".into(),
    "abstract:/tmp/.X11-unix/X0".into(),
    "abstract:/tmp/dbus-session".into(),
    // localhost, the host's LAN address, the internet.
    format!("tcp:127.0.0.1:{local}"),
    format!("tcp:127.0.0.1:{any}"),
    format!("tcp:169.254.1.2:{any}"),
    "tcp:1.1.1.1:443".into(),
    "tcp:8.8.8.8:53".into(),
    // The user's files.
    format!("read:{home}/.stitch/config.toml"),
    format!("read:{home}/.ssh/id_ed25519"),
    format!("read:{home}/.ssh/known_hosts"),
    // Writes outside the output folder.
    "write:/etc/evil".into(),
    "write:/usr/local/bin/evil".into(),
    "write:/in/evil".into(),
    // Running what it wrote.
    "write:/out/run.sh".into(),
    "exec:/out/run.sh".into(),
    // Nested namespaces, io_uring, mounts, keyrings, tracing.
    "userns".into(),
    "netns".into(),
    "clone-userns".into(),
    "io_uring".into(),
    "mount".into(),
    "keyctl".into(),
    "ptrace".into(),
  ];
  if let Some(ip) = lan_ip() {
    probes.push(format!("tcp:{ip}:{any}"));
  }
  if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
    probes.push(format!("unix:{sock}"));
  }
  let mut argv = vec!["/usr/local/bin/probe"];
  argv.extend(probes.iter().map(String::as_str));
  let (o, out) = env.run(&argv, |_| {}).await;
  eprintln!("{}\n{}", o.stdout, o.stderr);
  assert!(o.stdout.contains("UID 1000 GID 1000"), "{}", o.stdout);
  // Only its own output folder: written there, and nothing else.
  let reached = reached(&o);
  assert_eq!(reached, vec!["REACHED write:/out/run.sh"], "{}", o.stdout);
  let blocked = o
    .stdout
    .lines()
    .filter(|l| l.starts_with("BLOCKED"))
    .count();
  assert_eq!(blocked, probes.len() - 2, "{}", o.stdout);
  let (files, _) = run::collect(&out, 16, 1 << 20);
  assert_eq!(files.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn links_and_fifos_in_out_are_not_followed() {
  let Some(env) = env().await else { return };
  let (o, out) = env
    .run(
      &[
        "/usr/local/bin/probe",
        "symlink",
        "fifo",
        "write:/out/ok.txt",
      ],
      |_| {},
    )
    .await;
  assert!(o.stdout.contains("REACHED symlink in out"), "{}", o.stdout);
  let (files, skipped) = run::collect(&out, 16, 1 << 20);
  let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
  assert_eq!(names, vec!["ok.txt"]);
  let said = skipped.join("\n");
  assert!(
    said.contains("leak: a link") && said.contains("pipe: not a regular file"),
    "{said}"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timeout_kills_the_whole_container() {
  let Some(env) = env().await else { return };
  let marker = format!(
    "sleep 4{}",
    rand_suffix()[..3]
      .bytes()
      .map(|b| (b % 10).to_string())
      .collect::<String>()
  );
  let line = format!("setsid sh -c '{marker}' & {marker}");
  let started = std::time::Instant::now();
  let (o, _) = env
    .run(&["/bin/sh", "-c", &line], |c| c.timeout = Some(3))
    .await;
  assert!(
    o.stopped.as_deref().unwrap_or("").contains("were up"),
    "{o:?}"
  );
  assert!(started.elapsed() < Duration::from_secs(40));
  // No process of it is left on the host, the setsid grandchild included.
  let ps = std::process::Command::new("ps")
    .args(["-eo", "args"])
    .output()
    .unwrap();
  let left: Vec<String> = String::from_utf8_lossy(&ps.stdout)
    .lines()
    .filter(|l| l.contains(&marker) && !l.contains("sh -c"))
    .map(str::to_owned)
    .collect();
  assert!(left.is_empty(), "left running: {left:?}");
  let (_, ps, _) = env
    .podman
    .output(
      &[
        "ps",
        "-a",
        "-q",
        "--filter",
        &format!("label={L_NODE}={NODE}"),
      ],
      Duration::from_secs(30),
    )
    .await
    .unwrap();
  assert!(ps.trim().is_empty(), "containers left: {ps}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_memory_cap_holds() {
  let Some(env) = env().await else { return };
  let (o, _) = env
    .run(&["/usr/local/bin/probe", "alloc:400"], |c| {
      c.memory_mb = Some(128)
    })
    .await;
  assert!(!o.stdout.contains("REACHED alloc"), "{}", o.stdout);
  assert_ne!(o.exit, Some(0), "{o:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn outputs_past_their_cap_stop_the_run() {
  let Some(env) = env().await else { return };
  let (o, _) = env
    .run(
      &["/bin/sh", "-c", "mkdir /out/d && chmod 700 /out/d && dd if=/dev/zero of=/out/d/big bs=1M count=200; sleep 30"],
      |c| c.max_mb = Some(16),
    )
    .await;
  assert!(
    o.stopped
      .as_deref()
      .unwrap_or("")
      .contains("passed their caps"),
    "{o:?}"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_build_reaches_the_internet_not_the_host() {
  let Some(env) = env().await else { return };
  let (_l1, local) = listen("127.0.0.1:0");
  let (_l2, any) = listen("0.0.0.0:0");
  let lan = lan_ip().unwrap_or_else(|| "127.0.0.1".into());
  let ctx = env.dir.path().join("netctx");
  std::fs::create_dir_all(&ctx).unwrap();
  let targets = [
    format!("127.0.0.1:{local}"),
    format!("169.254.1.2:{local}"),
    format!("169.254.1.2:{any}"),
    format!("host.containers.internal:{any}"),
    format!("{lan}:{any}"),
  ];
  let mut run = String::from("RUN ");
  for t in &targets {
    run.push_str(&format!(
      "(wget -q -T 3 -O /dev/null http://{t}/ && echo REACHED-{t} || echo BLOCKED-{t}); "
    ));
  }
  run.push_str("(wget -q -T 10 -O /dev/null http://example.com/ && echo REACHED-internet || echo BLOCKED-internet)\n");
  std::fs::write(ctx.join("Containerfile"), format!("FROM {ALPINE}\n{run}")).unwrap();
  // Something answers on the host's ports, so a connection would succeed.
  for l in [&_l1, &_l2] {
    let l = l.try_clone().unwrap();
    std::thread::spawn(move || {
      for c in l.incoming().flatten() {
        use std::io::Write;
        let mut c = c;
        let _ = c.write_all(b"HTTP/1.0 200 OK\r\ncontent-length: 2\r\n\r\nok");
      }
    });
  }
  let args = build_args(
    &ctx,
    "localhost/stitch-live-tests:net",
    &[format!("{L_NODE}={NODE}")],
    "256m",
    super::build::BUILD_NETWORK,
  );
  let mut args = args;
  args.insert(1, "--no-cache".into());
  let refs: Vec<&str> = args.iter().map(String::as_str).collect();
  let (code, out, err) = env
    .podman
    .output(&refs, Duration::from_secs(300))
    .await
    .unwrap();
  let log = format!("{out}\n{err}");
  eprintln!("{log}");
  assert_eq!(code, 0, "{log}");
  for t in &targets {
    let blocked = format!("BLOCKED-{t}");
    assert!(
      log.lines().any(|l| l.trim() == blocked),
      "{t} reached: {log}"
    );
  }
  // The internet, when this machine has it.
  if !log.lines().any(|l| l.trim() == "REACHED-internet") {
    eprintln!("note: the internet was not reached from the build (offline?)");
  }
  let _ = env
    .podman
    .output(
      &["rmi", "-f", "localhost/stitch-live-tests:net"],
      Duration::from_secs(60),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gpu_command_sees_the_gpu_through_stitchs_cdi_spec() {
  if !Path::new("/usr/bin/nvidia-ctk").exists() {
    eprintln!("skipped: no NVIDIA Container Toolkit");
    return;
  }
  let Some(env) = env().await else { return };
  let dir = run::cdi(env.dir.path().join("cdi"))
    .await
    .expect("the CDI spec");
  let spec: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string(dir.join("nvidia.json")).unwrap()).unwrap();
  let hooks = engine::check_cdi(&spec).unwrap();
  eprintln!("hooks: {hooks:?}");
  assert!(
    hooks
      .iter()
      .all(|h| engine::CDI_HOOKS.contains(&h.as_str()))
  );
  let id = rand_suffix();
  let root = env.dir.path().join(&id);
  let (input, output) = (root.join("in"), root.join("out"));
  std::fs::create_dir_all(&input).unwrap();
  std::fs::create_dir_all(&output).unwrap();
  let cmd = Command {
    // The probe image is musl: the driver's (glibc) tools do not run in
    // it; its devices and libraries are what CDI gives.
    run: vec![
      "/bin/sh".into(),
      "-c".into(),
      "ls /dev/nvidiactl /dev/nvidia0 /usr/bin/nvidia-smi && echo GPU 0 present".into(),
    ],
    timeout: Some(60),
    memory_mb: Some(512),
    gpu: true,
    ..Default::default()
  };
  let name = format!("stitch-live-{id}");
  let labels = vec![format!("{L_NODE}={NODE}"), format!("stitch.run={id}")];
  let args = run::podman_args(
    &cmd,
    &env.image,
    &cmd.run,
    &name,
    &labels,
    &input,
    &output,
    &env.seccomp,
    None,
    Some(&dir),
  )
  .unwrap();
  let o = run::execute(&env.podman, &cmd, &args, &output)
    .await
    .unwrap();
  run::remove(&env.podman, &name, &id).await.unwrap();
  eprintln!("{}\n{}", o.stdout, o.stderr);
  assert_eq!(o.exit, Some(0), "{o:?}");
  assert!(o.stdout.contains("GPU 0"), "{}", o.stdout);
}
