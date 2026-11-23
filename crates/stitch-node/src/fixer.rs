//! What keeps a fixer of Stitch (docs/self-improve.md §4) from the user's
//! powers, and what only the user answers, in one place (a protected path,
//! `tools/protected-paths`): the fixers' mode (sandboxed, never yolo, its
//! records written only through the node), the questions for the user
//! alone, and asking whatever yolo says (`Asker`).

use stitch_records::RecordId;

use crate::node::Shared;
use crate::rules::{Granted, Live, Rules};
use crate::tools::{ApprovalRequest, Asker};

/// A fixer's rules: never yolo, whatever the user granted the node.
pub(crate) fn live(fixer: bool, live: Live) -> Live {
  if !fixer {
    return live;
  }
  Live::new(move || {
    let g = live.get();
    Granted { yolo: false, ..g }
  })
}

/// A fixer's approval mode: sandboxed where the system has a sandbox,
/// else asked.
pub(crate) fn mode() -> String {
  if crate::tools::sandbox_available() {
    "sandbox".into()
  } else {
    "ask".into()
  }
}

/// Once its rules are made: a fixer with no sandbox asks for every
/// command (unconfined, a command could pass for the user,
/// `peer::the_user_alone`).
pub(crate) fn confine_rules(fixer: bool, rules: &mut Rules) {
  if fixer && rules.mode != "sandbox" {
    rules.allow.clear();
  }
}

/// A fixer's sandbox (docs/self-improve.md §5):
/// - it writes nothing of the node's data directory: no record of its own
///   (an answer, settings, a `set` the user's); it asks the node, which
///   knows who asks. The node's mirror of the repository, where landing
///   and pushing run git, is there too.
/// - of the toolchains, only Cargo's registry and git checkouts: not
///   `~/.cargo/bin` (the installed `stitch` the service starts) nor
///   `~/.cargo/config.toml` (a `rustc-wrapper` the self-update's build
///   would run), not rustup's toolchains, nor the other package managers'
///   homes, whose `bin`s are on the user's PATH.
/// - it reads none of the credentials (`unreadable`): SSH keys, the `gh`
///   token, git's credential stores, the keychains.
pub(crate) fn confine_sandbox(
  fixer: bool,
  sandbox: &mut stitch_shell::Sandbox,
  data: &std::path::Path,
  env: &[(String, String)],
) {
  if !fixer {
    return;
  }
  use std::path::PathBuf;
  let home = std::env::var_os("HOME").map(PathBuf::from);
  let var = |name: &str| {
    env
      .iter()
      .find(|(k, _)| k == name)
      .map(|(_, v)| PathBuf::from(v))
      .or_else(|| std::env::var_os(name).map(PathBuf::from))
      .filter(|p| p.is_absolute())
  };
  let in_home = |rel: &str| home.as_ref().map(|h| h.join(rel));
  let cargo = var("CARGO_HOME").or_else(|| in_home(".cargo"));
  let mut taken: Vec<PathBuf> = Vec::new();
  taken.extend(cargo.clone());
  taken.extend(var("RUSTUP_HOME").or_else(|| in_home(".rustup")));
  taken.extend(var("GOPATH").or_else(|| in_home("go")));
  for rel in [".npm", ".bun", ".gradle", ".m2", ".pnpm-store"] {
    taken.extend(in_home(rel));
  }
  let unreadable = unreadable(env);
  sandbox.writable.retain(|p| {
    !p.starts_with(data)
      && !taken.contains(p)
      // Nothing holding a credential (`[shell] writable` of `~/.config`).
      && !unreadable.iter().any(|u| u.starts_with(p))
  });
  if let Some(cargo) = cargo {
    sandbox.writable.push(cargo.join("registry"));
    sandbox.writable.push(cargo.join("git"));
  }
  sandbox.unreadable = unreadable;
}

