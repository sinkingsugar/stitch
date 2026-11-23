//! deploy/node/self-update.sh against a git repository of its own (Stitch
//! with an origin; Otter and engram as its Cargo.lock pins them), a
//! `stitch` and a `just` that only say what they are asked: it builds only
//! what origin's branch has (not a main ahead or diverged, not while `just
//! dev-link` is active), remembers a failed build, promotes the exact
//! commit once healthy, and tries TestFlight again until it uploaded. And
//! the move from three checkouts (Stitch, ../otter, ../engram): the script
//! installed before it runs once more, and hands over to this one. The app
//! in its own checkout (STITCH_APP): held to the same rules, promoted with
//! Stitch, its justfile run by path for TestFlight.

use std::path::{Path, PathBuf};
use std::process::Command;

struct World {
  root: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> String {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .env("GIT_AUTHOR_NAME", "t")
    .env("GIT_AUTHOR_EMAIL", "t@t")
    .env("GIT_COMMITTER_NAME", "t")
    .env("GIT_COMMITTER_EMAIL", "t@t")
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {args:?}: {}",
    String::from_utf8_lossy(&out.stderr)
  );
  String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(p: &Path, text: &str, exec: bool) {
  std::fs::create_dir_all(p.parent().unwrap()).unwrap();
  std::fs::write(p, text).unwrap();
  if exec {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
  }
}

const STITCH: &str = r#"#!/bin/sh
[ "$1" = --version ] && { echo "stitch 0.1.0 (@LABEL@)"; exit 0; }
p=
for a in "$@"; do
  # $HOME/healthy: the seconds healthy (empty: forever).
  if [ "$p" = --healthy-for ]; then
    [ -f "$HOME/healthy" ] || exit 1
    s=$(cat "$HOME/healthy")
    [ -z "$s" ] || [ "$s" -ge "$a" ]
    exit $?
  fi
  p=$a
  [ "$a" = notify ] && { echo "$*" >> "$HOME/notify.log"; exit 0; }
done
exit 0
"#;

const INSTALL: &str = r#"#!/bin/sh
# The build: fails when asked to, else a `stitch` that says its label, as
# the real build does (build.rs, tools/build-label.sh).
[ -f "$HOME/fail-build" ] && { echo "build failed"; exit 1; }
l="$(tools/build-label.sh .)"
mkdir -p "$HOME/.cargo/bin"
sed "s/@LABEL@/$l/" "$HOME/stitch.in" > "$HOME/.cargo/bin/stitch"
chmod +x "$HOME/.cargo/bin/stitch"
echo "built $l"
"#;

const JUST: &str = r#"#!/bin/sh
echo "$* (STITCH_DIR=$STITCH_DIR)" >> "$HOME/just.log"
for a in "$@"; do
  [ "$a" = testflight ] && [ -f "$HOME/tf-fail" ] && exit 1
done
exit 0
"#;

/// A Cargo.lock that pins Otter and engram, as the real one does.
fn lock(otter: &str, engram: &str) -> String {
  format!(
    "version = 4\n\n[[package]]\nname = \"engram\"\nversion = \"0.1.0\"\nsource = \"git+https://github.com/sinkingsugar/engram#{engram}\"\n\n[[package]]\nname = \"otter-core\"\nversion = \"0.1.0\"\nsource = \"git+https://github.com/sinkingsugar/otter#{otter}\"\n"
  )
}

const OTTER_PIN: &str = "b3d75c341bdf7e0777241113072a3ebeefacc287";
const ENGRAM_PIN: &str = "a0087077e7c77abd6e503c5b746e398444a5f417";

impl World {
  /// Stitch alone, as it is now.
  fn new(name: &str) -> World {
    World::with(name, &["stitch"])
  }

