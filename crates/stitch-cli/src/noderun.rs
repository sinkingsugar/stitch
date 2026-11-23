//! `stitch node`: the node's process (docs/self-improve.md §2.1, steps
//! 2–6). It says whether a service runs it, takes the data directory's
//! lock, keeps its build, decides from what is known whether to run or to
//! give way to the healthy build, proves itself healthy (or gives up when
//! on trial), watches its binary for another build, and once the node
//! stops, starts it again when asked (exiting for its service, starting
//! its successor, or starting the service).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;

use crate::upgrade::{self, Builds, Id};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The exit status that asks a supervisor (the systemd unit, the
/// LaunchAgent: `stitch node service install`) to start the node again.
pub const EXIT_RESTART: i32 = 75;

/// The exit status of a build on trial that never got healthy (it hangs
/// at its start): an error, so its service or its watcher puts the healthy
/// build back.
pub const EXIT_UNHEALTHY: i32 = 70;

/// How often the node looks at its binary.
const WATCH_SECS: u64 = 10;

/// What runs this process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Supervisor {
  /// A service runs it and starts it again when it exits with
  /// `EXIT_RESTART` (or an error).
  pub supervised: bool,
  /// The systemd unit (`stitch.service`) or launchd label it runs in.
  pub unit: Option<String>,
}

static SUPERVISOR: OnceLock<Supervisor> = OnceLock::new();

/// The supervisor this process found at its start.
pub fn supervisor() -> Supervisor {
  SUPERVISOR.get().cloned().unwrap_or_default()
}

/// Finds out once, at the node's start, what runs this process, then
/// removes what said it from the environment, so neither the node's
/// shells nor a node one of them starts take it for theirs.
#[allow(unsafe_code)]
pub fn detect() -> Supervisor {
  let said = std::env::var("STITCH_SUPERVISED").is_ok_and(|v| v == "1");
  let found = detect_with(said);
  // SAFETY: called at the node's start, before its runtime or any other
  // thread exists (`run`'s first step).
  unsafe {
    std::env::remove_var("STITCH_SUPERVISED");
    std::env::remove_var("INVOCATION_ID");
  }
  let _ = SUPERVISOR.set(found.clone());
  found
}

#[cfg(target_os = "linux")]
fn detect_with(said: bool) -> Supervisor {
  let parent = std::os::unix::process::parent_id();
  let systemd =
    std::fs::read_to_string(format!("/proc/{parent}/comm")).is_ok_and(|c| c.trim() == "systemd");
  let unit = std::fs::read_to_string("/proc/self/cgroup")
    .ok()
    .and_then(|c| {
      c.lines()
        .filter_map(|l| l.trim_end().rsplit('/').next())
        .find(|u| u.ends_with(".service"))
        .map(str::to_owned)
    });
  let invocation = std::env::var_os("INVOCATION_ID").is_some();
  let Some(unit) = unit.filter(|_| systemd && invocation) else {
    return Supervisor::default();
  };
  // The unit's main process is this one: its own node, whatever the unit
  // says of itself (geek's from before STITCH_SUPERVISED says nothing).
  let main = systemd_main_pid(&unit);
  let supervised = match main {
    Some(pid) => pid == std::process::id(),
    // No user bus to ask: the unit's word, or a unit of Stitch's own.
    None => said || unit.starts_with("stitch"),
  };
  Supervisor {
    supervised,
    unit: Some(unit),
  }
}

#[cfg(not(target_os = "linux"))]
fn detect_with(said: bool) -> Supervisor {
  // launchd starts its jobs with XPC_SERVICE_NAME, their label.
  let label = std::env::var("XPC_SERVICE_NAME")
    .ok()
    .filter(|l| !l.is_empty() && l != "0");
  let launchd = std::os::unix::process::parent_id() == 1;
  Supervisor {
    supervised: said && launchd && label.is_some(),
    unit: label.filter(|_| launchd),
  }
}

/// The main process of a systemd user unit, if it runs and the user's
/// manager can be asked.
pub fn systemd_main_pid(unit: &str) -> Option<u32> {
  let out = std::process::Command::new("systemctl")
    .args(["--user", "show", "-p", "MainPID", "--value", unit])
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()?;
  if !out.status.success() {
    return None;
  }
  String::from_utf8_lossy(&out.stdout)
    .trim()
    .parse()
    .ok()
    .filter(|p| *p != 0)
}

/// The process of a launchd job of the user's GUI session, if it runs.
pub fn launchd_pid(data: &Path, label: &str) -> Option<u32> {
  let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(data).ok()?);
  let out = std::process::Command::new("launchctl")
    .args(["print", &format!("gui/{uid}/{label}")])
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()?;
  if !out.status.success() {
    return None;
  }
  String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
    l.trim()
      .strip_prefix("pid = ")
      .and_then(|p| p.trim().parse().ok())
  })
}

