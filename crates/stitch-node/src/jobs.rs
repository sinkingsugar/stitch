//! Jobs: what the node watches for a thread at no model cost, and tells
//! it of: a session's command ending, a line of its output, what its
//! screen shows (or stopped showing, or the screen going quiet), a file
//! changing, a line appearing in it, or it going quiet. A job fires once;
//! the thread is told, woven into its loop or woken (see `detached::notify`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use stitch_records::RecordId;
use stitch_shell::{Regex, Session, Status, StopSignal, Until, Wait};

/// Tells a thread something from the node (set once the node runs):
/// woven into its running loop, else waking it.
pub type Notifier = Arc<dyn Fn(RecordId, String) + Send + Sync>;

/// Weaves a note into a thread's running loop, never waking it; `false`
/// when none runs. A note the loop ends without is given back by the
/// `Untold`.
pub type Weaver = Arc<dyn Fn(RecordId, String, crate::chat::Untold) -> bool + Send + Sync>;

/// How long a person's typing pauses before the thread is told of it.
const TYPED_PAUSE: Duration = Duration::from_millis(1500);

/// What a job watches.
pub(crate) enum On {
  /// A session (by name): its command's end, a line of its output, its
  /// screen.
  Session {
    name: String,
    session: Session,
    until: Until,
  },
  /// A file.
  File { path: PathBuf, until: FileUntil },
}

/// Conditions on a file; those given must all hold, and with none it
/// fires on any change.
#[derive(Clone, Debug, Default)]
pub(crate) struct FileUntil {
  /// A line matching this was appended (since the job started).
  pub line: Option<Regex>,
  /// It has not changed for this long.
  pub quiet: Option<Duration>,
}

/// A job's time limit (`max_secs`): when it passes, the job fires as
/// timed out, and with `stop` the command it watches is stopped as
/// `job_stop` does.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Limit {
  pub max: Option<Duration>,
  pub stop: bool,
}

#[derive(Clone)]
enum Target {
  Session(String),
  File(PathBuf),
}

struct Job {
  id: u64,
  thread: RecordId,
  target: Target,
  /// The command it started on, for a session.
  command: String,
  /// When that command was entered: the one `job_stop` stops and
  /// `job_output` reads (none: nothing ran when it started).
  entered: Option<Instant>,
  /// What it waits for, as said.
  condition: String,
  /// The line pattern, for a session job on its output.
  line: Option<String>,
  /// A job on a session's screen.
  screen: bool,
  limit: Limit,
  started: jiff::Timestamp,
  task: tokio::task::AbortHandle,
}

impl Job {
  /// Its condition, with its time limit.
  fn said(&self) -> String {
    format!(
      "{}{}",
      self.condition,
      limit_said(self.limit, self.entered.is_some())
    )
  }
}

/// A job that fired, was stopped or cancelled: listed for an hour, its
/// output kept (`job_output`).
struct Ended {
  id: u64,
  thread: RecordId,
  target: Target,
  command: String,
  entered: Option<Instant>,
  condition: String,
  started: jiff::Timestamp,
  at: Instant,
  /// How it ended, as said: `ended with exit 0`, `stopped`, `timed out`, …
  state: String,
  /// What its command printed, at most `OUTPUT_MAX` (its end).
  output: Option<String>,
}

/// How long ended jobs stay listed.
const ENDED_KEPT: Duration = Duration::from_secs(3600);
/// Ended jobs kept at most, and the output they keep in all.
const ENDED_MAX: usize = 200;
const ENDED_BYTES: usize = 16 << 20;
/// A job's output kept, at most: its end.
const OUTPUT_MAX: usize = 1 << 20;

/// Where `job_stop` starts: Ctrl+C (then SIGTERM, then SIGKILL), SIGTERM
/// (then SIGKILL), or SIGKILL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopFrom {
  Int,
  Term,
  Kill,
}

impl StopFrom {
  pub(crate) fn parse(s: &str) -> Option<Self> {
    match s.trim().to_ascii_lowercase().trim_start_matches("sig") {
      "int" | "ctrl-c" | "" => Some(StopFrom::Int),
      "term" => Some(StopFrom::Term),
      "kill" => Some(StopFrom::Kill),
      _ => None,
    }
  }
}

/// How a command was stopped: what was done, as said, and whether it ended.
pub(crate) struct Stopped {
  pub said: Vec<String>,
  pub ended: bool,
}

/// How long SIGKILL is given (a list's next command, started meanwhile,
/// gets it too).
const KILL_WINDOW: Duration = Duration::from_secs(2);

fn cap_output(text: String) -> String {
  if text.len() <= OUTPUT_MAX {
    return text;
  }
  let mut from = text.len() - OUTPUT_MAX;
  while !text.is_char_boundary(from) {
    from += 1;
  }
  // From a line's start.
  let from = text[from..].find('\n').map_or(from, |i| from + i + 1);
  format!("[… {} bytes before cut …]\n{}", from, &text[from..])
}

/// Drops ended jobs past the hour, past `ENDED_MAX`, and the output of the
/// oldest past `ENDED_BYTES` in all.
fn prune(ended: &mut std::collections::VecDeque<Ended>) {
  while ended.front().is_some_and(|e| e.at.elapsed() > ENDED_KEPT) || ended.len() > ENDED_MAX {
    ended.pop_front();
  }
  let mut total: usize = ended
    .iter()
    .map(|e| e.output.as_ref().map_or(0, String::len))
    .sum();
  for e in ended.iter_mut() {
    if total <= ENDED_BYTES {
      break;
    }
    if let Some(o) = e.output.take() {
      total -= o.len();
    }
  }
}