  /// The repositories named, each with an origin; Stitch's with the real
  /// tools/build-label.sh and a Cargo.lock pinning Otter and engram.
  fn with(name: &str, repos: &[&str]) -> World {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
      .join(format!("stitch-selfupdate-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    write(&home.join("stitch.in"), STITCH, false);
    write(&home.join(".local/bin/just"), JUST, true);
    let w = World { root };
    for &name in repos {
      let origin = w.root.join(format!("origin/{name}.git"));
      std::fs::create_dir_all(&origin).unwrap();
      git(&origin, &["init", "--bare", "-q", "-b", "main"]);
      let dir = w.dir(name);
      std::fs::create_dir_all(&dir).unwrap();
      git(&dir, &["init", "-q", "-b", "main"]);
      git(&dir, &["remote", "add", "origin", origin.to_str().unwrap()]);
      write(&dir.join("README"), name, false);
      if name == "stitch" {
        write(&dir.join("deploy/node/install.sh"), INSTALL, true);
        let label = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/build-label.sh");
        write(
          &dir.join("tools/build-label.sh"),
          &std::fs::read_to_string(label).unwrap(),
          true,
        );
        write(&dir.join("Cargo.lock"), &lock(OTTER_PIN, ENGRAM_PIN), false);
      }
      git(&dir, &["add", "-A"]);
      git(&dir, &["commit", "-q", "-m", "start"]);
      git(&dir, &["push", "-q", "origin", "main"]);
    }
    w
  }

  fn dir(&self, name: &str) -> PathBuf {
    self.root.join("devel").join(name)
  }

  fn home(&self) -> PathBuf {
    self.root.join("home")
  }

  /// A commit on origin's main (through a clone of its own).
  fn upstream(&self, name: &str, file: &str, message: &str) -> String {
    let clone = self.root.join(format!("upstream-{name}"));
    if !clone.exists() {
      let origin = self.root.join(format!("origin/{name}.git"));
      git(
        &self.root,
        &[
          "clone",
          "-q",
          origin.to_str().unwrap(),
          clone.to_str().unwrap(),
        ],
      );
    }
    git(&clone, &["pull", "-q", "origin", "main"]);
    write(&clone.join(file), message, false);
    git(&clone, &["add", "-A"]);
    git(&clone, &["commit", "-q", "-m", message]);
    git(&clone, &["push", "-q", "origin", "main"]);
    git(&clone, &["rev-parse", "HEAD"])
  }

  fn run(&self, env: &[(&str, &str)], args: &[&str]) -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/node/self-update.sh");
    self.run_script(&script, env, args)
  }

  fn run_script(&self, script: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let out = Command::new("bash")
      .arg(script)
      .args(args)
      .env("HOME", self.home())
      .env("CARGO_HOME", self.home().join(".cargo"))
      .env("STITCH_CHECKOUT", self.dir("stitch"))
      .env("STITCH_DATA", self.root.join("data"))
      .env("STITCH_UPDATE_SETTLE", "0")
      .env("GIT_AUTHOR_NAME", "t")
      .env("GIT_AUTHOR_EMAIL", "t@t")
      .env("GIT_COMMITTER_NAME", "t")
      .env("GIT_COMMITTER_EMAIL", "t@t")
      .envs(env.iter().copied())
      .output()
      .unwrap();
    format!(
      "{}{}",
      String::from_utf8_lossy(&out.stdout),
      String::from_utf8_lossy(&out.stderr)
    )
  }

  fn installed(&self) -> String {
    std::fs::read_to_string(self.home().join(".cargo/bin/stitch")).unwrap_or_default()
  }

  fn branch(&self, name: &str, branch: &str) -> Option<String> {
    let origin = self.root.join(format!("origin/{name}.git"));
    Command::new("git")
      .arg("-C")
      .arg(origin)
      .args(["rev-parse", "--verify", "--quiet", branch])
      .output()
      .ok()
      .filter(|o| o.status.success())
      .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
  }
}

impl Drop for World {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.root);
  }
}