/// Whether `sup` is the service `stitch node service install` set up for
/// `data` (`<data>/service`), or there is none.
fn is_the_service(sup: &Supervisor, service: Option<&(String, String)>) -> bool {
  match service {
    None => true,
    Some((kind, name)) => {
      let unit = sup.unit.as_deref().unwrap_or("");
      if kind == "systemd" {
        unit == format!("{name}.service")
      } else {
        unit == name
      }
    }
  }
}

/// What restarting does, for the process's supervisor and the service set
/// up for its data directory.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
  /// Exit with `EXIT_RESTART`: the supervisor starts the node again.
  ExitForSupervisor,
  /// Start the service (the node is not its own) and end.
  StartService { kind: String, name: String },
  /// Start the successor here and watch it.
  Successor,
}

pub fn next(sup: &Supervisor, service: Option<(String, String)>) -> Next {
  let ours = is_the_service(sup, service.as_ref());
  match service {
    _ if sup.supervised && ours => Next::ExitForSupervisor,
    Some((kind, name)) if !ours => Next::StartService { kind, name },
    // A service of this data directory runs no node yet (the node runs
    // outside it, unsupervised): the service starts it.
    Some((kind, name)) if !sup.supervised => Next::StartService { kind, name },
    _ => Next::Successor,
  }
}

/// The marker a node stopping on request leaves (its pid): its watcher
/// then takes its end for a stop, not a failure.
fn stopping_marker(data: &Path) -> PathBuf {
  data.join("stopping")
}

fn stopped_on_request(data: &Path, pid: u32) -> bool {
  std::fs::read_to_string(stopping_marker(data)).is_ok_and(|t| t.trim() == pid.to_string())
}

/// Leaves `<data>/restarting` for clients (the successor removes it).
fn mark_restarting(data: &Path) {
  let _ = std::fs::write(
    stitch_node::restarting_marker(data),
    std::process::id().to_string(),
  );
}

/// The build a `stitch` binary says it is (`--version`), if it runs.
pub fn build_of(exe: &Path) -> Option<String> {
  let mut tries = 0;
  let out = loop {
    match std::process::Command::new(exe)
      .arg("--version")
      .stdin(std::process::Stdio::null())
      .output()
    {
      // Just installed, still "busy" for a moment (Linux).
      Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
        tries += 1;
        std::thread::sleep(Duration::from_millis(50));
      }
      other => break other.ok()?,
    }
  };
  let text = String::from_utf8_lossy(&out.stdout);
  let line = text.lines().next()?.trim();
  if !out.status.success() || !line.starts_with("stitch ") {
    return None;
  }
  let open = line.find('(')?;
  Some(line[open + 1..].trim_end_matches(')').to_owned())
}

/// The build installed at `exe`: its hash, and what it says it is
/// (`unrunnable` when it does not run).
fn installed(exe: &Path) -> Option<Id> {
  let hash = upgrade::hash_file(exe).ok()?;
  let label = build_of(exe).unwrap_or_else(|| "unrunnable".into());
  Some(Id { hash, label })
}

type FileId = (u64, u64, std::time::SystemTime, u64);

/// What identifies the file at `p` without reading it: replaced or
/// rewritten, it changes.
fn file_id(p: &Path) -> Option<FileId> {
  use std::os::unix::fs::MetadataExt;
  let m = std::fs::metadata(p).ok()?;
  Some((m.dev(), m.ino(), m.modified().ok()?, m.len()))
}

/// The binary and its helper, as files.
fn files_id(exe: &Path) -> Option<(FileId, Option<FileId>)> {
  Some((
    file_id(exe)?,
    exe
      .parent()
      .and_then(|d| file_id(&d.join("stitch-shell-helper"))),
  ))
}

