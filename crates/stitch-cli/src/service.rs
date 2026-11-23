//! `stitch node service`: the node as a systemd user unit (Linux) or a
//! LaunchAgent (macOS), with an optional self-update timer
//! (docs/self-improve.md §2.1, step 3; the templates in `deploy/node/`).
//!
//! The service runs the installed binary with `STITCH_SUPERVISED=1`: a
//! restart the node asks for (a new build installed, `stitch node
//! restart`) is an exit with status 75, which the service answers by
//! starting it again. `<data>/service` names the service, so a client that
//! finds no node starts the service rather than a node of its own; it is
//! written last, once the service is in place.
//!
//! Installing never kills a running node: one outside the service stops
//! once idle (the service's node, waiting for the lock meanwhile, takes
//! over); the service's own node restarts into the new unit once idle
//! (systemd), or keeps running until its agent is loaded again (launchd,
//! which keeps a loaded job's definition: it is said).

use std::path::{Path, PathBuf};
use std::process::Command;

use stitch_node::api::{Reply, Request};
use stitch_node::client::Client;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const UNIT: &str = include_str!("../../../deploy/node/stitch.service");
const UPDATE_UNIT: &str = include_str!("../../../deploy/node/stitch-update.service");
const UPDATE_TIMER: &str = include_str!("../../../deploy/node/stitch-update.timer");
const AGENT: &str = include_str!("../../../deploy/node/dev.stitch.node.plist");
const UPDATE_AGENT: &str = include_str!("../../../deploy/node/dev.stitch.update.plist");
const SELF_UPDATE: &str = include_str!("../../../deploy/node/self-update.sh");

/// What the service files are made from.
pub struct Spec {
  pub exe: PathBuf,
  pub data: PathBuf,
  pub home: PathBuf,
  /// The PATH the node gets (its MCP servers and shells find programs on it).
  pub path: String,
  pub shell: String,
  /// The checkout the self-update pulls and builds; with it, the timer.
  pub update: Option<PathBuf>,
  /// The rollout (docs/self-improve.md §4, 2): the branch the self-update
  /// follows (default main), the one it promotes a build healthy here to,
  /// and whether it uploads the app to TestFlight after (macOS).
  pub follow: Option<String>,
  pub promote: Option<String>,
  pub testflight: bool,
  /// The app's checkout TestFlight builds from (`STITCH_APP`: its
  /// justfile, by path). None with `testflight`: the checkout's own
  /// justfile when it holds the app, else nothing is uploaded.
  pub app: Option<PathBuf>,
}

impl Spec {
  /// The rollout's settings, as the self-update's environment.
  fn rollout(&self) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(f) = &self.follow {
      out.push(("STITCH_FOLLOW", f.clone()));
    }
    if let Some(p) = &self.promote {
      out.push(("STITCH_PROMOTE", p.clone()));
    }
    if self.testflight {
      out.push(("STITCH_TESTFLIGHT", "1".into()));
    }
    if let Some(a) = &self.app {
      out.push(("STITCH_APP", a.display().to_string()));
    }
    out
  }
}

/// A file to write: where, what, executable or not.
pub struct File {
  pub path: PathBuf,
  pub text: String,
  pub exec: bool,
}

/// The unit's name (systemd) or the agent's label (launchd): one per data
/// directory, `stitch` for the default one. The label is the service's
/// own, never that of the job a client (the browser host) starts a node in.
pub fn name(data: &Path, home: &Path, launchd: bool) -> String {
  if launchd {
    return stitch_node::client::service_label(data);
  }
  if data == home.join(".stitch") {
    return "stitch".into();
  }
  let s: String = data
    .to_string_lossy()
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect();
  format!("stitch-{}", s.trim_matches('-'))
}

fn xml(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
}

/// A word of a systemd command line or setting: `%` doubled (specifiers),
/// quoted when it has spaces.
fn sd(s: &str) -> String {
  let s = s.replace('%', "%%");
  if s.contains(char::is_whitespace) || s.contains('"') {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
  } else {
    s
  }
}

/// Inside a quoted systemd assignment (`Environment="PATH=…"`).
fn sdq(s: &str) -> String {
  s.replace('%', "%%")
    .replace('\\', "\\\\")
    .replace('"', "\\\"")
}