#[test]
fn self_update_builds_only_origin_and_promotes_what_ran_healthy() {
  let w = World::with("rollout", &["stitch", "app"]);
  let promote = [("STITCH_PROMOTE", "fleet")];

  // A first run: built and installed.
  let out = w.run(&promote, &[]);
  assert!(out.contains("installed: none"), "{out}");
  let out = w.run(&promote, &[]);
  assert!(out.contains("not promoting yet"), "{out}");
  let head = git(&w.dir("stitch"), &["rev-parse", "--short=12", "HEAD"]);
  assert!(w.installed().contains(&head), "{out}");
  assert_eq!(w.branch("stitch", "fleet"), None);

  // The label: Stitch's commit, Otter's and engram's pins.
  assert!(
    w.installed().contains(&format!(
      "{head} otter {} engram {}",
      &OTTER_PIN[..12],
      &ENGRAM_PIN[..12]
    )),
    "{}",
    w.installed()
  );

  // Healthy for its trial: the exact commit promoted.
  write(&w.home().join("healthy"), "", false);
  let out = w.run(&promote, &[]);
  assert!(out.contains("up to date"), "{out}");
  assert_eq!(
    w.branch("stitch", "fleet"),
    Some(git(&w.dir("stitch"), &["rev-parse", "HEAD"])),
    "{out}"
  );

  // A pin moved on origin (`just bump-otter`): built, and its label says so.
  w.upstream(
    "stitch",
    "Cargo.lock",
    &lock("1111111111112222222222223333333333334444", ENGRAM_PIN),
  );
  let out = w.run(&[], &[]);
  assert!(out.contains("built "), "{out}");
  assert!(
    w.installed().contains("otter 111111111111 engram"),
    "{}",
    w.installed()
  );
  let out = w.run(&[], &[]);
  assert!(out.contains("up to date"), "{out}");

  // `just dev-link` active: not built, said.
  write(
    &w.dir("stitch").join(".cargo/dev-link.toml"),
    "# link otter /x\n",
    false,
  );
  w.upstream("stitch", "README", "while linked");
  let before = w.installed();
  let out = w.run(&[], &[]);
  assert!(out.contains("dev-link is active"), "{out}");
  assert_eq!(w.installed(), before);
  std::fs::remove_file(w.dir("stitch").join(".cargo/dev-link.toml")).unwrap();
  let out = w.run(&[], &[]);
  assert!(out.contains("built "), "{out}");

  // A main ahead of origin (a commit of its own) is not built.
  write(&w.dir("stitch").join("local"), "mine", false);
  git(&w.dir("stitch"), &["add", "-A"]);
  git(&w.dir("stitch"), &["commit", "-q", "-m", "local work"]);
  let before = w.installed();
  let out = w.run(&[], &[]);
  assert!(out.contains("has commits origin/main has not"), "{out}");
  assert_eq!(w.installed(), before);
  // Diverged: said, nothing built.
  w.upstream("stitch", "up", "upstream work");
  let out = w.run(&[], &[]);
  assert!(out.contains("have diverged"), "{out}");
  assert_eq!(w.installed(), before);
  git(&w.dir("stitch"), &["reset", "-q", "--hard", "origin/main"]);

  // A build that fails is not tried again until a commit moves.
  write(&w.home().join("fail-build"), "", false);
  let out = w.run(&[], &[]);
  assert!(out.contains("failed"), "{out}");
  let out = w.run(&[], &[]);
  assert!(
    out.contains("not tried again until a commit moves"),
    "{out}"
  );
  std::fs::remove_file(w.home().join("fail-build")).unwrap();
  let out = w.run(&[], &["--force"]);
  assert!(out.contains("built "), "{out}");

  // Whatever origin's main holds is installed (what lands there was
  // approved there): a change to deploy/ included.
  w.upstream("stitch", "deploy/node/extra.sh", "landed");
  let out = w.run(&[], &[]);
  assert!(out.contains("built "), "{out}");

  // TestFlight: tried until it uploaded this commit, 3 times at most; the
  // app checkout's justfile, by path, against this checkout.
  let app = w.dir("app");
  let app = app.to_str().unwrap();
  let tf = [
    ("STITCH_TESTFLIGHT", "1"),
    ("STITCH_TESTFLIGHT_BACKOFF", "0"),
    ("STITCH_APP", app),
  ];
  let _ = w.run(&[], &[]);
  write(&w.home().join("tf-fail"), "", false);
  let out = w.run(&tf, &[]);
  assert!(out.contains("testflight failed (1 of 3)"), "{out}");
  std::fs::remove_file(w.home().join("tf-fail")).unwrap();
  let out = w.run(&tf, &[]);
  assert!(!out.contains("testflight failed"), "{out}");
  let log = std::fs::read_to_string(w.home().join("just.log")).unwrap();
  let core = w.dir("stitch");
  assert!(
    log.contains(&format!(
      "-f {app}/justfile testflight (STITCH_DIR={})",
      core.display()
    )),
    "{log}"
  );
  let uploads = log.matches("testflight").count();
  let _ = w.run(&tf, &[]);
  let log = std::fs::read_to_string(w.home().join("just.log")).unwrap();
  assert_eq!(log.matches("testflight").count(), uploads, "{log}");
  // A commit that never uploads: 3 tries, then the user is told.
  w.upstream("stitch", "README", "next");
  let _ = w.run(&[], &[]);
  write(&w.home().join("tf-fail"), "", false);
  for _ in 0..5 {
    let _ = w.run(&tf, &[]);
  }
  let log = std::fs::read_to_string(w.home().join("just.log")).unwrap();
  assert_eq!(log.matches("testflight").count(), uploads + 3, "{log}");
  let told = std::fs::read_to_string(w.home().join(".cache/stitch-self-update.notified")).unwrap();
  assert!(told.contains("testflight"), "{told}");

  // The app moved alone: a new pair, uploaded (no build of Stitch).
  std::fs::remove_file(w.home().join("tf-fail")).unwrap();
  let before = std::fs::read_to_string(w.home().join("just.log"))
    .unwrap()
    .matches("testflight")
    .count();
  w.upstream("app", "README", "app change");
  let out = w.run(&tf, &[]);
  assert!(out.contains("up to date"), "{out}");
  let log = std::fs::read_to_string(w.home().join("just.log")).unwrap();
  assert_eq!(
    log.matches("testflight").count(),
    before + 1,
    "{out}\n{log}"
  );
  assert_eq!(
    git(app.as_ref(), &["rev-parse", "HEAD"]),
    w.branch("app", "main").unwrap()
  );

  // Promoted with Stitch, the app's exact commit too.
  let out = w.run(&[("STITCH_PROMOTE", "fleet"), ("STITCH_APP", app)], &[]);
  assert_eq!(
    w.branch("app", "fleet"),
    Some(git(app.as_ref(), &["rev-parse", "HEAD"])),
    "{out}"
  );
  // The next node follows `fleet`: the app's checkout follows its `fleet`
  // too (promoted with the core, docs/self-improve.md §6).
  let out = w.run(&[("STITCH_FOLLOW", "fleet"), ("STITCH_APP", app)], &[]);
  assert!(
    out.contains(&format!(
      "following origin/fleet (app: {app}, origin/fleet)"
    )),
    "{out}"
  );
}