/// Stops what is left of a stopped command's process groups (children
/// that ignored Ctrl+C, orphaned there when the command ended): SIGTERM,
/// then SIGKILL a second later; what was done, as said.
async fn leftovers(groups: &[i32]) -> Option<String> {
  use stitch_shell::signal_group;
  let left: Vec<i32> = groups
    .iter()
    .copied()
    .filter(|g| signal_group(*g, None))
    .collect();
  if left.is_empty() {
    return None;
  }
  let settle = |gs: Vec<i32>| async move {
    for _ in 0..20 {
      if gs.iter().all(|g| !signal_group(*g, None)) {
        break;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    gs.into_iter()
      .filter(|g| signal_group(*g, None))
      .collect::<Vec<i32>>()
  };
  for g in &left {
    signal_group(*g, Some(StopSignal::Term));
  }
  let still = settle(left.clone()).await;
  for g in &still {
    signal_group(*g, Some(StopSignal::Kill));
  }
  let after = settle(still.clone()).await;
  let named = left
    .iter()
    .map(i32::to_string)
    .collect::<Vec<_>>()
    .join(", ");
  Some(format!(
    "processes left in its process group {named} once it ended got SIGTERM{}{}",
    if still.is_empty() {
      ""
    } else {
      ", then SIGKILL"
    },
    if after.is_empty() {
      String::new()
    } else {
      " (some are still there)".into()
    }
  ))
}

/// A job as its notes and lists name it: `Job 3 (session "bg3", `make`)`.
fn what_of(id: u64, target: &Target, command: &str) -> String {
  format!("Job {id} ({})", target_of(target, command))
}

/// What a job is on: `session "bg3", `make``, or `file <path>`.
fn target_of(target: &Target, command: &str) -> String {
  match target {
    Target::Session(name) if command.is_empty() => format!("session \"{name}\""),
    Target::Session(name) => format!("session \"{name}\", `{command}`"),
    Target::File(path) => format!("file {}", path.display()),
  }
}

/// Lines a listed job shows of its output, and a stopped one.
const LIST_LINES: usize = 5;
const STOP_LINES: usize = 20;
/// What one `job_output` call returns at most.
const OUTPUT_LINES_MAX: usize = 2000;
const OUTPUT_CHARS_MAX: usize = 32_000;

/// The last `n` lines of `text`.
fn tail_lines(text: &str, n: usize) -> String {
  let lines: Vec<&str> = text.lines().collect();
  lines[lines.len().saturating_sub(n)..].join("\n")
}

/// A listed job's last lines, indented under it.
fn last_lines(output: &str) -> String {
  let last = tail_lines(output.trim_end(), LIST_LINES);
  if last.is_empty() {
    return String::new();
  }
  let mut out = String::new();
  for l in last.lines() {
    out.push_str("\n  │ ");
    out.push_str(l);
  }
  out
}

/// `job_stop`'s result: what was done, then the job's last lines.
fn with_last_lines(text: String, output: Option<&str>, id: u64) -> String {
  let Some(o) = output.filter(|o| !o.trim().is_empty()) else {
    return text;
  };
  let n = o.lines().count();
  let more = if n > STOP_LINES {
    format!(" (job_output {{\"job\": {id}}} has all {n} lines)")
  } else {
    String::new()
  };
  format!(
    "{text}\nIts last lines{more}:\n{}",
    tail_lines(o, STOP_LINES)
  )
}

fn local_time(t: jiff::Timestamp) -> String {
  t.to_zoned(jiff::tz::TimeZone::system())
    .strftime("%H:%M:%S")
    .to_string()
}

/// A running job as `Shells::live` reads it: id, thread, target,
/// command, when it was entered, its condition, when it started.
type LiveJob = (
  u64,
  RecordId,
  Target,
  String,
  Option<Instant>,
  String,
  jiff::Timestamp,
);

/// A row of the jobs list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
  /// `shell` (a command running) or `watch` (a job on anything else).
  pub kind: &'static str,
  /// What `Shells::stop` takes.
  pub id: String,
  pub thread: RecordId,
  pub what: String,
  pub state: String,
  pub started: jiff::Timestamp,
}

/// The node's shell sessions, by thread and name, and its jobs.
#[derive(Default)]
pub struct Shells {
  pub(crate) sessions: tokio::sync::Mutex<HashMap<String, Session>>,
  /// By thread: the Ctrl+C going to the command a call entered and was
  /// cut short waiting on, for the interrupt to await.
  pub(crate) stopping_calls: std::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
  jobs: std::sync::Mutex<Vec<Job>>,
  /// Jobs that ended within the hour, oldest first.
  ended: std::sync::Mutex<std::collections::VecDeque<Ended>>,
  pub(crate) last_job: AtomicU64,
  /// Sessions the user stopped once: stopped again, they are closed.
  stopping: std::sync::Mutex<HashSet<String>>,
  notify: OnceLock<Notifier>,
  weave: OnceLock<Weaver>,
  /// Typing in attached sessions, by session: a count that a pause checks.
  typing: std::sync::Mutex<HashMap<String, u64>>,
  /// Sessions started for one background command (none named): closed
  /// once it ends and the thread is told (or it waited for it).
  auto: std::sync::Mutex<HashSet<String>>,
  /// Sessions started outside the sandbox on purpose (`outside`), by a
  /// loop that has one: kept so, unlike one left from yolo.
  pub(crate) outside: std::sync::Mutex<HashSet<String>>,
  /// Sessions started while their thread was read-only (in the read-only
  /// sandbox, where there is one); the others were started writable.
  pub(crate) read_only: std::sync::Mutex<HashSet<String>>,
  /// Sessions started writable for a read-only thread's command the user
  /// allowed once (a room participant's, design §2.27): its thread is
  /// read-only by its record, yet the session is not one a switch to
  /// read-only left (`writable_threads`); its next call starts it again
  /// read-only.
  pub(crate) once: std::sync::Mutex<HashSet<String>>,
  /// Text typed at a session's prompt without Enter (`shell_send`), by
  /// session: the command line the next Enter runs begins with it, and is
  /// judged whole (design §2.28).
  pub(crate) pending: std::sync::Mutex<HashMap<String, String>>,
}

pub(crate) fn session_key(thread: &RecordId, name: &str) -> String {
  format!("{thread}:{name}")
}

fn ago(since: Instant) -> jiff::Timestamp {
  jiff::Timestamp::now() - jiff::SignedDuration::try_from(since.elapsed()).unwrap_or_default()
}

/// How a job's condition is said: `tells when …`.
pub(crate) fn condition_of(until: &Until) -> String {
  match until {
    Until::Done => "tells when it ends".into(),
    Until::Line(re) => format!("tells when it prints a line matching \"{re}\" or ends"),
    Until::Screen(on) => {
      let mut parts = Vec::new();
      if let Some(re) = &on.shows {
        parts.push(format!("shows \"{re}\""));
      }
      if let Some(re) = &on.gone {
        parts.push(format!("no longer shows \"{re}\""));
      }
      if let Some(q) = on.quiet {
        parts.push(format!("has been quiet {}", spoken(q)));
      }
      let said = match parts.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
      };
      format!("tells when the screen {said}")
    }
  }
}

pub(crate) fn file_condition_of(until: &FileUntil) -> String {
  match (&until.line, until.quiet) {
    (None, None) => "tells when it changes".into(),
    (Some(re), None) => format!("tells when a line matching \"{re}\" is added"),
    (None, Some(q)) => format!("tells when it has not changed for {}", spoken(q)),
    (Some(re), Some(q)) => format!(
      "tells when a line matching \"{re}\" was added and it has not changed for {} since",
      spoken(q)
    ),
  }
}

impl Shells {
  pub async fn close_all(&self) {
    // No job tells of sessions closing because the node stops.
    for j in self.jobs.lock().expect("jobs lock").drain(..) {
      j.task.abort();
    }
    let all: Vec<Session> = self.sessions.lock().await.drain().map(|(_, s)| s).collect();
    for s in all {
      s.close().await;
    }
  }

  /// What closing every session would cut, by thread: its jobs (`job 3
  /// \`cmd\` in session "pt"`, `job 4 watching <file>`), then the commands
  /// its sessions run with no job on them. Threads with nothing running
  /// are left out.
  pub(crate) async fn running_by_thread(&self) -> Vec<(RecordId, Vec<String>)> {
    let sessions: Vec<(String, Session)> = self
      .sessions
      .lock()
      .await
      .iter()
      .map(|(k, s)| (k.clone(), s.clone()))
      .collect();
    let mut out: Vec<(RecordId, Vec<String>)> = Vec::new();
    let mut add = |t: &RecordId, what: String| match out.iter_mut().find(|(o, _)| o == t) {
      Some((_, v)) => v.push(what),
      None => out.push((t.clone(), vec![what])),
    };
    let mut watched: HashSet<String> = HashSet::new();
    let mut jobs: Vec<(u64, RecordId, Target, String)> = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .map(|j| (j.id, j.thread.clone(), j.target.clone(), j.command.clone()))
      .collect();
    jobs.sort_by_key(|j| j.0);
    for (id, t, target, command) in jobs {
      match target {
        Target::Session(name) => {
          watched.insert(session_key(&t, &name));
          if command.is_empty() {
            add(&t, format!("job {id} on session \"{name}\""));
          } else {
            add(&t, format!("job {id} `{command}` in session \"{name}\""));
          }
        }
        Target::File(path) => add(&t, format!("job {id} watching {}", path.display())),
      }
    }
    let mut loose: Vec<(String, String, RecordId)> = Vec::new();
    for (key, s) in &sessions {
      if watched.contains(key) {
        continue;
      }
      let Some((t, name)) = key.split_once(':') else {
        continue;
      };
      let Ok(t) = t.parse::<RecordId>() else {
        continue;
      };
      if let Some(c) = s.command().filter(|c| c.running) {
        loose.push((name.to_owned(), c.text, t));
      }
    }
    loose.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    for (name, text, t) in loose {
      add(&t, format!("`{text}` in session \"{name}\" (no job on it)"));
    }
    out
  }

  pub fn set_notifier(&self, notify: Notifier) {
    let _ = self.notify.set(notify);
  }

  pub fn set_weaver(&self, weave: Weaver) {
    let _ = self.weave.set(weave);
  }

  fn next_job(&self) -> u64 {
    self.last_job.fetch_add(1, Ordering::Relaxed) + 1
  }

  fn take_job(&self, id: u64) -> Option<Job> {
    let mut jobs = self.jobs.lock().expect("jobs lock");
    let i = jobs.iter().position(|j| j.id == id)?;
    Some(jobs.remove(i))
  }

  /// Drops the jobs on a session (it was waited for, or closed).
  pub(crate) fn forget(&self, thread: &RecordId, session: &str) -> Vec<u64> {
    let gone: Vec<Job> = {
      let mut jobs = self.jobs.lock().expect("jobs lock");
      let (gone, kept) = jobs.drain(..).partition(|j| {
        &j.thread == thread && matches!(&j.target, Target::Session(s) if s == session)
      });
      *jobs = kept;
      gone
    };
    let ids = gone.iter().map(|j| j.id).collect();
    for j in gone {
      j.task.abort();
      self.ended(j, "ended: had by shell_wait".into(), None);
    }
    ids
  }

