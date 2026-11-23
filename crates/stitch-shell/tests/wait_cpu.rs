//! A wait on the screen costs nothing while nothing changes: alone in its
//! own test binary, so the process's CPU time is the wait's.

use std::time::Duration;

use stitch_shell::{Config, Regex, ScreenUntil, Session, Until, Wait};

/// The CPU time this process has used, user and system.
fn cpu() -> Duration {
  if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
    // After the command's name: state, then 10 fields, then utime and
    // stime in clock ticks (100 a second).
    let rest = &stat[stat.rfind(')').unwrap() + 2..];
    let f: Vec<u64> = rest
      .split_whitespace()
      .skip(11)
      .take(2)
      .map(|v| v.parse().unwrap())
      .collect();
    return Duration::from_millis((f[0] + f[1]) * 10);
  }
  // macOS: `[hh:]mm:ss.cc`.
  let out = std::process::Command::new("ps")
    .args(["-o", "time=", "-p", &std::process::id().to_string()])
    .output()
    .unwrap();
  let text = String::from_utf8(out.stdout).unwrap();
  let secs = text
    .trim()
    .split(':')
    .fold(0.0, |acc, part| acc * 60.0 + part.parse::<f64>().unwrap());
  Duration::from_secs_f64(secs)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_screen_quiet_for_long_that_does_not_match_is_waited_on_idle() {
  // macOS reads the CPU time with `ps`, which an agent's sandbox refuses.
  if std::fs::metadata("/proc/self/stat").is_err()
    && !std::process::Command::new("ps")
      .args(["-p", &std::process::id().to_string()])
      .output()
      .is_ok_and(|o| o.status.success())
  {
    eprintln!("ps does not run here (a sandbox?): skipped");
    return;
  }
  let s = Session::spawn(Config::new(std::env::temp_dir()))
    .await
    .unwrap();
  s.run("echo idle", Wait::prompt(Duration::from_secs(10)))
    .await
    .unwrap();
  // Quiet for longer than asked, and never showing the pattern: once the
  // quiet held, the wait spun until its deadline.
  tokio::time::sleep(Duration::from_millis(300)).await;
  let until = Until::Screen(ScreenUntil {
    shows: Some(Regex::new("NEVER-SHOWN").unwrap()),
    quiet: Some(Duration::from_millis(100)),
    ..ScreenUntil::default()
  });
  let before = cpu();
  let w = s
    .wait_for(&until, Some(Duration::from_secs(2)))
    .await
    .unwrap();
  let spent = cpu() - before;
  assert!(w.matched.is_none() && !w.done, "{w:?}");
  assert!(spent < Duration::from_millis(300), "{spent:?} of CPU");
  s.close().await;
}
