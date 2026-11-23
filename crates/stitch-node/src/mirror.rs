//! The node's own copy of the Stitch repository, for what it does in git
//! for a fixer (docs/self-improve.md §5): landing on main (`stitch issue
//! land`), pushing a fixer's `fix-<issue>*` branches (`stitch issue
//! push`), reading CI. A bare repository in the node's data directory,
//! which no fixer writes (`fixer::confine_sandbox`), fetched from origin.
//!
//! Git never runs in the fixer's checkout for the node: that checkout's
//! `.git` is the fixer's to write (`.git/config` could name a credential
//! helper, an fsmonitor, a hooks directory, all of which run as whoever
//! runs git there). Here every git command also runs with no hooks, no
//! fsmonitor, no `ext::` transport and no prompt.
//!
//! Origin's URL is read once, when the mirror is made: from config.toml's
//! `[self_improve] remote` when set, else from the fixer checkout's
//! `.git/config` (read as a file, `git config --file`, which runs
//! nothing). Made at a fixer's admission, before it runs, and kept.
//!
//! One mirror per repository fixes go to (`[self_improve] places`,
//! docs/self-improve.md §6): `self-improve/<name>.git`. The one mirror of
//! the node before it knew two, `self-improve/mirror.git`, is the core's:
//! renamed to its name the first time it is needed.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

fn io(e: std::io::Error) -> Error {
  Error::Config(e.to_string())
}

/// A repository fixes go to (`selfimprove::places`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Repo {
  /// What issues and `--repo` call it, and its mirror's name.
  pub name: String,
  /// Its checkout on the fixer node.
  pub path: PathBuf,
  /// That checkout as the settings say it (`~/…` kept): a fixer's place,
  /// expanded on the node it runs on.
  pub spec: String,
  /// Origin's URL, when config.toml names it.
  pub remote: Option<String>,
  /// The repository it builds against and pins, if any.
  pub core: Option<String>,
  /// The file of its checkout holding the pinned core commit.
  pub pin: Option<String>,
  /// An issue's `area` words that send it here.
  pub areas: Vec<String>,
  /// The core: the nodes run its build; the one mirror before was its.
  pub default: bool,
}

impl Repo {
  /// The one repository of a node that knows one.
  #[cfg(test)]
  pub(crate) fn one(name: &str, path: &Path) -> Repo {
    Repo {
      name: name.into(),
      path: path.into(),
      spec: path.display().to_string(),
      remote: None,
      core: None,
      pin: None,
      areas: Vec::new(),
      default: true,
    }
  }
}