  /// Cancels a job of `thread` (any thread's when `None`), telling nothing.
  pub(crate) fn cancel(&self, id: u64, thread: Option<&RecordId>) -> bool {
    let job = {
      let mut jobs = self.jobs.lock().expect("jobs lock");
      let Some(i) = jobs
        .iter()
        .position(|j| j.id == id && thread.is_none_or(|t| &j.thread == t))
      else {
        return false;
      };
      jobs.remove(i)
    };
    job.task.abort();
    let state = match job.entered {
      Some(_) => "cancelled (unwatch): its command was left to run",
      None => "cancelled (unwatch)",
    };
    self.ended(job, state.into(), None);
    true
  }

  /// Keeps a job that ended, listed for an hour with its output.
  fn ended(&self, job: Job, state: String, output: Option<String>) {
    let condition = job.said();
    let mut ended = self.ended.lock().expect("ended lock");
    ended.push_back(Ended {
      id: job.id,
      thread: job.thread,
      target: job.target,
      command: job.command,
      entered: job.entered,
      condition,
      started: job.started,
      at: Instant::now(),
      state,
      output: output.map(cap_output),
    });
    prune(&mut ended);
  }

  /// Starts a job; returns its ID.
  pub(crate) fn watch(self: &Arc<Self>, thread: &RecordId, on: On, limit: Limit) -> u64 {
    let id = self.next_job();
    let screen = matches!(
      &on,
      On::Session {
        until: Until::Screen(_),
        ..
      }
    );
    let line = match &on {
      On::Session {
        until: Until::Line(re),
        ..
      } => Some(re.as_str().to_owned()),
      _ => None,
    };
    let (target, command, entered, condition, started) = match &on {
      On::Session {
        name,
        session,
        until,
      } => {
        let command = session.command().filter(|c| c.running);
        (
          Target::Session(name.clone()),
          command.as_ref().map(|c| c.text.clone()).unwrap_or_default(),
          command.as_ref().map(|c| c.started),
          condition_of(until),
          match (&command, until) {
            (Some(c), Until::Done | Until::Line(_)) => ago(c.started),
            _ => jiff::Timestamp::now(),
          },
        )
      }
      On::File { path, until } => (
        Target::File(path.clone()),
        String::new(),
        None,
        file_condition_of(until),
        jiff::Timestamp::now(),
      ),
    };
    // Held while it starts, so it is listed before it can fire.
    let mut jobs = self.jobs.lock().expect("jobs lock");
    let shells = self.clone();
    let task = tokio::spawn(async move {
      let mut on_session = None;
      let said = match on {
        On::Session {
          session,
          until,
          name,
        } => {
          let waited = session.wait_for(&until, limit.max).await;
          let timed_out =
            limit.max.is_some() && matches!(&waited, Ok(w) if !w.done && w.matched.is_none());
          // What the screen shows as it fired, for a screen job's note.
          let screen = session.screen().text;
          on_session = Some((name, session));
          if timed_out {
            Fired::TimedOut(waited, screen)
          } else {
            Fired::Session(waited, screen)
          }
        }
        On::File { path, until } => match limit.max {
          Some(max) => match tokio::time::timeout(max, file_until(&path, &until)).await {
            Ok(f) => Fired::File(f),
            Err(_) => Fired::File(FileFired::TimedOut(max)),
          },
          None => Fired::File(file_until(&path, &until).await),
        },
      };
      let Some(job) = shells.take_job(id) else {
        return;
      };
      // Stopped as `job_stop` does, when asked to at its time limit.
      let mut stopped = None;
      if let (Fired::TimedOut(..), Some((name, session)), Some(entered), true) =
        (&said, &on_session, job.entered, limit.stop)
      {
        stopped = Some(
          shells
            .stop_command(
              &job.thread,
              name,
              session,
              entered,
              StopFrom::Int,
              STOP_GRACE,
            )
            .await,
        );
      }
      let output = on_session
        .as_ref()
        .and_then(|(_, s)| output_of(s, job.entered))
        .or_else(|| match &said {
          Fired::Session(Ok(w), _) | Fired::TimedOut(Ok(w), _) => Some(w.outcome.output.clone()),
          _ => None,
        });
      let state = state_of(&said, stopped.as_ref());
      let ended = matches!(&said, Fired::Session(Ok(w), _) if w.done)
        || stopped.as_ref().is_some_and(|s| s.ended);
      let mut note = note_of(&job, said, stopped.as_ref());
      if ended
        && let Target::Session(name) = &job.target
        && shells.close_auto(&job.thread, name).await
      {
        note.push_str(&format!("\n[Session \"{name}\" is closed with it.]"));
      }
      let thread = job.thread.clone();
      shells.ended(job, state, output);
      if let Some(notify) = shells.notify.get() {
        notify(thread, note);
      }
    });
    jobs.push(Job {
      id,
      thread: thread.clone(),
      target,
      command,
      entered,
      condition,
      line,
      screen,
      limit,
      started,
      task: task.abort_handle(),
    });
    id
  }

  /// The jobs list: commands running in sessions (with the jobs on them),
  /// then the jobs on anything else.
  pub async fn listing(&self) -> Vec<Listed> {
    let sessions: Vec<(String, Session)> = self
      .sessions
      .lock()
      .await
      .iter()
      .map(|(k, s)| (k.clone(), s.clone()))
      .collect();
    let jobs = self.jobs.lock().expect("jobs lock");
    let mut covered = HashSet::new();
    let mut out = Vec::new();
    for (key, s) in sessions {
      let Some((thread, name)) = key.split_once(':') else {
        continue;
      };
      let Ok(thread) = thread.parse::<RecordId>() else {
        continue;
      };
      let Some(c) = s.command().filter(|c| c.running) else {
        continue;
      };
      let on: Vec<&Job> = jobs
        .iter()
        .filter(|j| j.thread == thread && matches!(&j.target, Target::Session(n) if n == name))
        .collect();
      covered.extend(on.iter().map(|j| j.id));
      out.push(Listed {
        kind: "shell",
        id: format!("{thread}/{name}"),
        thread,
        what: c.text,
        state: if on.is_empty() {
          "left running: nobody is told when it ends".into()
        } else {
          on.iter()
            .map(|j| format!("job {}: {}", j.id, j.said()))
            .collect::<Vec<_>>()
            .join("; ")
        },
        started: ago(c.started),
      });
    }
    for j in jobs.iter().filter(|j| !covered.contains(&j.id)) {
      out.push(Listed {
        kind: "watch",
        id: format!("job:{}", j.id),
        thread: j.thread.clone(),
        what: match &j.target {
          Target::Session(name) => format!("session {name}"),
          Target::File(path) => path.display().to_string(),
        },
        state: format!("job {}: {}", j.id, j.said()),
        started: j.started,
      });
    }
    out.sort_by_key(|r| r.started);
    out
  }

  /// Stops a `Listed::id`: a job (`job:N`) is cancelled, telling nothing;
  /// what runs in a session gets Ctrl+C, and the session is closed when
  /// stopped again (its jobs tell the thread how it ended).
  pub async fn stop(&self, id: &str) -> std::result::Result<String, String> {
    if let Some(n) = id.strip_prefix("job:") {
      let n: u64 = n.parse().map_err(|_| format!("no job {id}"))?;
      return if self.cancel(n, None) {
        Ok(format!("job {n} cancelled"))
      } else {
        Err(format!("no job {n}"))
      };
    }
    let Some((thread, name)) = id.split_once('/') else {
      return Err(format!("no job {id}"));
    };
    let thread: RecordId = thread.parse().map_err(|e| format!("{e}"))?;
    let key = session_key(&thread, name);
    let Some(session) = self.sessions.lock().await.get(&key).cloned() else {
      return Err(format!("no session {name} on thread {thread}"));
    };
    let again = !self
      .stopping
      .lock()
      .expect("stopping lock")
      .insert(key.clone());
    if again || !matches!(session.status(), Status::Running { .. }) {
      self.sessions.lock().await.remove(&key);
      self.stopping.lock().expect("stopping lock").remove(&key);
      session.close().await;
      return Ok(format!("session {name} closed"));
    }
    let wait = Wait {
      timeout: Duration::from_secs(2),
      idle: Some(Duration::from_millis(300)),
      screen: None,
    };
    let _ = session.send(b"\x03", wait).await;
    if !matches!(session.status(), Status::Running { .. }) {
      self.stopping.lock().expect("stopping lock").remove(&key);
    }
    Ok(format!(
      "sent Ctrl+C to session {name}{}",
      if matches!(session.status(), Status::Running { .. }) {
        "; it still runs: stop it again to close the session"
      } else {
        ""
      }
    ))
  }

