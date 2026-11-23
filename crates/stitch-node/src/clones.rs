//! Projects on a phone (design §2.16): a phone has no checkouts, so it
//! clones a project from its `origin` and keeps the clone fetched itself,
//! needing no other node online. gitoxide over HTTPS; the node's GitHub
//! sign-in for `github.com` (public repositories need none). The clone is
//! the project's checkout on the phone, so its threads read its
//! instructions and files.
//!
//! A clone follows a branch of `origin` (any; the default first) and can be
//! edited (the app's editor, `edit_file`, `write_file`). What differs from
//! the branch is its local changes: a fetch merges them three ways onto
//! what arrived, and touches nothing when they overlap. gitoxide cannot
//! push, so a commit goes through GitHub's API: the changed files as blobs,
//! a tree on the branch's, a commit, then the branch moved on (only if it
//! has not moved on GitHub meanwhile) or a new branch with a pull request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::{RecordId, RepoId};

use crate::node::Shared;
use crate::place::Place;
use crate::tools::{ApprovalRequest, ToolOutcome};
use crate::{Error, Result};

/// Whether this node keeps clones of its own: a phone, which has no
/// checkouts of the user's.
pub(crate) fn keeps_clones() -> bool {
  cfg!(any(target_os = "ios", target_os = "visionos"))
}

/// Where a project's clone is.
pub fn dir(data: &Path, repo: &RepoId) -> PathBuf {
  data.join("checkouts").join(repo.as_str())
}

/// One operation at a time per clone.
#[derive(Default)]
pub(crate) struct Clones {
  locks: Mutex<HashMap<RepoId, Arc<tokio::sync::Mutex<()>>>>,
  /// A pass after a sync is fetching: the next sync starts none.
  fetching: std::sync::atomic::AtomicBool,
  /// Projects whose first clone is under way (their `.git` may be there
  /// before their files are).
  first: Arc<Mutex<std::collections::HashSet<RepoId>>>,
}

/// A first clone under way, until dropped.
pub(crate) struct FirstClone {
  first: Arc<Mutex<std::collections::HashSet<RepoId>>>,
  repo: RepoId,
}

impl Drop for FirstClone {
  fn drop(&mut self) {
    self.first.lock().expect("first clones").remove(&self.repo);
  }
}

impl Clones {
  fn cloning(&self, repo: &RepoId) -> bool {
    self.first.lock().expect("first clones").contains(repo)
  }

  fn first_clone(&self, repo: &RepoId) -> FirstClone {
    self
      .first
      .lock()
      .expect("first clones")
      .insert(repo.clone());
    FirstClone {
      first: self.first.clone(),
      repo: repo.clone(),
    }
  }

  fn lock(&self, repo: &RepoId) -> Arc<tokio::sync::Mutex<()>> {
    self
      .locks
      .lock()
      .expect("clone locks")
      .entry(repo.clone())
      .or_default()
      .clone()
  }

  /// The clone's lock, held until the last clone of it is dropped: by the
  /// blocking work it is given to, whether or not its caller still waits.
  async fn hold(&self, repo: &RepoId) -> Held {
    Arc::new(self.lock(repo).lock_owned().await)
  }
}

/// A clone's lock, held.
type Held = Arc<tokio::sync::OwnedMutexGuard<()>>;

/// Blocking work on a clone, its lock held until the work ends: a caller
/// that stops waiting (`before_turn`'s timeout) lets no other operation on
/// the clone start beside it.
async fn blocking<T: Send + 'static>(
  held: Held,
  f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
  tokio::task::spawn_blocking(move || {
    let _held = held;
    f()
  })
  .await
  .map_err(|e| Error::Config(e.to_string()))?
}

/// Blocking work that changes a clone, its lock held, git's lock files an
/// interrupted update left (the process suspended or killed) removed
/// first: with the lock held no operation on the clone runs in this
/// process, so any there is stale, and gitoxide would refuse every ref
/// update until it is gone.
async fn changing<T: Send + 'static>(
  held: Held,
  clone: PathBuf,
  f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
  blocking(held, move || {
    stale_locks(&clone);
    f()
  })
  .await
}

/// Removes the lock files git and gitoxide take in a clone's `.git`
/// (`packed-refs.lock`, `index.lock`, `HEAD.lock`, `config.lock`,
/// `refs/**/*.lock`), saying each; how many. Only with the clone's lock
/// held.
fn stale_locks(clone: &Path) -> usize {
  fn refs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
      return;
    };
    for e in entries.flatten() {
      let path = e.path();
      match e.file_type() {
        Ok(t) if t.is_dir() => refs(&path, found),
        Ok(t) if t.is_file() && path.extension().is_some_and(|x| x == "lock") => found.push(path),
        _ => {}
      }
    }
  }
  let git = clone.join(".git");
  let mut found: Vec<PathBuf> = ["packed-refs.lock", "index.lock", "HEAD.lock", "config.lock"]
    .iter()
    .map(|f| git.join(f))
    .filter(|f| f.is_file())
    .collect();
  refs(&git.join("refs"), &mut found);
  let mut n = 0;
  for f in found {
    let name = f.strip_prefix(&git).unwrap_or(&f).display().to_string();
    match std::fs::remove_file(&f) {
      Ok(()) => {
        n += 1;
        eprintln!(
          "clone {}: removed a stale {name} left by an interrupted update",
          clone.display()
        );
      }
      Err(e) => eprintln!("clone {}: a stale {name} stays: {e}", clone.display()),
    }
  }
  n
}

/// What an update did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Updated {
  /// Cloned now, at this commit.
  Cloned(String),
  /// Fetched; at this commit, with this many files written or removed.
  /// Local changes that overlap what arrived: the clone was left as it was.
  Fetched {
    commit: String,
    files: usize,
    conflicts: Vec<String>,
  },
  /// Nothing to clone from: why.
  Not(String),
}

impl Updated {
  /// What it did, in a line.
  pub fn said(&self) -> String {
    let short = |c: &str| c.chars().take(10).collect::<String>();
    match self {
      Updated::Cloned(c) => format!("cloned at {}", short(c)),
      Updated::Fetched { conflicts, .. } if !conflicts.is_empty() => format!(
        "fetched, not moved: the branch changed {} where this clone has changes of its own; \
         discard them, or commit them to a new branch",
        conflicts.join(", ")
      ),
      Updated::Fetched {
        commit, files: 0, ..
      } => format!("up to date at {}", short(commit)),
      Updated::Fetched { commit, files, .. } => {
        format!("fetched: {files} file(s) changed, at {}", short(commit))
      }
      Updated::Not(why) => why.clone(),
    }
  }
}

/// A branch of `origin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
  pub name: String,
  /// The clone is on it.
  pub current: bool,
}

/// A file the clone changed from its branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Changed {
  pub path: String,
  /// `added`, `modified`, `deleted`.
  pub status: &'static str,
}

/// Where a commit goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
  /// The branch the clone is on.
  Current,
  /// A new branch, from the one the clone is on; with a pull request into
  /// it when asked.
  NewBranch { name: String, pull_request: bool },
}

/// What a push did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pushed {
  pub commit: String,
  pub branch: String,
  pub pull_request: Option<String>,
}

impl Pushed {
  pub fn said(&self) -> String {
    let mut out = format!(
      "committed {} to {}",
      self.commit.chars().take(10).collect::<String>(),
      self.branch
    );
    if let Some(url) = &self.pull_request {
      out.push_str(&format!("; pull request {url}"));
    }
    out
  }
}

/// A commit's makings: the commit the clone is on, its tree, the branch,
/// and the changed files.
pub(crate) struct Staged {
  pub head: String,
  pub tree: String,
  pub branch: String,
  pub files: Vec<StagedFile>,
}

