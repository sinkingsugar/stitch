//! A project's checkout on a node that runs programs (design §2.3, "A
//! project with no checkout here"). A node knows where a project is
//! checked out once Stitch has run in it there (`checkouts.toml`); a thread
//! of a project that node has not seen (a new chat the phone put on it, a
//! schedule) looks for an unseen checkout first, beside the ones it knows
//! and in the usual places, then asks the user to clone it from its
//! `origin`; declined, failed, or with nothing to clone from, the turn
//! fails saying so. Never a silent fallback to the home directory, where
//! the agent took another repository for the one the user picked.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use stitch_records::RecordId;
use stitch_store::RecordRoot;

use crate::node::Shared;
use crate::place::{Checkout, Place, Project, holds, recreates};
use crate::tools::ApprovalRequest;
use crate::{Error, Result};

/// Where projects are usually checked out, in the home directory, after
/// the parents of the checkouts a node knows.
const USUAL: &[&str] = &[
  "devel",
  "dev",
  "src",
  "code",
  "projects",
  "repos",
  "git",
  "Developer",
  "work",
];

/// How long a clone may take.
const CLONE_TIME: Duration = Duration::from_secs(1800);

/// One clone at a time per project on a node: a second turn of the project
/// waits and then finds the first one's checkout. Per project, since the
/// lock is held while the user is asked: an unanswered question for one
/// project must not hold up another's turns.
static CLONING: std::sync::LazyLock<
  std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(Default::default);

fn cloning(repo: &str) -> Arc<tokio::sync::Mutex<()>> {
  CLONING
    .lock()
    .expect("cloning lock")
    .entry(repo.to_owned())
    .or_default()
    .clone()
}