  /// Stops the command entered at `entered` in session `name` of `thread`
  /// for real: Ctrl+C (up to three times), then, still running after
  /// `grace`, SIGTERM to its process group, then after `grace` again
  /// SIGKILL (each group the command's list goes on to gets it too). A
  /// command the shell itself runs (a loop of builtins) that outlives it
  /// all has its session closed. `from` starts later in that order.
  pub(crate) async fn stop_command(
    &self,
    thread: &RecordId,
    name: &str,
    session: &Session,
    entered: Instant,
    from: StopFrom,
    grace: Duration,
  ) -> Stopped {
    let runs = || {
      session.status() != Status::Closed
        && session
          .command()
          .is_some_and(|c| c.started == entered && c.running)
    };
    let ended = |mut said: Vec<String>, seen: Vec<i32>| async move {
      said.push(match session.status() {
        Status::AtPrompt { exit } => format!("it ended with exit {exit}"),
        _ => "it ended".into(),
      });
      said.extend(leftovers(&seen).await);
      Stopped { said, ended: true }
    };
    if !runs() {
      return ended(vec!["it had ended already".into()], Vec::new()).await;
    }
    let mut said = Vec::new();
    // The process groups the command ran in: what is left of them once it
    // ended (a child that ignored Ctrl+C) is stopped too.
    let mut seen: Vec<i32> = session.command_group().into_iter().collect();
    if from == StopFrom::Int {
      let deadline = Instant::now() + grace;
      let mut sent = 0;
      while runs() && sent < 3 {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
          break;
        }
        sent += 1;
        let each = left.min((grace / 3).max(Duration::from_millis(300)));
        if session.send(b"\x03", Wait::prompt(each)).await.is_err() {
          break;
        }
        seen.extend(session.command_group().filter(|g| !seen.contains(g)));
      }
      while runs() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
      said.push(match sent {
        0 | 1 => "Ctrl+C".to_owned(),
        n => format!("Ctrl+C ({n} times)"),
      });
      if !runs() {
        return ended(said, seen).await;
      }
      said.push(format!("still running after {}", spoken(grace)));
    }
    for (signal, called, window) in [
      (StopSignal::Term, "SIGTERM", grace),
      (StopSignal::Kill, "SIGKILL", KILL_WINDOW),
    ] {
      if signal == StopSignal::Term && from == StopFrom::Kill {
        continue;
      }
      let deadline = Instant::now() + window;
      let mut groups: Vec<i32> = Vec::new();
      while runs() {
        // On a host: the group holding its terminal there, signalled
        // there (once: its number means nothing here).
        if session.is_remote() {
          if groups.is_empty()
            && let Some(g) = session.signal_remote(signal).await
          {
            groups.push(g);
          }
        } else if let Some(g) = session.command_group()
          && !groups.contains(&g)
          && let Some(g) = session.signal_command(signal)
        {
          groups.push(g);
          if !seen.contains(&g) {
            seen.push(g);
          }
        }
        if Instant::now() >= deadline {
          break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
      said.push(match groups.as_slice() {
        [] => format!("{called}: no process group of its own to send it to"),
        [g] => format!("{called} to its process group {g}"),
        gs => format!(
          "{called} to its process groups {}",
          gs.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
        ),
      });
      if !runs() {
        return ended(said, seen).await;
      }
      if signal == StopSignal::Term {
        said.push(format!("still running after {}", spoken(window)));
      }
    }
    // The shell itself runs it: only the session's end stops it.
    let key = session_key(thread, name);
    self.sessions.lock().await.remove(&key);
    self.auto.lock().expect("auto lock").remove(&key);
    self.outside.lock().expect("outside lock").remove(&key);
    session.close().await;
    said.push(format!(
      "it still ran (the shell itself runs it), so session \"{name}\" was closed"
    ));
    said.extend(leftovers(&seen).await);
    Stopped { said, ended: true }
  }

  /// `job_stop`: stops a job of `thread` (any thread's when `None`) for
  /// real. A job on a command stops that command (`stop_command`), its
  /// notice with it; one that already fired stops the command it watched
  /// when that still runs; a job on anything else is cancelled.
  pub(crate) async fn stop_job(
    &self,
    id: u64,
    thread: Option<&RecordId>,
    from: StopFrom,
    grace: Duration,
  ) -> Result<String, String> {
    let live = {
      let mut jobs = self.jobs.lock().expect("jobs lock");
      jobs
        .iter()
        .position(|j| j.id == id && thread.is_none_or(|t| &j.thread == t))
        .map(|i| jobs.remove(i))
    };
    if let Some(job) = live {
      job.task.abort();
      let what = what_of(job.id, &job.target, &job.command);
      let (Target::Session(name), Some(entered)) = (job.target.clone(), job.entered) else {
        self.ended(
          job,
          "stopped: cancelled (it watched no command)".into(),
          None,
        );
        return Ok(format!(
          "{what}: cancelled; it watched no command, so nothing else was stopped."
        ));
      };
      let session = self
        .sessions
        .lock()
        .await
        .get(&session_key(&job.thread, &name))
        .cloned();
      let Some(session) = session else {
        self.ended(job, "stopped: its session was gone".into(), None);
        return Ok(format!(
          "{what}: its session is gone, and its command with it."
        ));
      };
      let stopped = self
        .stop_command(&job.thread, &name, &session, entered, from, grace)
        .await;
      let output = output_of(&session, Some(entered));
      let thread = job.thread.clone();
      let state = if stopped.ended {
        "stopped"
      } else {
        "stopping failed: it runs on"
      };
      self.ended(job, state.into(), output.clone());
      let mut text = format!("{what}: {}.", stopped.said.join("; "));
      if stopped.ended && self.close_auto(&thread, &name).await {
        text.push_str(&format!(" Session \"{name}\" is closed with it."));
      }
      return Ok(with_last_lines(text, output.as_deref(), id));
    }
    // Fired already (a line, a time limit): its command may run on.
    let found = {
      let ended = self.ended.lock().expect("ended lock");
      ended
        .iter()
        .rfind(|e| e.id == id && thread.is_none_or(|t| &e.thread == t))
        .map(|e| {
          (
            e.thread.clone(),
            e.target.clone(),
            e.command.clone(),
            e.entered,
            e.state.clone(),
          )
        })
    };
    let Some((owner, target, command, entered, state)) = found else {
      return Err(format!("no job {id} of this thread (jobs lists them)"));
    };
    let what = what_of(id, &target, &command);
    let (Target::Session(name), Some(entered)) = (target, entered) else {
      return Err(format!("{what} is not running: {state}"));
    };
    let session = self
      .sessions
      .lock()
      .await
      .get(&session_key(&owner, &name))
      .cloned();
    let runs = session.as_ref().is_some_and(|s| {
      s.command()
        .is_some_and(|c| c.started == entered && c.running)
    });
    let (Some(session), true) = (session, runs) else {
      return Err(format!("{what} is not running: {state}"));
    };
    let stopped = self
      .stop_command(&owner, &name, &session, entered, from, grace)
      .await;
    let output = output_of(&session, Some(entered));
    if let Some(e) = self
      .ended
      .lock()
      .expect("ended lock")
      .iter_mut()
      .rfind(|e| e.id == id)
    {
      if stopped.ended {
        e.state = format!("{state}; then stopped");
      }
      if let Some(o) = &output {
        e.output = Some(cap_output(o.clone()));
      }
    }
    let mut text = format!("{what}: {}.", stopped.said.join("; "));
    if stopped.ended && self.close_auto(&owner, &name).await {
      text.push_str(&format!(" Session \"{name}\" is closed with it."));
    }
    Ok(with_last_lines(text, output.as_deref(), id))
  }

  /// `job_stop` on a session: the command running there, with no job.
  pub(crate) async fn stop_in_session(
    &self,
    thread: &RecordId,
    name: &str,
    from: StopFrom,
    grace: Duration,
  ) -> Result<String, String> {
    let session = self
      .sessions
      .lock()
      .await
      .get(&session_key(thread, name))
      .cloned()
      .ok_or_else(|| format!("no session {name}"))?;
    let Some(c) = session.command().filter(|c| c.running) else {
      return Err(format!("nothing runs in session {name}"));
    };
    // Its jobs are done with: what they would tell is said here.
    for id in self.jobs_on(thread, name) {
      if let Some(job) = self.take_job(id) {
        job.task.abort();
        self.ended(job, "stopped".into(), output_of(&session, Some(c.started)));
      }
    }
    let stopped = self
      .stop_command(thread, name, &session, c.started, from, grace)
      .await;
    let output = output_of(&session, Some(c.started));
    let mut text = format!(
      "Session \"{name}\", `{}`: {}.",
      c.text,
      stopped.said.join("; ")
    );
    if stopped.ended && self.close_auto(thread, name).await {
      text.push_str(&format!(" Session \"{name}\" is closed with it."));
    }
    let last = output
      .as_deref()
      .map(|o| tail_lines(o, STOP_LINES))
      .unwrap_or_default();
    Ok(if last.is_empty() {
      text
    } else {
      format!("{text}\n{last}")
    })
  }

  /// The thread a job (running, or ended within the hour) is of, and what
  /// it is, as said.
  pub(crate) fn owner_of(&self, id: u64) -> Option<(RecordId, String)> {
    let live = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .find(|j| j.id == id)
      .map(|j| (j.thread.clone(), what_of(j.id, &j.target, &j.command)));
    live.or_else(|| {
      self
        .ended
        .lock()
        .expect("ended lock")
        .iter()
        .rfind(|e| e.id == id)
        .map(|e| (e.thread.clone(), what_of(e.id, &e.target, &e.command)))
    })
  }

  fn jobs_on(&self, thread: &RecordId, name: &str) -> Vec<u64> {
    self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .filter(|j| &j.thread == thread && matches!(&j.target, Target::Session(n) if n == name))
      .map(|j| j.id)
      .collect()
  }

  /// `jobs`: the jobs of `thread` (every thread's when `None`), running,
  /// then ended within the hour, and the commands its sessions run with
  /// no job on them.
  pub(crate) async fn jobs_text(&self, thread: Option<&RecordId>) -> String {
    let sessions: Vec<(String, Session)> = self
      .sessions
      .lock()
      .await
      .iter()
      .map(|(k, s)| (k.clone(), s.clone()))
      .collect();
    let session_of = |t: &RecordId, name: &str| {
      let key = session_key(t, name);
      sessions
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, s)| s.clone())
    };
    let ours = |t: &RecordId| thread.is_none_or(|me| me == t);
    let owner = |t: &RecordId| match thread {
      Some(_) => String::new(),
      None => format!(" [thread {t}]"),
    };
    let now = jiff::Timestamp::now();
    let age = |t: jiff::Timestamp| spoken(now.duration_since(t).unsigned_abs());
    let mut out: Vec<String> = Vec::new();
    let mut watched: HashSet<(String, String)> = HashSet::new();
    let live: Vec<(String, RecordId, Target, Option<Instant>)> = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .filter(|j| ours(&j.thread))
      .map(|j| {
        let kind = if j.entered.is_some() {
          "shell"
        } else {
          "watch"
        };
        let row = format!(
          "job {} ({kind}){}: {}; started {} ({} ago); running: {}",
          j.id,
          owner(&j.thread),
          target_of(&j.target, &j.command),
          local_time(j.started),
          age(j.started),
          j.said(),
        );
        (row, j.thread.clone(), j.target.clone(), j.entered)
      })
      .collect();
    for (mut row, t, target, entered) in live {
      if let Target::Session(name) = &target {
        watched.insert((t.to_string(), name.clone()));
        if let Some(s) = session_of(&t, name)
          && let Some(o) = output_of(&s, entered)
        {
          row.push_str(&last_lines(&o));
        }
      }
      out.push(row);
    }
    for (key, s) in &sessions {
      let Some((t, name)) = key.split_once(':') else {
        continue;
      };
      let Ok(t) = t.parse::<RecordId>() else {
        continue;
      };
      if !ours(&t) || watched.contains(&(t.to_string(), name.to_owned())) {
        continue;
      }
      let Some(c) = s.command().filter(|c| c.running) else {
        continue;
      };
      let started = ago(c.started);
      let mut row = format!(
        "session \"{name}\"{}: `{}`; started {} ({} ago); running with no job (shell_watch it \
         to be told when it ends; job_stop with session stops it)",
        owner(&t),
        c.text,
        local_time(started),
        age(started),
      );
      if let Some((_, o)) = s.command_output() {
        row.push_str(&last_lines(&o));
      }
      out.push(row);
    }
    let mut ended = self.ended.lock().expect("ended lock");
    prune(&mut ended);
    for e in ended.iter().rev().filter(|e| ours(&e.thread)) {
      let kind = if e.entered.is_some() {
        "shell"
      } else {
        "watch"
      };
      let at = now - jiff::SignedDuration::try_from(e.at.elapsed()).unwrap_or_default();
      out.push(format!(
        "job {} ({kind}){}: {}; started {}; {} at {} ({} ago) (the job {})",
        e.id,
        owner(&e.thread),
        target_of(&e.target, &e.command),
        local_time(e.started),
        e.state,
        local_time(at),
        age(at),
        e.condition,
      ));
    }
    if out.is_empty() {
      return "No jobs: nothing runs in the background or is watched, and none ended in the \
              last hour. (Jobs end when the node restarts; the thread is told after.)"
        .into();
    }
    out.join("\n")
  }

  /// Every thread's jobs as apps' live data gives them (design §2.10), as
  /// `jobs {all: true}` lists them: the jobs running, the commands left
  /// running with no job, then the jobs that ended in the last hour,
  /// newest first. Each with a `key` that changes only when the job does
  /// (not its output), for telling a change.
  pub(crate) async fn live(&self) -> Vec<(String, serde_json::Value)> {
    use serde_json::json;
    let sessions: Vec<(String, Session)> = self
      .sessions
      .lock()
      .await
      .iter()
      .map(|(k, s)| (k.clone(), s.clone()))
      .collect();
    let session_of = |t: &RecordId, name: &str| {
      let key = session_key(t, name);
      sessions
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, s)| s.clone())
    };
    let stamp = |t: jiff::Timestamp| t.to_string();
    let target = |t: &Target| match t {
      Target::Session(n) => (Some(n.clone()), None),
      Target::File(p) => (None, Some(p.display().to_string())),
    };
    let mut out = Vec::new();
    let mut watched: HashSet<(String, String)> = HashSet::new();
    let live: Vec<LiveJob> = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .map(|j| {
        (
          j.id,
          j.thread.clone(),
          j.target.clone(),
          j.command.clone(),
          j.entered,
          j.said(),
          j.started,
        )
      })
      .collect();
    for (id, t, tg, command, entered, condition, started) in live {
      let mut lines = None;
      if let Target::Session(name) = &tg {
        watched.insert((t.to_string(), name.clone()));
        lines = session_of(&t, name)
          .and_then(|s| output_of(&s, entered))
          .map(|o| tail_lines(o.trim_end(), LIST_LINES));
      }
      let (session, file) = target(&tg);
      out.push((
        format!("job {id} running"),
        json!({
          "job": id,
          "kind": if entered.is_some() { "shell" } else { "watch" },
          "thread": t.to_string(),
          "session": session,
          "file": file,
          "command": (!command.is_empty()).then_some(command),
          "condition": condition,
          "started": stamp(started),
          "running": true,
          "last_lines": lines,
        }),
      ));
    }
    for (key, s) in &sessions {
      let Some((t, name)) = key.split_once(':') else {
        continue;
      };
      if watched.contains(&(t.to_owned(), name.to_owned())) {
        continue;
      }
      let Some(c) = s.command().filter(|c| c.running) else {
        continue;
      };
      let started = ago(c.started);
      out.push((
        format!("session {key} {started}"),
        json!({
          "job": null,
          "kind": "session",
          "thread": t,
          "session": name,
          "command": c.text,
          "condition": "running with no job: nobody is told when it ends",
          "started": stamp(started),
          "running": true,
          "last_lines": s.command_output().map(|(_, o)| tail_lines(o.trim_end(), LIST_LINES)),
        }),
      ));
    }
    let now = jiff::Timestamp::now();
    let mut ended = self.ended.lock().expect("ended lock");
    prune(&mut ended);
    for e in ended.iter().rev() {
      let at = now - jiff::SignedDuration::try_from(e.at.elapsed()).unwrap_or_default();
      let (session, file) = target(&e.target);
      out.push((
        format!("job {} ended", e.id),
        json!({
          "job": e.id,
          "kind": if e.entered.is_some() { "shell" } else { "watch" },
          "thread": e.thread.to_string(),
          "session": session,
          "file": file,
          "command": (!e.command.is_empty()).then_some(e.command.clone()),
          "condition": e.condition,
          "started": stamp(e.started),
          "running": false,
          "how": e.state,
          "ended": stamp(at),
          "last_lines": e.output.as_deref().map(|o| tail_lines(o.trim_end(), LIST_LINES)),
        }),
      ));
    }
    out
  }

  /// `job_output`: a shell job's output, live or ended: the last `lines`
  /// (default 200), or from line `from` (1 is the first).
  pub(crate) async fn job_output(
    &self,
    id: u64,
    thread: Option<&RecordId>,
    lines: Option<usize>,
    from: Option<usize>,
  ) -> Result<String, String> {
    let sessions: HashMap<String, Session> = self.sessions.lock().await.clone();
    let live = |t: &RecordId, target: &Target, entered: Option<Instant>| match target {
      Target::Session(name) => sessions
        .get(&session_key(t, name))
        .and_then(|s| output_of(s, entered)),
      Target::File(_) => None,
    };
    let found = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .find(|j| j.id == id && thread.is_none_or(|t| &j.thread == t))
      .map(|j| {
        let state = "running".to_owned();
        (
          j.thread.clone(),
          j.target.clone(),
          j.command.clone(),
          j.entered,
          state,
          None,
        )
      });
    let found = found.or_else(|| {
      self
        .ended
        .lock()
        .expect("ended lock")
        .iter()
        .rfind(|e| e.id == id && thread.is_none_or(|t| &e.thread == t))
        .map(|e| {
          (
            e.thread.clone(),
            e.target.clone(),
            e.command.clone(),
            e.entered,
            e.state.clone(),
            e.output.clone(),
          )
        })
    });
    let Some((t, target, command, entered, state, kept)) = found else {
      return Err(format!(
        "no job {id} of this thread (jobs lists them; ended ones are kept an hour)"
      ));
    };
    if let Target::File(path) = &target {
      return Err(format!(
        "job {id} watches a file, {}: it has no output of its own; read the file with the shell",
        path.display()
      ));
    }
    let Some(text) = live(&t, &target, entered).or(kept) else {
      return Err(format!(
        "job {id}'s output is no longer kept: its session ran something else since"
      ));
    };
    let all: Vec<&str> = text.lines().collect();
    let total = all.len();
    let want = lines.unwrap_or(200).clamp(1, OUTPUT_LINES_MAX);
    let start = match from {
      Some(f) => f.max(1) - 1,
      None => total.saturating_sub(want),
    }
    .min(total);
    let end = (start + want).min(total);
    let mut body = all[start..end].join("\n");
    let mut shown_end = end;
    if body.len() > OUTPUT_CHARS_MAX {
      let mut cut = OUTPUT_CHARS_MAX;
      while !body.is_char_boundary(cut) {
        cut -= 1;
      }
      body.truncate(cut);
      shown_end = start + body.lines().count();
    }
    let mut head = format!(
      "[{}: {state}; lines {}–{shown_end} of {total}",
      what_of(id, &target, &command),
      if total == 0 { 0 } else { start + 1 },
    );
    if shown_end < total {
      head.push_str(&format!("; more with from {}", shown_end + 1));
    }
    head.push(']');
    Ok(format!("{head}\n{body}"))
  }

  /// A session by its listed ID (`<thread>/<session>`).
  pub async fn session_of(&self, id: &str) -> Option<(RecordId, String, Session)> {
    let (thread, name) = id.split_once('/')?;
    let thread: RecordId = thread.parse().ok()?;
    let session = self
      .sessions
      .lock()
      .await
      .get(&session_key(&thread, name))
      .cloned()?;
    Some((thread, name.to_owned(), session))
  }

  /// A person typed into a session (attached): sent, and once the typing
  /// pauses, woven into the thread's loop when one runs; else the agent's
  /// next call on the session hears of it.
  pub async fn typed(
    self: &Arc<Self>,
    thread: &RecordId,
    name: &str,
    session: &Session,
    input: &[u8],
  ) -> stitch_shell::Result<()> {
    session.type_in(input).await?;
    let key = session_key(thread, name);
    let count = {
      let mut typing = self.typing.lock().expect("typing lock");
      let c = typing.entry(key.clone()).or_default();
      *c += 1;
      *c
    };
    let (shells, thread, name, session) = (
      self.clone(),
      thread.clone(),
      name.to_owned(),
      session.clone(),
    );
    tokio::spawn(async move {
      tokio::time::sleep(TYPED_PAUSE).await;
      {
        // A later keystroke's pause decides; the last one clears the entry.
        let mut typing = shells.typing.lock().expect("typing lock");
        if typing.get(&key) != Some(&count) {
          return;
        }
        typing.remove(&key);
      }
      let Some(weave) = shells.weave.get() else {
        return;
      };
      // Taken and woven at once, under the session's lock: an agent call
      // in between sees it either still there or already told.
      let back = session.clone();
      session.take_typed_if(|typed| {
        let typed_back = typed.to_owned();
        let untold = crate::chat::Untold(Arc::new(move || back.untake_typed(&typed_back)));
        weave(thread.clone(), typed_note(&name, typed), untold)
      });
    });
    Ok(())
  }

  /// Marks a session as one started for a single background command.
  pub(crate) fn mark_auto(&self, thread: &RecordId, name: &str) {
    self
      .auto
      .lock()
      .expect("auto lock")
      .insert(session_key(thread, name));
  }

  /// A session closed by the agent: no longer one of those.
  pub(crate) fn unmark_auto(&self, thread: &RecordId, name: &str) {
    self
      .auto
      .lock()
      .expect("auto lock")
      .remove(&session_key(thread, name));
  }

  /// Closes a session started for one background command, now that it
  /// ended and nothing else waits on it; `true` when it did.
  pub(crate) async fn close_auto(&self, thread: &RecordId, name: &str) -> bool {
    let key = session_key(thread, name);
    if !self.auto.lock().expect("auto lock").contains(&key) {
      return false;
    }
    let busy = self
      .jobs
      .lock()
      .expect("jobs lock")
      .iter()
      .any(|j| &j.thread == thread && matches!(&j.target, Target::Session(n) if n == name));
    if busy {
      return false;
    }
    let session = {
      let mut sessions = self.sessions.lock().await;
      match sessions.get(&key) {
        // Something else runs there now (the agent ran more in it).
        Some(s) if matches!(s.status(), Status::Running { .. }) => return false,
        _ => sessions.remove(&key),
      }
    };
    self.auto.lock().expect("auto lock").remove(&key);
    if let Some(s) = session {
      s.close().await;
    }
    true
  }

  /// Ends a thread's jobs, telling nothing (it reported, or was
  /// cancelled): none fires after this returns.
  pub(crate) fn end_jobs(&self, thread: &RecordId) {
    self.jobs.lock().expect("jobs lock").retain(|j| {
      if &j.thread == thread {
        j.task.abort();
      }
      &j.thread != thread
    });
  }

  /// The threads with a session here started while they were writable
  /// (not one started for a command allowed once: `once`).
  pub(crate) async fn writable_threads(&self) -> Vec<RecordId> {
    let keys: Vec<String> = self.sessions.lock().await.keys().cloned().collect();
    let read_only = self.read_only.lock().expect("read-only lock");
    let once = self.once.lock().expect("once lock");
    let mut out: Vec<RecordId> = keys
      .iter()
      .filter(|k| !read_only.contains(*k) && !once.contains(*k))
      .filter_map(|k| k.split_once(':').and_then(|(t, _)| t.parse().ok()))
      .collect();
    out.sort();
    out.dedup();
    out
  }

  /// Closes a thread's sessions, and what runs in them.
  pub(crate) async fn close_sessions(&self, thread: &RecordId) {
    let prefix = format!("{thread}:");
    let ours: Vec<Session> = {
      let mut sessions = self.sessions.lock().await;
      let keys: Vec<String> = sessions
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
      self
        .auto
        .lock()
        .expect("auto lock")
        .retain(|k| !k.starts_with(&prefix));
      self
        .read_only
        .lock()
        .expect("read-only lock")
        .retain(|k| !k.starts_with(&prefix));
      self
        .once
        .lock()
        .expect("once lock")
        .retain(|k| !k.starts_with(&prefix));
      keys.iter().filter_map(|k| sessions.remove(k)).collect()
    };
    for s in ours {
      s.close().await;
    }
  }
}

