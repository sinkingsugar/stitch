//! Real shells on real PTYs.

use std::time::Duration;

use stitch_shell::{Config, Session, Status, Wait, key};

mod common;
use common::without_setpgid;

fn wait(secs: u64) -> Wait {
  Wait::prompt(Duration::from_secs(secs))
}

async fn session() -> Session {
  let dir = std::env::temp_dir();
  Session::spawn(Config::new(dir)).await.unwrap()
}

/// Ctrl+C until the prompt is back, as a person presses it again: one that
/// lands while bash starts a list's or a loop's next command is taken by
/// bash, and the command it forked runs on.
async fn interrupt(s: &Session) {
  for _ in 0..3 {
    let o = s.send(b"\x03", wait(2)).await.unwrap();
    if matches!(o.status, Status::AtPrompt { .. }) {
      return;
    }
  }
  panic!("still running: {:?}", s.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_bashs_setpgid_line_is_left_out() {
  let out = "bash: child setpgid (24452 to 24452): Operation not permitted\n    1030\n\
    bash: child setpgid (1 to 2): Operation not permitted.\n\
    zsh: child setpgid (1 to 2): Operation not permitted\n\
    bash: child setpgid (x to 2): Operation not permitted\n\
    starting\nbash: child setpgid (28728 to 28728): Operation not permitted";
  assert_eq!(
    without_setpgid(out),
    "    1030\n\
     bash: child setpgid (1 to 2): Operation not permitted.\n\
     zsh: child setpgid (1 to 2): Operation not permitted\n\
     bash: child setpgid (x to 2): Operation not permitted\n\
     starting"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_report_output_and_exit_status() {
  let s = session().await;
  let o = s.run("echo hello; echo world", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "hello\nworld");
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  let o = s.run("false", wait(10)).await.unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 1 });
  assert_eq!(without_setpgid(&o.output), "");
  // `!` is a character, not a history reference.
  let o = s.run("echo \"hi!\" 'there!'", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "hi! there!");
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  // A line the shell rejects reports a failure, not the previous status.
  let o = s.run("echo ok; if then fi", wait(10)).await.unwrap();
  assert!(
    matches!(o.status, Status::AtPrompt { exit } if exit != 0),
    "{o:?}"
  );
  let o = s.run("no-such-command-here", wait(10)).await.unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 127 });
  // Output without a final newline is kept.
  let o = s.run("printf 'no newline'", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "no newline");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_keeps_its_directory_and_environment() {
  let s = session().await;
  let dir = tempfile::tempdir().unwrap();
  let canonical = std::fs::canonicalize(dir.path()).unwrap();
  s.run(&format!("cd '{}'", canonical.display()), wait(10))
    .await
    .unwrap();
  s.run("export STITCH_TEST_VAR=kept", wait(10))
    .await
    .unwrap();
  let o = s.run("pwd; echo $STITCH_TEST_VAR", wait(10)).await.unwrap();
  assert_eq!(
    without_setpgid(&o.output),
    format!("{}\nkept", canonical.display())
  );
  // The marker's secret is not visible to commands.
  let o = s
    .run("echo \"[$STITCH_SHELL_NONCE]\"", wait(10))
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "[]");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_bars_and_colours_leave_their_last_state() {
  let s = session().await;
  let o = s
    .run(
      r"printf '\033[1mbold\033[0m\n10%%\r50%%\r100%%\n'",
      wait(10),
    )
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "bold\n100%");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_asks_is_answered_with_send() {
  let s = session().await;
  let asking = Wait {
    timeout: Duration::from_secs(10),
    idle: Some(Duration::from_millis(500)),
    screen: None,
  };
  let o = s
    .run("read -p 'name? ' n; echo \"hi $n\"", asking.clone())
    .await
    .unwrap();
  // `read` is a builtin: the shell itself holds the terminal, but its
  // prompt has not come back, so it is still running, not at the prompt.
  assert!(o.output.ends_with("name?"), "{o:?}");
  assert!(
    matches!(&o.status, Status::Running { program, .. } if program.starts_with("bash (the shell itself")),
    "{o:?}"
  );
  assert!(
    s.run("echo nope", wait(1)).await.is_err(),
    "busy while read waits"
  );
  let o = s.send(b"bob\r", wait(10)).await.unwrap();
  assert_eq!(
    without_setpgid(&o.output).lines().last(),
    Some("hi bob"),
    "{o:?}"
  );
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });

  // A program (not a builtin) that asks shows as running.
  let o = s
    .run(
      "sh -c 'printf \"continue? \"; read a; echo \"got $a\"'",
      asking,
    )
    .await
    .unwrap();
  assert!(
    matches!(&o.status, Status::Running { program, .. } if program == "sh"),
    "{o:?}"
  );
  assert!(s.run("echo nope", wait(1)).await.is_err(), "busy");
  let o = s.send(b"yes\r", wait(10)).await.unwrap();
  assert!(o.output.contains("got yes"), "{o:?}");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_program_is_reported_running_and_interrupted() {
  let s = session().await;
  let o = s.run("echo start; sleep 100", wait(1)).await.unwrap();
  assert!(
    matches!(&o.status, Status::Running { program, .. } if program == "sleep"),
    "{o:?}"
  );
  assert_eq!(without_setpgid(&o.output), "start");
  let o = s.send(key("ctrl-c").unwrap(), wait(10)).await.unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 130 });
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_is_stopped_however_many_ctrl_c_it_takes_and_only_its_own() {
  use stitch_shell::Until;
  let s = session().await;
  let before = s.command().map(|c| c.started);
  // It takes its first Ctrl+C (the trap), and dies of the second.
  s.run(
    "bash -c 'trap \"trap - INT\" INT; echo armed; while :; do sleep 0.05; done'",
    Wait::prompt(Duration::ZERO),
  )
  .await
  .unwrap();
  let armed = regex::Regex::new("^armed").unwrap();
  let w = s
    .wait_for(&Until::Line(armed), Some(Duration::from_secs(10)))
    .await
    .unwrap();
  assert!(w.matched.is_some(), "{w:?}");
  // Entered before the caller's own: not the caller's to stop.
  let own = s.command().map(|c| c.started);
  assert!(
    s.stop_command(own, 3, Duration::from_secs(2))
      .await
      .unwrap()
  );
  assert!(s.command().unwrap().running);
  // Once is not enough, and that is said.
  assert!(
    !s.stop_command(before, 1, Duration::from_secs(2))
      .await
      .unwrap()
  );
  assert!(s.command().unwrap().running);
  // Again, within the bound: stopped, and seen to be.
  assert!(
    s.stop_command(before, 3, Duration::from_secs(2))
      .await
      .unwrap()
  );
  assert!(!s.command().unwrap().running);
  assert!(
    matches!(s.status(), Status::AtPrompt { .. }),
    "{:?}",
    s.status()
  );
  // Nothing runs: nothing is sent.
  assert!(
    s.stop_command(before, 3, Duration::from_secs(2))
      .await
      .unwrap()
  );
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn full_screen_programs_are_read_from_the_screen() {
  let s = session().await;
  let program = r"printf '\033[?1049h\033[2J\033[5;10HFULL SCREEN'; read x; printf '\033[?1049l'";
  let o = s
    .run(
      program,
      Wait {
        timeout: Duration::from_secs(10),
        idle: Some(Duration::from_millis(500)),
        screen: None,
      },
    )
    .await
    .unwrap();
  let screen = s.screen();
  assert!(screen.alternate);
  assert!(
    screen.text.lines().nth(4).unwrap().contains("FULL SCREEN"),
    "{}",
    screen.text
  );
  // The shell's own `read` holds it: still running, and the outcome
  // carries the full screen.
  assert!(matches!(o.status, Status::Running { .. }), "{o:?}");
  assert!(
    o.screen.as_deref().unwrap_or("").contains("FULL SCREEN"),
    "{o:?}"
  );
  let o = s.send(b"\r", wait(10)).await.unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  assert!(!s.screen().alternate);
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn output_cannot_fake_a_prompt() {
  let s = session().await;
  let o = s
    .run(
      r"printf '\033]777;stitch;guess;prompt;0\007'; sleep 1; echo real",
      wait(10),
    )
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "real");
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn long_output_is_cut_in_the_middle() {
  let s = session().await;
  // A builtin loop: no job is started, so nothing (a `setpgid` warning)
  // comes before the numbers.
  let o = s
    .run(
      "for ((i = 1; i <= 100000; i++)); do echo $i; done",
      wait(60),
    )
    .await
    .unwrap();
  let head: String = o.output.chars().take(80).collect();
  let tail: String = {
    let chars: Vec<char> = o.output.chars().collect();
    chars[chars.len().saturating_sub(80)..].iter().collect()
  };
  let seen = format!(
    "status {:?} after {:?}; head {head:?}; tail {tail:?}",
    o.status, o.elapsed
  );
  // Finished first: a wait that ran out returns what was printed so far.
  assert_eq!(o.status, Status::AtPrompt { exit: 0 }, "{seen}");
  assert!(o.truncated, "{seen}");
  assert!(o.output.starts_with("1\n2\n"), "{seen}");
  assert!(o.output.ends_with("99999\n100000"), "{seen}");
  assert!(o.output.contains("lines omitted"), "{seen}");
  assert!(o.output.chars().count() < 31_000, "{seen}");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_ends_the_shell_and_its_jobs() {
  let s = session().await;
  // Tagged: bash may say more (macOS now and then fails a job's
  // `setpgid`, and bash prints that after the PID).
  let o = s
    .run("sleep 1000 & echo \"pid=$!\"", wait(10))
    .await
    .unwrap();
  let pid: i32 = o
    .output
    .lines()
    .find_map(|l| l.trim().strip_prefix("pid="))
    .and_then(|p| p.parse().ok())
    .unwrap_or_else(|| panic!("no pid in {:?}", o.output));
  let alive = |pid: i32| {
    std::process::Command::new("kill")
      .args(["-0", &pid.to_string()])
      .status()
      .unwrap()
      .success()
  };
  assert!(alive(pid));
  s.close().await;
  for _ in 0..50 {
    if !alive(pid) {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert!(!alive(pid), "the background job outlived the session");
  assert_eq!(s.status(), Status::Closed);
  assert!(s.run("echo", wait(1)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_dropped_without_closing_do_not_hang() {
  for _ in 0..5 {
    let s = session().await;
    s.run("sleep 30 &", wait(10)).await.unwrap();
    drop(s);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_terminal_is_the_shells_controlling_terminal() {
  // `ps` reads the terminal; an agent's sandbox refuses it.
  if !std::process::Command::new("ps")
    .args(["-p", &std::process::id().to_string()])
    .output()
    .is_ok_and(|o| o.status.success())
  {
    eprintln!("ps does not run here (a sandbox?): skipped");
    return;
  }
  let s = session().await;
  let o = s
    .run(
      "[ \"/dev/$(ps -o tty= -p $$ | tr -d ' ')\" = \"$(tty)\" ] && echo controlling",
      wait(10),
    )
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "controlling");
  // Its signals are at their defaults, though the node ignores SIGPIPE.
  let o = s
    .run("yes | head -1; echo ${PIPESTATUS[0]}", wait(10))
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "y\n141");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_environment_is_the_clients_alone() {
  let mut cfg = Config::new(std::env::temp_dir());
  cfg.env = vec![
    ("HOME".into(), std::env::var("HOME").unwrap()),
    ("PATH".into(), std::env::var("PATH").unwrap()),
    ("FROM_THE_CLIENT".into(), "yes".into()),
  ];
  cfg.clean_env = true;
  let s = Session::spawn(cfg).await.unwrap();
  let o = s.run("echo $FROM_THE_CLIENT", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "yes");
  // Cargo sets this for the test process; the shell does not have it.
  let o = s.run("echo ${CARGO:-unset}", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "unset");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_command_is_waited_for_without_typing() {
  use stitch_shell::Until;
  let s = session().await;
  // It ends when the test says so (the file there), not on a clock.
  let dir = tempfile::tempdir().unwrap();
  let go = dir.path().join("go");
  // Started, and left running.
  let o = s
    .run(
      &format!(
        "echo starting; sleep 1; echo 'listening on 8080'; \
         while [ ! -e '{}' ]; do sleep 0.05; done; echo bye",
        go.display()
      ),
      Wait {
        timeout: Duration::from_millis(300),
        idle: None,
        screen: None,
      },
    )
    .await
    .unwrap();
  assert!(matches!(o.status, Status::Running { .. }), "{o:?}");
  let c = s.command().unwrap();
  assert!(c.running && c.text.starts_with("echo starting"), "{c:?}");
  // Not yet: the time runs out first.
  let w = s
    .wait_for(&Until::Done, Some(Duration::from_millis(200)))
    .await
    .unwrap();
  assert!(!w.done && w.matched.is_none());
  // A line of it.
  let re = regex::Regex::new("listening on \\d+").unwrap();
  let w = s
    .wait_for(&Until::Line(re.clone()), Some(Duration::from_secs(10)))
    .await
    .unwrap();
  assert_eq!(w.matched.as_deref(), Some("listening on 8080"));
  assert!(!w.done);
  std::fs::write(&go, "").unwrap();
  // Its end, with all it printed.
  let w = s
    .wait_for(&Until::Done, Some(Duration::from_secs(10)))
    .await
    .unwrap();
  assert!(w.done);
  assert_eq!(w.outcome.status, Status::AtPrompt { exit: 0 });
  assert_eq!(
    without_setpgid(&w.outcome.output),
    "starting\nlistening on 8080\nbye"
  );
  assert!(!s.command().unwrap().running);
  // Once ended, both answer at once: the line was printed, it is done.
  let w = s
    .wait_for(&Until::Line(re), Some(Duration::from_secs(1)))
    .await
    .unwrap();
  assert_eq!(w.matched.as_deref(), Some("listening on 8080"));
  let w = s.wait_for(&Until::Done, None).await.unwrap();
  assert!(w.done);
  // A line it never prints: the end says so.
  s.run("echo nothing here", wait(10)).await.unwrap();
  let w = s
    .wait_for(
      &Until::Line(regex::Regex::new("never").unwrap()),
      Some(Duration::from_secs(5)),
    )
    .await
    .unwrap();
  assert!(w.done && w.matched.is_none());
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spinner_is_quiet_and_the_screen_can_end_a_wait() {
  use stitch_shell::{ScreenUntil, Until};
  let s = session().await;
  // A spinner and a ticking timer: output all the time, nothing new.
  let started = std::time::Instant::now();
  let o = s
    .run(
      "i=0; while true; do for c in ⠋ ⠙ ⠹ ⠸; do i=$((i+1)); printf '\\r%s thinking (%ss)' $c $i; sleep 0.1; done; done",
      Wait {
        timeout: Duration::from_secs(20),
        idle: Some(Duration::from_secs(1)),
        screen: None,
      },
    )
    .await
    .unwrap();
  assert!(matches!(o.status, Status::Running { .. }), "{o:?}");
  assert!(
    started.elapsed() < Duration::from_secs(5),
    "{:?}",
    started.elapsed()
  );
  interrupt(&s).await;

  // What the screen shows ends a wait, prompt or not.
  let started = std::time::Instant::now();
  let o = s
    .run(
      "sleep 1; echo READY-$((6*7)); sleep 30",
      Wait {
        timeout: Duration::from_secs(20),
        idle: None,
        screen: Some(regex::Regex::new("READY-42").unwrap()),
      },
    )
    .await
    .unwrap();
  assert!(matches!(o.status, Status::Running { .. }), "{o:?}");
  assert!(started.elapsed() < Duration::from_secs(5));
  assert_eq!(without_setpgid(&o.output), "READY-42");

  // Screen conditions, all at once: it shows one thing, not another, and
  // has been quiet.
  let on = |shows: Option<&str>, gone: Option<&str>, quiet: Option<u64>| {
    Until::Screen(ScreenUntil {
      shows: shows.map(|p| regex::Regex::new(p).unwrap()),
      gone: gone.map(|p| regex::Regex::new(p).unwrap()),
      quiet: quiet.map(Duration::from_millis),
    })
  };
  let w = s
    .wait_for(
      &on(Some("READY-\\d+"), None, Some(500)),
      Some(Duration::from_secs(5)),
    )
    .await
    .unwrap();
  assert_eq!(w.matched.as_deref(), Some("READY-42"));
  let w = s
    .wait_for(
      &on(None, Some("READY"), None),
      Some(Duration::from_millis(500)),
    )
    .await
    .unwrap();
  assert!(w.matched.is_none() && !w.done, "still shown: not yet");
  interrupt(&s).await;
  // Shown after the last input, then cleared: gone.
  s.run(
    "printf 'REA%s\\n' DY-2; sleep 1; clear",
    Wait::prompt(Duration::from_millis(100)),
  )
  .await
  .unwrap();
  let w = s
    .wait_for(&on(None, Some("READY"), None), Some(Duration::from_secs(5)))
    .await
    .unwrap();
  assert!(w.matched.is_some(), "gone after the clear");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_screen_match_must_be_news() {
  let s = session().await;
  let screen = |p: &str| Wait {
    timeout: Duration::from_secs(10),
    idle: None,
    screen: Some(regex::Regex::new(p).unwrap()),
  };
  // Already on screen: what else changes (the echo, a spinner) is no answer.
  s.run("echo READY-1", wait(5)).await.unwrap();
  let started = std::time::Instant::now();
  let o = s
    .run("sleep 1; echo READY-2; sleep 30", screen("READY-\\d"))
    .await
    .unwrap();
  assert!(
    started.elapsed() >= Duration::from_millis(900),
    "fired on the old line"
  );
  assert_eq!(without_setpgid(&o.output), "READY-2");
  interrupt(&s).await;
  // A program that answers at once: its answer is not taken for the screen
  // as it was.
  s.run(
    "read x; echo GOT-$x; sleep 30",
    Wait::prompt(Duration::from_millis(300)),
  )
  .await
  .unwrap();
  let started = std::time::Instant::now();
  s.send(b"a\r", screen("GOT-a")).await.unwrap();
  assert!(
    started.elapsed() < Duration::from_secs(3),
    "the answer was missed"
  );
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_is_no_quiet_but_a_spinner_is() {
  let s = session().await;
  // A percentage on a line of its own: progress, not quiet.
  let started = std::time::Instant::now();
  s.run(
    "i=0; while true; do i=$((i+1)); printf '\\rdownloading %s%%' $i; sleep 0.1; done",
    Wait {
      timeout: Duration::from_secs(3),
      idle: Some(Duration::from_secs(1)),
      screen: None,
    },
  )
  .await
  .unwrap();
  assert!(
    started.elapsed() >= Duration::from_millis(2900),
    "quiet while it counted"
  );
  interrupt(&s).await;
  // Progress on a line that a bullet starts, or that shows one, or that
  // reads like a list item: no spinner's, no quiet.
  for line in [
    "• downloading %s%%",
    "copied · %s files",
    "  - step %s",
    "- %s/1000 done",
  ] {
    let started = std::time::Instant::now();
    s.run(
      &format!("i=0; while true; do i=$((i+1)); printf '\\r{line}' $i; sleep 0.1; done"),
      Wait {
        timeout: Duration::from_secs(3),
        idle: Some(Duration::from_secs(1)),
        screen: None,
      },
    )
    .await
    .unwrap();
    assert!(
      started.elapsed() >= Duration::from_millis(2900),
      "quiet while it counted: {line}"
    );
    interrupt(&s).await;
  }
  // An ASCII spinner with a timer: quiet.
  let started = std::time::Instant::now();
  s.run(
    "i=0; while true; do for c in '|' / - '\\'; do i=$((i+1)); printf '\\r%s working %ss' \"$c\" $i; sleep 0.1; done; done",
    Wait {
      timeout: Duration::from_secs(10),
      idle: Some(Duration::from_secs(1)),
      screen: None,
    },
  )
  .await
  .unwrap();
  assert!(
    started.elapsed() < Duration::from_secs(4),
    "{:?}",
    started.elapsed()
  );
  interrupt(&s).await;
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gone_means_it_was_there() {
  use stitch_shell::{ScreenUntil, Until};
  let s = session().await;
  let gone = || {
    Until::Screen(ScreenUntil {
      gone: Some(regex::Regex::new("WORKING").unwrap()),
      ..ScreenUntil::default()
    })
  };
  // Never shown: not gone, just not there yet.
  let w = s
    .wait_for(&gone(), Some(Duration::from_millis(800)))
    .await
    .unwrap();
  assert!(w.matched.is_none(), "{w:?}");
  // Shown, then cleared: gone. (The pattern is kept out of the command:
  // its echo shows nothing.)
  s.run(
    "printf 'WOR%s\\n' KING; sleep 1; clear",
    Wait::prompt(Duration::from_millis(200)),
  )
  .await
  .unwrap();
  let w = s
    .wait_for(&gone(), Some(Duration::from_secs(5)))
    .await
    .unwrap();
  assert!(w.matched.is_some(), "{w:?}");
  // Gone at `clear`, a moment before the prompt: the next command waits
  // for it (under load that moment was long enough for `run` to refuse).
  s.wait_for(&Until::Done, Some(Duration::from_secs(5)))
    .await
    .unwrap();
  // Only in the command's echo: never shown, so not gone.
  s.run(
    "true WORKING; sleep 1; clear",
    Wait::prompt(Duration::from_millis(200)),
  )
  .await
  .unwrap();
  let w = s
    .wait_for(&gone(), Some(Duration::from_secs(3)))
    .await
    .unwrap();
  assert!(w.matched.is_none(), "{w:?}");
  s.wait_for(&Until::Done, Some(Duration::from_secs(5)))
    .await
    .unwrap();
  // Row by row: a pattern for a row whole.
  s.run(
    "printf 'WOR%s\\n' KING; sleep 1; clear",
    Wait::prompt(Duration::from_millis(200)),
  )
  .await
  .unwrap();
  let w = s
    .wait_for(
      &Until::Screen(ScreenUntil {
        gone: Some(regex::Regex::new("^WORKING$").unwrap()),
        ..ScreenUntil::default()
      }),
      Some(Duration::from_secs(5)),
    )
    .await
    .unwrap();
  assert!(w.matched.is_some(), "{w:?}");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrapped_echo_of_what_was_sent_is_no_news() {
  use stitch_shell::{ScreenUntil, Until};
  let s = session().await;
  // A program like an agent's input box: it reads a message, shows it
  // again wrapped in a box of its own, works, then answers.
  s.run(
    "read -r m; sleep 0.3; printf '%s\\n' \"$m\" | fold -s -w 40 | sed 's/^/  │ /'; \
     sleep 2; echo '⏺ All done: STEP1-DONE'; sleep 30",
    Wait::prompt(Duration::from_millis(300)),
  )
  .await
  .unwrap();
  // Long enough that the terminal wraps its echo too (120 columns).
  let message = "Please read the design document and the tests first, then fix the \
    wrapping bug in the screen waits; when the first step is finished say \
    STEP1-DONE on a line of its own, and wait for the next instruction before \
    going on with anything else at all.";
  // Typed, then entered on its own, as `submit` does.
  s.send(message.as_bytes(), Wait::prompt(Duration::ZERO))
    .await
    .unwrap();
  tokio::time::sleep(Duration::from_millis(300)).await;
  let started = std::time::Instant::now();
  let o = s
    .send(
      b"\r",
      Wait {
        timeout: Duration::from_secs(10),
        idle: None,
        screen: Some(regex::Regex::new("STEP1-DONE").unwrap()),
      },
    )
    .await
    .unwrap();
  assert!(
    started.elapsed() >= Duration::from_millis(1800),
    "fired on the echo: {:?}",
    o.screen
  );
  assert!(s.screen().text.contains("⏺ All done"), "{:?}", o.screen);
  // A watch set after the answer fires at once on it; set before, only on
  // the answer, not on the message shown again.
  let w = s
    .wait_for(
      &Until::Screen(ScreenUntil {
        shows: Some(regex::Regex::new("STEP1-DONE").unwrap()),
        ..ScreenUntil::default()
      }),
      Some(Duration::from_secs(2)),
    )
    .await
    .unwrap();
  assert_eq!(
    w.matched.as_deref(),
    Some("⏺ All done: STEP1-DONE"),
    "{w:?}"
  );
  interrupt(&s).await;
  s.run(
    "read -r m; sleep 0.3; printf '%s\\n' \"$m\" | fold -s -w 40; sleep 2; echo STEP2-DONE; sleep 30",
    Wait::prompt(Duration::from_millis(300)),
  )
  .await
  .unwrap();
  let message = message.replace("STEP1", "STEP2");
  s.send(
    format!("{message}\r").as_bytes(),
    Wait::prompt(Duration::ZERO),
  )
  .await
  .unwrap();
  let started = std::time::Instant::now();
  let w = s
    .wait_for(
      &Until::Screen(ScreenUntil {
        shows: Some(regex::Regex::new("STEP2-DONE").unwrap()),
        ..ScreenUntil::default()
      }),
      Some(Duration::from_secs(10)),
    )
    .await
    .unwrap();
  assert!(
    started.elapsed() >= Duration::from_millis(1500),
    "the watch fired on the echo: {w:?}"
  );
  assert_eq!(w.matched.as_deref(), Some("STEP2-DONE"), "{w:?}");
  interrupt(&s).await;
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_command_runs_whole() {
  let s = session().await;
  // Typed, a line past 1024 bytes was cut at 1019 and lost its Enter.
  for n in [1030, 2000, 5000] {
    let o = s
      .run(&format!("printf %s {} | wc -c", "x".repeat(n)), wait(10))
      .await
      .unwrap();
    assert_eq!(without_setpgid(&o.output).trim(), n.to_string(), "{n}");
    assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  }
  // Many lines, run in this shell as typed: what they set stays.
  let lines: Vec<String> = (0..60)
    .map(|i| format!("STITCH_LONG_{i}=value-number-{i}"))
    .collect();
  let o = s
    .run(
      &format!("{}\necho \"$STITCH_LONG_59\"; false", lines.join("\n")),
      wait(10),
    )
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "value-number-59");
  assert_eq!(o.status, Status::AtPrompt { exit: 1 });
  let o = s.run("echo \"$STITCH_LONG_0\"", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "value-number-0");
  // Named by what it runs, not by how it was entered.
  let long = format!("sleep 0.5; : {}", "y".repeat(1500));
  s.run(&long, Wait::prompt(Duration::ZERO)).await.unwrap();
  assert_eq!(s.command().unwrap().text, long);
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn input_never_goes_on_top_of_a_persons_typing() {
  let s = session().await;
  // Typed (attached) and not told yet: a command is not typed after it,
  // and what they typed comes back with the refusal.
  s.type_in(b"echo the users").await.unwrap();
  match s.run("echo the agents", wait(10)).await {
    Err(stitch_shell::Error::Typed(t)) => assert_eq!(t, "echo the users"),
    other => panic!("{other:?}"),
  }
  s.type_in(b" words").await.unwrap();
  assert!(matches!(
    s.send_minding(b"x", wait(1)).await,
    Err(stitch_shell::Error::Typed(t)) if t == " words"
  ));
  // Told: their line is theirs to finish; the agent's goes after.
  let o = s.send(b"\r", wait(10)).await.unwrap();
  assert!(o.output.contains("the users words"), "{o:?}");
  let o = s.run("echo the agents", wait(10)).await.unwrap();
  assert_eq!(without_setpgid(&o.output), "the agents");
  s.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_takes_no_input_does_not_hold_the_session() {
  let s = session().await;
  // Raw and not reading: its terminal takes no more once full.
  s.run(
    "stty raw -echo; echo RAW; sleep 60",
    Wait::prompt(Duration::ZERO),
  )
  .await
  .unwrap();
  let raw = regex::Regex::new("^RAW").unwrap();
  let w = s
    .wait_for(
      &stitch_shell::Until::Line(raw),
      Some(Duration::from_secs(10)),
    )
    .await
    .unwrap();
  assert!(w.matched.is_some(), "{w:?}");
  let big = vec![b'a'; 1 << 20];
  let sent = tokio::time::timeout(
    Duration::from_secs(15),
    s.send(&big, Wait::prompt(Duration::ZERO)),
  )
  .await
  .expect("the write gave up");
  assert!(
    matches!(sent, Err(stitch_shell::Error::Stalled(_))),
    "{sent:?}"
  );
  // The session's input is free again (the write let go of it).
  let again = tokio::time::timeout(
    Duration::from_secs(15),
    s.send(b"\x03", Wait::prompt(Duration::ZERO)),
  )
  .await;
  assert!(again.is_ok(), "the next write was not held up for good");
  s.close().await;
  // A call cut short still writes what it started: the command runs, and
  // the session is not left busy with a line it never got.
  let s = session().await;
  let _ = tokio::time::timeout(
    Duration::from_millis(1),
    s.run("sleep 0.3; echo cut", wait(10)),
  )
  .await;
  let mut free = false;
  for _ in 0..50 {
    if matches!(s.status(), Status::AtPrompt { .. }) {
      free = true;
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  assert!(free, "{:?}", s.status());
  assert_eq!(s.command().unwrap().text, "sleep 0.3; echo cut");
  s.close().await;
}

/// The node's copy of the terminal's master is close-on-exec: a shell (or
/// a program it runs) holding it would keep a closed session's terminal
/// from hanging up. (macOS spawns with every descriptor closed but the
/// terminal.)
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn the_shell_holds_no_copy_of_the_terminals_master() {
  let s = session().await;
  let o = s
    .run(
      "for f in /proc/$$/fd/*; do readlink \"$f\"; done | grep -c ptmx",
      wait(10),
    )
    .await
    .unwrap();
  assert_eq!(without_setpgid(&o.output), "0");
  s.close().await;
}

/// Output that repeats part of the command is news: only the lines where
/// the input was echoed are its echo (the prompt line, and what follows
/// it directly as more of the same text).
#[tokio::test(flavor = "multi_thread")]
async fn output_repeating_the_command_is_news() {
  let s = session().await;
  let screen = |p: &str| Wait {
    timeout: Duration::from_secs(8),
    idle: None,
    screen: Some(regex::Regex::new(p).unwrap()),
  };
  let fired_on_output = |o: &stitch_shell::Outcome, started: std::time::Instant| {
    assert!(
      started.elapsed() < Duration::from_secs(6),
      "timed out: {o:?}"
    );
    assert!(
      started.elapsed() >= Duration::from_millis(900),
      "fired on the echo: {o:?}"
    );
  };
  // A program that names itself in its errors (`./build: Error …`,
  // `make: *** Error 2`).
  s.run(
    "build() { sleep 1; echo \"build: Error in step 2\"; sleep 30; }",
    wait(5),
  )
  .await
  .unwrap();
  let started = std::time::Instant::now();
  let o = s.run("build", screen("Error")).await.unwrap();
  fired_on_output(&o, started);
  interrupt(&s).await;
  // A long stretch of the command, printed.
  let started = std::time::Instant::now();
  let o = s
    .run(
      "sleep 1; echo 'All tests passed successfully'; sleep 30",
      screen("passed"),
    )
    .await
    .unwrap();
  fired_on_output(&o, started);
  interrupt(&s).await;
  // Short lines that together are a stretch of it.
  let started = std::time::Instant::now();
  let o = s
    .run(
      "sleep 1; for i in 1 2 3; do echo $i; done; sleep 30",
      screen("^3$"),
    )
    .await
    .unwrap();
  fired_on_output(&o, started);
  interrupt(&s).await;
  s.close().await;
}

/// Two commands at once on one session: one is typed, the other is told
/// a program is running (it is checked as the input is written, under the
/// lock the other takes).
#[tokio::test(flavor = "multi_thread")]
async fn two_commands_at_once_are_not_both_typed() {
  let s = session().await;
  for _ in 0..5 {
    let (a, b) = tokio::join!(
      s.run("sleep 0.5; echo first", wait(10)),
      s.run("echo second", wait(10)),
    );
    let busy = |r: &stitch_shell::Result<stitch_shell::Outcome>| {
      matches!(r, Err(stitch_shell::Error::Busy { .. }))
    };
    assert!(busy(&a) != busy(&b), "{a:?} {b:?}");
    let ran = if busy(&a) { b } else { a }.unwrap();
    assert!(
      matches!(ran.status, Status::AtPrompt { exit: 0 }),
      "{ran:?}"
    );
  }
  s.close().await;
}

/// A long command is sourced from a file of its own, gone once it ended.
#[tokio::test(flavor = "multi_thread")]
async fn a_long_commands_file_is_its_own_and_removed() {
  let s = session().await;
  let long = |n: u32| format!("echo \"$BASH_SOURCE\"; : {n} {}", "z".repeat(1000));
  let a = s.run(&long(1), wait(10)).await.unwrap();
  let b = s.run(&long(2), wait(10)).await.unwrap();
  let (a, b) = (without_setpgid(&a.output), without_setpgid(&b.output));
  assert!(!a.is_empty() && a != b, "{a} {b}");
  // Removed as its prompt came back (the reader saw it).
  for file in [a, b] {
    let mut gone = false;
    for _ in 0..20 {
      if !std::path::Path::new(&file).exists() {
        gone = true;
        break;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "{file} is still there");
  }
  s.close().await;
}

/// What a person types into a program that takes no input gives up, as
/// an agent's input does, and leaves the session's input free.
#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_program_that_takes_none_gives_up() {
  let s = session().await;
  s.run(
    "stty raw -echo; echo RAW; sleep 60",
    Wait::prompt(Duration::ZERO),
  )
  .await
  .unwrap();
  let w = s
    .wait_for(
      &stitch_shell::Until::Line(regex::Regex::new("^RAW").unwrap()),
      Some(Duration::from_secs(10)),
    )
    .await
    .unwrap();
  assert!(w.matched.is_some(), "{w:?}");
  let typed = tokio::time::timeout(Duration::from_secs(15), s.type_in(&vec![b'a'; 1 << 20]))
    .await
    .expect("typing held on");
  assert!(
    matches!(typed, Err(stitch_shell::Error::Stalled(_))),
    "{typed:?}"
  );
  let again = tokio::time::timeout(
    Duration::from_secs(15),
    s.send(b"\x03", Wait::prompt(Duration::ZERO)),
  )
  .await;
  assert!(again.is_ok(), "the next write was held up");
  s.close().await;
}

/// Issue 2026-11-01T205409Z-gamer-g749: a short multi-line command was typed
/// line by line, and the first line's prompt ended the wait.
#[tokio::test(flavor = "multi_thread")]
async fn multi_line_command_runs_as_one() {
  let s = session().await;
  let o = s.run("echo a\nsleep 1\necho b", wait(20)).await.unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  assert_eq!(without_setpgid(&o.output), "a\nb");
  let o = s
    .run("cat <<'EOF'\nin doc\nEOF\nsleep 1\necho after", wait(20))
    .await
    .unwrap();
  assert_eq!(o.status, Status::AtPrompt { exit: 0 });
  assert_eq!(without_setpgid(&o.output), "in doc\nafter");
}