/// The files `install` writes, for `launchd` (macOS) or systemd.
pub fn files(spec: &Spec, launchd: bool) -> Vec<File> {
  let name = name(&spec.data, &spec.home, launchd);
  let script = spec.home.join(".local/bin/stitch-self-update");
  let log = spec.data.join("node.log");
  let checkout = spec
    .update
    .as_deref()
    .map(|c| c.to_string_lossy().into_owned())
    .unwrap_or_default();
  let rollout_plist: String = spec
    .rollout()
    .iter()
    .map(|(k, v)| format!("\n    <key>{k}</key><string>{}</string>", xml(v)))
    .collect();
  let rollout_unit: String = spec
    .rollout()
    .iter()
    .map(|(k, v)| format!("\nEnvironment=\"{k}={}\"", sdq(v)))
    .collect();
  let fill = |t: &str, q: &dyn Fn(&str) -> String, qq: &dyn Fn(&str) -> String| {
    t.replace(
      "@ROLLOUT@",
      if launchd {
        &rollout_plist
      } else {
        &rollout_unit
      },
    )
    .replace("@QPATH@", &qq(&spec.path))
    .replace("@QSHELL@", &qq(&spec.shell))
    .replace("@QCHECKOUT@", &qq(&checkout))
    .replace("@QDATA@", &qq(&spec.data.to_string_lossy()))
    .replace("@EXE@", &q(&spec.exe.to_string_lossy()))
    .replace("@DATA@", &q(&spec.data.to_string_lossy()))
    .replace("@HOME@", &q(&spec.home.to_string_lossy()))
    .replace("@PATH@", &q(&spec.path))
    .replace("@SHELL@", &q(&spec.shell))
    .replace("@LOG@", &q(&log.to_string_lossy()))
    .replace("@LABEL@", &q(&name))
    .replace("@SCRIPT@", &q(&script.to_string_lossy()))
    .replace("@CHECKOUT@", &q(&checkout))
  };
  let mut out = Vec::new();
  if launchd {
    let agents = spec.home.join("Library/LaunchAgents");
    out.push(File {
      path: agents.join(format!("{name}.plist")),
      text: fill(AGENT, &xml, &xml),
      exec: false,
    });
    if spec.update.is_some() {
      out.push(File {
        path: agents.join("dev.stitch.update.plist"),
        text: fill(UPDATE_AGENT, &xml, &xml),
        exec: false,
      });
    }
  } else {
    let units = spec.home.join(".config/systemd/user");
    // Its output where the node's log has always been.
    let unit = fill(UNIT, &sd, &sdq).replace(
      "Restart=on-failure",
      &format!(
        "StandardOutput=append:{log}\nStandardError=append:{log}\nRestart=on-failure",
        log = log.to_string_lossy().replace('%', "%%")
      ),
    );
    out.push(File {
      path: units.join(format!("{name}.service")),
      text: unit,
      exec: false,
    });
    if spec.update.is_some() {
      out.push(File {
        path: units.join("stitch-update.service"),
        text: fill(UPDATE_UNIT, &sd, &sdq),
        exec: false,
      });
      out.push(File {
        path: units.join("stitch-update.timer"),
        text: UPDATE_TIMER.to_owned(),
        exec: false,
      });
    }
  }
  if spec.update.is_some() {
    out.push(File {
      path: script,
      text: SELF_UPDATE.to_owned(),
      exec: true,
    });
  }
  out
}

fn run(cmd: &mut Command) -> Result<()> {
  let out = cmd.output()?;
  if !out.status.success() {
    return Err(format!("{:?}: {}", cmd, String::from_utf8_lossy(&out.stderr).trim()).into());
  }
  Ok(())
}

fn uid(data: &Path) -> Result<u32> {
  Ok(std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(
    data,
  )?))
}

/// What to do with a node that runs while the service is installed.
#[derive(Debug, PartialEq, Eq)]
pub enum Running {
  /// None runs.
  None,
  /// The service's own: it restarts into the new definition once idle.
  Service,
  /// One outside the service: it stops once idle, for the service's.
  Outside(u32),
}

/// Whether the node holding the lock (`node`) is the service's (whose
/// main process is `service`).
pub fn running(node: Option<u32>, service: Option<u32>) -> Running {
  match node {
    None => Running::None,
    Some(p) if Some(p) == service => Running::Service,
    Some(p) => Running::Outside(p),
  }
}