/// What the agent is told of a person's typing in a session.
pub(crate) fn typed_note(name: &str, typed: &str) -> String {
  format!(
    "[The user typed into session \"{name}\" (attached to it): «{typed}». Look before sending more there: it may be theirs now.]"
  )
}

/// A duration as people say it: `45s`, `4m12s`, `2h05m`.
pub fn spoken(d: Duration) -> String {
  let s = d.as_secs();
  match s {
    0..60 => format!("{s}s"),
    60..3600 => format!("{}m{:02}s", s / 60, s % 60),
    _ => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
  }
}

/// How a file job fired.
enum FileFired {
  Line(String),
  Changed,
  Quiet(Duration),
  Failed(String),
  /// Its time limit passed first.
  TimedOut(Duration),
}

enum Fired {
  /// How the wait ended, and the screen as it did.
  Session(stitch_shell::Result<stitch_shell::Waited>, String),
  /// The time limit passed first: what it printed so far, and the screen.
  TimedOut(stitch_shell::Result<stitch_shell::Waited>, String),
  File(FileFired),
}

/// The grace `job_stop` gives each signal, by default.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(5);

/// What the command entered at `entered` printed, while it is still the
/// session's last.
fn output_of(session: &Session, entered: Option<Instant>) -> Option<String> {
  let entered = entered?;
  session
    .command_output()
    .filter(|(at, _)| *at == entered)
    .map(|(_, text)| text)
}