/// Makes sure a project thread's turn has the project's checkout here: the
/// one known, an unseen one found, or one cloned with the user's yes.
/// `root` is the thread's store, for the question; `None` (a new chat not
/// written yet) refuses instead of asking.
pub(crate) async fn ensure(
  s: &Arc<Shared>,
  place: &Place,
  thread: Option<&RecordId>,
  root: Option<&RecordRoot>,
) -> Result<()> {
  let Place::Project(repo) = place else {
    return Ok(());
  };
  // A phone keeps clones of its own (`clones.rs`); a node that runs no
  // programs (a phone, a shell turned off) has no shells to work in a
  // checkout: its turns read a place's records (settings, skills).
  if crate::clones::keeps_clones() || crate::elsewhere::no_programs(s).is_some() {
    return Ok(());
  }
  if s.place_dir(place).is_some() {
    return Ok(());
  }
  let lock = cloning(repo.as_str());
  let _one = lock.lock().await;
  if s.place_dir(place).is_some() {
    return Ok(());
  }
  let node = s.state().node.to_string();
  let places = s.places();
  let label = place.label(&node, &places);
  let project = places.projects.iter().find(|p| &p.repo == repo).cloned();
  let home = std::env::var_os("HOME").map(PathBuf::from);
  let parents = parents(&s.known_checkouts(), home.as_deref());
  let names = names(project.as_ref());
  let (r, projects, ps) = (repo.clone(), places.projects.clone(), parents.clone());
  // Its checkout; else, to say so, a repository re-created from a new
  // first commit at its origin, which only the user binds to it.
  let found = tokio::task::spawn_blocking(move || {
    if let Some(top) = find(&ps, &names, |co| holds(co, &r, &projects)) {
      return Ok(top);
    }
    Err(find(&ps, &names, |co| recreates(co, &r, &projects)))
  })
  .await
  .unwrap_or(Err(None));
  let none = format!("project {label} has no checkout on {node}");
  let recreated = |top: &Path| {
    Error::Config(format!(
      "{none}: {} holds a repository re-created from a new first commit at the project's \
       origin; if it is the same project, run `stitch project rebind` there",
      top.display()
    ))
  };
  match found {
    Ok(top) => {
      eprintln!("project {label}: found its checkout at {}", top.display());
      s.saw_checkout(repo, &top);
      return Ok(());
    }
    Err(Some(top)) => return Err(recreated(&top)),
    Err(None) => {}
  }
  let Some(project) = project else {
    return Err(Error::Config(format!(
      "{none}, and the home workspace does not list it, so there is nothing to clone"
    )));
  };
  let Some(remote) = project.remote.clone() else {
    return Err(Error::Config(format!(
      "{none}, and it has no origin to clone from: check it out there (Stitch finds it once a \
       chat runs in it), or start the chat on a node that has it"
    )));
  };
  let parent = destination_parent(&parents, home.as_deref())
    .ok_or_else(|| Error::Config(format!("{none}, and no home directory to clone it into")))?;
  let dest = parent.join(clone_name(&project));
  if dest.exists() {
    if Checkout::of(&dest).is_some_and(|co| recreates(&co, repo, &places.projects)) {
      return Err(recreated(&dest));
    }
    return Err(Error::Config(format!(
      "{none}: {} exists and is not a checkout of it; move it, or check the project out \
       elsewhere and run a chat in it once",
      dest.display()
    )));
  }
  let token = crate::github::token(&s.data);
  let ssh = token.is_none() && uses_ssh(&s.known_checkouts());
  // The origin is not echoed when refused: what it holds is not known.
  let url = clone_url(&remote, ssh).ok_or_else(|| {
    Error::Config(format!(
      "{none}, and its origin is not a host/path or a local path, so it cannot be cloned"
    ))
  })?;
  let command = format!(
    "git clone -- {} {}",
    sh_quote(&url),
    sh_quote(&dest.to_string_lossy())
  );
  let (Some(thread), Some(root)) = (thread, root) else {
    return Err(Error::Config(format!(
      "{none}: clone it there first (`{command}`), or start the chat on a node that has it"
    )));
  };
  // Asked whatever the approval mode, yolo too, as a phone's first clone
  // is: it writes a whole repository where the user did not.
  let req = ApprovalRequest::new(thread.clone(), crate::clones::CLONE_SESSION.into(), command);
  let d = crate::clones::ask(s.clone(), root.clone(), req).await;
  if !d.allow {
    let why = d.reason.map(|r| format!(" ({r})")).unwrap_or_default();
    return Err(Error::Config(format!(
      "{none}, and cloning it there was declined{why}: start the chat on a node that has it, \
       or allow the clone"
    )));
  }
  eprintln!("project {label}: cloning {url} into {}", dest.display());
  let github = remote.starts_with("github.com/");
  let token = token.filter(|_| github && !ssh);
  if let Err(e) = git_clone(&url, &dest, token.as_deref(), CLONE_TIME).await {
    return Err(Error::Config(format!(
      "{none}, and cloning {url} into {} failed: {e}",
      dest.display()
    )));
  }
  let co = Checkout::of(&dest).ok_or_else(|| {
    Error::Config(format!(
      "{none}: {} was cloned but git cannot read it",
      dest.display()
    ))
  })?;
  if recreates(&co, repo, &places.projects) {
    return Err(recreated(&co.top));
  }
  if !holds(&co, repo, &places.projects) {
    return Err(Error::Config(format!(
      "{none}: {url}, cloned into {}, is another project (its history differs)",
      dest.display()
    )));
  }
  s.saw_checkout(repo, &co.top);
  Ok(())
}