/// Asks the running node to stop once idle (`successor`: to restart).
fn ask(data: &Path, successor: bool) -> Option<Vec<String>> {
  crate::runtime().ok()?.block_on(async {
    let mut c = Client::connect(data).await.ok()?;
    match c
      .request(&Request::Restart {
        now: false,
        successor,
      })
      .await
    {
      Ok(Reply::Lines(lines)) => Some(lines),
      _ => None,
    }
  })
}

/// Writes the files, enables and starts the service, then says so in
/// `<data>/service`; a node already running is handed over to without
/// being cut (see the module's docs). With `print`, only says what it
/// would write.
pub fn install(spec: &Spec, print: bool) -> Result<Vec<String>> {
  let launchd = cfg!(target_os = "macos");
  let files = files(spec, launchd);
  if print {
    return Ok(
      files
        .iter()
        .map(|f| format!("=== {}\n{}", f.path.display(), f.text))
        .collect(),
    );
  }
  std::fs::create_dir_all(&spec.data)?;
  let mut said = Vec::new();
  for f in &files {
    if let Some(dir) = f.path.parent() {
      std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&f.path, &f.text)?;
    if f.exec {
      use std::os::unix::fs::PermissionsExt;
      std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o755))?;
    }
    said.push(format!("wrote {}", f.path.display()));
  }
  let name = name(&spec.data, &spec.home, launchd);
  let node = stitch_node::node_pid(&spec.data);
  let service_pid = if launchd {
    install_launchd(spec, &files, &name, node, &mut said)?
  } else {
    install_systemd(spec, &name, node, &mut said)?
  };
  // Last: clients start the service from now on.
  let kind = if launchd { "launchd" } else { "systemd" };
  std::fs::write(spec.data.join("service"), format!("{kind}\n{name}\n"))?;
  match running(node, service_pid) {
    Running::None => {}
    Running::Service => {
      if !launchd && let Some(lines) = ask(&spec.data, true) {
        said.extend(
          lines
            .iter()
            .map(|l| format!("the service's node, into the new unit: {l}")),
        );
      }
    }
    Running::Outside(pid) => match ask(&spec.data, false) {
      Some(lines) => said.extend(lines.iter().map(|l| {
        format!("the node outside the service (pid {pid}): {l}; the service's takes over then")
      })),
      None => said.push(format!(
        "a node outside the service runs (pid {pid}) and could not be asked to stop: \
         `stitch node stop --when-idle` hands over to the service"
      )),
    },
  }
  Ok(said)
}

fn install_systemd(
  spec: &Spec,
  name: &str,
  node: Option<u32>,
  said: &mut Vec<String>,
) -> Result<Option<u32>> {
  let unit = format!("{name}.service");
  run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
  run(Command::new("systemctl").args(["--user", "enable", &unit]))?;
  said.push(format!("enabled {unit}"));
  if spec.update.is_some() {
    run(Command::new("systemctl").args(["--user", "enable", "--now", "stitch-update.timer"]))?;
    said.push("enabled stitch-update.timer".into());
  }
  let main = crate::noderun::systemd_main_pid(&unit);
  if node.is_some() && node == main {
    said.push(format!("{unit} runs the node already"));
  } else {
    // Its node waits for the lock while one runs outside it.
    run(Command::new("systemctl").args(["--user", "start", &unit]))?;
    said.push(format!("started {unit}"));
  }
  if Command::new("loginctl")
    .args(["show-user", &uid(&spec.data)?.to_string(), "-p", "Linger"])
    .output()
    .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "Linger=no")
  {
    said.push(
      "lingering is off: the node runs only while you are logged in \
       (`sudo loginctl enable-linger $USER` to run it from boot)"
        .into(),
    );
  }
  Ok(crate::noderun::systemd_main_pid(&unit))
}