/// How a job that fired ended, as the jobs list says it.
fn state_of(fired: &Fired, stopped: Option<&Stopped>) -> String {
  let exit = |w: &stitch_shell::Waited| match &w.outcome.status {
    Status::AtPrompt { exit } => format!("ended with exit {exit}"),
    Status::Closed => "ended: the session closed".into(),
    Status::Running { .. } => "fired; its command runs on".into(),
  };
  match fired {
    Fired::Session(Ok(w), _) if w.done => exit(w),
    Fired::Session(Ok(w), _) => match &w.matched {
      Some(l) if !l.is_empty() => "fired: it printed a matching line; it runs on".into(),
      _ => "fired: the screen was as asked".into(),
    },
    Fired::Session(Err(e), _) => format!("failed: {e}"),
    Fired::TimedOut(..) => match stopped {
      Some(s) if s.ended => "timed out: stopped".into(),
      Some(_) => "timed out: stopping it failed; it runs on".into(),
      None => "timed out; its command runs on".into(),
    },
    Fired::File(FileFired::Failed(e)) => format!("failed: {e}"),
    Fired::File(FileFired::TimedOut(_)) => "timed out".into(),
    Fired::File(_) => "fired".into(),
  }
}

/// How often a file job looks.
const FILE_POLL: Duration = Duration::from_millis(500);
/// What a file job reads of what was added, per look, at most.
const FILE_READ_MAX: u64 = 4 << 20;
/// What a file job keeps of a line not ended yet, at most.
const LINE_MAX: usize = 1 << 20;