/// A fixer's own worktree (the stitch-dev skill's step 1, `just worktree
/// fix-<id>`), beside each checkout its sandbox has (`Sandbox::siblings`):
/// made by the node, empty, and writable. Linux's sandbox has no
/// patterns, and a worktree made later is not its (nor is the directory
/// holding the checkout, where git would make it): `git worktree add`
/// fills the empty directory instead. `issue` is the issue's ID.
pub(crate) fn own_worktree(sandbox: &mut stitch_shell::Sandbox, issue: &str) {
  // An issue's ID ends in a node's short code: never a path.
  let code = issue.rsplit('-').next().unwrap_or_default();
  if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
    return;
  }
  let name = crate::mirror::branch_prefix(issue);
  for main in sandbox.siblings.clone() {
    let (Some(parent), Some(base)) = (main.parent(), main.file_name()) else {
      continue;
    };
    let dir = parent.join(format!("{}-{name}", base.to_string_lossy()));
    match std::fs::create_dir(&dir) {
      Ok(()) => {}
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && dir.is_dir() => {}
      Err(e) => {
        eprintln!("a fixer's worktree {}: {e}", dir.display());
        continue;
      }
    }
    if !sandbox.writable.contains(&dir) {
      sandbox.writable.push(dir);
    }
  }
}

/// What a fixer's shells never read: SSH's keys, `gh`'s token, git's
/// credential stores, and on macOS the keychains' files.
pub(crate) fn unreadable(env: &[(String, String)]) -> Vec<std::path::PathBuf> {
  use std::path::PathBuf;
  let var = |name: &str| {
    env
      .iter()
      .find(|(k, _)| k == name)
      .map(|(_, v)| PathBuf::from(v))
      .or_else(|| std::env::var_os(name).map(PathBuf::from))
      .filter(|p| p.is_absolute())
  };
  let mut out = Vec::new();
  let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
    return out;
  };
  out.push(home.join(".ssh"));
  out.push(home.join(".git-credentials"));
  out.push(home.join(".config/gh"));
  out.push(home.join(".config/git/credentials"));
  out.push(home.join(".netrc"));
  if let Some(xdg) = var("XDG_CONFIG_HOME") {
    out.push(xdg.join("gh"));
    out.push(xdg.join("git/credentials"));
  }
  out.extend(var("GH_CONFIG_DIR"));
  if cfg!(target_os = "macos") {
    out.push(home.join("Library/Keychains"));
  }
  out.dedup();
  out
}

/// What a fixer's shells do not inherit: the SSH agent, the forges'
/// tokens, askpass helpers.
pub(crate) const ENV_REMOVED: &[&str] = &[
  "SSH_AUTH_SOCK",
  "GH_TOKEN",
  "GITHUB_TOKEN",
  "GH_ENTERPRISE_TOKEN",
  "GITHUB_ENTERPRISE_TOKEN",
  "GIT_ASKPASS",
  "SSH_ASKPASS",
];

/// What a fixer's shells get instead: git with no credential helper (the
/// user's `credential.helper`, `gh auth git-credential` included, cleared),
/// never prompting.
pub(crate) fn env_added(fixer: bool, env: &mut Vec<(String, String)>) {
  if !fixer {
    return;
  }
  env.retain(|(k, _)| {
    !ENV_REMOVED.contains(&k.as_str())
      && !k.starts_with("GIT_CONFIG_")
      && k != "GIT_TERMINAL_PROMPT"
  });
  for (k, v) in [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "credential.helper"),
    ("GIT_CONFIG_VALUE_0", ""),
  ] {
    env.push((k.into(), v.into()));
  }
}

/// Who a connection to the node's socket is, for what it may ask
/// (docs/self-improve.md §5): the command line is no gate (a script, `python
/// -c`, runs what it likes), so the node tells the fixers' commands from
/// their process and serves them only `fixer_may`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Connection {
  /// Anyone else: as before.
  Free,
  /// A shell of a thread in the fixers' mode (and what its commands
  /// start).
  Fixer(RecordId),
  /// A sandboxed process that is no thread's shell (detached from one,
  /// `setsid`), while self-improvement is on: it could be a fixer's.
  Confined,
}