#[test]
fn a_plain_testflight_without_an_app_checkout_says_there_is_none() {
  // Installed with `--testflight` alone (STITCH_TESTFLIGHT=1, no
  // STITCH_APP): no directory is guessed, even with an app checkout beside
  // this one; told, nothing run.
  let w = World::with("tf-none", &["stitch", "app"]);
  write(&w.home().join("healthy"), "", false);
  let tf = [
    ("STITCH_TESTFLIGHT", "1"),
    ("STITCH_TESTFLIGHT_BACKOFF", "0"),
  ];
  let _ = w.run(&tf, &[]);
  let out = w.run(&tf, &[]);
  assert!(out.contains("TestFlight: no app checkout"), "{out}");
  assert!(!w.home().join("just.log").exists(), "{out}");
}

#[test]
fn self_update_tells_once_when_a_worked_in_checkout_holds_it_back() {
  let w = World::new("stuck");
  let stuck = [("STITCH_STUCK_AFTER", "0")];
  let notices = || std::fs::read_to_string(w.home().join("notify.log")).unwrap_or_default();
  let _ = w.run(&[], &[]);
  assert!(!w.installed().is_empty());

  // The installed script follows the one origin has.
  let script = w.home().join(".local/bin/stitch-self-update");
  write(&script, "old", true);
  w.upstream("stitch", "deploy/node/self-update.sh", "new");
  let out = w.run(&[], &[]);
  assert!(out.contains("self-update script refreshed"), "{out}");
  assert_eq!(std::fs::read_to_string(&script).unwrap(), "new");

  // On a branch while origin/main has nothing new: said, nobody told.
  git(&w.dir("stitch"), &["checkout", "-q", "-b", "feature"]);
  let out = w.run(&stuck, &[]);
  assert!(out.contains("not main: not updating"), "{out}");
  assert_eq!(notices(), "");

  // origin/main moves on: told, once.
  w.upstream("stitch", "up", "upstream work");
  let out = w.run(&stuck, &[]);
  assert!(out.contains("stuck: told the user"), "{out}");
  let _ = w.run(&stuck, &[]);
  assert_eq!(notices().matches("notify").count(), 1, "{}", notices());
  assert!(notices().contains("is on feature"), "{}", notices());

  // Back on main: updated, and a later stall is told again.
  git(&w.dir("stitch"), &["checkout", "-q", "main"]);
  let out = w.run(&stuck, &[]);
  assert!(out.contains("built "), "{out}");
  assert!(!w.home().join(".cache/stitch-self-update.skipping").exists());
  write(&w.dir("stitch").join("local"), "mine", false);
  git(&w.dir("stitch"), &["add", "-A"]);
  git(&w.dir("stitch"), &["commit", "-q", "-m", "local work"]);
  w.upstream("stitch", "up2", "more upstream work");
  let out = w.run(&stuck, &[]);
  assert!(out.contains("have diverged"), "{out}");
  assert_eq!(notices().matches("notify").count(), 2, "{}", notices());
}