/// Waits on a file: polled (identity, size and modification time),
/// reading only what was added since it last looked when a line is wanted,
/// all of it, however much. A file replaced (rotated, recreated: another
/// inode) or shrunk (truncated) is read again from its start; one that is
/// not there yet counts from when it appears. A rewrite to the very same
/// length between two looks is seen as a change, not as new lines.
async fn file_until(path: &std::path::Path, until: &FileUntil) -> FileFired {
  use std::os::unix::fs::MetadataExt;
  use tokio::io::{AsyncReadExt, AsyncSeekExt};
  type Stamp = (u64, u64, u64, Option<std::time::SystemTime>);
  let stamp = |m: &std::fs::Metadata| -> Stamp { (m.dev(), m.ino(), m.len(), m.modified().ok()) };
  let mut last: Option<Stamp> = tokio::fs::metadata(path).await.ok().map(|m| stamp(&m));
  let mut offset = last.map_or(0, |(.., len, _)| len);
  let mut partial: Vec<u8> = Vec::new();
  let mut quiet_since = Instant::now();
  let mut matched: Option<String> = None;
  loop {
    tokio::time::sleep(FILE_POLL).await;
    let now = tokio::fs::metadata(path).await.ok().map(|m| stamp(&m));
    if now != last {
      let replaced = match (last, now) {
        (Some((d0, i0, ..)), Some((d1, i1, ..))) => (d0, i0) != (d1, i1),
        // Gone, then back: a new file, read from its start.
        (None, Some(_)) => true,
        _ => false,
      };
      last = now;
      quiet_since = Instant::now();
      if until.line.is_none() && until.quiet.is_none() {
        return FileFired::Changed;
      }
      if let (Some(re), Some((.., len, _))) = (&until.line, now)
        && matched.is_none()
      {
        if replaced || len < offset {
          offset = 0;
          partial.clear();
        }
        let mut file = match tokio::fs::File::open(path).await {
          Ok(f) => f,
          Err(e) => return FileFired::Failed(e.to_string()),
        };
        if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
          return FileFired::Failed(e.to_string());
        }
        // Up to the length seen, in slices, scanning as it goes.
        while offset < len && matched.is_none() {
          let mut added = Vec::new();
          match (&mut file)
            .take((len - offset).min(FILE_READ_MAX))
            .read_to_end(&mut added)
            .await
          {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => return FileFired::Failed(e.to_string()),
          }
          offset += added.len() as u64;
          partial.extend_from_slice(&added);
          let complete = partial
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
          matched = String::from_utf8_lossy(&partial[..complete])
            .lines()
            .find(|l| re.is_match(l))
            .map(str::to_owned);
          partial.drain(..complete);
          // A line without its end, however long, is kept to its last
          // `LINE_MAX` bytes: a file with no newlines grows nothing here.
          if matched.is_none() && partial.len() > LINE_MAX {
            let text = String::from_utf8_lossy(&partial).into_owned();
            if let Some(m) = re.find(&text) {
              // What matched, with some of what is around it.
              let mut from = m.start().saturating_sub(1000);
              while !text.is_char_boundary(from) {
                from -= 1;
              }
              let mut to = (m.end() + 1000).min(text.len());
              while !text.is_char_boundary(to) {
                to += 1;
              }
              matched = Some(text[from..to].to_owned());
            }
            partial.drain(..partial.len() - LINE_MAX);
          }
        }
      }
    }
    let line_ok = until.line.is_none() || matched.is_some();
    let quiet_ok = until.quiet.is_none_or(|q| quiet_since.elapsed() >= q);
    if line_ok && quiet_ok {
      return match (matched, until.quiet) {
        (Some(l), _) => FileFired::Line(l),
        (None, Some(q)) => FileFired::Quiet(q),
        (None, None) => FileFired::Changed,
      };
    }
  }
}

/// Lines of output a job's note carries, at most.
const NOTE_LINES: usize = 40;

/// The last lines of `text`; with `job`, a cut says `job_output` has the rest.
fn tail(text: &str, job: Option<u64>) -> String {
  let lines: Vec<&str> = text.lines().collect();
  let cut = match (lines.len() > NOTE_LINES, job) {
    (false, _) => String::new(),
    (true, Some(id)) => format!(
      "[… {} lines before: job_output {{\"job\": {id}}} has them]\n",
      lines.len() - NOTE_LINES
    ),
    (true, None) => format!("[… {} lines before]\n", lines.len() - NOTE_LINES),
  };
  format!(
    "{cut}{}",
    lines[lines.len().saturating_sub(NOTE_LINES)..].join("\n")
  )
}

/// What a job's time limit adds to its condition, as said.
pub(crate) fn limit_said(limit: Limit, command: bool) -> String {
  match limit.max {
    None => String::new(),
    Some(max) if limit.stop && command => format!(", or after {} (then stops it)", spoken(max)),
    Some(max) => format!(", or after {}", spoken(max)),
  }
}