/// Where to look for a project's checkout and where a clone goes: the
/// parents of the checkouts known here, the most used first, then the
/// usual folders in the home directory, then the home directory.
fn parents(known: &[PathBuf], home: Option<&Path>) -> Vec<PathBuf> {
  let mut counted: Vec<(PathBuf, usize)> = Vec::new();
  for p in known.iter().filter_map(|k| k.parent()) {
    match counted.iter_mut().find(|(q, _)| q == p) {
      Some((_, n)) => *n += 1,
      None => counted.push((p.to_owned(), 1)),
    }
  }
  // Stable: ties keep the order seen.
  counted.sort_by_key(|a| std::cmp::Reverse(a.1));
  let mut out: Vec<PathBuf> = counted.into_iter().map(|(p, _)| p).collect();
  if let Some(home) = home {
    out.extend(USUAL.iter().map(|u| home.join(u)));
    out.push(home.to_owned());
  }
  let mut seen = Vec::new();
  out.retain(|p| {
    let keep = p.is_dir() && !seen.contains(p);
    seen.push(p.clone());
    keep
  });
  out
}

/// Where a clone goes: the first of `parents` that is not the home
/// directory itself, else `~/devel`. Nothing is made here: `git clone`
/// makes the destination's parents, once the user said yes.
fn destination_parent(parents: &[PathBuf], home: Option<&Path>) -> Option<PathBuf> {
  if let Some(p) = parents.iter().find(|p| Some(p.as_path()) != home) {
    return Some(p.clone());
  }
  Some(home?.join("devel"))
}

/// A path or URL as a POSIX shell reads it: as it is when it holds only
/// safe characters, else in single quotes.
fn sh_quote(s: &str) -> String {
  let safe = !s.is_empty()
    && s
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || "/._-:@+=,%~".contains(c));
  if safe && !s.starts_with('-') && !s.starts_with('~') {
    return s.to_owned();
  }
  format!("'{}'", s.replace('\'', r"'\''"))
}

/// The names a project's checkout likely has: its name, its origin's.
fn names(project: Option<&Project>) -> Vec<String> {
  let mut out = Vec::new();
  if let Some(p) = project {
    out.push(p.name.clone());
    if let Some(n) = p.remote.as_deref().and_then(|r| r.rsplit('/').next()) {
      out.push(n.to_owned());
    }
  }
  out.retain(|n| !n.is_empty() && !n.contains('/') && n != "." && n != "..");
  out.dedup();
  out
}

/// The directory a clone is made in: its origin's last part, else its name.
fn clone_name(project: &Project) -> String {
  names(Some(project))
    .pop()
    .unwrap_or_else(|| project.repo.to_string())
}

/// A checkout under one of `parents` that `is_it`: first the directories
/// named as the project, then every other checkout directly in them.
fn find(
  parents: &[PathBuf],
  names: &[String],
  is_it: impl Fn(&Checkout) -> bool,
) -> Option<PathBuf> {
  let check = |dir: &Path| {
    Checkout::of(dir)
      .filter(|co| co.top == std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned()))
      .filter(&is_it)
      .map(|co| co.top)
  };
  let mut tried = Vec::new();
  for p in parents {
    for n in names {
      let d = p.join(n);
      if d.join(".git").exists() {
        if let Some(top) = check(&d) {
          return Some(top);
        }
        tried.push(d);
      }
    }
  }
  for p in parents {
    let Ok(entries) = std::fs::read_dir(p) else {
      continue;
    };
    let mut dirs: Vec<PathBuf> = entries
      .filter_map(|e| e.ok().map(|e| e.path()))
      .filter(|d| d.join(".git").exists() && !tried.contains(d))
      .collect();
    dirs.sort();
    for d in dirs {
      if let Some(top) = check(&d) {
        return Some(top);
      }
    }
  }
  None
}

/// Whether this node's checkouts are cloned over SSH (`git@…`, `ssh://`).
fn uses_ssh(known: &[PathBuf]) -> bool {
  known.iter().take(3).any(|k| {
    Command::new("git")
      .arg("-C")
      .arg(k)
      .args(["config", "--get", "remote.origin.url"])
      .output()
      .ok()
      .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
      .is_some_and(|u| u.starts_with("git@") || u.starts_with("ssh://"))
  })
}