/// The move from three checkouts: a node's installed script is the one
/// that followed Stitch, ../otter and ../engram (and labelled builds with
/// their HEADs). Its next run sees the new commit on origin: it refreshes
/// itself to this script, builds once (the new install.sh, Cargo.lock's
/// pins), and from then on this script is the one that runs, with no
/// second build, whether or not the checkouts beside it are at the pins.
#[test]
fn the_three_checkout_script_hands_over_once() {
  for at_pins in [true, false] {
    let w = World::with(
      if at_pins {
        "handover-pins"
      } else {
        "handover-moved"
      },
      &["stitch", "otter", "engram"],
    );
    let promote = [("STITCH_PROMOTE", "fleet")];
    // As it was: installed by the old script, labelled with the HEADs of
    // the checkouts beside Stitch.
    let otter = git(&w.dir("otter"), &["rev-parse", "HEAD"]);
    let engram = git(&w.dir("engram"), &["rev-parse", "HEAD"]);
    let old_label = format!(
      "{} otter {} engram {}",
      git(&w.dir("stitch"), &["rev-parse", "--short=12", "HEAD"]),
      &otter[..12],
      &engram[..12]
    );
    write(
      &w.home().join(".cargo/bin/stitch"),
      &STITCH.replace("@LABEL@", &old_label),
      true,
    );
    let old =
      Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/self-update-three-checkouts.sh");
    let installed_script = w.home().join(".local/bin/stitch-self-update");
    write(
      &installed_script,
      &std::fs::read_to_string(&old).unwrap(),
      true,
    );

    // What lands: this script, and a Cargo.lock pinning Otter and engram
    // (at the checkouts' HEADs, or elsewhere).
    let new = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/node/self-update.sh");
    let new = std::fs::read_to_string(new).unwrap();
    w.upstream("stitch", "deploy/node/self-update.sh", &new);
    let pins = if at_pins {
      lock(&otter, &engram)
    } else {
      lock(
        "5555555555556666666666667777777777778888",
        "9999999999990000000000001111111111112222",
      )
    };
    let head = w.upstream("stitch", "Cargo.lock", &pins);

    // The old script's last run: refreshed, built, installed.
    let out = w.run_script(&installed_script, &promote, &[]);
    assert!(out.contains("self-update script refreshed"), "{out}");
    assert!(out.contains("built "), "{out}");
    assert_eq!(std::fs::read_to_string(&installed_script).unwrap(), new);
    assert!(w.installed().contains(&head[..12]), "{}", w.installed());

    // This script from then on: up to date, nothing built again, and a
    // promotion of Stitch's commit alone.
    let out = w.run_script(&installed_script, &promote, &[]);
    assert!(out.contains("up to date"), "{out}");
    assert!(!out.contains("built "), "{out}");
    write(&w.home().join("healthy"), "", false);
    let out = w.run_script(&installed_script, &promote, &[]);
    assert!(out.contains("up to date"), "{out}");
    assert_eq!(w.branch("stitch", "fleet"), Some(head.clone()), "{out}");
    assert_eq!(w.branch("otter", "fleet"), None, "{out}");

    // The checkouts beside it no longer count: one on a branch of its own
    // holds nothing back.
    git(&w.dir("otter"), &["checkout", "-q", "-b", "mine"]);
    w.upstream("stitch", "README", "after");
    let out = w.run_script(&installed_script, &[], &[]);
    assert!(out.contains("built "), "{out}");
  }
}