/// Runs the node of `data` (`stitch node`); `after`: started by the node
/// of that process, restarting, whose lock it waits for.
pub fn run(data: &Path, after: Option<u32>) -> Result<()> {
  let sup = detect();
  stitch_node::set_supervision(stitch_node::Supervision {
    supervised: sup.supervised,
  });
  stitch_node::logstamp::install();
  // The lock first, before anything takes time (hashing and keeping the
  // build): no client starts a node of its own meanwhile.
  let lock = take_lock(data, &sup, after)?;
  // The binary as installed now (what a restart starts, and what is
  // watched for another build), and the image this process runs (another
  // build may have been installed over it since it started).
  let exe = upgrade::installed_exe()?;
  let builds = Builds::new(data);
  let image = match upgrade::own_image(builds.dir()) {
    Ok(i) => i,
    Err(e) => {
      eprintln!("upgrade: {e}: the build installed taken for this one");
      upgrade::Image::open(&exe)?
    }
  };
  let id = Id {
    hash: image.hash()?,
    label: stitch_node::build().to_owned(),
  };
  let me = id.slug();
  if !upgrade::is_build_output(&exe)
    && let Err(e) = builds.keep(&id, &image, &exe, stitch_node::STORE_FORMAT)
  {
    eprintln!("builds: {me} not kept: {e}");
  }
  drop(image);
  let mut state = builds.state();
  // A stop asked for before this process could say so (SIGTERM while it
  // started): that start of the build does not count against it.
  if early_term::stopped_last(data).as_deref() == Some(me.as_str()) {
    upgrade::stopped_cleanly(&mut state, &me);
  }
  let decided = upgrade::starting(&mut state, &me, stitch_node::STORE_FORMAT, |b| {
    builds.kept_format(b)
  });
  builds.save(&state)?;
  builds.prune(&state);
  if let upgrade::Start::RollBack { to, why } = decided {
    eprintln!("{why}: rolling back to {to}");
    match builds.install(&to, &exe) {
      Ok(()) => {
        mark_restarting(data);
        drop(lock);
        return restart_node(data, &exe, &format!("rolled back to {to}"));
      }
      // Never a loop of failed rollbacks: this build runs instead.
      Err(e) => eprintln!("upgrade: {to} not installed ({e}): running {me}"),
    }
  }
  if let Some(e) = &state.event {
    eprintln!("upgrade: {e}");
  }
  let on_trial = state.trying.as_deref() == Some(me.as_str());
  // Which build this process is, for `node builds --healthy-for` (its
  // trial counts this process's time up, not the build's first health).
  let _ = std::fs::write(
    data.join("bin").join("running"),
    format!("{} {me}\n", std::process::id()),
  );
  let _term = early_term::Guard::install(data, &me);
  // The trial's watchdog runs from now on, before the node's start: a
  // build that hangs there (its store, a migration, its socket) while
  // holding the lock gives up too.
  let done = Arc::new(AtomicBool::new(false));
  {
    let (data, me, done) = (data.to_owned(), me.clone(), done.clone());
    std::thread::spawn(move || {
      if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
      {
        rt.block_on(prove_healthy(data, me, on_trial, done));
      }
    });
  }
  // A newer client is handed over to by a restart only into another build
  // installed here (`stitch_node::set_installed_check`).
  {
    let (exe, own, data) = (exe.clone(), id.hash.clone(), data.to_owned());
    let at_start = files_id(&exe);
    stitch_node::set_installed_check(move || {
      if files_id(&exe) == at_start {
        return false;
      }
      installed(&exe).is_some_and(|i| {
        i.hash != own
          && i.label != "unrunnable"
          && !Builds::new(&data).state().failed.contains(&i.slug())
      })
    });
  }
  let restart = crate::runtime()?.block_on(async {
    // The node installs its own SIGTERM handling as it starts.
    _term.remove();
    let node = stitch_node::start_locked(data, lock).await?;
    let _ = std::fs::remove_file(stitch_node::restarting_marker(data));
    let socket = node.socket.as_deref().unwrap_or(Path::new("no socket"));
    println!(
      "node {} serving {} (build {}, {}{})",
      node.endpoint,
      socket.display(),
      stitch_node::build(),
      me,
      if on_trial { ", on trial" } else { "" }
    );
    let stop = node.stopper();
    tokio::spawn(async move {
      let _ = tokio::signal::ctrl_c().await;
      let _ = stop.send(true);
    });
    // A stop it was asked for, said before it ends: its watcher (a
    // restart's old process) takes its end for a stop, whatever its status.
    let mut stopping = node.stopper().subscribe();
    let marker = stopping_marker(data);
    let stopped = done.clone();
    tokio::spawn(async move {
      if stopping.wait_for(|s| *s).await.is_ok() {
        stopped.store(true, Ordering::SeqCst);
        let _ = std::fs::write(&marker, std::process::id().to_string());
      }
    });
    let watch = watch_build(exe.clone(), &node, data.to_owned(), id.hash.clone());
    let restart = node.wait_restart().await;
    done.store(true, Ordering::SeqCst);
    watch.abort();
    Ok::<_, Box<dyn std::error::Error>>(restart?)
  });
  done.store(true, Ordering::SeqCst);
  let restart = restart?;
  // It stopped because it was asked to: that start does not count against
  // a build on trial.
  let mut state = builds.state();
  upgrade::stopped_cleanly(&mut state, &me);
  builds.save(&state)?;
  if let Some(r) = restart {
    restart_node(data, &exe, &r.why)?;
  }
  Ok(())
}