/// What to clone a project's origin (`github.com/org/repo`, as the home
/// workspace lists it) from: HTTPS, or SSH when this node's checkouts use
/// it; a local path as it is. Anything else is refused: no user
/// information, port, query, fragment or option-like part can reach git, an
/// approval or a log.
fn clone_url(remote: &str, ssh: bool) -> Option<String> {
  if remote.starts_with('/') {
    return (!remote.chars().any(char::is_control)).then(|| remote.to_owned());
  }
  let (host, path) = remote.split_once('/')?;
  let host_ok = host.contains('.')
    && !host.starts_with(['-', '.'])
    && host
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
  let path_ok = !path.is_empty()
    && path.split('/').all(|seg| {
      !seg.is_empty()
        && seg != "."
        && seg != ".."
        && !seg.starts_with('-')
        && seg
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
    });
  if !host_ok || !path_ok {
    return None;
  }
  Some(if ssh {
    format!("git@{host}:{path}.git")
  } else {
    format!("https://{host}/{path}.git")
  })
}

/// `git clone`, never prompting; the node's GitHub sign-in, when given, in
/// the environment for this command only (not written into the clone).
/// Not done within `limit`, or no longer waited for (the turn cancelled),
/// git and what it started are killed and the partial clone removed, so
/// nothing writes there once the turn has said it failed.
async fn git_clone(
  url: &str,
  dest: &Path,
  token: Option<&str>,
  limit: Duration,
) -> std::result::Result<(), String> {
  use base64::Engine;
  let mut cmd = tokio::process::Command::new("git");
  cmd
    .args(["clone", "--quiet", "--", url])
    .arg(dest)
    .env("GIT_TERMINAL_PROMPT", "0")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::piped())
    .process_group(0);
  if std::env::var_os("GIT_SSH_COMMAND").is_none() {
    cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
  }
  if let Some(t) = token {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{t}"));
    cmd
      .env("GIT_CONFIG_COUNT", "1")
      .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
      .env(
        "GIT_CONFIG_VALUE_0",
        format!("Authorization: Basic {basic}"),
      );
  }
  let child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
  let mut guard = Unfinished {
    pgid: child.id(),
    dest: dest.to_owned(),
  };
  let out = tokio::time::timeout(limit, child.wait_with_output())
    .await
    .map_err(|_| format!("not done in {} minutes", limit.as_secs() / 60))?
    .map_err(|e| format!("git: {e}"))?;
  // Reaped: its group is gone (or another's now), and git removes a clone
  // it failed itself.
  guard.pgid = None;
  if out.status.success() {
    return Ok(());
  }
  let err = String::from_utf8_lossy(&out.stderr);
  let tail: Vec<&str> = err.lines().rev().take(4).collect();
  Err(tail.into_iter().rev().collect::<Vec<_>>().join(" / "))
}

/// A clone still running when it is no longer waited for: its process
/// group killed, its partial destination removed.
struct Unfinished {
  pgid: Option<u32>,
  dest: PathBuf,
}