/// tools/build-label.sh (the shell's builds, the self-update) says what
/// build.rs says: the same pins, from the same Cargo.lock.
#[test]
fn build_label_script_agrees_with_build_rs() {
  if std::env::var_os("STITCH_BUILD").is_some() {
    return; // the label was given, not computed
  }
  let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
  let out = Command::new("bash")
    .arg(root.join("tools/build-label.sh"))
    .arg(&root)
    .output()
    .unwrap();
  assert!(out.status.success());
  let script = String::from_utf8_lossy(&out.stdout).trim().to_owned();
  // Stitch's own commit may have moved since this was built; the rest not.
  let rest = |l: &str| {
    l.split_once(' ')
      .map(|(_, r)| r.to_owned())
      .unwrap_or_default()
  };
  let built = env!("STITCH_BUILD");
  assert_eq!(
    rest(&script),
    rest(built),
    "script: {script}; build.rs: {built}"
  );
  assert!(rest(built).starts_with("otter "), "{built}");
  assert!(rest(built).contains(" engram "), "{built}");
}

/// The canary's trial restarts with every main commit it installs, and the
/// next run comes after ~30 minutes, by when main has moved: a build in its
/// trial is finished (and promoted) before a newer main is taken.
#[test]
fn the_canary_finishes_a_trial_before_taking_a_newer_main() {
  let w = World::new("trial");
  let promote = [("STITCH_PROMOTE", "fleet")];
  let _ = w.run(&promote, &[]);
  let old = git(&w.dir("stitch"), &["rev-parse", "HEAD"]);
  let before = w.installed();
  assert!(before.contains(&old[..12]), "{before}");

  // Healthy, short of the trial; main moves: held, nothing promoted.
  write(&w.home().join("healthy"), "100", false);
  let new = w.upstream("stitch", "README", "main moved");
  let out = w.run(&promote, &[]);
  assert!(out.contains("holding "), "{out}");
  assert_eq!(w.installed(), before, "{out}");
  assert_eq!(w.branch("stitch", "fleet"), None, "{out}");

  // Past the trial: the old build is promoted, then the new main installed.
  write(&w.home().join("healthy"), "2000", false);
  let out = w.run(&promote, &[]);
  assert_eq!(w.branch("stitch", "fleet"), Some(old), "{out}");
  assert!(w.installed().contains(&new[..12]), "{out}");

  // Not healthy at all: nothing to finish, the new main is installed.
  std::fs::remove_file(w.home().join("healthy")).unwrap();
  let newer = w.upstream("stitch", "README", "again");
  let out = w.run(&promote, &[]);
  assert!(!out.contains("holding "), "{out}");
  assert!(w.installed().contains(&newer[..12]), "{out}");

  // A build that keeps restarting cannot hold main back for ever.
  write(&w.home().join("healthy"), "100", false);
  let last = w.upstream("stitch", "README", "and again");
  let out = w.run(
    &[("STITCH_PROMOTE", "fleet"), ("STITCH_TRIAL_HOLD", "0")],
    &[],
  );
  assert!(!out.contains("holding "), "{out}");
  assert!(w.installed().contains(&last[..12]), "{out}");
}