/// The data directory's lock: waited for under a service (a node outside
/// it stops for it to take over) and by a restart's successor (while the
/// old process, `after`, lets go of it: 60 s at most); otherwise taken at
/// once or refused.
fn take_lock(data: &Path, sup: &Supervisor, after: Option<u32>) -> Result<stitch_node::NodeLock> {
  if let Some(pid) = after {
    let start = Instant::now();
    loop {
      match stitch_node::node_pid(data) {
        // Another node took over meanwhile (a client's): it serves.
        Some(p) if p != pid => {
          return Err(format!("node {p} runs (not {pid}, which this one was to follow)").into());
        }
        Some(_) if start.elapsed() < Duration::from_secs(60) => {
          std::thread::sleep(Duration::from_millis(200));
        }
        _ => break,
      }
    }
    return Ok(stitch_node::lock_wait(data, Some(Duration::ZERO))?);
  }
  if sup.supervised {
    if let Some(pid) = stitch_node::node_pid(data) {
      // A node outside the service: asked once to stop when idle (as
      // `service install` does), so the service's takes over without
      // cutting its work, rather than waiting for ever.
      let asked = crate::runtime().ok().and_then(|rt| {
        rt.block_on(async {
          let mut c = Client::connect(data).await.ok()?;
          c.request(&Request::Restart {
            now: false,
            successor: false,
          })
          .await
          .ok()
        })
      });
      match asked {
        Some(_) => eprintln!("node {pid} runs outside the service: asked to stop once idle"),
        None => eprintln!("node {pid} runs and could not be asked to stop: waiting for it"),
      }
    }
    return Ok(stitch_node::lock_wait(data, None)?);
  }
  Ok(stitch_node::lock_wait(data, Some(Duration::ZERO))?)
}

/// The node stopped for a restart: under its service, exit for it to
/// start the node again; under none (or another job's) with a service set
/// up, start that; otherwise start the successor here, in its own process
/// group, its output appended to `node.log`, and watch it.
pub fn restart_node(data: &Path, exe: &Path, why: &str) -> Result<()> {
  let sup = supervisor();
  match next(&sup, stitch_node::client::service_of(data)) {
    Next::ExitForSupervisor => {
      eprintln!("restarting ({why}): exiting for the service");
      std::process::exit(EXIT_RESTART);
    }
    Next::StartService { kind, name } => {
      match stitch_node::client::start_service(data, &kind, &name) {
        Ok(()) => {
          eprintln!("restarting ({why}): started the service ({kind} {name})");
          return Ok(());
        }
        Err(e) => eprintln!("restarting ({why}): {e}; starting the node here"),
      }
    }
    Next::Successor => {}
  }
  let succ = installed(exe);
  let log_path = data.join("node.log");
  let log = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&log_path)?;
  let err = log.try_clone()?;
  let mut cmd = std::process::Command::new(exe);
  cmd
    .arg("--data")
    .arg(data)
    .arg("node")
    .arg("--after")
    .arg(std::process::id().to_string())
    .stdin(std::process::Stdio::null())
    .stdout(log)
    .stderr(err)
    .env_remove("STITCH_SUPERVISED")
    .env_remove("INVOCATION_ID");
  std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
  // A binary just installed may be "busy" for a moment on Linux (a fork
  // elsewhere still holding the descriptor it was written through).
  let mut tries = 0;
  let child = loop {
    match cmd.spawn() {
      Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
        tries += 1;
        std::thread::sleep(Duration::from_millis(50));
      }
      other => break other?,
    }
  };
  eprintln!(
    "restarting ({why}): started {} (pid {})",
    exe.display(),
    child.id()
  );
  watch_successor(data, exe, child, succ)
}