impl Drop for Unfinished {
  fn drop(&mut self) {
    if let Some(pgid) = self.pgid {
      crate::mcp::kill_group(pgid, libc::SIGKILL);
      let _ = std::fs::remove_dir_all(&self.dest);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
      .arg("-C")
      .arg(dir)
      .args(args)
      .output()
      .unwrap();
    assert!(
      out.status.success(),
      "{}",
      String::from_utf8_lossy(&out.stderr)
    );
  }

  fn repo_at(dir: &Path, msg: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "--quiet"]);
    git(
      dir,
      &[
        "-c",
        "user.name=T",
        "-c",
        "user.email=t@example.com",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        msg,
      ],
    );
  }

  #[test]
  fn clone_urls() {
    assert_eq!(
      clone_url("github.com/o/r", false).as_deref(),
      Some("https://github.com/o/r.git")
    );
    assert_eq!(
      clone_url("github.com/o/r", true).as_deref(),
      Some("git@github.com:o/r.git")
    );
    assert_eq!(
      clone_url("/srv/r.git", false).as_deref(),
      Some("/srv/r.git")
    );
    assert_eq!(clone_url("nohost", false), None);
    assert_eq!(
      clone_url("github.com/o/.github", false).as_deref(),
      Some("https://github.com/o/.github.git")
    );
    // Nothing but host/path reaches git, an approval or a log.
    for bad in [
      "user:secret@github.com/o/r",
      "github.com:8443/o/r",
      "github.com/o/r?token=secret",
      "github.com/o/r#frag",
      "github.com/o/r name",
      "github.com/-o/r",
      "github.com/o/../r",
      "github.com//r",
      "-oProxyCommand=x.y/o/r",
      "github.com/o/r\n",
      "/srv/r\u{1b}",
    ] {
      assert_eq!(clone_url(bad, false), None, "{bad:?}");
      assert_eq!(clone_url(bad, true), None, "{bad:?}");
    }
  }

  #[test]
  fn shell_quoting() {
    assert_eq!(sh_quote("/a/b-c.git"), "/a/b-c.git");
    assert_eq!(sh_quote("/a b/c"), "'/a b/c'");
    assert_eq!(sh_quote("/a'b;rm"), r"'/a'\''b;rm'");
    assert_eq!(sh_quote("-x"), "'-x'");
  }

  #[tokio::test]
  async fn a_clone_not_done_in_time_is_killed_and_removed() {
    let d = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(d.path()).unwrap();
    repo_at(&base.join("origin"), "origin");
    let src = base.join("origin");
    let dest = base.join("clones/copy");
    let e = git_clone(src.to_str().unwrap(), &dest, None, Duration::ZERO)
      .await
      .unwrap_err();
    assert!(e.starts_with("not done"), "{e}");
    assert!(!dest.exists());
    // In time, it is there.
    git_clone(src.to_str().unwrap(), &dest, None, CLONE_TIME)
      .await
      .unwrap();
    assert!(dest.join(".git").is_dir());
  }

  #[test]
  fn parents_put_the_most_used_first_then_the_usual_ones() {
    let d = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(d.path()).unwrap();
    for p in ["a/x", "a/y", "b/z", "devel"] {
      std::fs::create_dir_all(home.join(p)).unwrap();
    }
    let known = vec![home.join("b/z"), home.join("a/x"), home.join("a/y")];
    let got = parents(&known, Some(&home));
    assert_eq!(
      got,
      vec![
        home.join("a"),
        home.join("b"),
        home.join("devel"),
        home.clone()
      ]
    );
    assert_eq!(destination_parent(&got, Some(&home)), Some(home.join("a")));
    // Nothing known and no usual folder: ~/devel, not made before the
    // user says yes (git makes it).
    let e = tempfile::tempdir().unwrap();
    let bare = std::fs::canonicalize(e.path()).unwrap();
    let got = parents(&[], Some(&bare));
    assert_eq!(got, vec![bare.clone()]);
    assert_eq!(
      destination_parent(&got, Some(&bare)),
      Some(bare.join("devel"))
    );
    assert!(!bare.join("devel").exists());
  }

  #[test]
  fn finds_an_unseen_checkout_by_name_then_by_history() {
    let d = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(d.path()).unwrap();
    repo_at(&base.join("other"), "other");
    repo_at(&base.join("paragon"), "paragon");
    repo_at(&base.join("renamed"), "renamed");
    let root_of = |dir: &Path| Checkout::of(dir).unwrap().root;
    let paragon = root_of(&base.join("paragon"));
    let renamed = root_of(&base.join("renamed"));
    let parents = vec![base.clone()];
    let names = vec!["paragon".to_owned()];
    assert_eq!(
      find(&parents, &names, |co| co.root == paragon),
      Some(base.join("paragon"))
    );
    // Another name: found by its history.
    assert_eq!(
      find(&parents, &names, |co| co.root == renamed),
      Some(base.join("renamed"))
    );
    assert_eq!(find(&parents, &names, |co| co.root == "nope"), None);
  }
}