fn install_launchd(
  spec: &Spec,
  files: &[File],
  name: &str,
  node: Option<u32>,
  said: &mut Vec<String>,
) -> Result<Option<u32>> {
  let domain = format!("gui/{}", uid(&spec.data)?);
  let loaded = |label: &str| {
    Command::new("launchctl")
      .args(["print", &format!("{domain}/{label}")])
      .output()
      .is_ok_and(|o| o.status.success())
  };
  // An agent an earlier `service install` wrote under the label of the
  // browser host's job (dev.stitch.node.<path>): removed, so it does not
  // load at login beside this one; booted out unless its node runs (that
  // one is asked to stop once idle, below, as any node outside).
  let old = stitch_node::client::launchd_label(&spec.data);
  let old_plist = spec
    .home
    .join("Library/LaunchAgents")
    .join(format!("{old}.plist"));
  if old_plist.exists() {
    let pid = crate::noderun::launchd_pid(&spec.data, &old);
    if loaded(&old) && (pid.is_none() || pid != node) {
      let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{old}")])
        .output();
    }
    std::fs::remove_file(&old_plist)?;
    said.push(format!("removed the old agent {}", old_plist.display()));
  }
  for f in files {
    if f.path.extension().is_none_or(|e| e != "plist") {
      continue;
    }
    let label = f
      .path
      .file_stem()
      .map(|s| s.to_string_lossy().into_owned())
      .unwrap_or_default();
    if label == name && loaded(&label) {
      let pid = crate::noderun::launchd_pid(&spec.data, &label);
      if pid.is_some() && pid == node {
        // Never booted out under a running node: launchd keeps the loaded
        // definition until the agent is loaded again.
        said.push(format!(
          "{label} runs the node: its new definition is used from its next load \
           (at login, or `stitch node service install` again while no node runs)"
        ));
        continue;
      }
    }
    // Loaded again from the new file (a label loads once): a job with no
    // node of its own running (or the update agent).
    if loaded(&label) {
      run(Command::new("launchctl").args(["bootout", &format!("{domain}/{label}")]))?;
    }
    run(
      Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&f.path),
    )?;
    said.push(format!("loaded {label}"));
  }
  // Its node starts at load (RunAtLoad), waiting for the lock while one
  // runs outside it: its process, once it is there.
  let mut pid = None;
  for _ in 0..20 {
    pid = crate::noderun::launchd_pid(&spec.data, name);
    if pid.is_some() {
      break;
    }
    std::thread::sleep(std::time::Duration::from_millis(250));
  }
  Ok(pid)
}