pub(crate) struct StagedFile {
  pub path: String,
  /// `100644`, `100755` or `120000`.
  pub mode: &'static str,
  /// `None`: removed.
  pub content: Option<Vec<u8>>,
}

/// `github.com/org/repo` as GitHub's API names it: (`org`, `repo`).
fn github_repo(remote: &str) -> Option<(&str, &str)> {
  let rest = remote.strip_prefix("github.com/")?;
  let (owner, name) = rest.split_once('/')?;
  (!owner.is_empty() && !name.is_empty() && !name.contains('/')).then_some((owner, name))
}

/// The URL a project is cloned from: its `origin` as the home workspace
/// lists it (`github.com/org/repo`), over HTTPS.
#[cfg(any(target_os = "ios", target_os = "visionos", test))]
fn url_of(remote: &str) -> Option<String> {
  let (host, path) = remote.split_once('/')?;
  (host.contains('.') && !path.is_empty()).then(|| format!("https://{host}/{path}.git"))
}

/// What a node without clones says when asked to work on one.
#[cfg(not(any(target_os = "ios", target_os = "visionos", test)))]
fn no_clones<T>() -> Result<T> {
  Err(Error::Config(
    "this node keeps no clones: its projects are the user's checkouts".into(),
  ))
}

/// Runs `f` on a phone (and in tests); elsewhere says there are no clones.
macro_rules! on_phone {
  ($e:expr) => {{
    #[cfg(any(target_os = "ios", target_os = "visionos", test))]
    {
      $e
    }
    #[cfg(not(any(target_os = "ios", target_os = "visionos", test)))]
    {
      no_clones()
    }
  }};
}

/// Clones a project here, or fetches its clone and moves it to its remote
/// branch, its local changes merged in. Blocking: gitoxide's network client
/// is.
pub fn update(data: &Path, repo: &RepoId, remote: &str, token: Option<&str>) -> Result<Updated> {
  #[cfg(any(target_os = "ios", target_os = "visionos", test))]
  {
    git::update(data, repo, remote, token)
  }
  #[cfg(not(any(target_os = "ios", target_os = "visionos", test)))]
  {
    let _ = (data, repo, remote, token);
    Ok(Updated::Not(
      "this node clones nothing: its projects are the user's checkouts".into(),
    ))
  }
}

/// The branches of `origin`, as last fetched.
pub fn branches(data: &Path, repo: &RepoId) -> Result<Vec<Branch>> {
  let _ = (data, repo);
  on_phone!(git::branches(&dir(data, repo)))
}

/// What the clone changed from its branch.
pub fn changes(data: &Path, repo: &RepoId) -> Result<Vec<Changed>> {
  let _ = (data, repo);
  on_phone!(git::changes(&dir(data, repo)))
}

/// The clone's changes as a unified diff; only `path`'s when given.
pub fn diff(data: &Path, repo: &RepoId, path: Option<&str>) -> Result<String> {
  let _ = (data, repo, path);
  on_phone!(git::diff(&dir(data, repo), path))
}

/// Puts files back as the branch has them: `path`'s, or every change.
pub fn discard(data: &Path, repo: &RepoId, path: Option<&str>) -> Result<usize> {
  let _ = (data, repo, path);
  on_phone!(git::discard(&dir(data, repo), path))
}

/// Moves the clone to another branch of `origin`; refused while it has
/// changes of its own.
pub fn switch(data: &Path, repo: &RepoId, branch: &str) -> Result<usize> {
  let _ = (data, repo, branch);
  on_phone!(git::switch(&dir(data, repo), branch))
}

/// The project's remote and the clone's lock, for what changes a clone.
async fn project_of(s: &Shared, repo: &RepoId) -> Result<String> {
  let places = s.places();
  let project = places
    .projects
    .iter()
    .find(|p| &p.repo == repo)
    .ok_or_else(|| Error::Config(format!("{repo} is not a project the home workspace lists")))?;
  project
    .remote
    .clone()
    .ok_or_else(|| Error::Config(format!("{} has no origin to clone from", project.name)))
}

/// Where this device keeps the user's answers to "clone it here?".
fn decisions_file(data: &Path) -> PathBuf {
  data.join("clones.json")
}

fn decisions(data: &Path) -> HashMap<String, bool> {
  std::fs::read(decisions_file(data))
    .ok()
    .and_then(|b| serde_json::from_slice(&b).ok())
    .unwrap_or_default()
}

/// The user's answer for a project on this device: cloned when allowed,
/// never asked again when declined; none yet.
pub fn decision(data: &Path, repo: &RepoId) -> Option<bool> {
  decisions(data).get(repo.as_str()).copied()
}

/// Records the user's answer for a project on this device.
pub fn decide(data: &Path, repo: &RepoId, allow: bool) -> Result<()> {
  let mut all = decisions(data);
  all.insert(repo.as_str().to_owned(), allow);
  let file = decisions_file(data);
  let tmp = file.with_extension("json.tmp");
  std::fs::write(
    &tmp,
    serde_json::to_vec_pretty(&all).expect("a map serializes"),
  )
  .map_err(Error::io(tmp.display()))?;
  std::fs::rename(&tmp, &file).map_err(Error::io(file.display()))?;
  Ok(())
}

/// Whether a project is cloned here (its first clone done).
fn cloned(s: &Shared, repo: &RepoId) -> bool {
  dir(&s.data, repo).join(".git").is_dir() && !s.clones.cloning(repo)
}

/// A project's size on GitHub, said (`123 MB`); none when not known (not
/// GitHub, GitHub unreachable or not telling).
async fn size_on_github(remote: &str, token: Option<&str>) -> Option<String> {
  let (owner, name) = github_repo(remote)?;
  let mut req = crate::github::http()
    .get(format!("https://api.github.com/repos/{owner}/{name}"))
    .header("Accept", "application/vnd.github+json")
    .header("X-GitHub-Api-Version", "2022-11-28")
    .timeout(std::time::Duration::from_secs(15));
  if let Some(t) = token {
    req = req.bearer_auth(t);
  }
  let v: Value = req
    .send()
    .await
    .ok()?
    .error_for_status()
    .ok()?
    .json()
    .await
    .ok()?;
  Some(said_size(v.get("size")?.as_u64()?))
}

/// GitHub's `size` (KB) as a person reads it.
fn said_size(kb: u64) -> String {
  match kb {
    0..1_000 => format!("{kb} KB"),
    1_000..1_000_000 => format!("{} MB", kb / 1_000),
    _ => format!("{:.1} GB", kb as f64 / 1_000_000.0),
  }
}

/// What to ask before a project's first clone here: none when it is cloned
/// already, the user answered (`again`: only a yes counts), or this device
/// keeps no clones.
pub(crate) async fn question(s: &Arc<Shared>, repo: &RepoId, again: bool) -> Option<String> {
  let answered = match decision(&s.data, repo) {
    Some(allow) => allow || !again,
    None => false,
  };
  if !keeps_clones() || cloned(s, repo) || answered {
    return None;
  }
  let remote = project_of(s, repo).await.ok()?;
  let token = crate::github::token(&s.data);
  Some(match size_on_github(&remote, token.as_deref()).await {
    Some(size) => format!("Clone {remote} to this device ({size} on GitHub)"),
    None => format!("Clone {remote} to this device (its size not known)"),
  })
}

/// The session a question to clone a project names: no shell's.
pub const CLONE_SESSION: &str = "clone";

/// The user's answer to cloning a project here: recorded, and when it is
/// yes, the project cloned (or fetched) now.
pub(crate) async fn answer(s: &Arc<Shared>, repo: &RepoId, allow: bool) -> Result<Updated> {
  decide(&s.data, repo, allow)?;
  if allow {
    refresh(s, repo).await
  } else {
    Ok(Updated::Not("not cloned on this device".into()))
  }
}