/// Whether `name` may name a repository (it names a directory; `mirror`
/// is the one mirror of before).
pub(crate) fn valid_name(name: &str) -> bool {
  !name.is_empty()
    && name != "mirror"
    && name.len() <= 64
    && !name.starts_with(['.', '-'])
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// Where repository `name`'s mirror is in the node's data directory.
pub(crate) fn dir(data: &Path, name: &str) -> PathBuf {
  data.join("self-improve").join(format!("{name}.git"))
}

/// The one mirror of a node from before it knew two repositories.
fn legacy_dir(data: &Path) -> PathBuf {
  data.join("self-improve").join("mirror.git")
}

/// What every git command here runs with.
const HARDENED: &[&str] = &[
  "-c",
  "core.hooksPath=/dev/null",
  "-c",
  "core.fsmonitor=false",
  "-c",
  "protocol.ext.allow=never",
  "-c",
  "core.askPass=",
  "-c",
  "credential.interactive=never",
  "--no-pager",
];

/// Runs git in `mirror` (its `GIT_DIR`), hardened; its output or why not.
pub(crate) fn git(mirror: &Path, args: &[&str]) -> Result<String> {
  let out = std::process::Command::new("git")
    .current_dir(mirror)
    .env("GIT_DIR", mirror)
    .env("GIT_TERMINAL_PROMPT", "0")
    .env_remove("GIT_WORK_TREE")
    .env_remove("GIT_CONFIG_PARAMETERS")
    .env_remove("GIT_CONFIG_COUNT")
    .args(HARDENED)
    .args(args)
    .stdin(std::process::Stdio::null())
    .output()
    .map_err(|e| Error::Config(format!("git: {e}")))?;
  if !out.status.success() {
    return Err(Error::Config(format!(
      "git {}: {}",
      args.join(" "),
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Origin's URL as the fixer checkout `place` names it, read from its
/// `.git/config` as a file (nothing in it runs).
fn origin_of(place: &Path) -> Result<String> {
  let config = place.join(".git").join("config");
  if !config.is_file() {
    return Err(Error::Config(format!(
      "{} has no .git/config: set `[self_improve] remote` (origin's URL) in config.toml",
      place.display()
    )));
  }
  let out = std::process::Command::new("git")
    .current_dir(std::env::temp_dir())
    .env("GIT_CONFIG_NOSYSTEM", "1")
    .env_remove("GIT_DIR")
    .arg("config")
    .arg("--file")
    .arg(&config)
    .args(["--get", "remote.origin.url"])
    .stdin(std::process::Stdio::null())
    .output()
    .map_err(|e| Error::Config(format!("git: {e}")))?;
  let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
  if !out.status.success() || url.is_empty() || url.starts_with('-') {
    return Err(Error::Config(format!(
      "{} names no origin: set `[self_improve] remote` in config.toml",
      place.display()
    )));
  }
  Ok(url)
}

/// Repository `repo`'s mirror, made when missing (origin's URL from its
/// `remote`, else its checkout's), and fetched from origin now:
/// `refs/remotes/origin/*` as origin has them. The core's takes over the
/// node's one mirror of before (`mirror.git`), when it is there.
pub(crate) fn ready(data: &Path, repo: &Repo) -> Result<PathBuf> {
  if !valid_name(&repo.name) {
    return Err(Error::Config(format!(
      "{:?} names no repository (letters, digits, `-`, `_`, `.`)",
      repo.name
    )));
  }
  let mirror = dir(data, &repo.name);
  let legacy = legacy_dir(data);
  if repo.default && !mirror.join("HEAD").is_file() && legacy.join("HEAD").is_file() {
    std::fs::rename(&legacy, &mirror).map_err(io)?;
  }
  if !mirror.join("HEAD").is_file() {
    let url = match &repo.remote {
      Some(r) => r.to_owned(),
      None => origin_of(&repo.path)?,
    };
    let parent = mirror.parent().expect("a parent");
    std::fs::create_dir_all(parent).map_err(io)?;
    let fresh = parent.join(format!("{}-{}.tmp", repo.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&fresh);
    let out = std::process::Command::new("git")
      .current_dir(parent)
      .args(["init", "--quiet", "--bare", "--template="])
      .arg(&fresh)
      .stdin(std::process::Stdio::null())
      .output()
      .map_err(|e| Error::Config(format!("git: {e}")))?;
    if !out.status.success() {
      return Err(Error::Config(format!(
        "git init: {}",
        String::from_utf8_lossy(&out.stderr).trim()
      )));
    }
    git(&fresh, &["remote", "add", "origin", &url])?;
    std::fs::rename(&fresh, &mirror).map_err(io)?;
  }
  git(&mirror, &["fetch", "--quiet", "--prune", "origin"])?;
  Ok(mirror)
}

/// Reverts `from..fix` on origin's main (`stitch issue revert`, the
/// user's): in a scratch worktree of the mirror (removed after), with
/// hooks off and the node as author, `git revert` of those commits, newest
/// first, pushed as a fast-forward of main as it is now. What it did.
pub(crate) fn revert_on_main(mirror: &Path, from: &str, fix: &str) -> Result<String> {
  git(mirror, &["fetch", "--quiet", "--prune", "origin"])?;
  let main = git(mirror, &["rev-parse", "refs/remotes/origin/main"])?;
  let fix = commit(mirror, fix)?;
  let from = commit(mirror, from)?;
  git(mirror, &["merge-base", "--is-ancestor", &fix, &main])
    .map_err(|_| Error::Config(format!("{fix} is not on main: nothing to revert")))?;
  let scratch = scratch_dir(mirror);
  let s = scratch.to_string_lossy().into_owned();
  git(
    mirror,
    &["worktree", "add", "--quiet", "--detach", &s, &main],
  )?;
  let run_in = |args: &[&str], input: Option<&[u8]>| -> Result<String> {
    use std::io::Write;
    let mut child = std::process::Command::new("git")
      .current_dir(&scratch)
      .env("GIT_TERMINAL_PROMPT", "0")
      .env_remove("GIT_DIR")
      .env_remove("GIT_WORK_TREE")
      .env_remove("GIT_CONFIG_PARAMETERS")
      .env_remove("GIT_CONFIG_COUNT")
      .args(HARDENED)
      .args([
        "-c",
        "user.name=Stitch",
        "-c",
        "user.email=stitch@localhost",
      ])
      .args(args)
      .stdin(if input.is_some() {
        std::process::Stdio::piped()
      } else {
        std::process::Stdio::null()
      })
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped())
      .spawn()
      .map_err(|e| Error::Config(format!("git: {e}")))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
      stdin
        .write_all(bytes)
        .map_err(|e| Error::Config(format!("git: {e}")))?;
    }
    let out = child
      .wait_with_output()
      .map_err(|e| Error::Config(format!("git: {e}")))?;
    if !out.status.success() {
      return Err(Error::Config(format!(
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
      )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
  };
  let run = |args: &[&str]| run_in(args, None);
  let range = format!("{from}..{fix}");
  // The net change the landing made, taken back as one commit on main as
  // it is: merges in the range or not, later commits kept.
  let done = git(
    mirror,
    &[
      "diff",
      "--binary",
      "--no-ext-diff",
      "--no-textconv",
      &fix,
      &from,
    ],
  )
  .and_then(|patch| {
    if !patch.trim().is_empty() {
      run_in(
        &["apply", "--3way", "--index", "-"],
        Some(format!("{patch}\n").as_bytes()),
      )?;
    }
    run(&[
      "commit",
      "--quiet",
      "--no-verify",
      "--allow-empty",
      "-m",
      &format!("Revert the fix landed as {range}"),
    ])
  })
  .and_then(|_| {
    let head = run(&["rev-parse", "HEAD"])?;
    git(
      mirror,
      &[
        "push",
        "--quiet",
        "--no-verify",
        &format!("--force-with-lease=refs/heads/main:{main}"),
        "origin",
        &format!("{head}:refs/heads/main"),
      ],
    )?;
    let _ = git(mirror, &["fetch", "--quiet", "origin"]);
    Ok(head)
  });
  let _ = git(mirror, &["worktree", "remove", "--force", &s]);
  let _ = std::fs::remove_dir_all(&scratch);
  let _ = git(mirror, &["worktree", "prune"]);
  let head = done.map_err(|e| {
    Error::Config(format!(
      "not reverted ({e}): revert {range} on main by hand"
    ))
  })?;
  Ok(format!(
    "reverted {} on main as {}",
    range,
    &head[..head.len().min(12)]
  ))
}

/// A scratch worktree's place beside the mirror, its own for each
/// operation (two reverts at once never share or remove each other's).
fn scratch_dir(mirror: &Path) -> std::path::PathBuf {
  static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
  mirror.with_file_name(format!(
    "revert-{}-{}-{}.tmp",
    std::process::id(),
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map_or(0, |d| d.as_nanos()),
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
  ))
}

/// Origin's URL as the mirror has it.
pub(crate) fn url(mirror: &Path) -> Result<String> {
  git(mirror, &["config", "--get", "remote.origin.url"])
}

/// `OWNER/REPO` of a GitHub URL (`https://github.com/o/r.git`,
/// `git@github.com:o/r`, `ssh://git@github.com/o/r`), for `gh -R`.
pub(crate) fn github_repo(url: &str) -> Option<String> {
  let rest = url
    .strip_prefix("https://github.com/")
    .or_else(|| url.strip_prefix("git@github.com:"))
    .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
  let rest = rest.trim_end_matches('/');
  let rest = rest.strip_suffix(".git").unwrap_or(rest);
  let (owner, repo) = rest.split_once('/')?;
  let ok = |s: &str| {
    !s.is_empty()
      && !s.starts_with('-')
      && s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
  };
  (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

/// The commit a fixer names (a commit, or its branch as pushed), in the
/// mirror; never an option.
pub(crate) fn commit(mirror: &Path, name: &str) -> Result<String> {
  if name.starts_with('-') || name.is_empty() {
    return Err(Error::Config(format!("{name}: no such commit")));
  }
  for candidate in [name.to_owned(), format!("refs/remotes/origin/{name}")] {
    if let Ok(sha) = git(
      mirror,
      &[
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &format!("{candidate}^{{commit}}"),
      ],
    ) {
      return Ok(sha);
    }
  }
  Err(Error::Config(format!(
    "{name} is not in origin: push it first (`stitch issue push <issue>`)"
  )))
}

/// The core commit `sha` pins (the first line of its `file`, 40 hex
/// digits), read in the mirror; never the fixer's checkout.
pub(crate) fn pin_of(mirror: &Path, sha: &str, file: &str) -> Result<String> {
  let text = git(mirror, &["show", &format!("{sha}:{file}")])
    .map_err(|_| Error::Config(format!("{sha} has no {file}")))?;
  let first = text.lines().next().unwrap_or("").trim();
  let hex = first.get(..40).unwrap_or("");
  if hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit()) && first[40..].trim().is_empty()
  {
    Ok(hex.to_ascii_lowercase())
  } else {
    Err(Error::Config(format!(
      "{file} at {sha} names no commit on its first line"
    )))
  }
}

/// Whether `a` is `b` or an ancestor of it, in the mirror.
pub(crate) fn is_ancestor(mirror: &Path, a: &str, b: &str) -> bool {
  git(mirror, &["merge-base", "--is-ancestor", a, b]).is_ok()
}

/// The heads of issue `issue`'s own branches as origin has them.
pub(crate) fn own_heads(mirror: &Path, issue: &str) -> Vec<String> {
  git(
    mirror,
    &[
      "for-each-ref",
      "--format=%(objectname) %(refname)",
      "refs/remotes/origin/",
    ],
  )
  .unwrap_or_default()
  .lines()
  .filter_map(|l| {
    let (sha, name) = l.split_once(' ')?;
    let branch = name.strip_prefix("refs/remotes/origin/")?;
    is_own_branch(issue, branch).then(|| sha.to_owned())
  })
  .collect()
}

/// One CI run on a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CiRun {
  /// The workflow's ID: what runs are grouped by (names may repeat).
  pub workflow_id: u64,
  /// The workflow's name, for messages.
  pub workflow: String,
  /// RFC 3339 (UTC, so it sorts as text).
  pub created_at: String,
  /// The conclusion, or the status while it runs.
  pub state: String,
}

/// Whether `sha` may land, from its CI runs: the latest run of each workflow
/// decides, so a failed run is superseded by a later green one. `Err` is the
/// reason it may not.
pub(crate) fn ci_verdict(sha: &str, runs: &[CiRun]) -> std::result::Result<(), String> {
  let mut latest: Vec<&CiRun> = Vec::new();
  for r in runs {
    match latest.iter_mut().find(|l| l.workflow_id == r.workflow_id) {
      Some(l) if r.created_at > l.created_at => *l = r,
      Some(_) => {}
      None => latest.push(r),
    }
  }
  if let Some(r) = latest
    .iter()
    .find(|r| matches!(r.state.as_str(), "failure" | "cancelled" | "timed_out"))
  {
    return Err(format!(
      "CI failed on {sha}: the latest run of {} ({})",
      r.workflow, r.state
    ));
  }
  if latest.is_empty() || latest.iter().any(|r| r.state != "success") {
    return Err(format!(
      "CI has not passed on {sha} (yet): push its branch (`stitch issue push`) and wait \
       for it (`stitch issue ci`)"
    ));
  }
  Ok(())
}

/// CI's runs on `sha`: workflow, creation time, and conclusion (or status
/// while it runs) of each (`gh run list`, asked by the node with its own token).
pub(crate) fn ci(mirror: &Path, sha: &str) -> Result<Vec<CiRun>> {
  let mut cmd = std::process::Command::new("gh");
  cmd
    .current_dir(mirror)
    .env("GIT_DIR", mirror)
    .env("GH_PROMPT_DISABLED", "1")
    .args(["run", "list", "--commit", sha]);
  if let Some(repo) = url(mirror).ok().as_deref().and_then(github_repo) {
    cmd.args(["-R", &repo]);
  }
  let out = cmd
    .args([
      "--json",
      "conclusion,status,workflowName,workflowDatabaseId,createdAt",
      "--jq",
      ".[] | [(.workflowDatabaseId | tostring), .workflowName, .createdAt, if .status == \"completed\" then .conclusion else .status end] | @tsv",
    ])
    .stdin(std::process::Stdio::null())
    .output()
    .map_err(|e| Error::Config(format!("gh: {e}")))?;
  if !out.status.success() {
    return Err(Error::Config(format!(
      "gh run list: {}",
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  String::from_utf8_lossy(&out.stdout)
    .lines()
    .filter(|l| !l.trim().is_empty())
    .map(|l| {
      parse_ci_line(l).ok_or_else(|| {
        Error::Config(format!(
          "gh run list: unexpected line {:?}",
          l.chars().take(200).collect::<String>()
        ))
      })
    })
    .collect()
}

/// One line of `ci`'s listing: `<workflow ID>\t<name>\t<createdAt>\t<state>`.
/// None when it is not that (so a format mismatch fails, never reads as "no
/// runs").
fn parse_ci_line(l: &str) -> Option<CiRun> {
  let mut f = l.split('\t');
  let (id, name, created, state) = (f.next()?, f.next()?, f.next()?, f.next()?);
  if f.next().is_some() {
    return None;
  }
  let workflow_id = id.trim().parse::<u64>().ok().filter(|n| *n != 0)?;
  let (created, state) = (created.trim(), state.trim());
  if created.is_empty() || state.is_empty() {
    return None;
  }
  Some(CiRun {
    workflow_id,
    workflow: name.to_owned(),
    created_at: created.to_owned(),
    state: state.to_owned(),
  })
}

fn gh_command(mirror: &Path) -> std::process::Command {
  let mut cmd = std::process::Command::new("gh");
  cmd
    .current_dir(mirror)
    .env("GIT_DIR", mirror)
    .env("GH_PROMPT_DISABLED", "1")
    .stdin(std::process::Stdio::null());
  cmd
}

fn repo_args(cmd: &mut std::process::Command, mirror: &Path) {
  if let Some(repo) = url(mirror).ok().as_deref().and_then(github_repo) {
    cmd.args(["-R", &repo]);
  }
}

/// Most failed runs `stitch issue ci` details; the rest are counted.
const MAX_DETAILED_RUNS: usize = 3;
/// Most characters of `gh`'s stderr put in an error or a line.
const MAX_STDERR: usize = 500;

fn not_detailed_line(more: usize) -> Option<String> {
  match more {
    0 => None,
    1 => Some("(1 more failed run not detailed)".to_owned()),
    n => Some(format!("({n} more failed runs not detailed)")),
  }
}

/// `line` with token-like strings replaced by `[redacted]`: GitHub tokens
/// (`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` + alphanumerics, `github_pat_` +
/// `[A-Za-z0-9_]`) and the value after `Authorization:` or `Bearer `.
fn redact(line: &str) -> String {
  const MARK: &str = "[redacted]";
  let starts = |i: usize, p: &str| {
    line
      .get(i..i + p.len())
      .is_some_and(|s| s.eq_ignore_ascii_case(p))
  };
  // The end of the run of bytes `ok` from `i`.
  let run =
    |i: usize, ok: fn(u8) -> bool| i + line.as_bytes()[i..].iter().take_while(|b| ok(**b)).count();
  let word_end = |i: usize| {
    i + line[i..]
      .find(char::is_whitespace)
      .unwrap_or(line.len() - i)
  };
  let skip_ws = |i: usize| i + (line.len() - i - line[i..].trim_start().len());
  let mut out = String::with_capacity(line.len());
  let mut i = 0;
  while i < line.len() {
    if !line.is_char_boundary(i) {
      i += 1;
      continue;
    }
    let mut next = None;
    if starts(i, "github_pat_") {
      let end = run(i + 11, |b| b.is_ascii_alphanumeric() || b == b'_');
      if end > i + 11 {
        out.push_str(MARK);
        next = Some(end);
      }
    } else if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
      .iter()
      .any(|p| line[i..].starts_with(p))
    {
      let end = run(i + 4, |b| b.is_ascii_alphanumeric());
      if end > i + 4 {
        out.push_str(MARK);
        next = Some(end);
      }
    } else if starts(i, "bearer ") {
      let v = skip_ws(i + 7);
      let end = word_end(v);
      if end > v {
        out.push_str(&line[i..v]);
        out.push_str(MARK);
        next = Some(end);
      }
    } else if starts(i, "authorization:") {
      let v = skip_ws(i + 14);
      let mut end = word_end(v);
      if end > v {
        out.push_str(&line[i..v]);
        let scheme = &line[v..end];
        if ["bearer", "basic", "token"]
          .iter()
          .any(|s| scheme.eq_ignore_ascii_case(s))
        {
          let v2 = skip_ws(end);
          let end2 = word_end(v2);
          if end2 > v2 {
            out.push_str(&line[v..v2]);
            end = end2;
          }
        }
        out.push_str(MARK);
        next = Some(end);
      }
    }
    match next {
      Some(n) => i = n,
      None => {
        let c = line[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
      }
    }
  }
  out
}

/// `gh`'s stderr as one bounded, redacted piece of text.
fn stderr_text(bytes: &[u8]) -> String {
  let text = String::from_utf8_lossy(bytes);
  let text = text.trim();
  let mut s: String = text
    .lines()
    .map(|l| redact(l.trim_end()))
    .collect::<Vec<_>>()
    .join(" / ");
  if s.chars().count() > MAX_STDERR {
    s = s.chars().take(MAX_STDERR).chain(['…']).collect();
  }
  s
}

fn gh_output(mut cmd: std::process::Command, what: &str) -> Result<String> {
  let out = cmd
    .output()
    .map_err(|e| Error::Config(format!("gh: {e}")))?;
  if !out.status.success() {
    return Err(Error::Config(format!(
      "gh {what}: {}",
      stderr_text(&out.stderr)
    )));
  }
  Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_state(run: &serde_json::Value) -> &str {
  let s = |k: &str| run[k].as_str().unwrap_or("");
  if s("status") == "completed" {
    s("conclusion")
  } else {
    s("status")
  }
}

fn ci_summary(runs: &serde_json::Value) -> Vec<String> {
  runs
    .as_array()
    .map(|a| a.iter().map(|r| run_state(r).to_owned()).collect())
    .unwrap_or_default()
}

fn ci_run_line(run: &serde_json::Value) -> String {
  let name = run["workflowName"]
    .as_str()
    .filter(|n| !n.is_empty())
    .or(run["name"].as_str())
    .unwrap_or("?");
  format!(
    "{name} #{} {} {}",
    run["databaseId"],
    run_state(run),
    run["url"].as_str().unwrap_or("")
  )
}

fn failed_run_id(run: &serde_json::Value) -> Option<u64> {
  if run["status"] == "completed"
    && matches!(
      run["conclusion"].as_str(),
      Some("failure" | "cancelled" | "timed_out")
    )
  {
    run["databaseId"].as_u64()
  } else {
    None
  }
}

fn failed_jobs_lines(view: &serde_json::Value) -> Vec<String> {
  let bad = |v: &serde_json::Value| {
    matches!(
      v["conclusion"].as_str(),
      Some("failure" | "cancelled" | "timed_out")
    )
  };
  let mut lines = Vec::new();
  for job in view["jobs"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|j| bad(j))
  {
    lines.push(format!(
      "  failed job: {}",
      job["name"].as_str().unwrap_or("?")
    ));
    for step in job["steps"]
      .as_array()
      .into_iter()
      .flatten()
      .filter(|s| bad(s))
    {
      lines.push(format!(
        "    failed step: {}",
        step["name"].as_str().unwrap_or("?")
      ));
    }
  }
  lines
}

fn log_tail(log: &str, max_lines: usize, max_width: usize) -> Vec<String> {
  let lines: Vec<&str> = log.lines().collect();
  lines[lines.len().saturating_sub(max_lines)..]
    .iter()
    .map(|l| {
      let l = redact(l);
      if l.chars().count() > max_width {
        l.chars().take(max_width).chain(['…']).collect()
      } else {
        l
      }
    })
    .collect()
}

/// What `stitch issue ci` prints: the summary line, then each run (workflow,
/// ID, URL, state) and, for a failed one, its failed jobs and steps and the
/// tail of its failed log, as `gh run view` shows them.
pub(crate) fn ci_report(mirror: &Path, sha: &str) -> Result<Vec<String>> {
  let mut cmd = gh_command(mirror);
  cmd.args(["run", "list", "--commit", sha]);
  repo_args(&mut cmd, mirror);
  cmd.args([
    "--json",
    "databaseId,name,workflowName,url,status,conclusion",
  ]);
  let runs: serde_json::Value = serde_json::from_str(&gh_output(cmd, "run list")?)
    .map_err(|e| Error::Config(format!("gh run list: {e}")))?;
  let summary = ci_summary(&runs);
  let mut out = vec![if summary.is_empty() {
    format!("CI on {sha}: no run yet")
  } else {
    format!("CI on {sha}: {}", summary.join(", "))
  }];
  let mut detailed = 0;
  let mut skipped = 0;
  for run in runs.as_array().into_iter().flatten() {
    out.push(ci_run_line(run));
    let Some(id) = failed_run_id(run) else {
      continue;
    };
    if detailed >= MAX_DETAILED_RUNS {
      skipped += 1;
      continue;
    }
    detailed += 1;
    let id = id.to_string();
    let mut cmd = gh_command(mirror);
    cmd.args(["run", "view", &id]);
    repo_args(&mut cmd, mirror);
    cmd.args(["--json", "jobs"]);
    match gh_output(cmd, "run view").and_then(|t| {
      serde_json::from_str::<serde_json::Value>(&t)
        .map_err(|e| Error::Config(format!("gh run view: {e}")))
    }) {
      Ok(v) => out.extend(failed_jobs_lines(&v)),
      Err(e) => out.push(format!("  (failed jobs unavailable: {e})")),
    }
    let mut cmd = gh_command(mirror);
    cmd.args(["run", "view", &id]);
    repo_args(&mut cmd, mirror);
    cmd.arg("--log-failed");
    match gh_output(cmd, "run view --log-failed") {
      Ok(log) => {
        out.push("  failed log (tail):".to_owned());
        out.extend(
          log_tail(&log, 80, 300)
            .into_iter()
            .map(|l| format!("    {l}")),
        );
      }
      Err(e) => out.push(format!("  (failed log unavailable: {e})")),
    }
  }
  out.extend(not_detailed_line(skipped));
  Ok(out)
}

/// The branch names a fixer of issue `issue` pushes: `fix-<its ID's
/// last part>`, alone or followed by `-<anything>`.
pub(crate) fn branch_prefix(issue: &str) -> String {
  format!("fix-{}", issue.rsplit('-').next().unwrap_or(issue))
}

pub(crate) fn is_own_branch(issue: &str, branch: &str) -> bool {
  let prefix = branch_prefix(issue);
  let ok_chars = branch
    .chars()
    .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
  ok_chars
    && !branch.contains("..")
    && !branch.ends_with(".lock")
    && !branch.ends_with('/')
    && (branch == prefix
      || branch
        .strip_prefix(&prefix)
        .is_some_and(|r| r.starts_with('-') && r.len() > 1))
}

/// Pushes a fixer's branches from the bundle it made (`git bundle create`
/// in its checkout, by its own `stitch issue push`): copied here first (the
/// fixer's file is read once, as data), its heads checked to be the
/// issue's own `fix-<issue>*` branches, fetched into the mirror under
/// `refs/incoming/`, and pushed to origin, forced (its own branches). What
/// it pushed.
pub(crate) fn push_bundle(
  data: &Path,
  repo: &Repo,
  issue: &str,
  bundle: &Path,
) -> Result<Vec<String>> {
  const MAX: u64 = 1 << 30;
  let meta = std::fs::symlink_metadata(bundle)
    .map_err(|e| Error::Config(format!("{}: {e}", bundle.display())))?;
  if !meta.is_file() || meta.len() > MAX {
    return Err(Error::Config(format!(
      "{} is no bundle (a file of 1 GiB at most)",
      bundle.display()
    )));
  }
  let mirror = ready(data, repo)?;
  let copy = mirror
    .parent()
    .expect("a parent")
    .join(format!("incoming-{}.bundle", std::process::id()));
  let bytes = std::fs::read(bundle).map_err(io)?;
  if !(bytes.starts_with(b"# v2 git bundle\n") || bytes.starts_with(b"# v3 git bundle\n")) {
    return Err(Error::Config(format!(
      "{} is no bundle (`git bundle create`)",
      bundle.display()
    )));
  }
  std::fs::write(&copy, bytes).map_err(io)?;
  let result = (|| -> Result<Vec<String>> {
    let path = copy.to_string_lossy().into_owned();
    let heads = git(&mirror, &["bundle", "list-heads", &path])?;
    let mut branches = Vec::new();
    for line in heads.lines() {
      let Some((_, name)) = line.split_once(' ') else {
        continue;
      };
      let Some(branch) = name.strip_prefix("refs/heads/") else {
        return Err(Error::Config(format!(
          "the bundle holds {name}: only branches (refs/heads/{}*)",
          branch_prefix(issue)
        )));
      };
      if !is_own_branch(issue, branch) {
        return Err(Error::Config(format!(
          "{branch} is not this issue's: a fixer of issue {issue} pushes only {0} and {0}-<name>",
          branch_prefix(issue)
        )));
      }
      branches.push(branch.to_owned());
    }
    if branches.is_empty() {
      return Err(Error::Config("the bundle holds no branch".into()));
    }
    let mut said = Vec::new();
    for b in &branches {
      git(
        &mirror,
        &[
          "fetch",
          "--quiet",
          "--no-tags",
          &path,
          &format!("+refs/heads/{b}:refs/incoming/{b}"),
        ],
      )?;
      // This repository's history (the other's branch of the same name,
      // pushed from the wrong worktree, shares none with it).
      let main = git(
        &mirror,
        &[
          "rev-parse",
          "--verify",
          "--quiet",
          "refs/remotes/origin/main",
        ],
      )
      .ok();
      if let Some(main) = main.filter(|m| !m.is_empty())
        && git(
          &mirror,
          &["merge-base", &main, &format!("refs/incoming/{b}")],
        )
        .is_err()
      {
        let _ = git(
          &mirror,
          &["update-ref", "-d", &format!("refs/incoming/{b}")],
        );
        return Err(Error::Config(format!(
          "{b} shares no history with {}'s main: not this repository's branch (push it from \
           its own worktree, or name the repository: --repo)",
          repo.name
        )));
      }
      git(
        &mirror,
        &[
          "push",
          "--quiet",
          "--no-verify",
          "origin",
          &format!("+refs/incoming/{b}:refs/heads/{b}"),
        ],
      )?;
      let sha = git(&mirror, &["rev-parse", &format!("refs/incoming/{b}")])?;
      said.push(format!(
        "pushed {b} ({}) to origin",
        &sha[..sha.len().min(12)]
      ));
    }
    // Origin as it is now, the pushed branches with it.
    let _ = git(&mirror, &["fetch", "--quiet", "origin"]);
    Ok(said)
  })();
  let _ = std::fs::remove_file(&copy);
  result
}

#[cfg(test)]
mod tests {
  use super::*;

  fn run(workflow_id: u64, workflow: &str, created_at: &str, state: &str) -> CiRun {
    CiRun {
      workflow_id,
      workflow: workflow.into(),
      created_at: created_at.into(),
      state: state.into(),
    }
  }

  #[test]
  fn a_later_green_run_supersedes_an_older_failure() {
    // Newest first, as gh lists, and the other way round.
    let a = run(1, "CI", "2026-11-15T06:00:00Z", "failure");
    let b = run(1, "CI", "2026-11-15T07:00:00Z", "success");
    assert_eq!(ci_verdict("abc", &[b.clone(), a.clone()]), Ok(()));
    assert_eq!(ci_verdict("abc", &[a, b]), Ok(()));
  }

  #[test]
  fn a_later_failure_after_a_success_refuses() {
    let a = run(1, "CI", "2026-11-15T06:00:00Z", "success");
    let b = run(1, "CI", "2026-11-15T07:00:00Z", "failure");
    let e = ci_verdict("abc", &[a, b]).unwrap_err();
    assert!(e.starts_with("CI failed on abc"), "{e}");
  }

  #[test]
  fn a_newest_run_in_progress_refuses() {
    let a = run(1, "CI", "2026-11-15T06:00:00Z", "success");
    let b = run(1, "CI", "2026-11-15T07:00:00Z", "in_progress");
    let e = ci_verdict("abc", &[a, b]).unwrap_err();
    assert!(e.starts_with("CI has not passed on abc (yet)"), "{e}");
  }

  #[test]
  fn no_runs_refuses() {
    let e = ci_verdict("abc", &[]).unwrap_err();
    assert!(e.starts_with("CI has not passed on abc (yet)"), "{e}");
  }

  #[test]
  fn each_workflow_s_latest_run_must_pass() {
    let runs = [
      run(1, "CI", "2026-11-15T06:00:00Z", "success"),
      run(2, "Lint", "2026-11-15T06:00:00Z", "failure"),
    ];
    let e = ci_verdict("abc", &runs).unwrap_err();
    assert!(
      e.starts_with("CI failed on abc") && e.contains("Lint"),
      "{e}"
    );
  }

  #[test]
  fn two_workflows_with_the_same_name_are_kept_apart() {
    let runs = [
      run(1, "CI", "2026-11-15T06:00:00Z", "failure"),
      run(2, "CI", "2026-11-15T07:00:00Z", "success"),
    ];
    let e = ci_verdict("abc", &runs).unwrap_err();
    assert!(e.starts_with("CI failed on abc"), "{e}");
  }

  #[test]
  fn a_ci_line_is_parsed_or_refused() {
    let ok = parse_ci_line("42\tCI\t2026-11-15T06:00:00Z\tsuccess").unwrap();
    assert_eq!(ok, run(42, "CI", "2026-11-15T06:00:00Z", "success"));
    for bad in [
      "\tCI\t2026-11-15T06:00:00Z\tsuccess",
      "x\tCI\t2026-11-15T06:00:00Z\tsuccess",
      "0\tCI\t2026-11-15T06:00:00Z\tsuccess",
      "42\tCI\t\tsuccess",
      "42\tCI\t2026-11-15T06:00:00Z",
      "42\tCI\t2026-11-15T06:00:00Z\tsuccess\textra",
      "success",
    ] {
      assert_eq!(parse_ci_line(bad), None, "{bad:?}");
    }
  }

  /// Each revert works in a scratch worktree of its own.
  #[test]
  fn reverts_never_share_a_scratch_worktree() {
    let m = Path::new("/data/self-improve/mirror.git");
    let (a, b) = (scratch_dir(m), scratch_dir(m));
    assert_ne!(a, b);
    assert_eq!(a.parent(), m.parent());
  }

  /// A landing with a merge in it, and later commits on main: reverted as
  /// its net change, later work kept.
  #[test]
  fn a_landing_is_reverted_as_its_net_change() {
    let d = tempfile::tempdir().unwrap();
    let sh = |dir: &Path, args: &[&str]| {
      let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
      assert!(out.status.success(), "{args:?}: {out:?}");
      String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let origin = d.path().join("origin.git");
    let work = d.path().join("work");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    sh(&origin, &["init", "-q", "--bare", "-b", "main"]);
    sh(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a"), "old\n").unwrap();
    sh(&work, &["add", "-A"]);
    sh(&work, &["commit", "-qm", "start"]);
    let from = sh(&work, &["rev-parse", "HEAD"]);
    // The fix: a side branch merged in.
    sh(&work, &["checkout", "-qb", "side"]);
    std::fs::write(work.join("b"), "side\n").unwrap();
    sh(&work, &["add", "-A"]);
    sh(&work, &["commit", "-qm", "side"]);
    sh(&work, &["checkout", "-q", "main"]);
    std::fs::write(work.join("a"), "new\n").unwrap();
    sh(&work, &["commit", "-qam", "fix"]);
    sh(&work, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let fix = sh(&work, &["rev-parse", "HEAD"]);
    // Later work.
    std::fs::write(work.join("c"), "later\n").unwrap();
    sh(&work, &["add", "-A"]);
    sh(&work, &["commit", "-qm", "later"]);
    sh(&work, &["push", "-q", origin.to_str().unwrap(), "main"]);
    let mirror = d.path().join("mirror.git");
    sh(
      d.path(),
      &[
        "clone",
        "-q",
        "--bare",
        origin.to_str().unwrap(),
        mirror.to_str().unwrap(),
      ],
    );
    sh(
      &mirror,
      &[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
      ],
    );
    let said = revert_on_main(&mirror, &from, &fix).unwrap();
    assert!(said.starts_with("reverted"), "{said}");
    let main = sh(&origin, &["rev-parse", "main"]);
    assert_eq!(sh(&origin, &["show", &format!("{main}:a")]), "old");
    assert!(
      sh(&origin, &["ls-tree", "--name-only", &main])
        .lines()
        .all(|f| f != "b")
    );
    assert_eq!(sh(&origin, &["show", &format!("{main}:c")]), "later");
  }

  #[test]
  fn ci_report_lists_runs_and_failed_steps() {
    let runs = serde_json::json!([
      {"databaseId": 11, "name": "ci", "workflowName": "CI", "url": "https://x/11",
       "status": "completed", "conclusion": "failure"},
      {"databaseId": 12, "name": "lint", "workflowName": "Lint", "url": "https://x/12",
       "status": "completed", "conclusion": "success"},
      {"databaseId": 13, "name": "ci", "workflowName": "CI", "url": "https://x/13",
       "status": "in_progress", "conclusion": ""}
    ]);
    let summary = ci_summary(&runs);
    assert_eq!(summary, ["failure", "success", "in_progress"]);
    assert_eq!(ci_run_line(&runs[0]), "CI #11 failure https://x/11");
    assert_eq!(ci_run_line(&runs[1]), "Lint #12 success https://x/12");
    assert_eq!(ci_run_line(&runs[2]), "CI #13 in_progress https://x/13");
    assert_eq!(failed_run_id(&runs[0]), Some(11));
    assert_eq!(failed_run_id(&runs[1]), None);
    assert_eq!(failed_run_id(&runs[2]), None);
    let jobs = serde_json::json!({"jobs": [
      {"name": "test", "conclusion": "failure", "steps": [
        {"name": "Set up", "conclusion": "success"},
        {"name": "cargo test", "conclusion": "failure"}]},
      {"name": "fmt", "conclusion": "success", "steps": []}]});
    assert_eq!(
      failed_jobs_lines(&jobs),
      ["  failed job: test", "    failed step: cargo test"]
    );
  }

  #[test]
  fn redact_hides_tokens_and_leaves_normal_lines() {
    for t in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
      let line = format!("clone https://x-access-token:{t}AbC123xyz@github.com/o/r failed");
      let r = redact(&line);
      assert!(!r.contains("AbC123xyz"), "{r}");
      assert_eq!(
        r,
        "clone https://x-access-token:[redacted]@github.com/o/r failed"
      );
    }
    assert_eq!(
      redact("token github_pat_11AB_cd-ef end"),
      "token [redacted]-ef end"
    );
    assert_eq!(
      redact("Authorization: abc123 more"),
      "Authorization: [redacted] more"
    );
    assert_eq!(
      redact("authorization: Bearer abc123 more"),
      "authorization: Bearer [redacted] more"
    );
    assert_eq!(redact("curl -H BEARER s3cr3t"), "curl -H BEARER [redacted]");
    let normal = "error[E0425]: cannot find function `log_tail` — ghp is not a token";
    assert_eq!(redact(normal), normal);
  }

  #[test]
  fn ci_report_details_three_failed_runs_at_most() {
    let runs: Vec<serde_json::Value> = (0..5)
      .map(|i| serde_json::json!({"databaseId": i, "conclusion": "failure", "status": "completed"}))
      .collect();
    let ids: Vec<u64> = runs
      .iter()
      .filter_map(failed_run_id)
      .take(MAX_DETAILED_RUNS)
      .collect();
    assert_eq!(ids, [0, 1, 2]);
    assert_eq!(
      not_detailed_line(2),
      Some("(2 more failed runs not detailed)".to_owned())
    );
    assert_eq!(
      not_detailed_line(1),
      Some("(1 more failed run not detailed)".to_owned())
    );
    assert_eq!(not_detailed_line(0), None);
  }

  #[test]
  fn gh_stderr_is_redacted_and_capped() {
    let long = format!("boom ghp_SECRET123 {}\n", "y".repeat(2000));
    let s = stderr_text(long.as_bytes());
    assert!(!s.contains("SECRET123"));
    assert_eq!(s.chars().count(), 501);
    assert!(s.ends_with('…'));
    assert_eq!(stderr_text(b"  not found \n"), "not found");
  }

  #[test]
  fn log_tail_redacts() {
    let t = log_tail("ok\nAuthorization: Bearer abcdef\n", 80, 300);
    assert_eq!(t, ["ok", "Authorization: Bearer [redacted]"]);
  }

  #[test]
  fn ci_log_tail_keeps_last_lines_and_caps_width() {
    let log: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let tail = log_tail(&log, 80, 300);
    assert_eq!(tail.len(), 80);
    assert_eq!(tail[0], "line 20");
    assert_eq!(tail[79], "line 99");
    let long = format!("{}\n", "x".repeat(500));
    let t = log_tail(&long, 80, 300);
    assert_eq!(t[0].chars().count(), 301);
    assert!(t[0].ends_with('…'));
  }

  /// A mirror per repository; the one mirror of before is the core's,
  /// taken over at its first use; each pushes only its own history.
  #[test]
  fn a_mirror_per_repository() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let run = |dir: &Path, args: &[&str]| {
      let o = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
      assert!(o.status.success(), "git {args:?}: {o:?}");
      String::from_utf8_lossy(&o.stdout).trim().to_owned()
    };
    let data = root.join("data");
    let repo = |name: &str| {
      let (origin, place) = (root.join(format!("{name}.origin")), root.join(name));
      std::fs::create_dir_all(&origin).unwrap();
      std::fs::create_dir_all(&place).unwrap();
      run(&origin, &["init", "-q", "--bare", "-b", "main"]);
      run(&place, &["init", "-q", "-b", "main"]);
      run(
        &place,
        &["remote", "add", "origin", origin.to_str().unwrap()],
      );
      std::fs::write(place.join("f"), name).unwrap();
      run(&place, &["add", "f"]);
      run(&place, &["commit", "-qm", name]);
      run(&place, &["push", "-q", "origin", "main"]);
      (origin, place)
    };
    let (core_origin, core_place) = repo("stitch");
    let (app_origin, app_place) = repo("app");
    let core = Repo::one("stitch", &core_place);
    let app = Repo {
      default: false,
      core: Some("stitch".into()),
      ..Repo::one("app", &app_place)
    };
    // The node's one mirror from before, of the core.
    let legacy = data.join("self-improve/mirror.git");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    run(
      &root,
      &[
        "clone",
        "-q",
        "--bare",
        core_origin.to_str().unwrap(),
        legacy.to_str().unwrap(),
      ],
    );
    run(
      &legacy,
      &[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
      ],
    );
    run(&legacy, &["config", "stitch.marker", "kept"]);
    // The app's first: a mirror of its own, the old one left to the core.
    let a = ready(&data, &app).unwrap();
    assert_eq!(a, data.join("self-improve/app.git"));
    assert!(legacy.is_dir());
    assert_eq!(url(&a).unwrap(), app_origin.to_str().unwrap());
    let c = ready(&data, &core).unwrap();
    assert_eq!(c, data.join("self-improve/stitch.git"));
    assert!(!legacy.exists(), "renamed");
    assert_eq!(run(&c, &["config", "stitch.marker"]), "kept");
    assert_eq!(url(&c).unwrap(), core_origin.to_str().unwrap());
    // `mirror` names no repository (it was the one before), nor a path.
    for bad in ["mirror", "../x", "", ".x", "a/b"] {
      assert!(ready(&data, &Repo::one(bad, &core_place)).is_err(), "{bad}");
    }
    // A branch of the core's, pushed to the app's (the wrong worktree):
    // refused, nothing pushed there.
    let id = "2026-10-29T003345Z-gamer-cy9n";
    run(&core_place, &["checkout", "-qb", "fix-cy9n"]);
    std::fs::write(core_place.join("g"), "g").unwrap();
    run(&core_place, &["add", "g"]);
    run(&core_place, &["commit", "-qm", "g"]);
    let bundle = root.join("core.bundle");
    run(
      &core_place,
      &[
        "bundle",
        "create",
        "-q",
        bundle.to_str().unwrap(),
        "fix-cy9n",
      ],
    );
    let refused = push_bundle(&data, &app, id, &bundle).unwrap_err();
    assert!(
      refused.to_string().contains("shares no history"),
      "{refused}"
    );
    assert!(
      std::process::Command::new("git")
        .current_dir(&app_origin)
        .args(["rev-parse", "--verify", "-q", "fix-cy9n"])
        .output()
        .unwrap()
        .stdout
        .is_empty()
    );
    // To its own: pushed.
    let said = push_bundle(&data, &core, id, &bundle).unwrap();
    assert!(said[0].starts_with("pushed fix-cy9n"), "{said:?}");
    let sha = run(&core_place, &["rev-parse", "HEAD"]);
    assert_eq!(run(&core_origin, &["rev-parse", "fix-cy9n"]), sha);
    // The pin an app's commit holds, read in its mirror; the issue's own
    // heads in the core's.
    std::fs::write(app_place.join("core.pin"), format!("{sha}\n# the core\n")).unwrap();
    run(&app_place, &["add", "core.pin"]);
    run(&app_place, &["commit", "-qm", "pin"]);
    run(&app_place, &["push", "-q", "origin", "main"]);
    let a = ready(&data, &app).unwrap();
    let head = run(&app_place, &["rev-parse", "HEAD"]);
    assert_eq!(pin_of(&a, &head, "core.pin").unwrap(), sha);
    assert!(pin_of(&a, &head, "missing").is_err());
    assert!(
      pin_of(&a, &head, "f").is_err(),
      "no commit on its first line"
    );
    let c = ready(&data, &core).unwrap();
    assert_eq!(own_heads(&c, id), vec![sha.clone()]);
    assert!(own_heads(&c, "2026-10-29T003345Z-gamer-zzzz").is_empty());
    assert!(!is_ancestor(&c, &sha, "refs/remotes/origin/main"));
    assert!(is_ancestor(&c, "refs/remotes/origin/main", &sha));
  }

  #[test]
  fn a_fixer_pushes_its_issues_branches_only() {
    let id = "2026-10-29T003345Z-gamer-cy9n";
    assert_eq!(branch_prefix(id), "fix-cy9n");
    for ok in ["fix-cy9n", "fix-cy9n-exit-code", "fix-cy9n-a/b"] {
      assert!(is_own_branch(id, ok), "{ok}");
    }
    for no in [
      "main",
      "fleet",
      "fix-cy9",
      "fix-cy9nx",
      "fix-cy9n-",
      "fix-other",
      "fix-cy9n-..",
      "fix-cy9n-a.lock",
      "fix-cy9n-a:main",
      "fix-cy9n x",
    ] {
      assert!(!is_own_branch(id, no), "{no}");
    }
  }

  #[test]
  fn github_repos_from_their_urls() {
    for url in [
      "https://github.com/sinkingsugar/stitch.git",
      "https://github.com/sinkingsugar/stitch",
      "git@github.com:sinkingsugar/stitch.git",
      "ssh://git@github.com/sinkingsugar/stitch",
    ] {
      assert_eq!(
        github_repo(url).as_deref(),
        Some("sinkingsugar/stitch"),
        "{url}"
      );
    }
    assert_eq!(github_repo("/tmp/origin.git"), None);
    assert_eq!(github_repo("https://github.com/--x/y"), None);
  }

  /// The mirror runs none of what a checkout's git directory could hold:
  /// a hook in it never runs, and a fixer's checkout is never touched.
  #[test]
  fn the_mirror_fetches_and_pushes_without_running_hooks() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let run = |dir: &Path, args: &[&str]| {
      let o = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
      assert!(o.status.success(), "git {args:?}: {o:?}");
      String::from_utf8_lossy(&o.stdout).trim().to_owned()
    };
    let (origin, place, data) = (
      root.join("origin.git"),
      root.join("place"),
      root.join("data"),
    );
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&place).unwrap();
    run(&origin, &["init", "-q", "--bare", "-b", "main"]);
    run(&place, &["init", "-q", "-b", "main"]);
    run(&place, &["config", "user.name", "t"]);
    run(&place, &["config", "user.email", "t@t"]);
    run(
      &place,
      &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    std::fs::write(place.join("a"), "a").unwrap();
    run(&place, &["add", "a"]);
    run(&place, &["commit", "-qm", "a"]);
    run(&place, &["push", "-q", "origin", "main"]);
    // What a fixer could leave in its checkout's git directory.
    let marker = root.join("ran");
    let hooks = place.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for h in ["reference-transaction", "pre-push", "post-checkout"] {
      std::fs::write(
        hooks.join(h),
        format!("#!/bin/sh\necho {h} >> '{}'\n", marker.display()),
      )
      .unwrap();
      std::fs::set_permissions(
        hooks.join(h),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
      )
      .unwrap();
    }
    run(
      &place,
      &[
        "config",
        "core.fsmonitor",
        &format!("echo fsmonitor >> '{}'; false", marker.display()),
      ],
    );
    run(&place, &["checkout", "-qb", "fix-cy9n-x"]);
    std::fs::write(place.join("b"), "b").unwrap();
    run(&place, &["add", "b"]);
    run(&place, &["commit", "-qm", "b"]);
    let sha = run(&place, &["rev-parse", "HEAD"]);
    let bundle = root.join("fix.bundle");
    run(
      &place,
      &[
        "bundle",
        "create",
        "-q",
        bundle.to_str().unwrap(),
        "fix-cy9n-x",
      ],
    );
    run(&place, &["checkout", "-qb", "main2"]);
    let other = root.join("main.bundle");
    run(
      &place,
      &[
        "bundle",
        "create",
        "-q",
        other.to_str().unwrap(),
        "main2",
        "main",
      ],
    );
    // Only what the node runs from here on.
    let _ = std::fs::remove_file(&marker);

    let id = "2026-10-29T003345Z-gamer-cy9n";
    let said = push_bundle(&data, &Repo::one("stitch", &place), id, &bundle).unwrap();
    assert!(said[0].starts_with("pushed fix-cy9n-x"), "{said:?}");
    assert_eq!(run(&origin, &["rev-parse", "fix-cy9n-x"]), sha);
    let mirror = ready(&data, &Repo::one("stitch", &place)).unwrap();
    assert!(mirror.starts_with(&data));
    assert_eq!(commit(&mirror, "fix-cy9n-x").unwrap(), sha);
    assert!(commit(&mirror, "--upload-pack=x").is_err());

    // Another issue's branch, or main: refused, origin as it was.
    let refused = push_bundle(&data, &Repo::one("stitch", &place), id, &other).unwrap_err();
    assert!(
      refused.to_string().contains("is not this issue's"),
      "{refused}"
    );
    // A directory is no bundle (git would read it as a repository, and
    // run what its configuration says).
    let refused = push_bundle(&data, &Repo::one("stitch", &place), id, &place).unwrap_err();
    assert!(refused.to_string().contains("no bundle"), "{refused}");
    assert_ne!(run(&origin, &["rev-parse", "main"]), sha);
    assert!(
      !marker.exists(),
      "a hook of the checkout ran: {}",
      std::fs::read_to_string(&marker).unwrap_or_default()
    );
  }
}