/// Stops and removes the service (and the update timer); the node then
/// runs as clients start it, as before. Its node stops now.
pub fn uninstall(data: &Path, home: &Path) -> Result<Vec<String>> {
  let launchd = cfg!(target_os = "macos");
  let name = name(data, home, launchd);
  let mut said = Vec::new();
  let _ = std::fs::remove_file(data.join("service"));
  if launchd {
    let domain = format!("gui/{}", uid(data)?);
    for label in [name.as_str(), "dev.stitch.update"] {
      let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{label}")])
        .output();
      let plist = home.join(format!("Library/LaunchAgents/{label}.plist"));
      if std::fs::remove_file(&plist).is_ok() {
        said.push(format!("removed {}", plist.display()));
      }
    }
  } else {
    let units = home.join(".config/systemd/user");
    for unit in [format!("{name}.service"), "stitch-update.timer".into()] {
      let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", &unit])
        .output();
    }
    for f in [
      format!("{name}.service"),
      "stitch-update.service".into(),
      "stitch-update.timer".into(),
    ] {
      if std::fs::remove_file(units.join(&f)).is_ok() {
        said.push(format!("removed {f}"));
      }
    }
    let _ = Command::new("systemctl")
      .args(["--user", "daemon-reload"])
      .output();
  }
  Ok(said)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn spec() -> Spec {
    Spec {
      exe: "/home/u/.cargo/bin/stitch".into(),
      data: "/home/u/.stitch".into(),
      home: "/home/u".into(),
      path: "/home/u/.cargo/bin:/usr/bin".into(),
      shell: "/bin/bash".into(),
      update: Some("/home/u/devel/stitch".into()),
      follow: None,
      promote: None,
      testflight: false,
      app: None,
    }
  }

  #[test]
  fn the_rollout_goes_into_the_update_jobs() {
    let mut s = spec();
    s.follow = Some("fleet".into());
    s.promote = Some("fleet-mac".into());
    s.testflight = true;
    let unit = &files(&s, false)[1].text;
    assert!(
      unit.contains("Environment=\"STITCH_FOLLOW=fleet\"\n"),
      "{unit}"
    );
    assert!(
      unit.contains("Environment=\"STITCH_PROMOTE=fleet-mac\"\n"),
      "{unit}"
    );
    assert!(
      unit.contains("Environment=\"STITCH_TESTFLIGHT=1\"\n"),
      "{unit}"
    );
    let agent = &files(&s, true)[1].text;
    assert!(
      agent.contains("<key>STITCH_FOLLOW</key><string>fleet</string>"),
      "{agent}"
    );
    // A plain --testflight: no STITCH_APP (the checkout's own justfile,
    // when it holds the app).
    assert!(!unit.contains("STITCH_APP"), "{unit}");
    s.app = Some("/home/u/devel/app".into());
    let unit = &files(&s, false)[1].text;
    assert!(
      unit.contains("Environment=\"STITCH_APP=/home/u/devel/app\"\n"),
      "{unit}"
    );
    assert!(
      files(&s, true)[1]
        .text
        .contains("<key>STITCH_APP</key><string>/home/u/devel/app</string>")
    );
    // None set: nothing added.
    assert!(
      !files(&spec(), false)[1]
        .text
        .contains("Environment=\"STITCH_FOLLOW")
    );
    assert!(!files(&spec(), true)[1].text.contains("@ROLLOUT@"));
  }

  #[test]
  fn the_systemd_unit_runs_the_installed_binary_supervised() {
    let files = files(&spec(), false);
    let names: Vec<_> = files.iter().map(|f| f.path.display().to_string()).collect();
    assert_eq!(
      names,
      [
        "/home/u/.config/systemd/user/stitch.service",
        "/home/u/.config/systemd/user/stitch-update.service",
        "/home/u/.config/systemd/user/stitch-update.timer",
        "/home/u/.local/bin/stitch-self-update",
      ]
    );
    let unit = &files[0].text;
    assert!(unit.contains("ExecStart=/home/u/.cargo/bin/stitch --data /home/u/.stitch node\n"));
    assert!(unit.contains("Environment=STITCH_SUPERVISED=1\n"));
    assert!(unit.contains("Environment=\"PATH=/home/u/.cargo/bin:/usr/bin\"\n"));
    assert!(unit.contains("StandardOutput=append:/home/u/.stitch/node.log\n"));
    assert!(!unit.contains("network-online"), "{unit}");
    assert!(!unit.contains('@'), "{unit}");
    assert!(
      files[1]
        .text
        .contains("Environment=\"STITCH_CHECKOUT=/home/u/devel/stitch\"\n")
    );
    assert!(
      files[1]
        .text
        .contains("ExecStart=/home/u/.local/bin/stitch-self-update\n")
    );
    assert!(files[3].exec);
  }

  #[test]
  fn systemd_values_are_quoted_and_escaped() {
    let mut s = spec();
    s.path = "/a b:/c%d".into();
    s.data = "/home/u/my data".into();
    let unit = &files(&s, false)[0].text;
    assert!(unit.contains("Environment=\"PATH=/a b:/c%%d\"\n"), "{unit}");
    assert!(
      unit.contains("ExecStart=/home/u/.cargo/bin/stitch --data \"/home/u/my data\" node\n"),
      "{unit}"
    );
  }

  #[test]
  fn the_launch_agent_has_its_own_label_and_restarts_on_failure_only() {
    let files = files(&spec(), true);
    assert_eq!(
      files[0].path,
      Path::new("/home/u/Library/LaunchAgents/dev.stitch.service.home-u--stitch.plist")
    );
    let agent = &files[0].text;
    assert!(agent.contains("<key>Label</key><string>dev.stitch.service.home-u--stitch</string>"));
    // Not the label of the job the browser host starts a node in.
    assert_ne!(
      stitch_node::client::launchd_label(Path::new("/home/u/.stitch")),
      stitch_node::client::service_label(Path::new("/home/u/.stitch"))
    );
    assert!(agent.contains("<key>SuccessfulExit</key><false/>"));
    assert!(agent.contains("<string>/home/u/.cargo/bin/stitch</string>"));
    assert!(!agent.contains('@'), "{agent}");
    assert!(
      files[1]
        .text
        .contains("<string>/home/u/.local/bin/stitch-self-update</string>")
    );
  }

  #[test]
  fn other_data_directories_get_their_own_unit() {
    let home = Path::new("/home/u");
    assert_eq!(name(&home.join(".stitch"), home, false), "stitch");
    assert_eq!(name(Path::new("/tmp/x y"), home, false), "stitch-tmp-x-y");
  }

  #[test]
  fn a_running_node_is_the_services_only_when_it_is_its_main_process() {
    assert_eq!(running(None, Some(5)), Running::None);
    // geek: the node runs under the unit being reinstalled.
    assert_eq!(running(Some(5), Some(5)), Running::Service);
    // A node a client started, the service's waiting for the lock.
    assert_eq!(running(Some(5), Some(9)), Running::Outside(5));
    // No unit running (or no user bus to ask).
    assert_eq!(running(Some(5), None), Running::Outside(5));
  }
}