/// The old process stays a while as its successor's watcher: a new build
/// whose node ends with an error (or a crash) before it is healthy gives
/// way to the healthy build, installed back and started again. One that
/// ends because it was asked to stop (status 0, or its `stopping` marker)
/// is let be.
fn watch_successor(
  data: &Path,
  exe: &Path,
  mut child: std::process::Child,
  succ: Option<Id>,
) -> Result<()> {
  let builds = Builds::new(data);
  let succ = succ.map(|i| i.slug());
  let healthy_at_start = builds.state().healthy;
  let new = succ.is_some() && succ != healthy_at_start;
  // Until it takes the lock: a successor that never gets that far is
  // stopped then. Once it holds it, as long as it takes: on trial, it
  // gives up by itself (`prove_healthy`, exit 70) when it hangs.
  let deadline =
    Instant::now() + upgrade::healthy_after() + upgrade::trial_grace() + Duration::from_secs(15);
  loop {
    std::thread::sleep(Duration::from_millis(500));
    let Some(status) = child.try_wait()? else {
      let holds = stitch_node::node_pid(data) == Some(child.id());
      // Not new here (the healthy build again): serving is enough. New:
      // until it is healthy.
      if (!new && holds) || (new && builds.state().healthy == succ) {
        return Ok(());
      }
      if holds || Instant::now() < deadline {
        continue;
      }
      // Still there, never healthy, and not even holding the lock (it
      // hangs before the node's start): stopped, and the healthy build
      // comes back.
      let Some(trying) = succ.clone().filter(|_| new) else {
        return Ok(());
      };
      let _ = child.kill();
      let _ = child.wait();
      return roll_back(
        data,
        exe,
        &builds,
        &trying,
        format!("build {trying} did not start within its trial"),
      );
    };
    if status.success() || stopped_on_request(data, child.id()) || early_term::stopped(data) {
      eprintln!("restarting: the successor stopped ({status})");
      return Ok(());
    }
    // Another node runs (a client started one): not the successor's
    // business any more.
    if stitch_node::running(data) {
      return Ok(());
    }
    let Some(trying) = succ.clone() else {
      eprintln!(
        "restarting: the node ended at its start ({status}), and its binary cannot be read"
      );
      return Ok(());
    };
    if !new {
      eprintln!(
        "restarting: the node ended at its start ({status}); it is the healthy build: nothing to roll back to"
      );
      return Ok(());
    }
    return roll_back(
      data,
      exe,
      &builds,
      &trying,
      format!("build {trying} ended before it was healthy ({status})"),
    );
  }
}

/// Puts the healthy build back in place of `trying` and starts it (when
/// the store's format allows; else says so).
fn roll_back(data: &Path, exe: &Path, builds: &Builds, trying: &str, why: String) -> Result<()> {
  let mut state = builds.state();
  let format = builds.format(trying).unwrap_or(stitch_node::STORE_FORMAT);
  let decided = upgrade::rollback_to(&mut state, trying, format, |b| builds.kept_format(b), why);
  builds.save(&state)?;
  match decided {
    upgrade::Start::RollBack { to, why } => {
      eprintln!("{why}: rolling back to {to}");
      if let Err(e) = builds.install(&to, exe) {
        eprintln!("upgrade: {to} not installed ({e}): nothing rolled back");
        return Ok(());
      }
      mark_restarting(data);
      restart_node(data, exe, &format!("rolled back to {to}"))
    }
    upgrade::Start::Run => {
      if let Some(e) = &state.event {
        eprintln!("upgrade: {e}");
      }
      Ok(())
    }
  }
}