/// Clones or fetches a project here, and makes the clone its checkout.
pub(crate) async fn refresh(s: &Arc<Shared>, repo: &RepoId) -> Result<Updated> {
  let held = s.clones.hold(repo).await;
  let remote = match project_of(s, repo).await {
    Ok(r) => r,
    Err(e) => return Ok(Updated::Not(e.to_string())),
  };
  // A first clone only once the user said yes on this device.
  let first = keeps_clones() && !dir(&s.data, repo).join(".git").is_dir();
  if first && decision(&s.data, repo) != Some(true) {
    return Ok(Updated::Not(
      "not cloned on this device: the user has not let it be".into(),
    ));
  }
  let token = crate::github::token(&s.data);
  let (data, r) = (s.data.clone(), repo.clone());
  let _cloning = first.then(|| s.clones.first_clone(repo));
  let clone = dir(&data, &r);
  let updated = changing(held, clone, move || {
    update(&data, &r, &remote, token.as_deref())
  })
  .await?;
  if !matches!(updated, Updated::Not(_)) {
    s.saw_checkout(repo, &dir(&s.data, repo));
  }
  Ok(updated)
}

/// Switches a clone to another branch (fetched first).
pub(crate) async fn switch_branch(s: &Arc<Shared>, repo: &RepoId, branch: &str) -> Result<usize> {
  refresh(s, repo).await?;
  let held = s.clones.hold(repo).await;
  let (data, r, b) = (s.data.clone(), repo.clone(), branch.to_owned());
  changing(held, dir(&data, &r), move || switch(&data, &r, &b)).await
}

/// Discards a clone's changes (`path`'s, or all).
pub(crate) async fn discard_changes(
  s: &Arc<Shared>,
  repo: &RepoId,
  path: Option<&str>,
) -> Result<usize> {
  let held = s.clones.hold(repo).await;
  let (data, r, p) = (s.data.clone(), repo.clone(), path.map(str::to_owned));
  changing(held, dir(&data, &r), move || {
    discard(&data, &r, p.as_deref())
  })
  .await
}

/// Commits a clone's changes and pushes them through GitHub's API: to the
/// branch it is on (only if that has not moved on GitHub meanwhile: fetch
/// and try again), or to a new branch, with a pull request when asked.
pub(crate) async fn commit_push(
  s: &Arc<Shared>,
  repo: &RepoId,
  message: &str,
  target: Target,
) -> Result<Pushed> {
  let message = message.trim();
  if message.is_empty() {
    return Err(Error::Config("a commit needs a message".into()));
  }
  if let Target::NewBranch { name, .. } = &target
    && !valid_branch(name)
  {
    return Err(Error::Config(format!("{name} is not a branch name")));
  }
  // Up to date first: the commit goes on the branch's latest.
  if let Updated::Fetched { conflicts, .. } = refresh(s, repo).await?
    && !conflicts.is_empty()
  {
    return Err(Error::Config(format!(
      "the branch changed {} where this clone has changes of its own: commit to a new branch, \
       or discard them",
      conflicts.join(", ")
    )));
  }
  let held = s.clones.hold(repo).await;
  let remote = project_of(s, repo).await?;
  let (owner, name) = github_repo(&remote).ok_or_else(|| {
    Error::Config(format!(
      "{remote} is not on GitHub: commits from here go through GitHub's API"
    ))
  })?;
  let token = crate::github::token(&s.data)
    .ok_or_else(|| Error::Config("sign in to GitHub first (Settings, Accounts)".into()))?;
  let (data, r) = (s.data.clone(), repo.clone());
  let staged = blocking(held.clone(), move || -> Result<Staged> {
    let _ = (&data, &r);
    on_phone!(git::staged(&dir(&data, &r)))
  })
  .await?;
  if staged.files.is_empty() {
    return Err(Error::Config(
      "nothing to commit: the clone is as its branch".into(),
    ));
  }
  let client = crate::github::http();
  let api = format!("https://api.github.com/repos/{owner}/{name}");
  let call = |method: reqwest::Method, path: String, body: Value| {
    let (client, token) = (client.clone(), token.clone());
    async move {
      let resp = client
        .request(method, path)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Config(format!("GitHub: {e}")))?;
      let status = resp.status();
      let v: Value = resp.json().await.unwrap_or(Value::Null);
      if status.is_success() {
        Ok(v)
      } else {
        Err((
          status.as_u16(),
          v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        ))
        .map_err(|(code, msg)| Error::Config(format!("GitHub said {code}: {msg}")))
      }
    }
  };
  // The files, then a tree on the branch's, then the commit.
  let mut tree = Vec::new();
  for f in &staged.files {
    match &f.content {
      Some(bytes) => {
        use base64::Engine;
        let blob = call(
          reqwest::Method::POST,
          format!("{api}/git/blobs"),
          json!({"content": base64::engine::general_purpose::STANDARD.encode(bytes), "encoding": "base64"}),
        )
        .await?;
        tree.push(json!({"path": f.path, "mode": f.mode, "type": "blob", "sha": blob["sha"]}));
      }
      None => tree.push(json!({"path": f.path, "mode": "100644", "type": "blob", "sha": null})),
    }
  }
  let new_tree = call(
    reqwest::Method::POST,
    format!("{api}/git/trees"),
    json!({"base_tree": staged.tree, "tree": tree}),
  )
  .await?;
  let account = match crate::github::account(&s.data).filter(|a| a.id.is_some()) {
    Some(a) => a,
    None => crate::github::user(&client, "https://api.github.com", &token).await?,
  };
  let email = match account.id {
    Some(id) => format!("{id}+{}@users.noreply.github.com", account.login),
    None => format!("{}@users.noreply.github.com", account.login),
  };
  let commit = call(
    reqwest::Method::POST,
    format!("{api}/git/commits"),
    json!({
      "message": message,
      "tree": new_tree["sha"],
      "parents": [staged.head],
      "author": {
        "name": account.name.clone().unwrap_or_else(|| account.login.clone()),
        "email": email,
        "date": jiff::Timestamp::now().to_string(),
      },
    }),
  )
  .await?;
  let sha = commit["sha"].as_str().unwrap_or_default().to_owned();
  let (branch, pull_request) = match &target {
    Target::Current => {
      // Only forward: a branch that moved on GitHub meanwhile is not
      // overwritten.
      call(
        reqwest::Method::PATCH,
        format!("{api}/git/refs/heads/{}", staged.branch),
        json!({"sha": sha, "force": false}),
      )
      .await
      .map_err(|e| {
        Error::Config(format!(
          "{e}: the branch {} moved on GitHub meanwhile; fetch and commit again",
          staged.branch
        ))
      })?;
      (staged.branch.clone(), None)
    }
    Target::NewBranch { name, pull_request } => {
      call(
        reqwest::Method::POST,
        format!("{api}/git/refs"),
        json!({"ref": format!("refs/heads/{name}"), "sha": sha}),
      )
      .await?;
      let pr = if *pull_request {
        let (title, body) = message.split_once('\n').unwrap_or((message, ""));
        let pr = call(
          reqwest::Method::POST,
          format!("{api}/pulls"),
          json!({
            "title": title.trim(),
            "body": format!("{}\n\nCommitted from Stitch on {}.", body.trim(), s.state().node).trim().to_owned(),
            "head": name,
            "base": staged.branch,
          }),
        )
        .await?;
        pr["html_url"].as_str().map(str::to_owned)
      } else {
        None
      };
      (name.clone(), pr)
    }
  };
  // The clone at the commit, on its branch.
  let (data, r, b, c, t) = (
    s.data.clone(),
    repo.clone(),
    branch.clone(),
    sha.clone(),
    token.clone(),
  );
  let clone = dir(&data, &r);
  changing(held, clone, move || -> Result<()> {
    let _ = (&data, &r, &t, &b, &c);
    on_phone!(git::pushed(&dir(&data, &r), &t, &b, &c))
  })
  .await?;
  Ok(Pushed {
    commit: sha,
    branch,
    pull_request,
  })
}

