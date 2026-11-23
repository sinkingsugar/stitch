//! A node under systemd (docs/self-improve.md §2.1): run in a transient
//! user unit (`systemd-run --user`), it knows it is supervised, a restart
//! is an exit with status 75 that systemd answers, and a stop asked for
//! stays stopped. Linux only, and skipped where there is no user
//! manager to ask (a container, CI without a user session).

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct Unit {
  name: String,
  data: PathBuf,
  exe: PathBuf,
}

fn systemctl(args: &[&str]) -> String {
  let out = Command::new("systemctl")
    .arg("--user")
    .args(args)
    .output()
    .expect("systemctl");
  String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

impl Unit {
  fn stitch(&self, args: &[&str]) -> String {
    let out = Command::new(&self.exe)
      .arg("--data")
      .arg(&self.data)
      .args(args)
      .env_remove("STITCH_SUPERVISED")
      .env_remove("INVOCATION_ID")
      .output()
      .expect("stitch runs");
    format!(
      "{}{}",
      String::from_utf8_lossy(&out.stdout),
      String::from_utf8_lossy(&out.stderr)
    )
  }

  fn log(&self) -> String {
    std::fs::read_to_string(self.data.join("node.log")).unwrap_or_default()
  }

  fn main_pid(&self) -> u32 {
    systemctl(&[
      "show",
      "-p",
      "MainPID",
      "--value",
      &format!("{}.service", self.name),
    ])
    .parse()
    .unwrap_or(0)
  }

  fn until(&self, what: &str, f: impl Fn(&Unit) -> bool) {
    let start = Instant::now();
    while !f(self) {
      if start.elapsed() > Duration::from_secs(120) {
        let unit = format!("{}.service", self.name);
        let status = Command::new("systemctl")
          .args(["--user", "status", "--no-pager", &unit])
          .output()
          .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
          .unwrap_or_default();
        let journal = Command::new("journalctl")
          .args(["--user", "-u", &unit, "-n", "40", "--no-pager"])
          .output()
          .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
          .unwrap_or_default();
        panic!(
          "{what}: not within 120 s\n--- node.log\n{}\n--- systemctl status\n{status}\n--- journal\n{journal}",
          self.log()
        );
      }
      std::thread::sleep(Duration::from_millis(250));
    }
  }
}

impl Drop for Unit {
  fn drop(&mut self) {
    let unit = format!("{}.service", self.name);
    let _ = Command::new("systemctl")
      .args(["--user", "stop", &unit])
      .output();
    let _ = Command::new("systemctl")
      .args(["--user", "reset-failed", &unit])
      .output();
    let _ = std::fs::remove_dir_all(self.data.parent().unwrap_or(Path::new("/nonexistent")));
  }
}

#[test]
fn a_node_under_systemd_restarts_through_it() {
  if !Command::new("systemctl")
    .args(["--user", "is-system-running"])
    .output()
    .is_ok_and(|o| !o.stdout.is_empty())
  {
    eprintln!("no systemd user manager here: skipped");
    return;
  }
  let dir =
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stitch-unit-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&dir);
  std::fs::create_dir_all(dir.join("bin")).unwrap();
  let exe = dir.join("bin/stitch");
  std::fs::copy(env!("CARGO_BIN_EXE_stitch"), &exe).unwrap();
  let unit = Unit {
    name: format!("stitch-test-{}", std::process::id()),
    data: dir.join("data"),
    exe,
  };
  let setup = unit.stitch(&["setup", "--name", "unittest", "--local"]);
  assert!(setup.contains("node unittest"), "{setup}");
  let log = unit.data.join("node.log");
  let started = Command::new("systemd-run")
    .args(["--user", "--unit", &unit.name])
    .args(["-p", "Restart=on-failure", "-p", "RestartSec=1"])
    .arg(format!(
      "--property=StandardOutput=append:{}",
      log.display()
    ))
    .arg(format!("--property=StandardError=append:{}", log.display()))
    .args([
      "--setenv=STITCH_SUPERVISED=1",
      "--setenv=STITCH_HEALTHY_AFTER_SECS=2",
    ])
    .arg(&unit.exe)
    .arg("--data")
    .arg(&unit.data)
    .arg("node")
    .output()
    .unwrap();
  if !started.status.success() {
    eprintln!(
      "systemd-run --user failed ({}): skipped",
      String::from_utf8_lossy(&started.stderr).trim()
    );
    return;
  }
  unit.until("serving", |u| u.log().contains("serving"));
  let first = unit.main_pid();
  assert_ne!(first, 0);

  // A restart: the node exits 75, systemd starts it again.
  let said = unit.stitch(&["node", "restart"]);
  assert!(said.contains("restarts now"), "{said}");
  unit.until("restarted by systemd", |u| {
    u.log().contains("exiting for the service") && u.log().matches("serving").count() == 2
  });
  let second = unit.main_pid();
  assert!(second != 0 && second != first, "{first} {second}");

  // A stop asked for stays stopped (status 0).
  unit.stitch(&["node", "stop"]);
  unit.until("stopped", |u| u.main_pid() == 0);
  std::thread::sleep(Duration::from_secs(3));
  assert_eq!(unit.main_pid(), 0, "{}", unit.log());
}