/// Who `peer` is, once per connection.
pub(crate) async fn connection(s: &std::sync::Arc<Shared>, peer: crate::peer::Peer) -> Connection {
  let crate::peer::Peer::Process(Some(pid)) = peer else {
    return Connection::Free;
  };
  match crate::peer::identify(s, peer).await {
    crate::peer::Caller::Thread(t) if crate::selfimprove::in_fixer_mode(s, &t) => {
      Connection::Fixer(t)
    }
    crate::peer::Caller::Node
      if crate::peer::confined(pid as i32) && crate::selfimprove::settings(s).on =>
    {
      Connection::Confined
    }
    _ => Connection::Free,
  }
}

/// Why the node does not serve `req` to `who`, if it does not: a fixer's
/// commands ask about issues (and land, push, read CI through the node),
/// send to the threads of their own issue (`selfimprove::may_send`),
/// notify, and read; never a chat, a spawn, an answer, an approval, yolo,
/// tool sets, schedules, keys, secrets, the node's settings or restarts.
pub(crate) fn refuses(s: &Shared, who: &Connection, req: &crate::api::Request) -> Option<String> {
  use crate::api::Request as R;
  let thread = match who {
    Connection::Free => return None,
    Connection::Fixer(t) => Some(t),
    Connection::Confined => None,
  };
  let served = match req {
    R::Issue { .. }
    | R::Notify { .. }
    | R::Status
    | R::Jobs
    | R::Search { .. }
    | R::Models { .. }
    | R::EffectiveModel { .. }
    | R::Sync { .. }
    | R::FetchBlobs { .. } => true,
    R::Send { thread: to, .. } => {
      return match (thread, to.parse::<RecordId>()) {
        (Some(from), Ok(to)) => crate::selfimprove::may_send(s, from, &to)
          .err()
          .map(|e| e.to_string()),
        (None, _) => Some(
          "a sandboxed process that is no thread's shell sends to no thread while \
           self-improvement is on"
            .into(),
        ),
        (_, Err(e)) => Some(e.to_string()),
      };
    }
    _ => false,
  };
  (!served).then(|| {
    let what = format!("{req:?}");
    let what = what.split([' ', '{', '(']).next().unwrap_or("this");
    format!(
      "the node does not serve {what} to a fixer's commands (docs/self-improve.md §5): \
       `stitch issue` and `stitch send` to its own issue's threads are a fixer's"
    )
  })
}

/// Whether a question is the user's alone to answer (`ApprovalRequest::
/// review`): the socket takes its answer only from the user
/// (`peer::the_user_alone`), and only the user's allowing answer counts.
pub(crate) fn user_only(s: &Shared, question: &RecordId) -> bool {
  let index = s.state().index.lock().expect("index lock");
  index.record(question).ok().flatten().is_some_and(|r| {
    r.fields
      .get("user_only")
      .and_then(serde_json::Value::as_bool)
      == Some(true)
  })
}

impl Asker {
  pub(crate) async fn ask(&self, thread: &RecordId, what: &str, line: &str) -> Result<(), String> {
    if self.read_only {
      return Err(format!(
        "this thread is read-only: `{line}` acts on another thread's work, so it is refused"
      ));
    }
    if self.yolo {
      return Ok(());
    }
    self.ask_always(thread, what, line).await
  }

  /// `ask`, whatever yolo says (what yolo was never meant to cover:
  /// starting a fixer of Stitch's own code).
  pub(crate) async fn ask_always(
    &self,
    thread: &RecordId,
    what: &str,
    line: &str,
  ) -> Result<(), String> {
    if self.read_only {
      return Err(format!(
        "this thread is read-only: `{line}` acts beyond it, so it is refused"
      ));
    }
    let Some(approver) = &self.approver else {
      return Err("this needs the user's approval, and nobody is here to give it".into());
    };
    let decision = approver(ApprovalRequest::new(
      thread.clone(),
      what.to_owned(),
      line.to_owned(),
    ))
    .await;
    if decision.allow {
      Ok(())
    } else {
      Err(match decision.reason {
        Some(r) => format!("the user declined: {r}"),
        None => "the user declined".into(),
      })
    }
  }
}