/// A branch name git takes, kept plain.
fn valid_branch(name: &str) -> bool {
  !name.is_empty()
    && name.len() <= 200
    && !name.starts_with(['-', '/'])
    && !name.ends_with(['/', '.'])
    && !name.contains("..")
    && !name.contains("//")
    && !name.ends_with(".lock")
    && name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Asks the user, boxed with its type said: asking reaches a sync, which
/// reaches turns, which reach `before_turn` (a cycle the compiler cannot
/// see through).
pub(crate) fn ask(
  s: Arc<Shared>,
  root: stitch_store::RecordRoot,
  req: ApprovalRequest,
) -> crate::tools::BoxFuture<'static, crate::tools::Decision> {
  Box::pin(async move { crate::detached::ask(&s, &root, req, None, None).await })
}

/// How long a turn waits for a project's first clone.
const FIRST_CLONE: std::time::Duration = std::time::Duration::from_secs(600);

/// Before a turn on a project, on a phone: its clone fetched (20 seconds
/// at most, then the turn goes on with what is there), or, with none yet
/// or its first still under way (the app asks when a chat of it opens),
/// cloned before the turn reads anything, so its instructions and files
/// are in its context from the first message (10 minutes at most). A
/// first clone the user has not answered yet is asked here, as an approval
/// of `thread`, and the turn waits for the answer; declined, or with no
/// thread to ask for, the turn goes on without a clone.
pub(crate) async fn before_turn(s: &Arc<Shared>, place: &Place, thread: Option<&RecordId>) {
  let Place::Project(repo) = place else { return };
  if !keeps_clones() {
    return;
  }
  if let Some(command) = question(s, repo, false).await {
    let Some((root, thread)) = thread.and_then(|t| Some((s.thread_place(t).ok()?.0, t))) else {
      return;
    };
    let mut req = ApprovalRequest::new(thread.clone(), CLONE_SESSION.into(), command);
    // Answered in the app meanwhile: this one goes.
    let (s2, r2) = (s.clone(), repo.clone());
    req.allowed_now = Some(Arc::new(move || decision(&s2.data, &r2) == Some(true)));
    let d = ask(s.clone(), root, req).await;
    let withdrawn = d
      .reason
      .as_deref()
      .is_some_and(|r| r.starts_with("withdrawn") || r.starts_with("the approval could not"));
    if decision(&s.data, repo).is_none()
      && (d.allow || !withdrawn)
      && let Err(e) = decide(&s.data, repo, d.allow)
    {
      eprintln!("project {repo}: {e}");
    }
  }
  let first = !cloned(s, repo);
  let wait = if first {
    FIRST_CLONE
  } else {
    std::time::Duration::from_secs(20)
  };
  match tokio::time::timeout(wait, refresh(s, repo)).await {
    Ok(Ok(Updated::Not(why))) => eprintln!("project {repo}: {why}"),
    Ok(Ok(_)) => {}
    Ok(Err(e)) => eprintln!("project {repo}: {e}"),
    Err(_) => eprintln!("project {repo}: not fetched in {}s", wait.as_secs()),
  }
}

/// After a sync, on a phone: every clone here fetched, in the background
/// (GitHub slow or unreachable holds no sync up: the app waits for its
/// sync), one pass at a time.
pub(crate) fn after_sync(s: &Arc<Shared>) {
  use std::sync::atomic::Ordering;
  if !keeps_clones() || s.clones.fetching.swap(true, Ordering::AcqRel) {
    return;
  }
  let s = s.clone();
  tokio::spawn(async move {
    let places = s.places();
    for p in places.listed() {
      if dir(&s.data, &p.repo).join(".git").is_dir()
        && let Err(e) = refresh(&s, &p.repo).await
      {
        eprintln!("project {}: {e}", p.repo);
      }
    }
    s.clones.fetching.store(false, Ordering::Release);
  });
}

/// `commit_push`, for a phone's turn in a project: its clone's changes
/// committed and pushed, the user asked first.
pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: "commit_push".into(),
    description: "Commit this project's changes on this device (made with edit_file and \
      write_file) and push them to GitHub: to the branch the clone is on, or to a new branch \
      (new_branch), with a pull request into the current one when pull_request is true. The user \
      is asked first, with the files and the message. Prefer a new branch with a pull request for \
      anything the user has not asked to land directly."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "message": {"type": "string", "description": "The commit message: a summary line, then details."},
        "new_branch": {"type": "string", "description": "A new branch to push to, from the current one."},
        "pull_request": {"type": "boolean", "description": "With new_branch: open a pull request into the current branch."}
      },
      "required": ["message"]
    }),
  }
}

/// Runs `commit_push` for `thread` in the clone of `repo`.
pub(crate) async fn tool(
  s: Arc<Shared>,
  editing: crate::edits::Editing,
  thread: RecordId,
  repo: RepoId,
  input: Value,
) -> ToolOutcome {
  let message = input
    .get("message")
    .and_then(Value::as_str)
    .unwrap_or("")
    .to_owned();
  let new_branch = input
    .get("new_branch")
    .and_then(Value::as_str)
    .map(str::to_owned);
  let pull_request = input.get("pull_request").and_then(Value::as_bool) == Some(true);
  if editing.read_only {
    return ToolOutcome::err("this thread is read-only: commit_push is refused");
  }
  let (data, r) = (s.data.clone(), repo.clone());
  let changed = tokio::task::spawn_blocking(move || changes(&data, &r))
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r.map_err(|e| e.to_string()));
  let changed = match changed {
    Ok(c) if c.is_empty() => {
      return ToolOutcome::err("nothing to commit: the clone is as its branch");
    }
    Ok(c) => c,
    Err(e) => return ToolOutcome::err(e),
  };
  let target = match &new_branch {
    Some(name) => Target::NewBranch {
      name: name.clone(),
      pull_request,
    },
    None => Target::Current,
  };
  let Some(approver) = &editing.approver else {
    return ToolOutcome::err(
      "commit_push needs the user's approval, and nobody is here to give it",
    );
  };
  let files: Vec<String> = changed
    .iter()
    .map(|c| format!("{} {}", c.status, c.path))
    .collect();
  let to = match &target {
    Target::Current => "the current branch".to_owned(),
    Target::NewBranch {
      name,
      pull_request: true,
    } => format!("new branch {name}, with a pull request"),
    Target::NewBranch { name, .. } => format!("new branch {name}"),
  };
  let decision = approver(ApprovalRequest::new(
    thread,
    "git".into(),
    format!(
      "commit_push to {to}: {} ({})",
      message.lines().next().unwrap_or(""),
      files.join(", ")
    ),
  ))
  .await;
  if !decision.allow {
    return ToolOutcome::err(match decision.reason {
      Some(r) => format!("the user declined the commit: {r}"),
      None => "the user declined the commit".into(),
    });
  }
  match commit_push(&s, &repo, &message, target).await {
    Ok(p) => ToolOutcome::ok(format!("(approved by the user)\n{}", p.said())),
    Err(e) => ToolOutcome::err(e.to_string()),
  }
}

/// gitoxide, built only where it is used (it is large).
#[cfg(any(target_os = "ios", target_os = "visionos", test))]
mod git {
  use std::collections::BTreeMap;
  use std::path::Path;
  use std::sync::atomic::AtomicBool;

  use gix::object::tree::EntryKind;
  use stitch_records::RepoId;

  use super::{Branch, Changed, Staged, StagedFile, Updated, dir, url_of};
  use crate::{Error, Result};

  type Failure = Box<dyn std::error::Error + Send + Sync>;