/// After `healthy_after()` running, and answering on its socket, the build
/// counts as healthy here; what the user is to be told (a rollback) is
/// sent as a notice then. A build on trial that does not answer within
/// `trial_grace()` more gives up (exits with an error), so its service or
/// watcher puts the healthy build back.
async fn prove_healthy(data: PathBuf, me: String, on_trial: bool, done: Arc<AtomicBool>) {
  let started = Instant::now();
  tokio::time::sleep(upgrade::healthy_after()).await;
  let give_up = upgrade::healthy_after() + upgrade::trial_grace();
  let mut said = false;
  loop {
    // The node stopped (asked to): nothing to prove or give up on.
    if done.load(Ordering::SeqCst) {
      return;
    }
    let answers = match tokio::time::timeout(Duration::from_secs(10), Client::connect(&data)).await
    {
      Ok(Ok(mut c)) => matches!(
        tokio::time::timeout(Duration::from_secs(10), c.request(&Request::Status)).await,
        Ok(Ok(Reply::Status { .. }))
      ),
      _ => false,
    };
    if answers {
      break;
    }
    if on_trial && started.elapsed() >= give_up && !done.load(Ordering::SeqCst) {
      eprintln!(
        "upgrade: build {me}, on trial, does not answer on its socket after {} s: giving up for the healthy build",
        started.elapsed().as_secs()
      );
      std::process::exit(EXIT_UNHEALTHY);
    }
    if !said {
      eprintln!("upgrade: the node does not answer on its socket yet: build {me} not healthy yet");
      said = true;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
  }
  if done.load(Ordering::SeqCst) {
    return;
  }
  let builds = Builds::new(&data);
  let mut state = builds.state();
  let new = state.healthy.as_deref() != Some(me.as_str());
  upgrade::healthy(&mut state, &me);
  let event = state.event.take();
  if let Err(e) = builds.save(&state) {
    eprintln!("upgrade: {e}");
    return;
  }
  builds.prune(&state);
  if new {
    eprintln!("upgrade: build {me} healthy");
  }
  if let Some(body) = event
    && let Ok(mut c) = Client::connect(&data).await
  {
    eprintln!("upgrade: telling the user: {body}");
    let _ = c
      .request(&Request::Notify {
        title: Some("Stitch node".into()),
        body,
        thread: None,
      })
      .await;
  }
}

/// Watches the node's binary: once another build is installed there (the
/// binary and its helper unchanged over two looks, so not half-written; a
/// binary of another hash that runs and did not fail here before), the
/// node restarts into it once idle (docs/self-improve.md §2.1, step 2).
fn watch_build(
  exe: PathBuf,
  node: &stitch_node::NodeHandle,
  data: PathBuf,
  own: String,
) -> tokio::task::JoinHandle<()> {
  let restart = node.restart_when_idle_fn();
  tokio::spawn(async move {
    // Nothing known at first: the build installed now is compared with
    // this one's at the first looks, not taken for it. The binary may
    // have been replaced since this process started (while it waited for
    // the lock, under a service: incident 2026-11-02, a node ran its old
    // build for 9 hours), and a notice to restart may have gone to
    // another process; this node finds a newer build by itself.
    let mut known = None;
    let mut seen = None;
    loop {
      tokio::time::sleep(Duration::from_secs(WATCH_SECS)).await;
      let now = files_id(&exe);
      if now.is_none() || now == known {
        seen = None;
        continue;
      }
      if seen != now {
        // Changed since the last look: looked at again at the next.
        seen = now;
        continue;
      }
      known = now;
      seen = None;
      let exe2 = exe.clone();
      let Ok(Some(found)) = tokio::task::spawn_blocking(move || installed(&exe2)).await else {
        continue;
      };
      if found.hash == own {
        continue;
      }
      let slug = found.slug();
      if found.label == "unrunnable" {
        eprintln!(
          "{}: replaced, but it does not run (`--version`): not restarting into it",
          exe.display()
        );
      } else if Builds::new(&data).state().failed.contains(&slug) {
        eprintln!(
          "{}: build {slug} failed here before: not restarting into it",
          exe.display()
        );
      } else {
        restart(format!("build {slug} installed"));
        return;
      }
    }
  })
}

/// `stitch node builds`: what `<data>/bin/state.json` says, the build
/// installed where this `stitch` is, and with `healthy_for`, whether that
/// build runs and has been healthy here that long (the rollout's trial:
/// docs/self-improve.md §4, 2).
pub fn builds_said(data: &Path, healthy_for: Option<u64>) -> Result<(Vec<String>, bool)> {
  let builds = Builds::new(data);
  let state = builds.state();
  let exe = upgrade::installed_exe()?;
  let here = installed(&exe).map(|i| i.slug());
  let mut out = vec![format!(
    "installed: {}",
    here.as_deref().unwrap_or("none (unreadable)")
  )];
  let since = state
    .healthy_since
    .as_deref()
    .and_then(|t| t.parse::<jiff::Timestamp>().ok());
  match (&state.healthy, since) {
    (Some(h), Some(t)) => out.push(format!(
      "healthy: {h} since {t} ({} min)",
      jiff::Timestamp::now().duration_since(t).as_secs() / 60
    )),
    (Some(h), None) => out.push(format!("healthy: {h}")),
    (None, _) => out.push("healthy: none yet".into()),
  }
  if let Some(p) = &state.previous {
    out.push(format!("previous: {p}"));
  }
  if let Some(t) = &state.trying {
    out.push(format!("on trial: {t} (start {})", state.attempts));
  }
  for f in &state.failed {
    out.push(format!("failed: {f}"));
  }
  if let Some(e) = &state.event {
    out.push(format!("last: {e}"));
  }
  let runs = stitch_node::running(data);
  out.push(format!(
    "node: {}",
    if runs { "running" } else { "not running" }
  ));
  // The process running now: which build, and how long it has been up
  // (a monotonic count where the system gives one).
  let pid = stitch_node::node_pid(data);
  let running = std::fs::read_to_string(data.join("bin").join("running")).unwrap_or_default();
  let mut running = running.split_whitespace();
  let (r_pid, r_build) = (
    running.next().and_then(|p| p.parse::<u32>().ok()),
    running.next().map(str::to_owned),
  );
  let up = pid.filter(|p| Some(*p) == r_pid).and_then(process_age);
  if let (Some(b), Some(u)) = (&r_build, up) {
    out.push(format!("running: {b}, up {} min", u.as_secs() / 60));
  }
  // A restart it waits to make (into the build installed, most often):
  // since when, and what keeps it busy.
  if runs
    && let Some(p) = crate::runtime().ok().and_then(|rt| {
      rt.block_on(async {
        let mut c = Client::connect(data).await.ok()?;
        match c.request(&Request::Status).await.ok()? {
          Reply::Status { pending, .. } => pending,
          _ => None,
        }
      })
    })
  {
    out.push(format!("waiting: {p}"));
  }
  let ok = match healthy_for {
    None => true,
    Some(secs) => {
      runs
        && here.is_some()
        && state.healthy == here
        && r_build == here
        && state.trying.is_none()
        && since.is_some()
        && up.is_some_and(|u| u.as_secs() >= secs)
        && answers(data)
    }
  };
  Ok((out, ok))
}

/// Whether the node answers on its socket now (within 10 s).
fn answers(data: &Path) -> bool {
  let Ok(rt) = crate::runtime() else {
    return false;
  };
  rt.block_on(async {
    let probe = async {
      let mut c = Client::connect(data).await.ok()?;
      c.request(&Request::Status).await.ok()
    };
    matches!(
      tokio::time::timeout(Duration::from_secs(10), probe).await,
      Ok(Some(Reply::Status { .. }))
    )
  })
}

/// How long process `pid` has been running: on Linux from its start in
/// clock ticks since boot (monotonic), on macOS from `proc_pidinfo` (not
/// `ps`: it is setuid, and the sandbox threads' shells run under refuses to
/// exec setuid binaries), elsewhere from `ps`.
#[allow(unsafe_code)]
fn process_age(pid: u32) -> Option<Duration> {
  if cfg!(target_os = "linux") {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The fields after the command's closing parenthesis; starttime is
    // the 22nd field, the 20th after it.
    let rest = &stat[stat.rfind(')')? + 2..];
    let start: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
      .ok()?
      .split_whitespace()
      .next()?
      .parse()
      .ok()?;
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let ticks = if ticks > 0 { ticks as f64 } else { 100.0 };
    return Some(Duration::from_secs_f64((uptime - start / ticks).max(0.0)));
  }
  #[cfg(target_os = "macos")]
  {
    // SAFETY: proc_bsdinfo is plain data, zero is a valid value; the buffer
    // and its size are those of `info`.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
      libc::proc_pidinfo(
        pid as libc::c_int,
        libc::PROC_PIDTBSDINFO,
        0,
        &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
        size,
      )
    };
    if got <= 0 {
      return None;
    }
    let start = Duration::new(
      info.pbi_start_tvsec,
      (info.pbi_start_tvusec as u32).saturating_mul(1000),
    );
    let now = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .ok()?;
    Some(now.saturating_sub(start))
  }
  #[cfg(not(target_os = "macos"))]
  {
    let out = std::process::Command::new("ps")
      .args(["-o", "etime=", "-p", &pid.to_string()])
      .output()
      .ok()?;
    parse_etime(String::from_utf8_lossy(&out.stdout).trim())
  }
}