#[cfg(test)]
mod tests {
  use std::path::{Path, PathBuf};

  use super::*;

  /// A fixer's sandbox writes Cargo's registry and git checkouts, not the
  /// rest of `~/.cargo` (its `bin`, its `config.toml`), nor rustup's
  /// toolchains or the node's data; it reads no credential.
  #[test]
  fn a_fixers_sandbox_keeps_to_its_work() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let place = home.join("devel/some-checkout");
    let data = PathBuf::from("/data");
    let env = vec![("CARGO_HOME".to_owned(), "/opt/cargo".to_owned())];
    let base = || {
      crate::sandbox::sandbox(
        Some(&place),
        &place,
        &data,
        &["~/.config".into()],
        &env,
        false,
      )
    };
    let mut s = base();
    assert!(s.writable.contains(&PathBuf::from("/opt/cargo")));
    assert!(s.writable.contains(&home.join(".config")));
    assert!(s.unreadable.is_empty());
    confine_sandbox(true, &mut s, &data, &env);
    for no in [
      PathBuf::from("/opt/cargo"),
      PathBuf::from("/opt/cargo/bin"),
      home.join(".rustup"),
      home.join(".bun"),
      home.join("go"),
      home.join(".config"),
      data.join("home"),
    ] {
      assert!(
        !s.writable.contains(&no),
        "{}: {:?}",
        no.display(),
        s.writable
      );
    }
    for yes in [
      PathBuf::from("/opt/cargo/registry"),
      PathBuf::from("/opt/cargo/git"),
      place.clone(),
    ] {
      assert!(
        s.writable.contains(&yes),
        "{}: {:?}",
        yes.display(),
        s.writable
      );
    }
    for secret in [
      home.join(".ssh"),
      home.join(".config/gh"),
      home.join(".git-credentials"),
      home.join(".config/git/credentials"),
    ] {
      assert!(s.unreadable.contains(&secret), "{}", secret.display());
    }
    // Anyone else's: as it was.
    let mut other = base();
    confine_sandbox(false, &mut other, &data, &env);
    assert_eq!(other, base());
  }

  /// A fixer's worktree (issue 2026-10-29T120308Z-geek-3byv): the node
  /// makes `<checkout>-fix-<id>` and its sandbox writes it, so `just
  /// worktree fix-<id>` runs inside; nothing for a place that is no
  /// checkout.
  #[test]
  fn a_fixer_writes_its_own_worktree() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let main = root.join("stitch");
    std::fs::create_dir_all(main.join(".git")).unwrap();
    let data = root.join("data");
    let mut s = crate::sandbox::sandbox(Some(&main), &main, &data, &[], &[], false);
    confine_sandbox(true, &mut s, &data, &[]);
    let wt = root.join("stitch-fix-3byv");
    assert!(!s.writable.contains(&wt));
    own_worktree(&mut s, "2026-10-29T120308Z-geek-3byv");
    assert!(wt.is_dir());
    assert!(s.writable.contains(&wt), "{:?}", s.writable);
    assert!(!s.writable.contains(&root));
    // Again (the next turn): the same one, once.
    own_worktree(&mut s, "2026-10-29T120308Z-geek-3byv");
    assert_eq!(s.writable.iter().filter(|p| **p == wt).count(), 1);
    // A place that is no checkout: none made.
    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let mut f = crate::sandbox::sandbox(Some(&folder), &folder, &data, &[], &[], false);
    own_worktree(&mut f, "2026-10-29T120308Z-geek-abcd");
    assert!(!root.join("folder-fix-abcd").exists());
    // No ID to name it by: none.
    own_worktree(&mut s, "../x");
    own_worktree(&mut s, "");
    let made: Vec<_> = std::fs::read_dir(&root)
      .unwrap()
      .flatten()
      .map(|e| e.file_name().to_string_lossy().into_owned())
      .filter(|n| n.starts_with("stitch-"))
      .collect();
    assert_eq!(made, vec!["stitch-fix-3byv".to_owned()]);
  }

  /// An app's fixer (docs/self-improve.md §6): its sandbox writes the
  /// core's checkout too, and the node makes the pair of worktrees,
  /// `<app>-fix-<id>` and `<core>-fix-<id>`, as the app's `just worktree`
  /// fills them; nothing else beside them.
  #[test]
  fn an_apps_fixer_writes_the_paired_worktrees() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let (app, core) = (root.join("app"), root.join("stitch"));
    std::fs::create_dir_all(app.join(".git")).unwrap();
    std::fs::create_dir_all(core.join(".git")).unwrap();
    let data = root.join("data");
    let mut s = crate::sandbox::sandbox(Some(&app), &app, &data, &[], &[], false);
    confine_sandbox(true, &mut s, &data, &[]);
    assert!(!s.writable.contains(&core));
    crate::sandbox::add_checkout(&mut s, &core);
    assert!(s.writable.contains(&core), "{:?}", s.writable);
    assert!(s.writable.contains(&core.join(".git")));
    // Twice (each turn): once.
    crate::sandbox::add_checkout(&mut s, &core);
    assert_eq!(s.siblings.iter().filter(|p| **p == core).count(), 1);
    own_worktree(&mut s, "2026-10-29T120308Z-mini-3byv");
    for wt in [root.join("app-fix-3byv"), root.join("stitch-fix-3byv")] {
      assert!(wt.is_dir(), "{}", wt.display());
      assert!(s.writable.contains(&wt), "{:?}", s.writable);
    }
    assert!(!s.writable.contains(&root));
    // No checkout there: nothing.
    let mut f = crate::sandbox::sandbox(Some(&app), &app, &data, &[], &[], false);
    let before = f.writable.clone();
    crate::sandbox::add_checkout(&mut f, &root.join("nothing"));
    assert_eq!(f.writable, before);
  }

  #[test]
  fn a_fixers_shells_get_no_credentials() {
    let mut env = vec![
      ("GH_TOKEN".to_owned(), "x".to_owned()),
      ("GIT_CONFIG_COUNT".to_owned(), "3".to_owned()),
      ("PATH".to_owned(), "/bin".to_owned()),
    ];
    env_added(true, &mut env);
    assert!(!env.iter().any(|(k, _)| k == "GH_TOKEN"));
    assert!(env.contains(&("GIT_CONFIG_COUNT".into(), "1".into())));
    assert!(env.contains(&("GIT_CONFIG_KEY_0".into(), "credential.helper".into())));
    assert!(env.contains(&("GIT_CONFIG_VALUE_0".into(), String::new())));
    assert!(env.contains(&("PATH".into(), "/bin".into())));
    for k in ["SSH_AUTH_SOCK", "GH_TOKEN", "GITHUB_TOKEN"] {
      assert!(ENV_REMOVED.contains(&k));
    }
  }

  #[test]
  fn view_file_reads_no_credential_for_a_fixer() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let denied = unreadable(&[]);
    for path in [
      "~/.ssh/id_ed25519",
      "~/.config/gh/hosts.yml",
      "~/.config/gh",
    ] {
      let why =
        crate::viewer::unreadable(&serde_json::json!({"path": path}), Path::new("/"), &denied);
      assert!(why.is_some(), "{path}");
    }
    let why = crate::viewer::unreadable(
      &serde_json::json!({"path": "../.ssh/config"}),
      &home.join("devel"),
      &denied,
    );
    assert!(why.is_some());
    assert!(
      crate::viewer::unreadable(
        &serde_json::json!({"path": "~/.config/other.toml"}),
        Path::new("/"),
        &denied
      )
      .is_none()
    );
  }
}