/// What a thread is told when its job fires.
fn note_of(job: &Job, fired: Fired, stopped: Option<&Stopped>) -> String {
  let ran = spoken(
    jiff::Timestamp::now()
      .duration_since(job.started)
      .unsigned_abs(),
  );
  let what = match &job.target {
    Target::Session(name) if job.command.is_empty() => {
      format!("Job {} (session \"{name}\")", job.id)
    }
    Target::Session(name) => format!("Job {} (session \"{name}\", `{}`)", job.id, job.command),
    Target::File(path) => format!("Job {} (file {})", job.id, path.display()),
  };
  let w = match fired {
    Fired::File(f) => {
      return match f {
        FileFired::Line(l) => format!("[{what}: a line matching the pattern was added]\n{l}"),
        FileFired::Changed => format!("[{what}: it changed]"),
        FileFired::Quiet(q) => format!("[{what}: it has not changed for {}]", spoken(q)),
        FileFired::Failed(e) => format!("[{what}: watching it failed: {e}]"),
        FileFired::TimedOut(max) => {
          format!(
            "[{what}: timed out after {}, with nothing to tell]",
            spoken(max)
          )
        }
      };
    }
    Fired::Session(Err(e), _) | Fired::TimedOut(Err(e), _) => {
      return format!("[{what}: watching it failed: {e}]");
    }
    Fired::TimedOut(Ok(w), screen) => {
      let id = job.id;
      let max = spoken(job.limit.max.unwrap_or_default());
      let head = match stopped {
        Some(s) => format!("timed out after {max}; stopped: {}", s.said.join("; ")),
        None if job.entered.is_some() => format!(
          "timed out after {max} (it started {ran} ago); it runs on (job_stop {{\"job\": {id}}} \
           stops it)"
        ),
        None => format!("timed out after {max}"),
      };
      let shown = if job.screen {
        tail(screen.trim_end(), None)
      } else {
        tail(&w.outcome.output, Some(id))
      };
      return format!("[{what} {head}]\n{shown}");
    }
    Fired::Session(Ok(w), screen) => (w, screen),
  };
  let (w, screen) = w;
  let ended = match &w.outcome.status {
    Status::AtPrompt { exit } => format!("ended with exit {exit} after {ran}"),
    Status::Closed => format!("ended after {ran}: the session closed"),
    Status::Running { .. } => format!("runs on after {ran}"),
  };
  // A screen job: what the screen shows now.
  if job.screen {
    let head = match (&w.matched, w.done) {
      (Some(line), false) if !line.is_empty() => format!(
        "{}: \"{line}\"",
        job.condition.replacen("tells when ", "", 1)
      ),
      (Some(_), false) => job.condition.replacen("tells when ", "", 1),
      (_, _) => format!("the session {ended}"),
    };
    return format!("[{what}: {head}]\n{}", tail(screen.trim_end(), None));
  }
  let pattern = job.line.as_deref().unwrap_or("");
  let head = match (&w.matched, w.done) {
    (Some(line), false) => {
      return format!(
        "[{what} printed a line matching \"{pattern}\" after {ran}, and runs on]\n{line}"
      );
    }
    (Some(line), true) => format!("printed a line matching \"{pattern}\" and {ended}:\n{line}"),
    (None, _) if job.line.is_some() => format!("{ended}, with no line matching \"{pattern}\""),
    (None, _) => ended,
  };
  format!("[{what} {head}]\n{}", tail(&w.outcome.output, Some(job.id)))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn screen_conditions_read_as_a_sentence() {
    let on = |shows: Option<&str>, gone: Option<&str>, quiet: Option<u64>| {
      condition_of(&Until::Screen(stitch_shell::ScreenUntil {
        shows: shows.map(|p| Regex::new(p).unwrap()),
        gone: gone.map(|p| Regex::new(p).unwrap()),
        quiet: quiet.map(Duration::from_secs),
      }))
    };
    assert_eq!(
      on(None, Some("esc to interrupt"), Some(10)),
      "tells when the screen no longer shows \"esc to interrupt\" and has been quiet 10s"
    );
    assert_eq!(
      on(None, None, Some(5)),
      "tells when the screen has been quiet 5s"
    );
    assert_eq!(
      on(Some("ready"), Some("busy"), Some(5)),
      "tells when the screen shows \"ready\", no longer shows \"busy\" and has been quiet 5s"
    );
  }

  fn line(p: &str) -> FileUntil {
    FileUntil {
      line: Some(Regex::new(p).unwrap()),
      quiet: None,
    }
  }

  async fn fired(path: &std::path::Path, until: FileUntil, act: impl FnOnce()) -> FileFired {
    let path = path.to_owned();
    let task = tokio::spawn(async move { file_until(&path, &until).await });
    tokio::time::sleep(Duration::from_millis(700)).await;
    act();
    tokio::time::timeout(Duration::from_secs(10), task)
      .await
      .expect("it fired")
      .unwrap()
  }

  fn append(path: &std::path::Path, text: &[u8]) {
    use std::io::Write;
    std::fs::OpenOptions::new()
      .append(true)
      .create(true)
      .open(path)
      .unwrap()
      .write_all(text)
      .unwrap();
  }

  #[tokio::test]
  async fn a_big_write_is_read_to_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("big.log");
    std::fs::write(&f, "start\n").unwrap();
    let mut big = "filler line\n".repeat(600_000).into_bytes();
    big.extend_from_slice(b"BIG-DONE\n");
    assert!(big.len() as u64 > FILE_READ_MAX, "past one slice");
    let got = fired(&f, line("BIG-DONE"), || append(&f, &big)).await;
    assert!(matches!(got, FileFired::Line(l) if l == "BIG-DONE"));
  }

  #[tokio::test]
  async fn a_file_gone_then_back_is_read_from_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("app.log");
    std::fs::write(&f, "old line\n".repeat(100)).unwrap();
    let path = f.clone();
    let task = tokio::spawn(async move { file_until(&path, &line("BACK-DONE")).await });
    tokio::time::sleep(Duration::from_millis(700)).await;
    // Gone for a look or two, then back, longer than it was.
    std::fs::remove_file(&f).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let mut new = b"BACK-DONE\n".to_vec();
    new.extend("new line\n".repeat(500).into_bytes());
    std::fs::write(&f, new).unwrap();
    let got = tokio::time::timeout(Duration::from_secs(10), task)
      .await
      .expect("it fired")
      .unwrap();
    assert!(matches!(got, FileFired::Line(l) if l == "BACK-DONE"));
  }

  #[tokio::test]
  async fn a_line_without_end_is_bounded_and_still_matched() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("progress.log");
    std::fs::write(&f, "").unwrap();
    // No newline ever: what is kept of it is bounded, and a match in it
    // counts.
    let got = fired(&f, line("NO-END-DONE"), || {
      let mut text = "x".repeat(3 * LINE_MAX).into_bytes();
      text.extend_from_slice(b"NO-END-DONE");
      append(&f, &text);
    })
    .await;
    assert!(matches!(got, FileFired::Line(l) if l.ends_with("NO-END-DONE") && l.len() <= 2100));
  }

  #[tokio::test]
  async fn a_rotated_file_is_read_from_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("app.log");
    std::fs::write(&f, "old line\n".repeat(100)).unwrap();
    // Rotated: the new file is longer than the old one was by the next look.
    let got = fired(&f, line("ROT-DONE"), || {
      std::fs::rename(&f, dir.path().join("app.log.1")).unwrap();
      let mut new = b"ROT-DONE\n".to_vec();
      new.extend("new line\n".repeat(500).into_bytes());
      std::fs::write(&f, new).unwrap();
    })
    .await;
    assert!(matches!(got, FileFired::Line(l) if l == "ROT-DONE"));
  }

  #[tokio::test]
  async fn a_truncated_file_and_one_not_there_yet() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("t.log");
    std::fs::write(&f, "x\n".repeat(1000)).unwrap();
    let got = fired(&f, line("TRUNC-DONE"), || {
      std::fs::write(&f, "TRUNC-DONE\n").unwrap();
    })
    .await;
    assert!(matches!(got, FileFired::Line(l) if l == "TRUNC-DONE"));
    let later = dir.path().join("later.log");
    let got = fired(&later, line("NEW-DONE"), || append(&later, b"NEW-DONE\n")).await;
    assert!(matches!(got, FileFired::Line(l) if l == "NEW-DONE"));
  }
}