/// `ps`'s elapsed time: `[[dd-]hh:]mm:ss`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn parse_etime(s: &str) -> Option<Duration> {
  let (days, rest) = match s.split_once('-') {
    Some((d, r)) => (d.parse::<u64>().ok()?, r),
    None => (0, s),
  };
  let parts: Vec<u64> = rest
    .split(':')
    .map(|p| p.parse().ok())
    .collect::<Option<_>>()?;
  let secs = match parts.as_slice() {
    [m, s] => m * 60 + s,
    [h, m, s] => h * 3600 + m * 60 + s,
    _ => return None,
  };
  Some(Duration::from_secs(days * 86400 + secs))
}

/// SIGTERM while the node starts, before it handles the signal itself (a
/// `systemctl stop`, a `launchctl bootout`, `stitch node stop`'s fallback):
/// a stop asked for, not a failed start. The handler only appends `!` to
/// `<data>/bin/early-stop` (which names the build), with `write`, then
/// ends the process as the signal would; the next start and a restart's
/// watcher read it.
#[allow(unsafe_code)]
mod early_term {
  use std::os::fd::IntoRawFd;
  use std::path::{Path, PathBuf};
  use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

  static FD: AtomicI32 = AtomicI32::new(-1);

  fn path(data: &Path) -> PathBuf {
    data.join("bin").join("early-stop")
  }

  extern "C" fn on_term(_: libc::c_int) {
    let fd = FD.load(Ordering::SeqCst);
    // SAFETY: write, signal and raise are async-signal-safe.
    unsafe {
      if fd >= 0 {
        libc::write(fd, b"!".as_ptr().cast(), 1);
      }
      libc::signal(libc::SIGTERM, libc::SIG_DFL);
      libc::raise(libc::SIGTERM);
    }
  }