  /// An error with its causes: gix's top error alone ("Transport handshake
  /// failed") hides what failed. A cause its error already says is not
  /// said again.
  pub(super) fn chain(e: &dyn std::error::Error) -> String {
    let mut said = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
      let text = c.to_string();
      if !text.is_empty() && !said.contains(&text) {
        said.push_str(": ");
        said.push_str(&text);
      }
      cause = c.source();
    }
    said
  }

  fn err(e: impl std::fmt::Display) -> Error {
    Error::Config(e.to_string())
  }

  /// Opens a clone with an identity for its reference logs: a phone has no
  /// git configuration.
  pub(super) fn open(dir: &Path) -> std::result::Result<gix::Repository, Failure> {
    let opts = gix::open::Options::isolated()
      .config_overrides(["user.name=Stitch", "user.email=stitch@localhost"]);
    Ok(gix::open_opts(dir, opts)?)
  }

  pub fn update(data: &Path, repo: &RepoId, remote: &str, token: Option<&str>) -> Result<Updated> {
    let Some(url) = url_of(remote) else {
      return Ok(Updated::Not(format!(
        "{remote} is not a remote to clone from"
      )));
    };
    // The token goes to GitHub only.
    let token = token
      .filter(|_| remote.starts_with("github.com/"))
      .map(str::to_owned);
    let dir = dir(data, repo);
    let fail = |what: &str, e: &dyn std::error::Error| {
      Error::Config(format!("{what} {remote}: {}", chain(e)))
    };
    if dir.join(".git").is_dir() {
      let repo = open(&dir).map_err(|e| fail("opening the clone of", &*e))?;
      let before = repo.head_id().ok().map(|id| id.detach());
      fetch(&repo, token).map_err(|e| fail("fetching", &*e))?;
      let target = remote_head(&repo).map_err(|e| fail("reading what was fetched of", &*e))?;
      if before == Some(target) {
        return Ok(Updated::Fetched {
          commit: target.to_string(),
          files: 0,
          conflicts: Vec::new(),
        });
      }
      let moved = advance(&repo, &dir, before, target).map_err(|e| fail("checking out", &*e))?;
      if moved.conflicts.is_empty() {
        set_branch(&repo, target).map_err(|e| fail("moving the branch of", &*e))?;
      }
      return Ok(Updated::Fetched {
        commit: target.to_string(),
        files: moved.files,
        conflicts: moved.conflicts,
      });
    }
    // A clone that stopped half way is started again.
    let partial = dir.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    if let Some(parent) = dir.parent() {
      std::fs::create_dir_all(parent).map_err(Error::io(parent.display()))?;
    }
    let mut prepare =
      gix::prepare_clone(url.as_str(), &partial).map_err(|e| fail("cloning", &e))?;
    if let Some(t) = token {
      prepare = prepare.configure_connection(move |c| {
        let t = t.clone();
        c.set_credentials(move |action| credentials(&t, action));
        Ok(())
      });
    }
    let (mut checkout, _) = prepare
      .fetch_then_checkout(gix::progress::Discard, &AtomicBool::new(false))
      .map_err(|e| fail("cloning", &e))?;
    let (cloned, _) = checkout
      .main_worktree(gix::progress::Discard, &AtomicBool::new(false))
      .map_err(|e| fail("checking out", &e))?;
    let head = cloned
      .head_id()
      .map_err(|e| fail("reading the clone of", &e))?
      .to_string();
    drop(cloned);
    std::fs::rename(&partial, &dir).map_err(Error::io(dir.display()))?;
    Ok(Updated::Cloned(head))
  }

  /// The token, as git over HTTPS takes it from GitHub's users: the password,
  /// with any user name.
  fn credentials(
    token: &str,
    action: gix::credentials::helper::Action,
  ) -> gix::Result<Option<gix::credentials::protocol::Outcome>> {
    match action {
      gix::credentials::helper::Action::Get(ctx) => Ok(Some(gix::credentials::protocol::Outcome {
        identity: gix::sec::identity::Account {
          username: "x-access-token".into(),
          password: token.to_owned(),
          oauth_refresh_token: None,
        },
        next: ctx.into(),
      })),
      _ => Ok(None),
    }
  }

  pub(super) fn fetch(
    repo: &gix::Repository,
    token: Option<String>,
  ) -> std::result::Result<(), Failure> {
    let remote = repo.find_remote("origin")?;
    let mut connection = remote.connect(gix::remote::Direction::Fetch)?;
    if let Some(t) = token {
      connection.set_credentials(move |action| credentials(&t, action));
    }
    connection
      .prepare_fetch(gix::progress::Discard, Default::default())?
      .receive(gix::progress::Discard, &AtomicBool::new(false))?;
    Ok(())
  }

  /// The commit the clone's branch is at on `origin`, as fetched: the
  /// remote branch of the branch the clone is on (`origin/HEAD` is written
  /// once, at the clone, and a fetch does not move it).
  pub(super) fn remote_head(repo: &gix::Repository) -> std::result::Result<gix::ObjectId, Failure> {
    if let Some(head) = repo.head_name()? {
      let short = head.shorten().to_string();
      if let Ok(mut r) = repo.find_reference(format!("refs/remotes/origin/{short}").as_str()) {
        return Ok(r.peel_to_id()?.detach());
      }
    }
    for name in [
      "refs/remotes/origin/HEAD",
      "refs/remotes/origin/main",
      "refs/remotes/origin/master",
    ] {
      if let Ok(mut r) = repo.find_reference(name) {
        return Ok(r.peel_to_id()?.detach());
      }
    }
    Err("no branch of origin to follow".into())
  }

  /// A commit's files: path, blob, and whether it is executable or a link.
  fn files(
    repo: &gix::Repository,
    commit: Option<gix::ObjectId>,
  ) -> std::result::Result<BTreeMap<String, (gix::ObjectId, EntryKind)>, Failure> {
    let mut out = BTreeMap::new();
    let Some(commit) = commit else { return Ok(out) };
    let tree = repo.find_commit(commit)?.tree()?;
    let mut rec = gix::traverse::tree::Recorder::default();
    tree.traverse().breadthfirst(&mut rec)?;
    for e in rec.records {
      if e.mode.is_tree() || e.mode.is_commit() {
        continue;
      }
      out.insert(e.filepath.to_string(), (e.oid, e.mode.kind()));
    }
    Ok(out)
  }

  fn blob(repo: &gix::Repository, oid: gix::ObjectId) -> std::result::Result<Vec<u8>, Failure> {
    Ok(repo.find_object(oid)?.detach().data)
  }

  /// A path another commit names that may not be written here.
  fn bad_path(path: &str) -> bool {
    path
      .split('/')
      .any(|c| c == ".." || c == ".git" || c.is_empty())
  }

  /// A work tree file as git would store it: its content (a link's target
  /// for a link) and kind.
  fn on_disk(file: &Path) -> Option<(Vec<u8>, EntryKind)> {
    let meta = std::fs::symlink_metadata(file).ok()?;
    if meta.file_type().is_symlink() {
      let target = std::fs::read_link(file).ok()?;
      return Some((
        target.to_string_lossy().into_owned().into_bytes(),
        EntryKind::Link,
      ));
    }
    if !meta.is_file() {
      return None;
    }
    #[cfg(unix)]
    let exec = {
      use std::os::unix::fs::PermissionsExt;
      meta.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let exec = false;
    let bytes = std::fs::read(file).ok()?;
    Some((
      bytes,
      if exec {
        EntryKind::BlobExecutable
      } else {
        EntryKind::Blob
      },
    ))
  }

  /// What the work tree changed from `head`: a path's new content and
  /// kind, or `None` when it was removed.
  type Local = BTreeMap<String, Option<(Vec<u8>, EntryKind)>>;

  fn local(
    repo: &gix::Repository,
    dir: &Path,
    head: Option<gix::ObjectId>,
  ) -> std::result::Result<Local, Failure> {
    let tracked = files(repo, head)?;
    let mut out = Local::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut walk = walkdir::WalkDir::new(dir).min_depth(1).into_iter();
    while let Some(Ok(e)) = walk.next() {
      if e.file_name() == ".git" {
        if e.file_type().is_dir() {
          walk.skip_current_dir();
        }
        continue;
      }
      if e.file_type().is_dir() {
        continue;
      }
      let Some(rel) = e.path().strip_prefix(dir).ok().and_then(|r| r.to_str()) else {
        continue;
      };
      let rel = rel.replace('\\', "/");
      let Some((bytes, kind)) = on_disk(e.path()) else {
        continue;
      };
      seen.insert(rel.clone());
      let oid = gix::objs::compute_hash(repo.object_hash(), gix::object::Kind::Blob, &bytes)?;
      if tracked.get(&rel) != Some(&(oid, kind)) {
        out.insert(rel, Some((bytes, kind)));
      }
    }
    for path in tracked.keys().filter(|p| !seen.contains(*p)) {
      out.insert(path.clone(), None);
    }
    Ok(out)
  }

  /// What moving the work tree did.
  pub(super) struct Moved {
    pub files: usize,
    /// Paths where the work tree's own changes and the commit's overlap:
    /// nothing was touched.
    pub conflicts: Vec<String>,
  }

  /// Moves the work tree and the index from `before` to `after`, its own
  /// changes kept: a file both changed is merged three ways, and when the
  /// two overlap nothing is touched (the conflicts are said). The branch is
  /// the caller's to move.
  pub(super) fn advance(
    repo: &gix::Repository,
    dir: &Path,
    before: Option<gix::ObjectId>,
    after: gix::ObjectId,
  ) -> std::result::Result<Moved, Failure> {
    let ours = local(repo, dir, before)?;
    let (old, new) = (files(repo, before)?, files(repo, Some(after))?);
    let mut writes: BTreeMap<String, Option<(Vec<u8>, EntryKind)>> = BTreeMap::new();
    let mut conflicts = Vec::new();
    let paths: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for path in paths {
      let (o, n) = (old.get(path), new.get(path));
      if o == n || bad_path(path) {
        continue;
      }
      let theirs = match n {
        Some((oid, kind)) => Some((blob(repo, *oid)?, *kind)),
        None => None,
      };
      match ours.get(path) {
        None => {
          writes.insert(path.clone(), theirs);
        }
        Some(mine) => match (mine, theirs) {
          // Both made it the same.
          (Some(m), Some(t)) if *m == t => {}
          (None, None) => {}
          (Some((m, kind)), Some((t, _))) => {
            let base = match o {
              Some((oid, _)) => blob(repo, *oid)?,
              None => Vec::new(),
            };
            let merged = match (
              String::from_utf8(base),
              std::str::from_utf8(m),
              std::str::from_utf8(&t),
            ) {
              (Ok(b), Ok(m), Ok(t)) if o.is_some() => diffy::merge(&b, m, t).ok(),
              _ => None,
            };
            match merged {
              Some(text) => {
                writes.insert(path.clone(), Some((text.into_bytes(), *kind)));
              }
              None => conflicts.push(path.clone()),
            }
          }
          // Removed on one side, changed on the other.
          _ => conflicts.push(path.clone()),
        },
      }
    }
    if !conflicts.is_empty() {
      return Ok(Moved {
        files: 0,
        conflicts,
      });
    }
    let n = writes.len();
    for (path, content) in writes {
      write(dir, &path, content)?;
    }
    let tree = repo.find_commit(after)?.tree_id()?;
    let mut index = repo.index_from_tree(&tree)?;
    index.write(Default::default())?;
    Ok(Moved {
      files: n,
      conflicts,
    })
  }

  /// Writes a work tree file (or removes it, and directories it leaves
  /// empty).
  fn write(
    dir: &Path,
    path: &str,
    content: Option<(Vec<u8>, EntryKind)>,
  ) -> std::result::Result<(), Failure> {
    let file = dir.join(path);
    match content {
      Some((data, kind)) => {
        if let Some(p) = file.parent() {
          std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::remove_file(&file);
        match kind {
          EntryKind::Link => {
            #[cfg(unix)]
            std::os::unix::fs::symlink(String::from_utf8_lossy(&data).as_ref(), &file)?;
          }
          _ => {
            std::fs::write(&file, &data)?;
            #[cfg(unix)]
            if kind == EntryKind::BlobExecutable {
              use std::os::unix::fs::PermissionsExt;
              std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
            }
          }
        }
      }
      None => {
        if std::fs::remove_file(&file).is_ok() {
          let mut d = file.parent();
          while let Some(x) = d.filter(|x| *x != dir) {
            if std::fs::remove_dir(x).is_err() {
              break;
            }
            d = x.parent();
          }
        }
      }
    }
    Ok(())
  }

  /// The branch the clone is on (HEAD itself when detached) set to `to`.
  fn set_branch(repo: &gix::Repository, to: gix::ObjectId) -> std::result::Result<(), Failure> {
    let message = format!("stitch: at {to}");
    let name = match repo.head_name()? {
      Some(name) => name.as_bstr().to_string(),
      None => "HEAD".into(),
    };
    repo.reference(
      name.as_str(),
      to,
      gix::refs::transaction::PreviousValue::Any,
      message,
    )?;
    Ok(())
  }

  /// HEAD made to name branch `branch` (`refs/heads/<branch>`, set to `at`).
  fn checkout_branch(
    repo: &gix::Repository,
    branch: &str,
    at: gix::ObjectId,
  ) -> std::result::Result<(), Failure> {
    use gix::refs::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
    let full = format!("refs/heads/{branch}");
    repo.reference(
      full.as_str(),
      at,
      PreviousValue::Any,
      format!("stitch: {branch} at {at}"),
    )?;
    repo.edit_reference(RefEdit {
      change: Change::Update {
        log: LogChange {
          mode: RefLog::AndReference,
          force_create_reflog: false,
          message: format!("stitch: on {branch}").into(),
        },
        expected: PreviousValue::Any,
        new: gix::refs::Target::Symbolic(full.as_str().try_into()?),
      },
      name: "HEAD".try_into()?,
      deref: false,
    })?;
    Ok(())
  }

  pub fn branches(dir: &Path) -> Result<Vec<Branch>> {
    let repo = open(dir).map_err(err)?;
    let current = repo
      .head_name()
      .map_err(err)?
      .map(|n| n.shorten().to_string());
    let mut out = Vec::new();
    let refs = repo.references().map_err(err)?;
    for r in refs.remote_branches().map_err(err)? {
      let r = r.map_err(err)?;
      let full = r.name().as_bstr().to_string();
      let Some(name) = full.strip_prefix("refs/remotes/origin/") else {
        continue;
      };
      if name == "HEAD" {
        continue;
      }
      out.push(Branch {
        current: current.as_deref() == Some(name),
        name: name.to_owned(),
      });
    }
    out.sort_by(|a, b| (!a.current, &a.name).cmp(&(!b.current, &b.name)));
    Ok(out)
  }

  pub fn changes(dir: &Path) -> Result<Vec<Changed>> {
    let repo = open(dir).map_err(err)?;
    let head = repo.head_id().ok().map(|i| i.detach());
    let tracked = files(&repo, head).map_err(err)?;
    Ok(
      local(&repo, dir, head)
        .map_err(err)?
        .into_iter()
        .map(|(path, c)| Changed {
          status: match (c, tracked.contains_key(&path)) {
            (None, _) => "deleted",
            (Some(_), true) => "modified",
            (Some(_), false) => "added",
          },
          path,
        })
        .collect(),
    )
  }

  pub fn diff(dir: &Path, only: Option<&str>) -> Result<String> {
    let repo = open(dir).map_err(err)?;
    let head = repo.head_id().ok().map(|i| i.detach());
    let tracked = files(&repo, head).map_err(err)?;
    let mut out = String::new();
    for (path, c) in local(&repo, dir, head).map_err(err)? {
      if only.is_some_and(|o| o != path) {
        continue;
      }
      let old = match tracked.get(&path) {
        Some((oid, _)) => blob(&repo, *oid).map_err(err)?,
        None => Vec::new(),
      };
      let new = c.map(|(b, _)| b).unwrap_or_default();
      let (a, b) = (String::from_utf8_lossy(&old), String::from_utf8_lossy(&new));
      let from = if tracked.contains_key(&path) {
        format!("a/{path}")
      } else {
        "/dev/null".into()
      };
      let to = if new.is_empty() && !tracked.contains_key(&path) || !dir.join(&path).exists() {
        "/dev/null".to_owned()
      } else {
        format!("b/{path}")
      };
      out.push_str(
        &similar::TextDiff::from_lines(a.as_ref(), b.as_ref())
          .unified_diff()
          .header(&from, &to)
          .to_string(),
      );
    }
    Ok(out)
  }

  pub fn discard(dir: &Path, only: Option<&str>) -> Result<usize> {
    let repo = open(dir).map_err(err)?;
    let head = repo.head_id().ok().map(|i| i.detach());
    let tracked = files(&repo, head).map_err(err)?;
    let mut n = 0;
    for (path, _) in local(&repo, dir, head).map_err(err)? {
      if only.is_some_and(|o| o != path) {
        continue;
      }
      let back = match tracked.get(&path) {
        Some((oid, kind)) => Some((blob(&repo, *oid).map_err(err)?, *kind)),
        None => None,
      };
      write(dir, &path, back).map_err(err)?;
      n += 1;
    }
    Ok(n)
  }

  pub fn switch(dir: &Path, branch: &str) -> Result<usize> {
    let repo = open(dir).map_err(err)?;
    let head = repo.head_id().ok().map(|i| i.detach());
    if !local(&repo, dir, head).map_err(err)?.is_empty() {
      return Err(Error::Config(
        "the clone has changes of its own: commit or discard them before switching".into(),
      ));
    }
    let target = repo
      .find_reference(format!("refs/remotes/origin/{branch}").as_str())
      .map_err(|_| Error::Config(format!("no branch {branch} on origin")))?
      .peel_to_id()
      .map_err(err)?
      .detach();
    let moved = advance(&repo, dir, head, target).map_err(err)?;
    checkout_branch(&repo, branch, target).map_err(err)?;
    Ok(moved.files)
  }

  pub fn staged(dir: &Path) -> Result<Staged> {
    let repo = open(dir).map_err(err)?;
    let head = repo.head_id().map_err(err)?.detach();
    let branch = repo
      .head_name()
      .map_err(err)?
      .ok_or_else(|| Error::Config("the clone is on no branch".into()))?
      .shorten()
      .to_string();
    let tree = repo
      .find_commit(head)
      .map_err(err)?
      .tree_id()
      .map_err(err)?
      .to_string();
    let files = local(&repo, dir, Some(head))
      .map_err(err)?
      .into_iter()
      .map(|(path, c)| StagedFile {
        mode: match c.as_ref().map(|(_, k)| *k) {
          Some(EntryKind::BlobExecutable) => "100755",
          Some(EntryKind::Link) => "120000",
          _ => "100644",
        },
        content: c.map(|(b, _)| b),
        path,
      })
      .collect();
    Ok(Staged {
      head: head.to_string(),
      tree,
      branch,
      files,
    })
  }

  /// After a push: the commit fetched, and the clone on `branch` at it (its
  /// files are already as the commit has them).
  pub fn pushed(dir: &Path, token: &str, branch: &str, commit: &str) -> Result<()> {
    let repo = open(dir).map_err(err)?;
    fetch(&repo, Some(token.to_owned())).map_err(err)?;
    let commit: gix::ObjectId = commit.parse().map_err(err)?;
    checkout_branch(&repo, branch, commit).map_err(err)?;
    let tree = repo
      .find_commit(commit)
      .map_err(err)?
      .tree_id()
      .map_err(err)?;
    let mut index = repo.index_from_tree(&tree).map_err(err)?;
    index.write(Default::default()).map_err(err)?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::process::Command;

  fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
      .arg("-C")
      .arg(dir)
      .args(args)
      .env("GIT_AUTHOR_NAME", "t")
      .env("GIT_AUTHOR_EMAIL", "t@t")
      .env("GIT_COMMITTER_NAME", "t")
      .env("GIT_COMMITTER_EMAIL", "t@t")
      .status()
      .unwrap()
      .success();
    assert!(ok, "git {args:?}");
  }

  #[test]
  fn names_and_urls() {
    assert_eq!(
      url_of("github.com/org/repo").as_deref(),
      Some("https://github.com/org/repo.git")
    );
    assert_eq!(url_of("/srv/git/repo"), None);
    assert_eq!(github_repo("github.com/org/repo"), Some(("org", "repo")));
    assert_eq!(github_repo("gitlab.com/org/repo"), None);
    assert!(valid_branch("stitch/fix-docs"));
    for bad in ["", "-x", "a..b", "a/", "x.lock", "a b", "a//b"] {
      assert!(!valid_branch(bad), "{bad}");
    }
  }

  #[test]
  fn the_answer_to_cloning_is_kept_per_project() {
    let d = tempfile::tempdir().unwrap();
    let a: RepoId = "p-0123456789abcdef".parse().unwrap();
    let b: RepoId = "p-fedcba9876543210".parse().unwrap();
    assert_eq!(decision(d.path(), &a), None);
    decide(d.path(), &a, false).unwrap();
    decide(d.path(), &b, true).unwrap();
    assert_eq!(decision(d.path(), &a), Some(false));
    decide(d.path(), &a, true).unwrap();
    assert_eq!(decision(d.path(), &a), Some(true));
    assert_eq!(decision(d.path(), &b), Some(true));
    assert_eq!(said_size(512), "512 KB");
    assert_eq!(said_size(123_456), "123 MB");
    assert_eq!(said_size(2_345_678), "2.3 GB");
  }

  /// A clone of a local repository (file://, as gitoxide clones any URL),
  /// as `update` makes one.
  fn cloned(d: &Path) -> (PathBuf, PathBuf) {
    let origin = d.join("origin");
    std::fs::create_dir_all(origin.join("src")).unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    std::fs::write(origin.join("AGENTS.md"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    std::fs::write(origin.join("src/old.rs"), "old\n").unwrap();
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-qm", "one"]);
    git(&origin, &["branch", "docs"]);
    let clone = d.join("clone");
    let url = format!("file://{}", origin.display());
    let mut prepare = gix::prepare_clone(url.as_str(), &clone).unwrap();
    let (mut co, _) = prepare
      .fetch_then_checkout(
        gix::progress::Discard,
        &std::sync::atomic::AtomicBool::new(false),
      )
      .unwrap();
    co.main_worktree(
      gix::progress::Discard,
      &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    (origin, clone)
  }

  /// A failed fetch says its causes, not gix's top error alone ("Transport
  /// handshake failed" was all the phone said).
  #[test]
  fn a_failed_fetch_says_its_causes() {
    #[derive(Debug)]
    struct E(&'static str, Option<Box<E>>);
    impl std::fmt::Display for E {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
      }
    }
    impl std::error::Error for E {
      fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1.as_deref().map(|e| e as _)
      }
    }
    let e = E(
      "Transport handshake failed",
      Some(Box::new(E(
        "handshake failed",
        Some(Box::new(E("connection refused", None))),
      ))),
    );
    // A cause the error says already is not said again.
    assert_eq!(
      git::chain(&e),
      "Transport handshake failed: connection refused"
    );
    // A real one: the remote gone.
    let d = tempfile::tempdir().unwrap();
    let (origin, clone) = cloned(d.path());
    std::fs::remove_dir_all(&origin).unwrap();
    let repo = git::open(&clone).unwrap();
    let failed = git::fetch(&repo, None).unwrap_err();
    let said = git::chain(&*failed);
    assert!(
      said.len() > failed.to_string().len(),
      "{said:?} says no more than {failed}"
    );
  }

  /// What `update` does on a clone, without its URL rules.
  fn pull(clone: &Path) -> (usize, Vec<String>) {
    let repo = git::open(clone).unwrap();
    let before = repo.head_id().ok().map(|i| i.detach());
    git::fetch(&repo, None).unwrap();
    let target = git::remote_head(&repo).unwrap();
    if Some(target) == before {
      return (0, Vec::new());
    }
    let moved = git::advance(&repo, clone, before, target).unwrap();
    if moved.conflicts.is_empty() {
      let message = format!("stitch: at {target}");
      let name = repo.head_name().unwrap().unwrap().as_bstr().to_string();
      repo
        .reference(
          name.as_str(),
          target,
          gix::refs::transaction::PreviousValue::Any,
          message,
        )
        .unwrap();
    }
    (moved.files, moved.conflicts)
  }

  /// The phone's clone of stitch: an empty `packed-refs.lock` left by an
  /// update the process did not finish, and every fetch since failed
  /// ("Failed to update references to their new position…").
  #[test]
  fn a_stale_lock_left_by_an_interrupted_update_is_cleared() {
    let d = tempfile::tempdir().unwrap();
    let (origin, clone) = cloned(d.path());
    std::fs::write(origin.join("new.md"), "new\n").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "two"]);
    let git_dir = clone.join(".git");
    let left = [
      git_dir.join("packed-refs.lock"),
      git_dir.join("refs/remotes/origin/main.lock"),
    ];
    for f in &left {
      std::fs::create_dir_all(f.parent().unwrap()).unwrap();
      std::fs::write(f, "").unwrap();
    }
    // Their own branches' lock files are not git's.
    std::fs::write(clone.join("mine.lock"), "mine\n").unwrap();
    let repo = git::open(&clone).unwrap();
    let failed = git::fetch(&repo, None).unwrap_err();
    let said = git::chain(&*failed);
    assert!(
      said.contains("Failed to update references"),
      "the bug: {said}"
    );
    drop(repo);
    assert_eq!(stale_locks(&clone), 2);
    for f in &left {
      assert!(!f.exists(), "{} removed", f.display());
    }
    assert!(clone.join("mine.lock").exists());
    let (n, conflicts) = pull(&clone);
    assert!(conflicts.is_empty());
    assert_eq!(n, 1);
    assert!(clone.join("new.md").exists());
    assert_eq!(stale_locks(&clone), 0);
  }

  /// A refresh its caller stopped waiting for (`before_turn`'s 20 seconds)
  /// keeps the clone's lock until its blocking work ends: no second fetch
  /// of the clone runs beside it.
  #[tokio::test]
  async fn a_timed_out_operation_keeps_the_clone_locked_until_it_ends() {
    let clones = Clones::default();
    let repo: RepoId = "p-0123456789abcdef".parse().unwrap();
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let (done_tx, done) = tokio::sync::oneshot::channel();
    let held = clones.hold(&repo).await;
    let work = blocking(held, move || {
      wait.recv().ok();
      done_tx.send(()).ok();
      Ok(())
    });
    let timed_out = tokio::time::timeout(std::time::Duration::from_millis(50), work).await;
    assert!(timed_out.is_err());
    assert!(
      clones.lock(&repo).try_lock().is_err(),
      "still held by the blocking work"
    );
    go.send(()).unwrap();
    done.await.unwrap();
    // The blocking closure dropped its guard as it returned.
    tokio::time::timeout(std::time::Duration::from_secs(5), clones.hold(&repo))
      .await
      .expect("free once the work ended");
  }

  #[test]
  fn a_clone_follows_its_remote_its_own_changes_kept() {
    let d = tempfile::tempdir().unwrap();
    let (origin, clone) = cloned(d.path());
    // Upstream: AGENTS.md's first line, a file gone, one new.
    std::fs::write(origin.join("AGENTS.md"), "ONE\ntwo\nthree\nfour\nfive\n").unwrap();
    std::fs::remove_file(origin.join("src/old.rs")).unwrap();
    std::fs::write(origin.join("new.md"), "new\n").unwrap();
    git(&origin, &["add", "-A"]);
    git(&origin, &["commit", "-qm", "two"]);
    // Here: its last line, and a file of its own.
    std::fs::write(clone.join("AGENTS.md"), "one\ntwo\nthree\nfour\nFIVE\n").unwrap();
    std::fs::write(clone.join("mine.md"), "mine\n").unwrap();
    let (n, conflicts) = pull(&clone);
    assert!(conflicts.is_empty(), "{conflicts:?}");
    assert_eq!(n, 3);
    assert_eq!(
      std::fs::read_to_string(clone.join("AGENTS.md")).unwrap(),
      "ONE\ntwo\nthree\nfour\nFIVE\n",
      "merged three ways"
    );
    assert!(clone.join("new.md").exists() && !clone.join("src").exists());
    let changes = git::changes(&clone).unwrap();
    let shown: Vec<(&str, &str)> = changes
      .iter()
      .map(|c| (c.path.as_str(), c.status))
      .collect();
    assert_eq!(shown, [("AGENTS.md", "modified"), ("mine.md", "added")]);
    let diff = git::diff(&clone, Some("AGENTS.md")).unwrap();
    assert!(diff.contains("-five") && diff.contains("+FIVE"), "{diff}");

    // Overlapping: nothing is touched, the file is named.
    std::fs::write(
      origin.join("AGENTS.md"),
      "ONE\ntwo\nthree\nfour\nfive, upstream\n",
    )
    .unwrap();
    git(&origin, &["commit", "-qam", "three"]);
    let (_, conflicts) = pull(&clone);
    assert_eq!(conflicts, ["AGENTS.md"]);
    assert_eq!(
      std::fs::read_to_string(clone.join("AGENTS.md")).unwrap(),
      "ONE\ntwo\nthree\nfour\nFIVE\n"
    );

    // Discarded, then the pull goes through; then another branch.
    assert_eq!(git::discard(&clone, None).unwrap(), 2);
    assert!(git::changes(&clone).unwrap().is_empty());
    let (_, conflicts) = pull(&clone);
    assert!(conflicts.is_empty());
    assert_eq!(
      std::fs::read_to_string(clone.join("AGENTS.md")).unwrap(),
      "ONE\ntwo\nthree\nfour\nfive, upstream\n"
    );
    let names: Vec<String> = git::branches(&clone)
      .unwrap()
      .into_iter()
      .map(|b| b.name)
      .collect();
    assert_eq!(names, ["main", "docs"]);
    std::fs::write(clone.join("x.md"), "x\n").unwrap();
    assert!(
      git::switch(&clone, "docs").is_err(),
      "not with changes of its own"
    );
    std::fs::remove_file(clone.join("x.md")).unwrap();
    git::switch(&clone, "docs").unwrap();
    assert_eq!(
      std::fs::read_to_string(clone.join("AGENTS.md")).unwrap(),
      "one\ntwo\nthree\nfour\nfive\n",
      "the docs branch, as it was made"
    );
    let b = git::branches(&clone).unwrap();
    assert!(b.iter().any(|b| b.name == "docs" && b.current));
    // What a commit from here would carry.
    std::fs::write(clone.join("AGENTS.md"), "edited\n").unwrap();
    let staged = git::staged(&clone).unwrap();
    assert_eq!(staged.branch, "docs");
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].content.as_deref(), Some(&b"edited\n"[..]));
  }
}