  /// The build whose start was ended by SIGTERM last, once (the record
  /// is removed as it is read).
  pub fn stopped_last(data: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path(data)).ok()?;
    let _ = std::fs::remove_file(path(data));
    text.strip_suffix('!').map(str::to_owned)
  }

  /// Whether the last start was ended so (for a restart's watcher).
  pub fn stopped(data: &Path) -> bool {
    std::fs::read_to_string(path(data)).is_ok_and(|t| t.ends_with('!'))
  }

  pub struct Guard {
    path: PathBuf,
    on: AtomicBool,
  }

  impl Guard {
    pub fn install(data: &Path, me: &str) -> Guard {
      let path = path(data);
      let on = std::fs::write(&path, me).is_ok()
        && std::fs::OpenOptions::new()
          .append(true)
          .open(&path)
          .is_ok_and(|f| {
            FD.store(f.into_raw_fd(), Ordering::SeqCst);
            // SAFETY: the handler only does async-signal-safe calls.
            unsafe { libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t) };
            true
          });
      Guard {
        path,
        on: AtomicBool::new(on),
      }
    }

    /// Back to the default, before the node installs its own handling.
    pub fn remove(&self) {
      if !self.on.swap(false, Ordering::SeqCst) {
        return;
      }
      // SAFETY: restoring the default disposition; the descriptor is ours.
      unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
        let fd = FD.swap(-1, Ordering::SeqCst);
        if fd >= 0 {
          libc::close(fd);
        }
      }
      let _ = std::fs::remove_file(&self.path);
    }
  }

  impl Drop for Guard {
    fn drop(&mut self) {
      self.remove();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sup(supervised: bool, unit: Option<&str>) -> Supervisor {
    Supervisor {
      supervised,
      unit: unit.map(str::to_owned),
    }
  }

  fn svc(kind: &str, name: &str) -> Option<(String, String)> {
    Some((kind.into(), name.into()))
  }

  #[test]
  fn a_restart_goes_to_the_service_that_runs_the_node() {
    // No service set up: as it runs.
    assert_eq!(next(&sup(false, None), None), Next::Successor);
    assert_eq!(
      next(&sup(true, Some("stitch.service")), None),
      Next::ExitForSupervisor
    );
    // The service's own node (geek's unit, the LaunchAgent).
    assert_eq!(
      next(&sup(true, Some("stitch.service")), svc("systemd", "stitch")),
      Next::ExitForSupervisor
    );
    assert_eq!(
      next(
        &sup(true, Some("dev.stitch.service.x")),
        svc("launchd", "dev.stitch.service.x")
      ),
      Next::ExitForSupervisor
    );
    // A node outside it (a client's, the browser host's launchd job):
    // the service starts the next one, never a node of its own.
    assert_eq!(
      next(&sup(false, None), svc("systemd", "stitch")),
      Next::StartService {
        kind: "systemd".into(),
        name: "stitch".into()
      }
    );
    assert_eq!(
      next(
        &sup(true, Some("dev.stitch.node.x")),
        svc("launchd", "dev.stitch.service.x")
      ),
      Next::StartService {
        kind: "launchd".into(),
        name: "dev.stitch.service.x".into()
      }
    );
  }

  #[test]
  fn ps_elapsed_times_are_read() {
    assert_eq!(parse_etime("05:07"), Some(Duration::from_secs(307)));
    assert_eq!(parse_etime("01:00:00"), Some(Duration::from_secs(3600)));
    assert_eq!(
      parse_etime("2-00:00:01"),
      Some(Duration::from_secs(2 * 86400 + 1))
    );
    assert_eq!(parse_etime(""), None);
    // This process, whatever the system.
    assert!(process_age(std::process::id()).is_some_and(|d| d < Duration::from_secs(86400)));
  }

  #[test]
  fn a_sigterm_at_the_start_is_read_once() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("bin")).unwrap();
    assert_eq!(early_term::stopped_last(t.path()), None);
    // Started, no signal: nothing said.
    std::fs::write(t.path().join("bin/early-stop"), "b1").unwrap();
    assert!(!early_term::stopped(t.path()));
    assert_eq!(early_term::stopped_last(t.path()), None);
    // The handler appended its mark.
    std::fs::write(t.path().join("bin/early-stop"), "b1!").unwrap();
    assert!(early_term::stopped(t.path()));
    assert_eq!(early_term::stopped_last(t.path()).as_deref(), Some("b1"));
    assert_eq!(early_term::stopped_last(t.path()), None);
  }

  #[test]
  fn the_watcher_reads_a_stop_from_its_marker() {
    let t = tempfile::tempdir().unwrap();
    assert!(!stopped_on_request(t.path(), 42));
    std::fs::write(stopping_marker(t.path()), "42").unwrap();
    assert!(stopped_on_request(t.path(), 42));
    assert!(!stopped_on_request(t.path(), 43));
  }
}
