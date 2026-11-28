//! Detached threads (design §2.3): the agent loop running on the node with
//! no client attached, toward a goal, within a budget.
//!
//! - Approvals become `question` records (on the attention list) and the
//!   loop waits for an `answer` record, written here or arriving by sync;
//!   a chat's loop asks its client inline as well (`ask`). A question for
//!   the same command is reused while open, or when it was answered after
//!   the thread's latest step was written, so a restarted loop that asks
//!   again finds the question (and maybe its answer, given while the node
//!   was down). One answer is for one call. A question nothing asks any
//!   more is withdrawn (`Withdraw`, `sweep`).
//! - When the loop ends it writes a `report` (outcome and last reply) that
//!   refers to the thread and, when it has one, the thread that started it;
//!   that thread's agent is then woken with the report.
//! - Two loops never run on one thread at once.
//! - A node resumes, at start, the detached threads it started and that
//!   have no report yet.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use stitch_records::{Author, Draft, RecordId, Ref, kinds};
use stitch_store::RecordRoot;
use tokio::sync::{Notify, oneshot};

use crate::chat::{self, ChatContext, ChatEvent, ChatInput, ChatOutcome, Detached, End, Limits};
use crate::node::Shared;
use crate::tools::{ApprovalRequest, Approver, Decision};
use crate::{Error, Result};

/// One lock per thread, so loops on a thread take turns.
#[derive(Default)]
pub(crate) struct ThreadLocks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl ThreadLocks {
  pub(crate) fn of(&self, thread: &RecordId) -> Arc<tokio::sync::Mutex<()>> {
    self
      .0
      .lock()
      .expect("locks lock")
      .entry(thread.to_string())
      .or_default()
      .clone()
  }
}

#[derive(Default)]
pub(crate) struct Detachments {
  pub(crate) locks: ThreadLocks,
  /// Turns' catch-ups with the peers still running (`weave::catch_up`).
  pub(crate) catch_ups: Arc<crate::weave::CatchUps>,
  /// Woken when an answer is written on this node.
  pub(crate) answered: Notify,
  /// Woken when an approval question is written here or may have arrived
  /// by sync: `wait` tells of a thread it waits for blocked on the user.
  pub(crate) asked: Notify,
  /// When this node process started: open questions it wrote before are
  /// an earlier run's.
  pub(crate) since: std::sync::OnceLock<jiff::Timestamp>,
  /// Approval questions a waiting thread was told of (`parent question`):
  /// told once, not at every `wait`.
  pub(crate) told_blocked: Mutex<std::collections::HashSet<String>>,
  /// Woken when a report is written here or arrives by sync.
  pub(crate) reported: Notify,
  /// Woken when a loop task ends (its `Registered` dropped): what waits
  /// for a thread's turn to be over (`schedules::release_when_done`).
  pub(crate) loop_ended: Notify,
  /// Threads whose report the thread they report to has had (by `wait`,
  /// or woken with it): it is not delivered again.
  pub(crate) delivered: Mutex<std::collections::HashSet<String>>,
  /// Threads started as fixers of Stitch (`Spec::fixer`): their first
  /// turn's tools are made before their record is written, so
  /// `selfimprove::is_fixer` knows them from here until it is.
  pub(crate) fixers: Mutex<std::collections::HashSet<String>>,
  /// Threads spawned to start once the spawning step is written, by the
  /// thread that spawned them.
  pub(crate) starting: Mutex<HashMap<String, String>>,
  /// The threads each thread spawned or sent to, and when: what `wait`
  /// waits for.
  pub(crate) outstanding: Mutex<HashMap<String, Vec<(String, jiff::Timestamp)>>>,
  /// Threads in `wait`: their reports are not woken with.
  pub(crate) waiting: Mutex<std::collections::HashSet<String>>,
  /// The loops running here or waiting for their turn, by thread: what
  /// `cancel` stops.
  running: Mutex<HashMap<String, Vec<Loop>>>,
  last_loop: std::sync::atomic::AtomicU64,
  /// Assignments this node took, so two passes never start one twice.
  pub(crate) claimed: Mutex<std::collections::BTreeSet<String>>,
  /// New threads' assignments whose start failed here: when it first did,
  /// and how often. Tried again at each pass, until it has failed for
  /// `weave::START_GIVE_UP`; then a `failed` report ends it.
  pub(crate) start_failures: Mutex<HashMap<String, (jiff::Timestamp, u32)>>,
  /// Threads whose loops wait on approval questions, and how many.
  pub(crate) asking: Mutex<HashMap<String, usize>>,
  /// Approval questions whose answer a loop here had: asking again asks
  /// anew (one answer is for one call).
  pub(crate) had: Mutex<std::collections::HashSet<String>>,
  /// The loops running here, by thread: what is said to a thread while
  /// one runs is woven into it.
  pub(crate) steers: Mutex<HashMap<String, chat::Steer>>,
  /// When each of those loops started: a rewind written elsewhere since
  /// stops it (`crate::rewind::stop_overtaken`).
  pub(crate) started: Mutex<HashMap<String, jiff::Timestamp>>,
  /// The node is stopping its loops: nothing starts a new one (a loop
  /// aborted then leaves notes behind, which would wake its thread).
  pub(crate) stopping: std::sync::atomic::AtomicBool,
  /// Reports for idle threads, gathered before they wake them once.
  gathering: Mutex<HashMap<String, Gathered>>,
}

/// Reports gathered for an idle thread, and when the last came.
struct Gathered {
  texts: Vec<String>,
  reports: Vec<RecordId>,
  last: std::time::Instant,
}

/// How long reports for an idle thread are gathered: until none came for
/// `[detached] gather_secs` (default 5), and at most a minute in all.
fn gather_times(s: &Shared) -> (Duration, Duration) {
  let quiet = s
    .config
    .detached
    .as_ref()
    .and_then(|d| d.gather_secs)
    .map_or(Duration::from_secs(5), |q| Duration::from_secs(q.min(60)));
  (quiet, Duration::from_secs(60).max(quiet))
}

/// Gathers reports for an idle thread (its reports, already marked
/// delivered): the first starts the wait; when none came for a while,
/// they go together into a loop that runs on it by then (the user's
/// chat, say), else wake it once.
fn gather(s: &Arc<Shared>, thread: &RecordId, text: String, reports: Vec<RecordId>) {
  let first = {
    let mut g = s.detached.gathering.lock().expect("gathering lock");
    let e = g.entry(thread.to_string()).or_insert_with(|| Gathered {
      texts: Vec::new(),
      reports: Vec::new(),
      last: std::time::Instant::now(),
    });
    let first = e.texts.is_empty();
    e.texts.push(text);
    e.reports.extend(reports);
    e.last = std::time::Instant::now();
    first
  };
  if !first {
    return;
  }
  let Ok(rt) = tokio::runtime::Handle::try_current() else {
    return;
  };
  let (s, thread) = (s.clone(), thread.clone());
  rt.spawn(async move {
    let (quiet, most) = gather_times(&s);
    let began = std::time::Instant::now();
    loop {
      let last = s
        .detached
        .gathering
        .lock()
        .expect("gathering lock")
        .get(thread.as_str())
        .map_or(began, |g| g.last);
      let due = (last + quiet).min(began + most);
      let now = std::time::Instant::now();
      if now >= due {
        break;
      }
      tokio::time::sleep(due - now).await;
    }
    let Some(g) = s
      .detached
      .gathering
      .lock()
      .expect("gathering lock")
      .remove(thread.as_str())
    else {
      return;
    };
    let text = g.texts.join("\n\n");
    let steer = s
      .detached
      .steers
      .lock()
      .expect("steers lock")
      .get(thread.as_str())
      .cloned();
    let woven = steer.is_some_and(|st| {
      st.push(chat::Interjection {
        id: None,
        from: Some("stitch".into()),
        text: text.clone(),
        attachments: Vec::new(),
        reports: g.reports.clone(),
        untold: None,
        asker: None,
        relayed: None,
        scope: None,
      })
    });
    if woven {
      return;
    }
    if !wakes(&s, &thread, true) {
      eprintln!("thread {thread}: reports after it ended, kept in it, not woken with");
      keep_unrun(&s, &thread, "stitch", &text, ENDED_NOTE);
      return;
    }
    {
      let mut delivered = s.detached.delivered.lock().expect("delivered lock");
      for r in &g.reports {
        delivered.remove(r.as_str());
      }
    }
    turn(
      s.clone(),
      thread,
      text,
      "stitch".into(),
      None,
      g.reports,
      false,
    );
  });
}

/// Marks the reports a wake carries as delivered: `None` when every one was
/// had already (`wait` took them while the thread ran, or another wake
/// did), else the wake's message and reports, the message saying only those
/// not had. A report not taken by a loop is handed to a turn that waits for
/// the thread's lock, and `deliver` may hand it again meanwhile, with what
/// came since: the later wake runs for the new ones, without saying the old
/// one again (issue 2026-11-11T154611Z-beast-5vp5).
fn take_reports(
  s: &Shared,
  thread: &RecordId,
  message: String,
  reports: &[RecordId],
) -> Option<(String, Vec<RecordId>)> {
  let fresh: Vec<RecordId> = {
    let mut delivered = s.detached.delivered.lock().expect("delivered lock");
    reports
      .iter()
      .filter(|r| delivered.insert(r.to_string()))
      .cloned()
      .collect()
  };
  if fresh.is_empty() {
    return None;
  }
  if fresh.len() == reports.len() {
    return Some((message, fresh));
  }
  Some((crate::weave::reports_message(s, thread, &fresh), fresh))
}

/// What a cancelled thread says of reports that came for it after.
const ENDED_NOTE: &str = "[Stitch: no turn ran for this: the thread was cancelled. It is kept \
  here; a message from the user continues the thread.]";

/// Writes into a thread what came for it when no turn runs to take it
/// (`chat::write_unrun`): never dropped unseen (design §2.7, "Reports
/// are never dropped unseen").
fn keep_unrun(s: &Arc<Shared>, thread: &RecordId, from: &str, text: &str, note: &str) {
  let written = s.thread_place(thread).and_then(|(root, _)| {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    chat::write_unrun(&ctx, thread, from, text, note)
  });
  if let Err(e) = written {
    eprintln!("thread {thread}: what came for it not kept: {e}");
  }
}

/// A loop task on a thread, as `cancel` and `stop_reported` see it.
struct Loop {
  serial: u64,
  task: tokio::task::AbortHandle,
  /// The thread's reports when it started: a report written since that
  /// says it was cancelled (elsewhere) stops it.
  reports: Vec<RecordId>,
}

/// Held by a loop task while it runs: when it ends (or is aborted) the
/// loop is no longer listed.
struct Registered {
  s: Arc<Shared>,
  /// The thread it is listed under: the one it runs on, which a
  /// compaction as its turn starts changes (`moved_to`). Read and written
  /// under the `running` lock only.
  thread: Arc<Mutex<String>>,
  serial: u64,
}

impl Registered {
  /// Its loop runs on `thread` from now (it was compacted into it, or the
  /// turn followed a compaction made while it waited): listed there, so
  /// `is_running`, `cancel` and `stop_reported`, which look the thread up
  /// as it is now (`fork::current`), find it (the steering is the new
  /// thread's already). Before or after `register` alike: that one lists
  /// it under the thread this says.
  fn moved_to(&self, thread: &RecordId) {
    // What that thread was told so far is not news to stop it with
    // (`stop_reported`): one cancelled before is running again.
    let reports = reports_of(&self.s, thread);
    let mut running = self.s.detached.running.lock().expect("running lock");
    let mut key = self.thread.lock().expect("listed lock");
    if key.as_str() == thread.as_str() {
      return;
    }
    if let Some(loops) = running.get_mut(key.as_str()) {
      let mut mine: Vec<Loop> = loops.extract_if(.., |l| l.serial == self.serial).collect();
      if loops.is_empty() {
        running.remove(key.as_str());
      }
      for l in &mut mine {
        l.reports.extend(reports.iter().cloned());
      }
      running.entry(thread.to_string()).or_default().extend(mine);
    }
    *key = thread.to_string();
  }
}

impl Drop for Registered {
  fn drop(&mut self) {
    let mut running = self.s.detached.running.lock().expect("running lock");
    let key = self.thread.lock().expect("listed lock").clone();
    if let Some(loops) = running.get_mut(&key) {
      loops.retain(|l| l.serial != self.serial);
      if loops.is_empty() {
        running.remove(&key);
      }
    }
    drop(running);
    self.s.detached.loop_ended.notify_waiters();
  }
}

/// Where `register` lists a loop: the thread its `Registered` says now.
type Listed = Arc<Mutex<String>>;

/// A number for a loop task about to be spawned, the guard its task
/// holds, and where it is listed.
fn registration(s: &Arc<Shared>, thread: &RecordId) -> (u64, Registered, Listed) {
  let serial = s
    .detached
    .last_loop
    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    + 1;
  let listed = Arc::new(Mutex::new(thread.to_string()));
  (
    serial,
    Registered {
      s: s.clone(),
      thread: listed.clone(),
      serial,
    },
    listed,
  )
}

/// A thread's reports as they stand.
fn reports_of(s: &Shared, thread: &RecordId) -> Vec<RecordId> {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "thread", thread, None)
    .unwrap_or_default()
    .into_iter()
    .map(|r| r.id)
    .collect()
}

/// Lists a loop task, unless it already ended, under the thread it runs
/// on now (`Registered::moved_to` may have moved it since it was spawned).
fn register(s: &Shared, listed: &Listed, serial: u64, task: tokio::task::AbortHandle) {
  let (mut running, key, reports) = loop {
    let thread = listed.lock().expect("listed lock").clone();
    let Ok(id) = thread.parse::<RecordId>() else {
      return;
    };
    let reports = reports_of(s, &id);
    let running = s.detached.running.lock().expect("running lock");
    // Moved meanwhile: its reports are the new thread's.
    if *listed.lock().expect("listed lock") == thread {
      break (running, thread, reports);
    }
  };
  let loops = running.entry(key).or_default();
  loops.retain(|l| !l.task.is_finished());
  if !task.is_finished() {
    loops.push(Loop {
      serial,
      task,
      reports,
    });
  }
}

/// Takes a thread's loop tasks off the list: those still running.
fn take_loops(s: &Shared, thread: &RecordId) -> Vec<tokio::task::AbortHandle> {
  s.detached
    .running
    .lock()
    .expect("running lock")
    .remove(thread.as_str())
    .unwrap_or_default()
    .into_iter()
    .map(|l| l.task)
    .filter(|t| !t.is_finished())
    .collect()
}

/// Whether a note may wake `thread` (start a turn) when no loop runs on it.
/// A detached thread that reported has ended its work: a cancelled one is
/// woken by nothing the node says, a finished one only by a report (the
/// reply to what it asked: it may have ended its turn counting on it).
/// What is sent to it (`send`) always runs it.
fn wakes(s: &Shared, thread: &RecordId, report: bool) -> bool {
  let index = s.state().index.lock().expect("index lock");
  let Some(record) = index.record(thread).ok().flatten() else {
    return true;
  };
  match chat::detached_state(&index, &record) {
    Ok(Some(state)) => match state.as_str() {
      "running" | "waiting" => true,
      "cancelled" => false,
      _ => report,
    },
    _ => true,
  }
}

/// A turn accepted and not begun (`Live::queue`), for its thread's
/// followers and the thread list: dropped when its loop begins, or when
/// it runs none.
struct QueuedTurn {
  s: Arc<Shared>,
  thread: String,
}

impl QueuedTurn {
  fn new(s: &Arc<Shared>, thread: &RecordId) -> Self {
    s.live.queue(thread.as_str());
    s.apps.live.bump(&["threads"]);
    QueuedTurn {
      s: s.clone(),
      thread: thread.to_string(),
    }
  }
}

impl Drop for QueuedTurn {
  fn drop(&mut self) {
    self.s.live.unqueue(&self.thread);
    self.s.apps.live.bump(&["threads"]);
  }
}

/// A loop running on a thread, taking notes for it while it runs. When it
/// ends, notes it did not take wake the thread after it (see `wakes`).
pub(crate) struct Steering {
  s: Arc<Shared>,
  thread: RecordId,
  steer: chat::Steer,
}

impl Steering {
  pub(crate) fn new(s: &Arc<Shared>, thread: &RecordId, steer: chat::Steer) -> Self {
    s.detached
      .steers
      .lock()
      .expect("steers lock")
      .insert(thread.to_string(), steer.clone());
    s.detached
      .started
      .lock()
      .expect("started lock")
      .insert(thread.to_string(), jiff::Timestamp::now());
    s.live.begin(thread.as_str());
    Steering {
      s: s.clone(),
      thread: thread.clone(),
      steer,
    }
  }
}

impl Drop for Steering {
  fn drop(&mut self) {
    {
      let mut steers = self.s.detached.steers.lock().expect("steers lock");
      if steers
        .get(self.thread.as_str())
        .is_some_and(|st| st.same(&self.steer))
      {
        steers.remove(self.thread.as_str());
        self
          .s
          .detached
          .started
          .lock()
          .expect("started lock")
          .remove(self.thread.as_str());
      }
    }
    self.s.live.end(self.thread.as_str());
    // The client's own messages are the client's to send again.
    let left: Vec<chat::Interjection> = self
      .steer
      .close()
      .into_iter()
      .filter(|i| i.id.is_none())
      .collect();
    let mut notes = Vec::new();
    // Reports left untaken wake it once, together (one turn, not one each).
    let mut reports: Vec<RecordId> = Vec::new();
    let mut report_texts = Vec::new();
    for i in left {
      if let Some(asker) = i.asker {
        // Sent to it, too late for this loop: a turn of its own, as a
        // message sent while nothing runs.
        turn(
          self.s.clone(),
          self.thread.clone(),
          i.text,
          i.from.unwrap_or_else(|| "user".into()),
          asker.thread,
          Vec::new(),
          asker.keep,
        );
        continue;
      }
      if let Some(untold) = &i.untold {
        // Not worth a wake: given back.
        (untold.0)();
        continue;
      }
      if i.reports.is_empty() {
        notes.push(i.text);
      } else {
        reports.extend(i.reports);
        report_texts.push(i.text);
      }
    }
    if !reports.is_empty() {
      if !wakes(&self.s, &self.thread, true) {
        // Had, as far as delivering goes: nothing runs on it now. Kept in
        // it, so they are not lost from sight.
        eprintln!(
          "thread {}: {} report(s) came after it ended, kept in it",
          self.thread,
          reports.len()
        );
        keep_unrun(
          &self.s,
          &self.thread,
          "stitch",
          &report_texts.join("\n\n"),
          ENDED_NOTE,
        );
      } else {
        // Not had after all: delivered by its own turn.
        {
          let mut delivered = self.s.detached.delivered.lock().expect("delivered lock");
          for r in &reports {
            delivered.remove(r.as_str());
          }
        }
        turn(
          self.s.clone(),
          self.thread.clone(),
          report_texts.join("\n\n"),
          "stitch".into(),
          None,
          reports,
          false,
        );
      }
    }
    if !notes.is_empty() && !wakes(&self.s, &self.thread, false) {
      eprintln!(
        "thread {}: {} note(s) after its report, not woken with",
        self.thread,
        notes.len()
      );
    } else if !notes.is_empty() {
      turn(
        self.s.clone(),
        self.thread.clone(),
        notes.join("\n\n"),
        "stitch".into(),
        None,
        Vec::new(),
        false,
      );
    }
  }
}

/// The steer of the loop running on a thread here, if one runs.
pub(crate) fn running_steer(s: &Shared, thread: &RecordId) -> Option<chat::Steer> {
  s.detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned()
}

/// Weaves a note into the loop running on a thread here; `false` when
/// none runs (nothing is woken). Not taken before the loop ends, it is
/// given back by `untold`, not woken with.
pub(crate) fn weave(
  s: &Arc<Shared>,
  thread: &RecordId,
  text: String,
  untold: chat::Untold,
) -> bool {
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned();
  steer.is_some_and(|st| {
    st.push(chat::Interjection {
      id: None,
      from: Some("stitch".into()),
      text,
      attachments: Vec::new(),
      reports: Vec::new(),
      untold: Some(untold),
      asker: None,
      relayed: None,
      scope: None,
    })
  })
}

/// Tells a thread something from the node (a background job ended,
/// reports came): woven into its loop when one runs here, else it wakes
/// it. A report is delivered once.
pub(crate) fn notify(s: &Arc<Shared>, thread: &RecordId, text: String, reports: Vec<RecordId>) {
  // A thread compacted into another is that one now.
  let thread = &crate::fork::current(s, thread);
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned();
  let had = reports.len();
  let reports: Vec<RecordId> = {
    let mut delivered = s.detached.delivered.lock().expect("delivered lock");
    reports
      .into_iter()
      .filter(|r| delivered.insert(r.to_string()))
      .collect()
  };
  if had > 0 && reports.is_empty() {
    return;
  }
  // Some had meanwhile: the text says only the others.
  let text = if reports.len() < had {
    crate::weave::reports_message(s, thread, &reports)
  } else {
    text
  };
  let woven = steer.is_some_and(|st| {
    st.push(chat::Interjection {
      id: None,
      from: Some("stitch".into()),
      text: text.clone(),
      attachments: Vec::new(),
      reports: reports.clone(),
      untold: None,
      asker: None,
      relayed: None,
      scope: None,
    })
  });
  if !woven {
    if !wakes(s, thread, !reports.is_empty()) {
      eprintln!("thread {thread}: a note after its report, not woken with");
      if !reports.is_empty() {
        keep_unrun(s, thread, "stitch", &text, ENDED_NOTE);
      }
      return;
    }
    if !reports.is_empty() {
      // Idle: reports wake it once, together with those that follow.
      gather(s, thread, text, reports);
      return;
    }
    {
      let mut delivered = s.detached.delivered.lock().expect("delivered lock");
      for r in &reports {
        delivered.remove(r.as_str());
      }
    }
    turn(
      s.clone(),
      thread.clone(),
      text,
      "stitch".into(),
      None,
      reports,
      false,
    );
  }
}

/// Weaves a message sent to a thread (`send` with `interject`) into the
/// loop running on it here, as the user's typing is: `false` when none
/// runs. `from` names the sender as the transcript writes it; the loop's
/// last reply, when it ends, is reported to `asker.thread` (`answer_askers`).
/// Not taken before the loop ends, it starts a turn of its own. With the
/// files it came with (a message typed with them on another node), and the
/// threads and efforts attached to it (`Part::Scope`).
pub(crate) fn interject_with(
  s: &Arc<Shared>,
  thread: &RecordId,
  text: &str,
  from: &str,
  asker: chat::Asker,
  attachments: Vec<stitch_records::Attachment>,
  scope: Option<crate::scopes::Scoped>,
) -> bool {
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned();
  // The model reads who it is from, and that its reply goes back to it.
  let text = match &asker.thread {
    Some(t) => {
      let title = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .record(t)
        .ok()
        .flatten()
        .and_then(|r| r.title);
      let who = match title {
        Some(title) => format!("thread {t} \"{title}\""),
        None => format!("thread {t}"),
      };
      format!(
        "[Message from {who}: your last reply of this turn goes back to it as a report]\n{text}"
      )
    }
    None => text.to_owned(),
  };
  steer.is_some_and(|st| {
    st.push(chat::Interjection {
      id: None,
      from: Some(from.to_owned()),
      text,
      attachments,
      reports: Vec::new(),
      untold: None,
      asker: Some(asker),
      relayed: None,
      scope,
    })
  })
}

/// A loop ended that took messages sent to its thread (`interject`): each
/// sender's thread gets the loop's last reply as a report of its own
/// (`woven = true`), but `main`, which its own report goes to already.
pub(crate) fn answer_askers(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  result: &Result<ChatOutcome>,
  askers: Vec<chat::Asker>,
  main: Option<&RecordId>,
) {
  let mut told: Vec<RecordId> = main.into_iter().cloned().collect();
  for a in askers {
    let Some(to) = a.thread else { continue };
    if told.contains(&to) {
      continue;
    }
    told.push(to.clone());
    let (outcome, text, _) = outcome_of(result, false);
    let delivery = if a.keep {
      Delivery::Kept
    } else {
      Delivery::Wake
    };
    if let Err(e) = write_report_as(s, root, thread, outcome, &text, Some(to), delivery, true) {
      eprintln!("thread {thread}: no reply to a woven message: {e}");
    }
  }
}

/// What to start.
pub(crate) struct Spec {
  pub goal: String,
  /// `provider/model`; the default when `None`.
  pub model: Option<String>,
  pub repo: Option<String>,
  pub cwd: Option<String>,
  pub parent: Option<RecordId>,
  pub budget: Limits,
  /// It belongs to no folder.
  pub global: bool,
  /// Where it lives, when not placed from `repo` and `cwd`: with the
  /// thread it continues.
  pub at: Option<(RecordRoot, crate::place::Place)>,
  /// The assignment it runs, and the conversation it starts from.
  pub assignment: Option<RecordId>,
  pub continues: Option<chat::Continues>,
  /// Its ID, chosen beforehand (an ask names it before it starts).
  pub id: Option<RecordId>,
  /// Its system prompt when not the node's (an asked model's).
  pub system: Option<String>,
  /// It runs without tools.
  pub no_tools: bool,
  /// Changes to its tool and skill sets, written before it starts
  /// (design §2.14).
  pub tools: Vec<crate::toolset::Entry>,
  pub skills: Vec<crate::toolset::Entry>,
  /// Its report is kept for the user, waking nothing (`wake = false`).
  pub keep: bool,
  /// Memory remembers it; else it is out of memory from its start
  /// (design §2.9).
  pub remember: bool,
  /// Out of memory, it may still search memory (`recall`); else it
  /// recalls nothing either, as with `/memory off`. Remembered, it
  /// recalls anyway.
  pub recall: bool,
  /// Read-only: what the rules do not allow is refused, never asked.
  pub read_only: bool,
  /// A fixer of Stitch: in the fixers' approval mode, whatever the node's
  /// (`selfimprove::is_fixer`).
  pub fixer: bool,
  /// Started by a schedule (design §2.3): the schedule, the budget of its
  /// first turn (the thread itself has none), whom it reports to, and
  /// who its goal is from.
  pub scheduled: Option<Scheduled>,
  /// The app that created it (its conversation and name): a family
  /// thread, within the app's grant (design §2.22).
  pub app: Option<(RecordId, String)>,
  /// Files its goal comes with (an app's images).
  pub attachments: Vec<stitch_records::Attachment>,
  /// Threads and efforts attached to its goal (`spawn`'s `scopes`, design
  /// §2.27), resolved: written with it as `Part::Scope`.
  pub scope: Option<crate::scopes::Scoped>,
  /// The capability profile of the schedule that starts it
  /// (docs/events.md): the ceiling of its first turn.
  pub profile: Option<crate::profile::Profile>,
}

/// A thread a schedule starts.
#[derive(Clone)]
pub(crate) struct Scheduled {
  pub schedule: RecordId,
  pub turn_budget: Limits,
  pub reports_to: Option<RecordId>,
}

/// The root holding a record.
fn root_of(s: &Shared, id: &RecordId) -> Result<RecordRoot> {
  s.thread_place(id).map(|(root, _)| root)
}

fn one_line(s: &str) -> String {
  let line = s.lines().next().unwrap_or("").trim();
  if line.chars().count() > 100 || s.lines().count() > 1 {
    format!("{}…", line.chars().take(100).collect::<String>())
  } else {
    line.to_owned()
  }
}

/// An approver for loops nobody is attached to.
pub(crate) fn headless(s: Arc<Shared>, root: RecordRoot) -> Approver {
  headless_for(s, root, None)
}

/// A call run here for another node (`remote`), and the approvals it asks.
#[derive(Clone)]
pub(crate) struct ForPeer {
  /// The node that asked: a new question is synced to it at once, where
  /// the user may well be.
  pub peer: String,
  /// What a question still open is answered with (declined, by this
  /// node) when the call is dropped while it asks.
  pub withdrawn: &'static str,
}

/// As `headless`, for a call run for another node. Only an unanswered
/// question is taken up again: whether an answered one was had cannot be
/// told from the asking thread's steps, which this node may not have yet,
/// so an old answer never approves a new call.
pub(crate) fn headless_for(s: Arc<Shared>, root: RecordRoot, peer: Option<ForPeer>) -> Approver {
  Arc::new(move |req: ApprovalRequest| {
    let (s, root, peer) = (s.clone(), root.clone(), peer.clone());
    Box::pin(async move { ask(&s, &root, req, peer, None).await })
  })
}

/// The client a chat runs for, asked on its connection too. The question
/// is a record all the same (design §2.3): the first answer wins, from
/// there or from any client's attention list, and a client that leaves
/// answers nothing.
pub(crate) struct Inline {
  pub tx: tokio::sync::mpsc::UnboundedSender<crate::api::Reply>,
  pub waiting: crate::node::Waiting,
  /// The approval's ID on that connection.
  pub id: u64,
}

/// What a question still open says when the loop that asked it stops.
/// One left by a stopping node stays open: the thread resumes when the
/// node starts again and takes it up (an answer given meanwhile, from the
/// phone, included).
const WITHDRAWN: &str = "withdrawn: the loop that asked it stopped (interrupted or cancelled)";

/// An approval: a `question` record, waited on until it is answered (or
/// yolo is turned on meanwhile). Never declined for want of an answer;
/// withdrawn if the asking stops first.
pub(crate) async fn ask(
  s: &Arc<Shared>,
  root: &RecordRoot,
  req: ApprovalRequest,
  peer: Option<ForPeer>,
  inline: Option<Inline>,
) -> Decision {
  ask_and_wait(s, root, req, peer, inline)
    .await
    .unwrap_or_else(|e| Decision::decline(Some(format!("the approval could not be recorded: {e}"))))
}

/// Whether an answer only withdrew its question: it approves nothing and
/// declines nothing a loop asking again would take up.
fn withdrawal(answer: &stitch_store::IndexedRecord) -> bool {
  answer.fields.get("withdrawn").and_then(Value::as_bool) == Some(true)
}

/// Open approval questions of a thread.
fn open_questions(
  s: &Shared,
  root: &RecordRoot,
  thread: &RecordId,
) -> Result<Vec<stitch_store::IndexedRecord>> {
  let mut index = s.state().index.lock().expect("index lock");
  index.refresh(root)?;
  let mut open = Vec::new();
  for q in index.sources(kinds::QUESTION, "thread", thread, None)? {
    let approval = q.fields.get("approval").and_then(Value::as_bool) == Some(true);
    if approval
      && index
        .sources(kinds::ANSWER, "answers", &q.id, Some(""))?
        .is_empty()
    {
      open.push(q);
    }
  }
  Ok(open)
}

/// Approval questions of a thread that a loop asking again takes up: the
/// open ones, and answered ones asked after the thread's latest step was
/// written (the step that asked was never written, the node having
/// stopped: the answer is for that step, done again). One asked before the
/// latest step was had; asking again asks anew; one withdrawn is moot. The
/// question and the step are both written by the node running the loop, so
/// its clock alone decides (the answer may come from another node's).
fn reusable_questions(
  s: &Shared,
  root: &RecordRoot,
  thread: &RecordId,
) -> Result<Vec<stitch_store::IndexedRecord>> {
  let written = {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root,
      index: &state.index,
      blobs: &state.blobs,
    };
    state.index.lock().expect("index lock").refresh(root)?;
    chat::last_step_at(&ctx, thread)?
  };
  let index = s.state().index.lock().expect("index lock");
  let mut out = Vec::new();
  for q in index.sources(kinds::QUESTION, "thread", thread, None)? {
    if q.fields.get("approval").and_then(Value::as_bool) != Some(true) {
      continue;
    }
    let answers = index.sources(kinds::ANSWER, "answers", &q.id, Some(""))?;
    let pending = match answers.last() {
      None => true,
      Some(a) if withdrawal(a) => false,
      Some(_) => {
        written.is_none_or(|w| q.created > w)
          && !s
            .detached
            .had
            .lock()
            .expect("had lock")
            .contains(q.id.as_str())
      }
    };
    if pending {
      out.push(q);
    }
  }
  Ok(out)
}

async fn ask_and_wait(
  s: &Arc<Shared>,
  root: &RecordRoot,
  req: ApprovalRequest,
  peer: Option<ForPeer>,
  inline: Option<Inline>,
) -> Result<Decision> {
  /// Counted while it asks, however the asking ends (an abort included):
  /// a question of a thread nothing here asks for is stale (`sweep`).
  struct Asking<'a>(&'a Shared, String);
  impl Drop for Asking<'_> {
    fn drop(&mut self) {
      let mut asking = self.0.detached.asking.lock().expect("asking lock");
      if let Some(n) = asking.get_mut(&self.1) {
        *n -= 1;
        if *n == 0 {
          asking.remove(&self.1);
        }
      }
    }
  }
  let thread = req.thread.to_string();
  *s.detached
    .asking
    .lock()
    .expect("asking lock")
    .entry(thread.clone())
    .or_default() += 1;
  let _asking = Asking(s, thread);
  // A node stopping while it asks replays the call, as often as it takes
  // (`chat::waiting_on_user`): unmarked only once the asking ends.
  let id = req.thread.clone();
  chat::waiting_on_user(&s.state().index, &id, true);
  let decision = ask_and_wait_inner(s, root, req, peer, inline).await;
  chat::waiting_on_user(&s.state().index, &id, false);
  decision
}

/// The inline prompt on a client's connection, taken back however the
/// asking ends: the client is told, so it stops showing it.
struct Prompted {
  inline: Inline,
}

impl Drop for Prompted {
  fn drop(&mut self) {
    let id = self.inline.id;
    self
      .inline
      .waiting
      .lock()
      .expect("approvals lock")
      .remove(&id);
    let _ = self.inline.tx.send(crate::api::Reply::Answered { id });
  }
}

async fn ask_and_wait_inner(
  s: &Arc<Shared>,
  root: &RecordRoot,
  req: ApprovalRequest,
  peer: Option<ForPeer>,
  inline: Option<Inline>,
) -> Result<Decision> {
  let node = s.state().node.clone();
  let open = match peer {
    Some(_) => open_questions(s, root, &req.thread)?,
    None => reusable_questions(s, root, &req.thread)?,
  };
  let question = match open
    .iter()
    .find(|q| q.fields.get("command").and_then(Value::as_str) == Some(req.command.as_str()))
  {
    Some(q) => q.id.clone(),
    None => {
      // A thread resumed after a restart that asks something else than
      // before: what it asked then is moot.
      if peer.is_none()
        && let Some(since) = s.detached.since.get()
      {
        for q in open.iter().filter(|q| {
          q.created < *since
            && q.node == node.as_str()
            && q.fields.get("command").and_then(Value::as_str) != Some(req.command.as_str())
        }) {
          withdraw(
            s,
            root,
            &q.id,
            "withdrawn: its thread resumed after its node restarted and asked something else",
          )?;
        }
      }
      let mut choices: Vec<toml::Value> = vec!["allow".into()];
      if !req.grant.is_empty() {
        choices.push("always".into());
      }
      choices.push("decline".into());
      let mut draft = Draft::new(kinds::QUESTION, Author::node(node.clone()))?
        .field("approval", true)?
        .field("command", req.command.as_str())?
        .field("session", req.session.as_str())?
        .field("choices", toml::Value::Array(choices))?;
      // The hazard guard's verdict (design §2.28): with the user's answer,
      // what calibrates it.
      if let Some(v) = &req.verdict {
        draft = draft
          .field("guard_mode", v.mode.as_str())?
          .field("guard_threshold", v.threshold)?
          .field("guard_asked", v.asks())?;
        if let Some(p) = v.score {
          draft = draft.field("guard_score", p)?;
        }
        if let Some(h) = &v.hard {
          draft = draft.field("guard_hard_stop", h.as_str())?;
        }
        if let Some(e) = &v.error {
          draft = draft.field("guard_error", e.as_str())?;
        }
        if !v.files.is_empty() {
          draft = draft.field(
            "guard_files",
            toml::Value::Array(v.files.iter().map(|f| f.as_str().into()).collect()),
          )?;
        }
      }
      if !req.grant.is_empty() {
        // What "always" allows from then on, where: the asking node saves
        // it, wherever the answer came from.
        draft = draft.field(
          "grant",
          toml::Value::Array(req.grant.iter().map(|g| g.as_str().into()).collect()),
        )?;
        if let Some(server) = &req.grant_server {
          draft = draft.field("grant_server", server.as_str())?;
        }
        if req.grant_otter {
          draft = draft.field("grant_otter", true)?;
        }
        // A read-only room participant's "always": the thread writable.
        if req.writable {
          draft = draft.field("grant_writable", req.thread.as_str())?;
        }
        if req.browser.is_some() {
          // Sites, kept in `[browser.places]`.
          draft = draft.field("grant_browser", true)?;
        }
        if let Ok((_, place)) = s.thread_place(&req.thread) {
          draft = draft.field("place", place.key())?;
        }
      }
      // A browser question shows what the agent wants to do, with the
      // tab as it is (the target outlined), on every client and the phone.
      if let Some(image) = req.browser.as_ref().and_then(|b| b.image.as_ref())
        && let Ok((hash, bytes)) = s.state().blobs.put_file(image)
      {
        draft = draft.attach(stitch_records::Attachment {
          hash,
          media_type: "image/png".into(),
          name: Some("tab.png".into()),
          bytes,
        });
      }
      // What the hazard guard stopped is the user's alone to answer too
      // (design §2.28): an agent's process answering it would undo it.
      if req.user_only && req.review.is_none() {
        draft = draft.field("user_only", true)?;
      }
      // For the user alone (a landing, the fixers' settings): what they
      // read first, written by the node, and the whole diff as a file.
      if let Some(review) = &req.review {
        draft = draft
          .field("user_only", true)?
          .field("detail", review.detail.as_str())?;
        if let Some((name, bytes)) = &review.file
          && let Ok(hash) = s.state().blobs.put_bytes(bytes)
        {
          // Named in fields too, for clients that read the index alone.
          draft = draft
            .field("detail_file", hash.to_string())?
            .field("detail_file_name", name.as_str())?
            .attach(stitch_records::Attachment {
              hash,
              media_type: "text/x-diff".into(),
              name: Some(name.clone()),
              bytes: bytes.len() as u64,
            });
        }
      }
      let body = if let Some(review) = &req.review {
        format!(
          "# {}?\n\n```\n{}\n```\n",
          one_line(&req.command),
          review.detail.replace("```", "'''")
        )
      } else if req.session == crate::tools::STEPS_SESSION {
        format!("# {}\n", one_line(&req.command))
      } else if req.session == crate::folders::SYNC_SESSION
        || req.session == crate::clones::CLONE_SESSION
      {
        format!("# {}?\n", one_line(&req.command))
      } else if req.browser.is_some() {
        format!("# In your browser: {}?\n", one_line(&req.command))
      } else if req.writable {
        format!("# {}?\n", one_line(&req.command))
      } else {
        format!("# Run `{}`?\n", one_line(&req.command))
      };
      let draft = draft
        .reference("thread", Ref::to(req.thread.clone()))?
        .body(body);
      let q = root.publish(&node, draft)?;
      s.state().index.lock().expect("index lock").refresh(root)?;
      s.detached.asked.notify_waiters();
      if peer.is_none() {
        crate::weave::tell_blocked(s, &req.thread, &q.id);
      }
      // Where the user may be: the asking node for a call run for it,
      // every node otherwise (the phone's attention list); and the phones
      // told, when this node pushes, so it reaches one with the app closed.
      let (s, to) = (s.clone(), peer.as_ref().map(|p| p.peer.clone()));
      // A review's notification carries its TL;DR (its first lines, up
      // to the first blank one); the rest is read in the app.
      let command = match &req.review {
        Some(r) => {
          let tldr: Vec<&str> = r
            .detail
            .lines()
            .take_while(|l| !l.trim().is_empty())
            .take(6)
            .collect();
          format!("{}\n{}", one_line(&req.command), tldr.join("\n"))
        }
        None => one_line(&req.command),
      };
      let id = q.id.to_string();
      let title = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .title(&req.thread)
        .ok()
        .flatten();
      // The sync and the push apart: syncing waits on peers that cannot be
      // reached (the phone itself, its app closed: minutes of timeouts),
      // and the push is for exactly that phone; its record is here.
      {
        let (s, to) = (s.clone(), to.clone());
        tokio::spawn(async move { crate::node::sync_all(&s, to.as_deref()).await });
      }
      tokio::spawn(async move {
        if to.is_none() {
          let waiting = s
            .state()
            .index
            .lock()
            .expect("index lock")
            .open_questions()
            .map(|a| a.len())
            .ok();
          let heading = format!("Approve on {}?", s.state().node);
          for line in crate::node::push(
            &s,
            &crate::apns::Push {
              title: &heading,
              subtitle: title.as_deref(),
              body: &command,
              badge: waiting,
              question: Some(&id),
              node: s.state().node.as_str(),
              answered: false,
              thread: None,
            },
          )
          .await
          {
            // Said whatever it is, a phone not registered included.
            eprintln!("apns: question {id}: {line}");
          }
        }
      });
      q.id
    }
  };
  // Once answered or withdrawn (dropped after `_withdraw`), the phones
  // take its notification away.
  let _cleared = Cleared {
    s,
    question: question.clone(),
    pushed: peer.is_none(),
  };
  // Answered, it withdraws nothing.
  let _withdraw = Withdraw {
    s,
    root,
    question: question.clone(),
    reason: peer.as_ref().map_or(WITHDRAWN, |p| p.withdrawn),
    peer: peer.as_ref().map(|p| p.peer.clone()),
  };
  // Asked on the client's connection too, while it is there.
  let mut decided = None;
  let mut _prompted = None;
  if let Some(inline) = inline {
    let (answer, rx) = oneshot::channel();
    inline
      .waiting
      .lock()
      .expect("approvals lock")
      .insert(inline.id, answer);
    let sent = inline.tx.send(crate::api::Reply::Approve {
      id: inline.id,
      session: req.session.clone(),
      command: req.command.clone(),
      question: question.to_string(),
      grant: req.grant.clone(),
    });
    if sent.is_ok() {
      decided = Some(rx);
    }
    _prompted = Some(Prompted { inline });
  }
  loop {
    let answered = s.detached.answered.notified();
    if let Some(d) = answer_of(s, root, &question)? {
      s.detached
        .had
        .lock()
        .expect("had lock")
        .insert(question.to_string());
      note_in_room(s, root, &question);
      if d.always {
        apply_grant(s, root, &question);
        // Others waiting on what it grants go too.
        s.detached.answered.notify_waiters();
      }
      return Ok(d);
    }
    // Allowed by the rules meanwhile (yolo turned on, what it needs granted
    // by an "always" answer to another question), it goes too.
    if req.allowed_now() {
      let draft = Draft::new(kinds::ANSWER, Author::node(node.clone()))?
        .field("choice", "allow")?
        .reference("answers", Ref::to(question.clone()))?
        .body("allowed: the rules allow it now (yolo, or granted where its thread works)");
      root.publish(&node, draft)?;
      s.state().index.lock().expect("index lock").refresh(root)?;
      s.detached.answered.notify_waiters();
      continue;
    }
    // Answers written here wake us at once; synced ones are seen within
    // seconds.
    let outside = req.browser.as_ref().and_then(|b| b.outside.clone());
    tokio::select! {
      _ = answered => {}
      _ = tokio::time::sleep(Duration::from_secs(2)) => {}
      // Answered in the page: written as any answer, saying so.
      _ = async { outside.as_ref().expect("checked").given.notified().await }, if outside.is_some() => {
        let d = outside.as_ref().and_then(|o| o.decision.lock().expect("outside lock").take());
        if let Some(d) = d {
          answer(s, &question, d)?;
          let s = s.clone();
          tokio::spawn(async move {
            crate::node::sync_all(&s, None).await;
          });
        }
      }
      d = async { decided.as_mut().expect("checked").await }, if decided.is_some() => match d {
        // Written as any answer is, so every node sees it was had.
        Ok(d) => {
          decided = None;
          answer(s, &question, d)?;
          let s = s.clone();
          tokio::spawn(async move {
            crate::node::sync_all(&s, None).await;
          });
        }
        // The client left: the question waits in the attention list.
        Err(_) => decided = None,
      },
    }
  }
}

/// A question still open when its asking stops (the loop interrupted,
/// cancelled or stopped with its node; the caller of a call run for
/// another node gone) is answered then: declined, by this node, marked
/// withdrawn, so no client shows it as waiting and nothing takes it up as
/// a decision.
struct Withdraw<'a> {
  s: &'a Arc<Shared>,
  root: &'a RecordRoot,
  question: RecordId,
  reason: &'static str,
  /// The node to tell at once, for a call run for it; every node else.
  peer: Option<String>,
}

impl Drop for Withdraw<'_> {
  fn drop(&mut self) {
    // A stopping node's loops resume at its next start and ask again;
    // a call for another node does not resume.
    let stopping = self
      .s
      .detached
      .stopping
      .load(std::sync::atomic::Ordering::SeqCst);
    if (stopping && self.peer.is_none())
      || !matches!(answer_of(self.s, self.root, &self.question), Ok(None))
    {
      return;
    }
    match withdraw(self.s, self.root, &self.question, self.reason) {
      Ok(()) => {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
          let (s, to) = (self.s.clone(), self.peer.clone());
          rt.spawn(async move {
            crate::node::sync_all(&s, to.as_deref()).await;
          });
        }
      }
      Err(e) => eprintln!("question {}: not withdrawn: {e}", self.question),
    }
  }
}

/// A question this node pushed to the phones, no longer asked (answered
/// here or elsewhere, or withdrawn): a background push tells them, so the
/// notification goes even with the app closed (design §5, milestone 4).
/// Not when the node stops: its loops ask again at its next start.
struct Cleared<'a> {
  s: &'a Arc<Shared>,
  question: RecordId,
  /// Pushed when asked (a call run for another node is not).
  pushed: bool,
}

impl Drop for Cleared<'_> {
  fn drop(&mut self) {
    let stopping = self
      .s
      .detached
      .stopping
      .load(std::sync::atomic::Ordering::SeqCst);
    if !self.pushed || stopping {
      return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
      return;
    };
    let (s, id) = (self.s.clone(), self.question.to_string());
    rt.spawn(async move {
      let waiting = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .open_questions()
        .map(|a| a.len())
        .ok();
      for line in crate::node::push(
        &s,
        &crate::apns::Push {
          title: "",
          subtitle: None,
          body: "",
          badge: waiting,
          question: Some(&id),
          node: s.state().node.as_str(),
          answered: true,
          thread: None,
        },
      )
      .await
      {
        eprintln!("apns: question {id} answered: {line}");
      }
    });
  }
}

/// Answers a question nobody asks any more: declined, marked withdrawn.
fn withdraw(s: &Arc<Shared>, root: &RecordRoot, question: &RecordId, reason: &str) -> Result<()> {
  let node = s.state().node.clone();
  let draft = Draft::new(kinds::ANSWER, Author::node(node.clone()))?
    .field("choice", "decline")?
    .field("withdrawn", true)?
    .reference("answers", Ref::to(question.clone()))?
    .body(reason);
  root.publish(&node, draft)?;
  s.state().index.lock().expect("index lock").refresh(root)?;
  s.detached.answered.notify_waiters();
  note_in_room(s, root, question);
  Ok(())
}

/// Withdraws the approval questions this node asked that nothing here asks
/// any more and nothing will: their thread has no loop here (one resumed
/// at start is listed before this runs, and takes its question up). Left
/// by a chat its node stopped under, a crash, an older version, a loop
/// that ended some other way. Run after threads resume at start, and now
/// and then after. Returns how many.
pub(crate) fn sweep(s: &Arc<Shared>) -> usize {
  let me = s.state().node.to_string();
  // The questions first, then who asks: a loop counts itself as asking
  // before it writes its question, so one listed here and asked for is
  // in the count read after.
  let open: Vec<(RecordId, RecordId)> = {
    let index = s.state().index.lock().expect("index lock");
    let Ok(questions) = index.records(Some(kinds::QUESTION)) else {
      return 0;
    };
    questions
      .into_iter()
      .filter(|q| q.node == me && q.fields.get("approval").and_then(Value::as_bool) == Some(true))
      .filter(|q| {
        index
          .sources(kinds::ANSWER, "answers", &q.id, Some(""))
          .is_ok_and(|a| a.is_empty())
      })
      .filter_map(|q| {
        let thread = index
          .edges_from(&q.id)
          .ok()?
          .into_iter()
          .find(|e| e.rel == "thread")?
          .dst
          .id;
        Some((q.id, thread))
      })
      .collect()
  };
  // One lock at a time (each is taken with others elsewhere).
  let asking: Vec<String> = s
    .detached
    .asking
    .lock()
    .expect("asking lock")
    .keys()
    .cloned()
    .collect();
  let running: Vec<String> = s
    .detached
    .running
    .lock()
    .expect("running lock")
    .keys()
    .cloned()
    .collect();
  let steered: Vec<String> = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .keys()
    .cloned()
    .collect();
  let stale: Vec<(RecordId, RecordId)> = open
    .into_iter()
    .filter(|(_, thread)| {
      let t = thread.as_str();
      ![&asking, &running, &steered]
        .iter()
        .any(|l| l.iter().any(|x| x == t))
    })
    .collect();
  let mut n = 0;
  for (question, _) in stale {
    let Ok(root) = root_of(s, &question) else {
      continue;
    };
    match withdraw(
      s,
      &root,
      &question,
      "withdrawn: nothing on its node asks it any more (the node restarted, or its loop ended)",
    ) {
      Ok(()) => n += 1,
      Err(e) => eprintln!("question {question}: not withdrawn: {e}"),
    }
  }
  if n > 0 {
    let s = s.clone();
    tokio::spawn(async move {
      crate::node::sync_all(&s, None).await;
    });
  }
  n
}

fn answer_of(s: &Shared, root: &RecordRoot, question: &RecordId) -> Result<Option<Decision>> {
  let answers = answers_of(s, root, question)?;
  let Some(a) = answers.last() else {
    return Ok(None);
  };
  let choice = a.fields.get("choice").and_then(Value::as_str);
  let reason = root
    .read_record(&a.id)
    .ok()
    .map(|r| r.body.trim().to_owned())
    .filter(|b| !b.is_empty());
  Ok(Some(match choice {
    Some("allow") => Decision::allow(),
    Some("always") => Decision::always(),
    _ => Decision::decline(reason),
  }))
}

/// The answers that count for `question`, the last deciding: a question
/// for the user alone takes no allowing answer but theirs.
fn answers_of(
  s: &Shared,
  root: &RecordRoot,
  question: &RecordId,
) -> Result<Vec<stitch_store::IndexedRecord>> {
  let answers = {
    let mut index = s.state().index.lock().expect("index lock");
    index.refresh(root)?;
    let mut answers = index.sources(kinds::ANSWER, "answers", question, Some(""))?;
    // A question for the user alone takes no allowing answer but theirs.
    if index
      .record(question)?
      .is_some_and(|q| q.fields.get("user_only").and_then(Value::as_bool) == Some(true))
    {
      answers
        .retain(|a| a.by_user || a.fields.get("choice").and_then(Value::as_str) == Some("decline"));
    }
    answers
  };
  Ok(answers)
}

/// A room participant's question settled (answered, or withdrawn): a note
/// of it in its room (design §2.27, `rooms::note_question`), written by
/// the node that asked, once.
fn note_in_room(s: &Arc<Shared>, root: &RecordRoot, question: &RecordId) {
  let Ok(answers) = answers_of(s, root, question) else {
    return;
  };
  let Some(a) = answers.last() else {
    return;
  };
  let Ok(q) = root.read_record(question) else {
    return;
  };
  let Some(thread) = q.refs.first("thread").map(|r| r.id.clone()) else {
    return;
  };
  let reason = root
    .read_record(&a.id)
    .ok()
    .map(|r| r.body.trim().to_owned())
    .filter(|b| !b.is_empty());
  let settled = if a.fields.get("withdrawn").and_then(Value::as_bool) == Some(true) {
    crate::rooms::Settled::Withdrawn
  } else {
    match a.fields.get("choice").and_then(Value::as_str) {
      Some("allow") => crate::rooms::Settled::Allowed,
      Some("always") => crate::rooms::Settled::Always,
      _ => crate::rooms::Settled::Declined(reason),
    }
  };
  let asked = crate::rooms::Asked {
    question: question.clone(),
    thread,
    command: q.field_str("command").unwrap_or_default().to_owned(),
    writable: q.field_str("grant_writable").is_some(),
    grant: q
      .fields
      .get("grant")
      .and_then(toml::Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(|v| v.as_str().map(str::to_owned))
          .collect()
      })
      .unwrap_or_default(),
  };
  crate::rooms::note_question(s, asked, settled);
}

/// An "always" answer: the question's `grant` (command prefixes, tools of
/// its `grant_server`, or Otter tools with `grant_otter`) is allowed from
/// now on in its `place`, on this node (the one that asked), saved in
/// config.toml.
fn apply_grant(s: &Shared, root: &RecordRoot, question: &RecordId) {
  let Ok(q) = root.read_record(question) else {
    return;
  };
  let grant: Vec<String> = q
    .fields
    .get("grant")
    .and_then(toml::Value::as_array)
    .map(|a| {
      a.iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
    })
    .unwrap_or_default();
  // A read-only room participant made writable (design §2.27): a `set`
  // of its `approval` for the user, as their switch is, so it syncs.
  if let Some(thread) = q.field_str("grant_writable") {
    let Ok(thread) = thread.parse::<RecordId>() else {
      return;
    };
    match crate::meta::set(
      s,
      &thread,
      "approval",
      "default".into(),
      stitch_records::Author::user(s.state().node.clone()),
    ) {
      Ok(()) => eprintln!("approvals: {thread} made writable"),
      Err(e) => eprintln!("approvals: {thread} not made writable: {e}"),
    }
    return;
  }
  let Some(place) = q.field_str("place").map(str::to_owned) else {
    return;
  };
  if grant.is_empty() {
    return;
  }
  let otter = grants_otter(&q.fields);
  let browser = q.fields.get("grant_browser").and_then(toml::Value::as_bool) == Some(true);
  let (granted, what) = match q.field_str("grant_server") {
    None if browser => (
      s.grant_browser(&place, &grant),
      format!("browser sites: {}", grant.join(", ")),
    ),
    Some(server) => (
      s.grant_mcp(&place, server, &grant),
      format!("{server}: {}", grant.join(", ")),
    ),
    None if otter => (
      s.grant_otter(&place, &grant),
      format!("otter: {}", grant.join(", ")),
    ),
    None => (s.grant(&place, &grant), grant.join(", ")),
  };
  match granted {
    Ok(()) => eprintln!("approvals: {what} allowed from now on in {place}"),
    Err(e) => eprintln!("approvals: {what} not granted in {place}: {e}"),
  }
}

/// Whether a question's `grant` names Otter tools: `grant_otter`, or
/// `grant_shards` in one asked before the language was renamed.
fn grants_otter(fields: &toml::Table) -> bool {
  ["grant_otter", "grant_shards"]
    .iter()
    .any(|f| fields.get(*f).and_then(toml::Value::as_bool) == Some(true))
}

/// Answers an approval question.
pub(crate) fn answer(s: &Shared, question: &RecordId, decision: Decision) -> Result<()> {
  let root = root_of(s, question)?;
  let node = s.state().node.clone();
  let choice = match (decision.allow, decision.always) {
    (true, true) => "always",
    (true, false) => "allow",
    (false, _) => "decline",
  };
  let draft = Draft::new(kinds::ANSWER, Author::user(node.clone()))?
    .field("choice", choice)?
    .reference("answers", Ref::to(question.clone()))?
    .body(decision.reason.unwrap_or_default());
  root.publish(&node, draft)?;
  s.state().index.lock().expect("index lock").refresh(&root)?;
  s.detached.answered.notify_waiters();
  Ok(())
}

impl Detachments {
  /// Stops every loop (the node is stopping); they resume at next start.
  pub(crate) fn abort_all(&self) {
    self
      .stopping
      .store(true, std::sync::atomic::Ordering::SeqCst);
    let all: Vec<Loop> = self
      .running
      .lock()
      .expect("running lock")
      .drain()
      .flat_map(|(_, loops)| loops)
      .collect();
    for l in all {
      l.task.abort();
    }
  }
}

/// What a thread asked to be remembered is told when its place's
/// `memory = false` keeps it out (design §2.9).
pub(crate) const PLACE_MEMORY_OFF: &str = "[Stitch: this thread was asked to be remembered, but \
  its place's settings say `memory = false`, which wins: it is out of memory (what it does is \
  not remembered), and may still recall. Only the user brings it back (`/memory on`).]";

/// Where a detached thread lives: `Spec::at`, else placed from its `repo`
/// and `cwd`.
pub(crate) fn placed(s: &Shared, spec: &Spec) -> Result<(RecordRoot, crate::place::Place)> {
  match spec.at.clone() {
    Some(at) => Ok(at),
    None => {
      let here = s.place_of_dir(spec.cwd.as_deref());
      s.placement(spec.repo.as_deref(), spec.global, &here)
    }
  }
}

/// Whether a thread asked to be remembered starts out of memory all the
/// same, its place's `memory = false` winning (design §2.9).
pub(crate) fn place_keeps_out(s: &Shared, spec: &Spec) -> bool {
  spec.remember && placed(s, spec).is_ok_and(|(_, place)| s.place_memory_off(&place))
}

/// Starts a detached thread; returns its ID once it exists.
pub(crate) async fn start(s: &Arc<Shared>, spec: Spec) -> Result<RecordId> {
  let (root, place) = placed(s, &spec)?;
  // Chosen here, so notes for it are taken from its first step.
  let id = spec
    .id
    .clone()
    .unwrap_or_else(|| RecordId::random(jiff::Timestamp::now(), &s.state().node));
  // A folder's files as other nodes left them, before it reads them; what
  // it changes, published when it ends (design §2.16). A phone's project
  // cloned first, so its directory is the place's. A project with no
  // checkout here: found, else refused (no thread to ask for a clone yet).
  crate::checkout::ensure(s, &place, Some(&id), None).await?;
  let folder_turn = crate::folders::turn(s, &place, Some(&id)).await;
  let start = s.start_dir(&place, spec.cwd.as_deref().map(Path::new), Some(&id));
  crate::toolset::write_changes(
    &root,
    &s.state().index,
    &s.state().node,
    &id,
    &spec.tools,
    &spec.skills,
  )?;
  // Out of memory before its first step, unless asked: nothing of it is
  // observed, and nothing of it recalled anywhere (the fact syncs with the
  // thread); it searches memory itself unless told not to.
  // A place whose settings say `memory = false` wins over `remember`
  // (the spawn tool's, an assignment's, a schedule's): only the user's
  // own switch brings it back (design §2.9).
  let place_off = spec.remember && s.place_memory_off(&place) && !s.memory.is_switched(&id);
  if !spec.remember || place_off {
    s.memory.switch(s.state(), &root, &id, false, spec.recall)?;
  }
  if place_off {
    eprintln!(
      "thread {id}: out of memory, as its place's `memory = false` says (asked to be remembered)"
    );
  }
  let mut selection = s.selection(&place, start.as_deref(), Some(&id));
  selection.profiles.extend(spec.profile.clone());
  let context = s.context(&place, start.as_deref(), &selection, Some(&id));
  let folder = s.folder_of(&place, spec.cwd.as_deref());
  // As named (`quick`, `default`, or none: the user's, for its place).
  let full_model = crate::modelset::named(s, Some(&place), spec.model.as_deref()).await;
  let (provider, provider_name, model) = s
    .config
    .resolve_authenticated(&s.data, Some(&full_model))
    .await?;
  // What it may outgrow midway (`goes_on_compacted`).
  let outgrows = format!("{provider_name}/{model}");
  let (tx, rx) = oneshot::channel::<RecordId>();
  let s2 = s.clone();
  let (serial, registered, listed) = registration(s, &id);
  let limits = loop_limits(
    s,
    spec
      .scheduled
      .as_ref()
      .map_or(spec.budget, |sc| sc.turn_budget),
  );
  let task = tokio::spawn(async move {
    let _registered = registered;
    let _folder_turn = folder_turn;
    // Nothing else runs on it meanwhile (a report or a message for it
    // waits its turn).
    let lock = s2.detached.locks.of(&id);
    let _held = lock.lock().await;
    // The assignment may have been withdrawn during setup or while taking
    // this lock. No thread or provider call is needed to honour it.
    if spec
      .assignment
      .as_ref()
      .is_some_and(|a| crate::weave::assignment_cancelled(&s2, a))
    {
      let _ = tx.send(id);
      return;
    }
    let steer = chat::Steer::open();
    let _steering = Steering::new(&s2, &id, steer.clone());
    if spec.fixer {
      s2.detached
        .fixers
        .lock()
        .expect("fixers lock")
        .insert(id.to_string());
    }
    let tools = tools_for(
      &s2,
      &root,
      &place,
      spec.read_only,
      &id,
      start.as_deref(),
      spec.no_tools,
      &selection,
      &full_model,
      false,
    );
    let system = if spec.no_tools {
      spec.system.clone()
    } else {
      let root_prompt = match &spec.app {
        // An app's thread: a prompt of its own, nothing of the user's
        // (design §2.22).
        Some((_, name)) => crate::apps::threads::system_prompt(name, &context),
        None => s2.system_prompt(true, &context),
      };
      Some(match &spec.system {
        Some(extra) => format!("{root_prompt}\n\n{extra}"),
        None => root_prompt,
      })
    };
    let input = ChatInput {
      id: Some(id.clone()),
      thread: None,
      text: spec.goal.clone(),
      attachments: spec.attachments.clone(),
      provider,
      model,
      system,
      // A spawned thread's goal is its spawner's message, not the user's;
      // a scheduled one's, its schedule's.
      from: match (&spec.scheduled, &spec.app) {
        (Some(sc), _) => Some(format!("schedule {}", sc.schedule)),
        (None, Some((_, name))) => Some(format!("app \"{name}\"")),
        (None, None) => spec.parent.as_ref().map(|p| format!("thread {p}")),
      },
      detached: Some(Detached {
        parent: spec.parent.clone(),
        budget: spec.budget,
        model: Some(full_model),
        cwd: spec.cwd.clone(),
        assignment: spec.assignment.clone(),
        continues: spec.continues.clone(),
        no_tools: spec.no_tools,
        keep: spec.keep,
        read_only: spec.read_only,
        fixer: spec.fixer,
        scheduled_by: spec.scheduled.as_ref().map(|sc| sc.schedule.clone()),
        reports_to: spec.scheduled.as_ref().and_then(|sc| sc.reports_to.clone()),
        app: spec.app.clone(),
      }),
      limits,
      recalled: None,
      // Told, so it does not believe what it leaves is remembered.
      instruction: place_off.then(|| PLACE_MEMORY_OFF.to_owned()),
      context: None,
      folder,
      steer: Some(steer.clone()),
      spoken: false,
      scope: spec.scope.clone(),
      room_turn: None,
    };
    let mut tx = Some(tx);
    let mut thread: Option<RecordId> = None;
    // What its steps spend from here (`left_after`).
    let since = jiff::Timestamp::now();
    let result = {
      let state = s2.state();
      let ctx = ChatContext {
        node: &state.node,
        root: &root,
        index: &state.index,
        blobs: &state.blobs,
      };
      let live = &s2.live;
      let mut on_event = |e: ChatEvent| {
        if let ChatEvent::Thread(t) = e {
          thread = Some(t.clone());
          if let Some(tx) = tx.take() {
            let _ = tx.send(t);
          }
        } else if let Some(t) = &thread {
          live.event(t.as_str(), &e);
        }
      };
      chat::send(&ctx, input, &*tools, &mut on_event).await
    };
    if let Some(t) = thread {
      let midway = goes_on_compacted(
        &s2,
        &t,
        &result,
        &outgrows,
        since,
        limits,
        None,
        false,
        spec.profile.clone(),
      )
      .await;
      if matches!(midway, Midway::GoesOn) {
        return;
      }
      answer_askers(
        &s2,
        &root,
        &t,
        &result,
        steer.askers(),
        spec.parent.as_ref(),
      );
      match midway {
        Midway::Spent(outcome, text) => {
          finish_said(&s2, &root, &t, (outcome, text, true), None, false).await
        }
        _ => finish(&s2, &root, &t, result).await,
      }
    }
  });
  register(s, &listed, serial, task.abort_handle());
  let thread = rx
    .await
    .map_err(|_| Error::Protocol("the detached thread did not start".into()))?;
  Ok(thread)
}

/// Whether a loop's result is a model call refused as too long for its
/// model, unwritten (`chat::failed`): what a midway compaction answers.
fn too_long_unwritten(result: &Result<ChatOutcome>) -> bool {
  matches!(result, Err(e) if !matches!(e, Error::Kept { .. }) && crate::fork::too_long(e))
}

/// What is left of a loop's `limits` after its try on `thread` since
/// `since` was refused as too long: its model steps (those written, and
/// the refused call, which counts as one: `Limits::steps` is the model
/// calls a turn takes), its tokens and money (the usage its steps wrote,
/// its tools' included; the refused call spent none) taken off; its
/// deadline as it was. A loop continued after a midway compaction runs
/// under these, so the two tries together never take more than the turn
/// was given. `Err` when nothing is left: the outcome and the words of the
/// report the limit makes instead.
fn left_after(
  s: &Arc<Shared>,
  thread: &RecordId,
  since: jiff::Timestamp,
  limits: Limits,
) -> std::result::Result<Limits, (&'static str, String)> {
  let mut steps: u64 = 1;
  if let Ok((root, _)) = s.thread_place(thread) {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    for e in chat::thread_events(&ctx, thread).unwrap_or_default() {
      let model_step = match &e {
        stitch_records::Event::Msg(m) => m.role == stitch_records::Role::Assistant && m.at >= since,
        stitch_records::Event::Other(v) => {
          v.get("t").and_then(Value::as_str) == Some("refusal")
            && v
              .get("at")
              .and_then(Value::as_str)
              .and_then(|a| a.parse::<jiff::Timestamp>().ok())
              .is_some_and(|at| at >= since)
        }
        _ => false,
      };
      steps += u64::from(model_step);
    }
  }
  let (usage, usd) = spent_by(s, thread, Some(since)).unwrap_or_default();
  let tokens = usage.input_tokens + usage.output_tokens;
  let max_steps = limits.steps.unwrap_or(chat::MAX_STEPS);
  let left = Limits {
    usd: limits.usd.map(|u| u - usd.unwrap_or(0.0)),
    tokens: limits.tokens.map(|t| t.saturating_sub(tokens)),
    until: limits.until,
    steps: Some(max_steps.saturating_sub(steps)),
  };
  if left.steps == Some(0) {
    return Err((
      "stopped",
      format!(
        "Stopped after its most steps ({steps} of {max_steps}): its last model call was \
         refused as too long for the model's window, and no step was left to go on with in a \
         compacted thread."
      ),
    ));
  }
  if let Some(why) = left.spent() {
    return Err((
      "over-budget",
      format!(
        "Stopped: {why}: its last model call was refused as too long for the model's window, \
         and nothing was left to go on with in a compacted thread."
      ),
    ));
  }
  Ok(left)
}

/// What became of a loop refused as too long midway (`goes_on_compacted`).
enum Midway {
  /// It goes on in the compacted thread: the caller reports nothing.
  GoesOn,
  /// Nothing of its limits was left: the caller reports this instead of
  /// the failure (`finish_said`).
  Spent(&'static str, String),
  /// Not that, or not compacted: the caller reports the result as it is.
  No,
}

/// A loop whose model call was too long for its model (the turn grew past
/// the window midway, issue 2026-11-19T095120Z-gamer-rwg5), on `thread`
/// since `since`, under `limits`: its thread is compacted, and the turn
/// goes on in the new one (`MIDWAY_NOTE`) under what is left of `limits`
/// (`left_after`), whose report it is. That turn may compact once more
/// (`turn_with`'s retry), under what is left again. Not for an app's thread
/// that may not compact, nor when the compaction fails (nothing written
/// yet to summarize: the first step too long).
#[allow(clippy::too_many_arguments)]
async fn goes_on_compacted(
  s: &Arc<Shared>,
  thread: &RecordId,
  result: &Result<ChatOutcome>,
  full_model: &str,
  since: jiff::Timestamp,
  limits: Limits,
  report_to: Option<RecordId>,
  keep: bool,
  profile: Option<crate::profile::Profile>,
) -> Midway {
  if !too_long_unwritten(result) {
    return Midway::No;
  }
  let left = match left_after(s, thread, since, limits) {
    Ok(left) => left,
    Err((outcome, text)) => return Midway::Spent(outcome, text),
  };
  if let Some(owner) = crate::apps::threads::owner_of(s, thread) {
    let place = s.thread_place(thread).ok().map(|(_, p)| p);
    if !crate::apps::threads::may_spend(s, &owner, place.as_ref()).await {
      return Midway::No;
    }
  }
  match compact_midway(s, thread, full_model, left).await {
    Compacted::Into(new, left) => {
      eprintln!("thread {thread}: too long for {full_model} midway: goes on in {new}");
      turn_with(
        s.clone(),
        new,
        MIDWAY_NOTE.to_owned(),
        "stitch".into(),
        report_to,
        Vec::new(),
        keep,
        TurnOpts {
          budget: Some(left),
          // A schedule's capability profile holds the rest of its turn.
          profile,
          ..TurnOpts::default()
        },
      );
      Midway::GoesOn
    }
    // Its summary took what was left: the new thread's report says so.
    Compacted::SpentOn(new, outcome, text) => {
      match s.thread_place(&new) {
        Ok((root, _)) => {
          finish_said(s, &root, &new, (outcome, text, true), report_to, keep).await;
        }
        Err(e) => eprintln!("thread {new}: no report: {e}"),
      }
      Midway::GoesOn
    }
    Compacted::Unaffordable(outcome, text) => Midway::Spent(outcome, text),
    Compacted::Failed => Midway::No,
  }
}

/// What came of compacting a thread midway (`compact_midway`).
enum Compacted {
  /// Into the new thread, with what is left of the turn's limits after
  /// its summary.
  Into(RecordId, Limits),
  /// Into the new thread, its summary taking what was left: no step
  /// there; the outcome and words of its report.
  SpentOn(RecordId, &'static str, String),
  /// Not compacted: its summary may cost more than is left. The outcome
  /// and words of the report.
  Unaffordable(&'static str, String),
  /// Not compacted: the compaction failed (the turn reports its error).
  Failed,
}

/// Compacts `thread` for a turn refused as too long midway, with `left`
/// of its limits (`left_after`). Its summary is a model call too: with
/// money or tokens limited, not when the most it may spend
/// (`fork::compaction_bound`) is not under what is left (nor when its price
/// is not known beforehand); compacted, what it spent is taken off.
async fn compact_midway(
  s: &Arc<Shared>,
  thread: &RecordId,
  full_model: &str,
  left: Limits,
) -> Compacted {
  const WHY: &str = "its last model call was refused as too long for the model's window";
  if left.usd.is_some() || left.tokens.is_some() {
    let bound = match crate::fork::compaction_bound(s, thread).await {
      Ok(b) => b,
      Err(e) => {
        eprintln!("thread {thread}: too long, and not compacted: {e}");
        return Compacted::Failed;
      }
    };
    let short = match (left.usd, bound.usd) {
      (Some(u), Some(c)) if c >= u => Some(format!(
        "compacting it to go on would ask a summary that may cost up to ${c:.4}, and ${u:.4} \
         of its budget is left"
      )),
      (Some(u), None) => Some(format!(
        "compacting it to go on would ask a summary whose price is not known beforehand, and \
         ${u:.4} of its budget is left"
      )),
      _ => None,
    }
    .or_else(|| match left.tokens {
      Some(t) if bound.tokens() >= t => Some(format!(
        "compacting it to go on would ask a summary that may use up to {} tokens, and {t} of \
         its budget are left",
        bound.tokens()
      )),
      _ => None,
    });
    if let Some(short) = short {
      return Compacted::Unaffordable("over-budget", format!("Stopped: {WHY}, and {short}."));
    }
  }
  match crate::fork::compact_counted(s, thread, full_model).await {
    Ok((new, _, spent)) => {
      // Written as spent on the thread compacted (`fork::compact_as`), so
      // later turns and caps count it; this turn's limits take it off here.
      let left = Limits {
        usd: left.usd.map(|u| u - spent.usd.unwrap_or(0.0)),
        tokens: left.tokens.map(|t| t.saturating_sub(spent.tokens())),
        ..left
      };
      match left.spent() {
        Some(why) => Compacted::SpentOn(
          new,
          "over-budget",
          format!("Stopped: {WHY}; compacted, its summary took the rest: {why}."),
        ),
        None => Compacted::Into(new, left),
      }
    }
    Err(c) => {
      eprintln!("thread {thread}: too long, and not compacted: {c}");
      Compacted::Failed
    }
  }
}

/// A typed chat's turn (one forwarded from the node the user typed on)
/// takes `[chat] max_steps` model calls, as a local chat does; the rest of
/// its limits stay the thread's own (its budget, which the user's writing
/// lifted at the writer, design §2.7).
fn with_chat_steps(limits: Limits, chat_steps: u64) -> Limits {
  Limits {
    steps: Some(chat_steps),
    ..limits
  }
}

/// A detached loop's limits: its budget, with the node's step limit
/// (`[detached] max_steps`) when it set none.
fn loop_limits(s: &Shared, budget: Limits) -> Limits {
  Limits {
    steps: budget.steps.or_else(|| {
      s.config
        .detached
        .as_ref()
        .and_then(|d| d.max_steps)
        .filter(|n| *n > 0)
    }),
    ..budget
  }
}

/// Whether a thread's shells outlive its turn (`[detached] keep_shells`).
fn keeps_shells(s: &Shared) -> bool {
  s.config
    .detached
    .as_ref()
    .and_then(|d| d.keep_shells)
    .unwrap_or(false)
}

/// Stops a detached thread; its report says it was cancelled. One
/// running on another node stops when the report reaches it (it is
/// nudged).
pub(crate) fn cancel(s: &Arc<Shared>, thread: &RecordId) -> Result<()> {
  cancel_with(s, thread, "Cancelled by the user.").map(|_| ())
}

/// Where a cancelled thread stopped.
pub(crate) enum CancelledAt {
  /// Its loops here, with its jobs and sessions.
  Here,
  /// Another node, which stops it when the report reaches it.
  There(String),
  /// Its assignment is known, but its start has not been confirmed here.
  Pending(String),
}

/// `cancel`, its report saying `text`.
pub(crate) fn cancel_with(s: &Arc<Shared>, thread: &RecordId, text: &str) -> Result<CancelledAt> {
  // A thread compacted into another runs there.
  let thread = &crate::fork::current(s, thread);
  if s
    .state()
    .index
    .lock()
    .expect("index lock")
    .record(thread)?
    .is_none()
  {
    return crate::weave::cancel_pending(s, thread, text).map(CancelledAt::Pending);
  }
  let root = root_of(s, thread)?;
  let loops = take_loops(s, thread);
  if !loops.is_empty() {
    // Its jobs end first, so none tells it anything; then the report, so
    // what its loop leaves untaken as it stops wakes nothing (`wakes`).
    stop_here(s, thread, loops, || {
      write_report(s, &root, thread, "cancelled", text)
    })?;
    return Ok(CancelledAt::Here);
  }
  // No loop of its own, but a chat's turn runs on it here (the web app's
  // Cancel of a turn it typed, from another connection than the chat's):
  // that turn is interrupted, as the chat's own Esc does. Not a detached
  // thread's work, so no report; its command stops with the turn.
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned();
  if let Some(st) = steer {
    st.interrupt();
    return Ok(CancelledAt::Here);
  }
  let record = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .record(thread)?
    .ok_or_else(|| Error::Config(format!("no thread {thread} here")))?;
  let open = chat::detached_state(&s.state().index.lock().expect("index lock"), &record)?;
  if !matches!(open.as_deref(), Some("running" | "waiting"))
    || record.node == s.state().node.as_str()
  {
    return Err(Error::Config(format!("{thread} is not running")));
  }
  let said = match text.strip_suffix('.') {
    Some(t) if !t.contains('\n') => format!("{t} on {}.", s.state().node),
    _ => format!("{text}\n\n(Cancelled on {}.)", s.state().node),
  };
  write_report(s, &root, thread, "cancelled", &said)?;
  let (s, node) = (s.clone(), record.node.clone());
  let there = node.clone();
  tokio::spawn(async move { crate::weave::nudge(&s, &node, None).await });
  Ok(CancelledAt::There(there))
}

/// Stops whatever runs on a thread the user deleted, here: its loops
/// (no report: it would go with the thread), a chat's turn (interrupted),
/// its jobs and shell sessions. Whether anything ran.
pub(crate) fn stop_deleted(s: &Arc<Shared>, thread: &RecordId) -> bool {
  let loops = take_loops(s, thread);
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned();
  let ran = !loops.is_empty() || steer.is_some();
  if let Some(st) = steer {
    st.interrupt();
  }
  stop_here(s, thread, loops, || ());
  ran
}

/// Stops a thread's loops here, with its jobs and sessions: the jobs
/// end, then `before` runs (the report is written), then the loops stop and
/// the sessions close.
fn stop_here<T>(
  s: &Arc<Shared>,
  thread: &RecordId,
  loops: Vec<tokio::task::AbortHandle>,
  before: impl FnOnce() -> T,
) -> T {
  s.shells.end_jobs(thread);
  let out = before();
  let aborted = !loops.is_empty();
  for task in loops {
    task.abort();
  }
  // An aborted task drops its loop (`Steering`, which unlists it) only
  // when a worker next takes it up: unlisted now, so what is said of the
  // thread once this returns (`threads`: ended, not working) is so.
  // Its `Steering` then finds another's steer, or none, and leaves it.
  if aborted {
    s.detached
      .steers
      .lock()
      .expect("steers lock")
      .remove(thread.as_str());
    s.detached
      .started
      .lock()
      .expect("started lock")
      .remove(thread.as_str());
  }
  let (s, thread) = (s.clone(), thread.clone());
  tokio::spawn(async move { s.shells.close_sessions(&thread).await });
  out
}

/// Stops the detached threads running here that were cancelled elsewhere:
/// a `cancelled` report written since their loop started.
pub(crate) fn stop_reported(s: &Arc<Shared>) {
  let running: Vec<(String, Vec<RecordId>)> = s
    .detached
    .running
    .lock()
    .expect("running lock")
    .iter()
    .map(|(id, loops)| {
      let known = loops.iter().flat_map(|l| l.reports.clone()).collect();
      (id.clone(), known)
    })
    .collect();
  for (id, known) in running {
    let Ok(thread) = id.parse::<RecordId>() else {
      continue;
    };
    let cancelled = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .sources(kinds::REPORT, "thread", &thread, None)
      .unwrap_or_default()
      .into_iter()
      .any(|r| {
        r.fields.get("outcome").and_then(Value::as_str) == Some("cancelled")
          && !chat::is_woven_reply(&r)
          && !known.contains(&r.id)
      });
    if cancelled {
      let loops = take_loops(s, &thread);
      if !loops.is_empty() {
        stop_here(s, &thread, loops, || ());
      }
    }
  }
}

/// What a thread woken by reports alone answers when they change nothing
/// in what it reported (design §2.7): its turn then sends no report.
const NOTHING_NEW: &str = "[nothing new]";
const NOTHING_NEW_HINT: &str = "[You reported to whoever started you already. If this \
changes nothing in what you reported, answer with just `[nothing new]`: no report \
goes out then. Otherwise your answer is your report, as before.]";

/// What a turn compacted midway goes on with in the new thread (its
/// message and steps so far are in the summary and the last messages).
const MIDWAY_NOTE: &str = "[Stitch: this thread was compacted in the middle of your turn: its \
context grew past the model's window, and it continues here, from a summary and its last \
messages. Go on with the turn from where it stopped; do not start it over.]";

/// The outcome a last reply declares in the agent's own words: its first
/// line starting `Blocked:` or `Not done:` (design §2.7).
fn declared(text: &str) -> Option<&'static str> {
  let first = text
    .trim_start()
    .lines()
    .next()
    .unwrap_or("")
    .trim_start_matches(['*', '_', '#', '>', ' '])
    .to_ascii_lowercase();
  if first.starts_with("blocked:") || first.starts_with("blocked**:") {
    Some("blocked")
  } else if first.starts_with("not done:") || first.starts_with("not done**:") {
    Some("not-done")
  } else {
    None
  }
}

/// How a loop ended, as a report's outcome and text, and whether it has
/// news (`quiet_ok`: it may say it has none, see `NOTHING_NEW`). An
/// answer is `done` unless the agent says otherwise (`declared`) or every
/// tool call it made failed (`not-done`: the work never ran).
fn outcome_of(result: &Result<ChatOutcome>, quiet_ok: bool) -> (&'static str, String, bool) {
  match result {
    Ok(o) => match &o.end {
      End::Answered => {
        if quiet_ok && o.text.trim_start().starts_with(NOTHING_NEW) {
          return ("done", o.text.clone(), false);
        }
        match declared(&o.text) {
          Some(d) => (d, o.text.clone(), true),
          None if o.calls > 0 && o.failed_calls == o.calls => (
            "not-done",
            format!(
              "Every tool call it made failed ({} of {}).\n\n{}",
              o.failed_calls, o.calls, o.text
            ),
            true,
          ),
          None => ("done", o.text.clone(), true),
        }
      }
      End::OverBudget(why) => (
        "over-budget",
        format!("Stopped: {why}.\n\n{}", o.text),
        true,
      ),
      End::Steps => (
        "stopped",
        format!("Stopped after its most steps.\n\n{}", o.text),
        true,
      ),
      End::Interrupted => (
        "cancelled",
        format!("Interrupted by the user.\n\n{}", o.text),
        true,
      ),
    },
    Err(e) => ("failed", e.to_string(), true),
  }
}

/// A loop on a detached thread ended: report, close its questions, wake
/// whoever started it.
async fn finish(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  result: Result<ChatOutcome>,
) {
  finish_to(s, root, thread, result, None, false, false).await;
}

/// A loop ended: its report goes to `to` (whoever started the thread, when
/// `None`). A detached thread's work ends with it: its jobs (none tells it
/// anything after its report) and, unless `[detached] keep_shells`, its
/// sessions and what runs in them. A turn with nothing new (`quiet_ok`,
/// `outcome_of`) writes a report that is not delivered, keeping the
/// outcome its last one had.
async fn finish_to(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  result: Result<ChatOutcome>,
  to: Option<RecordId>,
  quiet_ok: bool,
  keep: bool,
) {
  finish_said(s, root, thread, outcome_of(&result, quiet_ok), to, keep).await;
}

/// `finish_to`, with its report's outcome, words and whether they are news
/// said (`outcome_of`, or a limit's when a loop could not go on).
async fn finish_said(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  said: (&'static str, String, bool),
  to: Option<RecordId>,
  keep: bool,
) {
  let (mut outcome, text, news) = said;
  // Kept for the user, waking nothing: asked so for this message, or, for
  // the report to whoever started it, when it was spawned so.
  // A thread untrusted events reached (docs/events.md): what its model
  // wrote after them wakes no other thread.
  let keep = keep
    || (to.is_none()
      && root
        .read_record(thread)
        .is_ok_and(|r| r.field_bool("wake") == Some(false)))
    || !crate::profile::taint_of(&s.state().index.lock().expect("index lock"), thread).is_empty();
  if !news && let Some(last) = last_outcome(s, thread) {
    outcome = last;
  }
  let detached = root
    .read_record(thread)
    .is_ok_and(|r| r.field_bool("detached") == Some(true));
  if detached {
    s.shells.end_jobs(thread);
  }
  if let Err(e) = write_report_to(
    s,
    root,
    thread,
    outcome,
    &text,
    to,
    Delivery::of(news, keep),
  ) {
    eprintln!("detached {thread}: no report: {e}");
  }
  if detached && !keeps_shells(s) {
    s.shells.close_sessions(thread).await;
  }
  // All of a detached thread happened here: memory observes it.
  remember(s, root, thread, thread.timestamp(), &text);
}

/// The outcome of a thread's latest report, as one of the known ones.
fn last_outcome(s: &Shared, thread: &RecordId) -> Option<&'static str> {
  let index = s.state().index.lock().expect("index lock");
  let last = index
    .sources(kinds::REPORT, "thread", thread, None)
    .ok()?
    .into_iter()
    .rfind(|r| !chat::is_woven_reply(r))?;
  let o = last.fields.get("outcome").and_then(Value::as_str)?;
  [
    "done",
    "blocked",
    "not-done",
    "failed",
    "over-budget",
    "stopped",
    "cancelled",
  ]
  .into_iter()
  .find(|k| *k == o)
}

/// Whether a thread has written a report of its own (a reply to a woven
/// message is not one).
fn has_reported(s: &Shared, thread: &RecordId) -> bool {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "thread", thread, None)
    .is_ok_and(|r| r.iter().any(|r| !chat::is_woven_reply(r)))
}

/// A turn of a detached thread that reported starts here: a `claim` of the
/// thread, synced, so every node (and every app) reads it as working until
/// the turn's report (`chat::turn_open`), not as the outcome it reported
/// last. It names whom the turn reports to (`reports-to`, when not whoever
/// started the thread) and `wake = false` when kept, so a turn cut off by
/// the node stopping resumes at its next start toward the same one
/// (`resume_all`). Before its first report a thread reads as running
/// anyway: nothing is written.
fn mark_turn(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  report_to: Option<&RecordId>,
  keep: bool,
) -> Result<()> {
  if !has_reported(s, thread) {
    return Ok(());
  }
  let node = s.state().node.clone();
  let mut draft = Draft::new(kinds::CLAIM, Author::node(node.clone()))?
    .reference("thread", Ref::to(thread.clone()))?
    .body(format!("# a turn on {node}\n"));
  if let Some(to) = report_to {
    draft = draft.reference("reports-to", Ref::to(to.clone()))?;
  }
  if keep {
    draft = draft.field("wake", false)?;
  }
  root.publish(&node, draft)?;
  s.state().index.lock().expect("index lock").refresh(root)?;
  // Told to the other nodes now, not at the next sync.
  let (s, thread) = (s.clone(), thread.clone());
  tokio::spawn(async move { crate::weave::catch_up(&s, &thread, true).await });
  Ok(())
}

/// Memory observes a thread's messages from `since`, in the background.
fn remember(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  since: jiff::Timestamp,
  answer: &str,
) {
  crate::digests::after_turn(s, thread);
  if !s.memory.available() {
    return;
  }
  let (s, st, root, t, answer) = (
    s.clone(),
    s.state().clone(),
    root.clone(),
    thread.clone(),
    answer.to_owned(),
  );
  tokio::spawn(async move {
    let _ = s
      .memory
      .after_turn(&st, &root, &t, since, Vec::new(), &answer)
      .await;
    crate::node::sleep_memory(&s).await;
  });
}

fn write_report(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  outcome: &str,
  text: &str,
) -> Result<()> {
  write_report_to(s, root, thread, outcome, text, None, Delivery::Wake)
}

/// How a report reaches the thread it goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
  /// It wakes it (with others that came while it was idle).
  Wake,
  /// Its turn had nothing new to say: never delivered.
  NothingNew,
  /// Kept for the user (`wake = false`): shown to them, waking nothing.
  Kept,
}

impl Delivery {
  fn of(news: bool, keep: bool) -> Self {
    match (news, keep) {
      (false, _) => Delivery::NothingNew,
      (true, true) => Delivery::Kept,
      (true, false) => Delivery::Wake,
    }
  }
}

/// Writes a thread's report, to `to` (whoever started it, when `None`),
/// and has it delivered: here, or on the node that owns `to`.
fn write_report_to(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  outcome: &str,
  text: &str,
  to: Option<RecordId>,
  delivery: Delivery,
) -> Result<()> {
  write_report_as(s, root, thread, outcome, text, to, delivery, false)
}

/// `write_report_to`; `woven`: a reply to a message woven into the turn
/// (`answer_askers`), not the thread's own report, which leaves its
/// questions as they are.
#[allow(clippy::too_many_arguments)]
fn write_report_as(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  outcome: &str,
  text: &str,
  to: Option<RecordId>,
  delivery: Delivery,
  woven: bool,
) -> Result<()> {
  // A detached thread's turn ended (done, failed, cancelled): its browser
  // session ends with it, its tabs left open for the user, ungrouped.
  if let Ok(rt) = tokio::runtime::Handle::try_current() {
    let (s, key) = (s.clone(), thread.to_string());
    rt.spawn(async move { s.browsers.engine.end(&key).await });
  }
  let node = s.state().node.clone();
  let record = root.read_record(thread)?;
  let title = record.title().unwrap_or("thread").to_owned();
  let parent = to
    .map(Ref::to)
    .or_else(|| record.refs.first("reports-to").cloned());
  let mut draft = Draft::new(kinds::REPORT, Author::node(node.clone()))?
    .field("final", true)?
    .field("outcome", outcome)?
    .reference("thread", Ref::to(thread.clone()))?
    .body(format!("# {title}: {outcome}\n\n{}\n", text.trim()));
  match delivery {
    Delivery::Wake => {}
    // Written (its turn ended), not delivered: nothing new in it.
    Delivery::NothingNew => draft = draft.field("news", false)?,
    // Delivered to nobody's loop: kept for the user (design §2.7).
    Delivery::Kept => draft = draft.field("wake", false)?,
  }
  if woven {
    draft = draft.field("woven", true)?;
  }
  if let Some(p) = &parent {
    draft = draft.reference("reports-to", p.clone())?;
  }
  root.publish(&node, draft)?;
  // Questions it left open are moot now.
  let open = if woven {
    Vec::new()
  } else {
    open_questions(s, root, thread)?
  };
  for q in open {
    let d = Draft::new(kinds::ANSWER, Author::node(node.clone()))?
      .field("choice", "decline")?
      .field("withdrawn", true)?
      .reference("answers", Ref::to(q.id.clone()))?
      .body("withdrawn: the thread ended without it");
    root.publish(&node, d)?;
  }
  s.state().index.lock().expect("index lock").refresh(root)?;
  s.detached.answered.notify_waiters();
  s.detached.reported.notify_waiters();
  if let Some(parent) = parent {
    let started_on = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .record(&parent.id)?
      .map(|r| r.node);
    // The node that runs it now (it continued it last), not the one that
    // started it.
    let owner = started_on.map(|o| crate::weave::runner(s, &parent.id, &o));
    match owner {
      Some(o) if o != node.as_str() => {
        let s = s.clone();
        tokio::spawn(async move { crate::weave::nudge(&s, &o, None).await });
      }
      _ => crate::weave::deliver(s),
    }
  }
  Ok(())
}

/// Runs a turn of a thread with nobody attached: a message (`from` says
/// who it is from) or a report (`report`, delivered once). Its reply is
/// reported to `report_to` when given; else a detached thread reports to
/// whoever started it. A detached thread's turn gets what is left of its
/// budget; with nothing left it does not run, and says so in its report.
pub(crate) fn turn(
  s: Arc<Shared>,
  thread: RecordId,
  message: String,
  from: String,
  report_to: Option<RecordId>,
  reports: Vec<RecordId>,
  keep: bool,
) {
  turn_with(
    s,
    thread,
    message,
    from,
    report_to,
    reports,
    keep,
    TurnOpts::default(),
  );
}

/// A chat's turn passed on from another node, whose follower is told it
/// is over when this goes (`Live::done`), however the turn ended.
struct Served {
  s: Arc<Shared>,
  thread: String,
  assignment: RecordId,
}

impl Drop for Served {
  fn drop(&mut self) {
    self.s.live.done(&self.thread, self.assignment.as_str());
  }
}

/// How a turn nobody typed here runs (`turn_with`).
#[derive(Clone, Debug, Default)]
pub(crate) struct TurnOpts {
  /// A budget of its own (a schedule's firing, design §2.3) instead of
  /// what is left of the thread's.
  pub(crate) budget: Option<Limits>,
  /// The user typed it into the thread on another node, which follows
  /// it live (design §2.7, "Continue on another node"): the assignment
  /// that brought it. A chat's turn: its reply is kept for the user, and
  /// wakes no thread it reports to.
  pub(crate) chat: Option<RecordId>,
  /// Files the message came with (a chat's, typed on another node).
  pub(crate) attachments: Vec<stitch_records::Attachment>,
  /// The model its client picked (a chat's): used when this node runs
  /// it, else the thread's, and the follower is told so.
  pub(crate) model: Option<String>,
  /// Said in a voice call on the follower (a chat's): its reply is read
  /// aloud there (`ChatInput::spoken`).
  pub(crate) spoken: bool,
  /// Threads and efforts attached to the message (design §2.27).
  pub(crate) scope: Option<crate::scopes::Scoped>,
  /// A room's messages relayed to this participant from another node
  /// (`Part::RoomTurn`, design §2.27).
  pub(crate) room_turn: Option<stitch_records::Part>,
  /// The capability profile of the schedule that woke it
  /// (docs/events.md): the ceiling of this turn.
  pub(crate) profile: Option<crate::profile::Profile>,
}

/// Whether a loop runs on `thread` here now: on it, or on the thread it
/// was compacted into (a turn is listed under the thread it runs on,
/// `Registered::moved_to`).
pub(crate) fn is_running(s: &Shared, thread: &RecordId) -> bool {
  let now = crate::fork::current(s, thread);
  let running = s.detached.running.lock().expect("running lock");
  [thread, &now].into_iter().any(|t| {
    running
      .get(t.as_str())
      .is_some_and(|loops| loops.iter().any(|l| !l.task.is_finished()))
  })
}

/// `turn`, as `opts` says.
#[allow(clippy::too_many_arguments)]
pub(crate) fn turn_with(
  s: Arc<Shared>,
  thread: RecordId,
  message: String,
  from: String,
  report_to: Option<RecordId>,
  reports: Vec<RecordId>,
  keep: bool,
  opts: TurnOpts,
) {
  let TurnOpts {
    budget,
    chat,
    attachments,
    model: picked,
    spoken,
    scope,
    room_turn,
    profile,
  } = opts;
  // A chat's reply is the user's to read where they typed it: kept.
  let keep = keep || chat.is_some();
  // Its follower hears when it is over, however it ends.
  let served = chat.map(|a| Served {
    s: s.clone(),
    thread: thread.to_string(),
    assignment: a,
  });
  // Not while the runtime goes away, nor once the node stops its loops
  // (the node stopping drops what it ran; they resume at its next start).
  let Ok(rt) = tokio::runtime::Handle::try_current() else {
    return;
  };
  if s
    .detached
    .stopping
    .load(std::sync::atomic::Ordering::SeqCst)
  {
    eprintln!("thread {thread}: not woken: the node is stopping");
    return;
  }
  // A thread compacted into another is that one now.
  let thread = crate::fork::current(&s, &thread);
  let (serial, registered, listed) = registration(&s, &thread);
  // Followers read it as queued from now until its loop begins (or it
  // runs none): its preamble and the loop before it may take a while.
  let mut queued = Some(QueuedTurn::new(&s, &thread));
  let s2 = s.clone();
  let task = rt.spawn(async move {
    let s = s2;
    let registered = registered;
    let served = served;
    // Nothing else runs on it meanwhile: a turn running on it now is
    // waited for before it may be compacted, or two turns would run on its
    // goal at once, the compacted thread's beside the original's (issue
    // 2026-11-04T102014Z-gamer-mfnh). Locks are taken oldest first along
    // its compactions, never the other way, so they cannot deadlock.
    let lock = s.detached.locks.of(&thread);
    let _held = lock.lock_owned().await;
    // A turn before this one may have compacted it meanwhile: held too
    // (whatever runs the newest waits for it, or it for that).
    let mut _newer = Vec::new();
    let mut thread = thread;
    loop {
      let now = crate::fork::current(&s, &thread);
      if now == thread {
        break;
      }
      _newer.push(s.detached.locks.of(&now).lock_owned().await);
      thread = now;
    }
    // Listed under the thread it runs on (what `is_running` and `cancel`
    // ask of, as it is now), from here on.
    registered.moved_to(&thread);
    // Past its share of its model's window: it goes on in a new thread
    // (`fork::compact`), from a summary and its last messages.
    let own = picked
      .clone()
      .or_else(|| crate::meta::thread_model(&s.state().index.lock().expect("index lock"), &thread));
    let place = s.thread_place(&thread).ok().map(|(_, p)| p);
    let model = crate::modelset::named(&s, place.as_ref(), own.as_deref()).await;
    // An app's thread compacts only while its grant stands and its
    // family has money left today (the summary is counted to it).
    let may_compact = match crate::apps::threads::owner_of(&s, &thread) {
      Some(owner) => crate::apps::threads::may_spend(&s, &owner, place.as_ref()).await,
      None => true,
    };
    // Nothing left of its budget (its chain's, or what it was given): no
    // turn runs below, so no summary is paid for one either.
    let spent = budget
      .unwrap_or_else(|| crate::weave::remaining(&s, &thread))
      .spent()
      .is_some();
    let mut thread =
      if may_compact && !spent && crate::fork::needs_compacting(&s, &thread, &model).await {
        match crate::fork::compact(&s, &thread, &model).await {
          Ok((new, _)) => {
            // The new thread's turn from now: an ask for it is woven in, a
            // cancel of it stops this.
            registered.moved_to(&new);
            // Its turn runs under the new thread's lock too: what is sent
            // to it from now waits for this turn.
            _newer.push(s.detached.locks.of(&new).lock_owned().await);
            new
          }
          Err(e) => {
            eprintln!("thread {thread}: not compacted: {e}");
            thread
          }
        }
      } else {
        thread
      };
    let woken_by_reports = !reports.is_empty();
    let (message, reports) = if woken_by_reports {
      match take_reports(&s, &thread, message, &reports) {
        Some(taken) => taken,
        None => return,
      }
    } else {
      (message, reports)
    };
    // Continued elsewhere since this node last heard: caught up first,
    // and what nobody typed here goes to the node that runs it now.
    crate::weave::catch_up(&s, &thread, true).await;
    let started_on = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .record(&thread)
      .ok()
      .flatten()
      .map(|r| r.node);
    if let Some(started_on) = started_on {
      let runner = crate::weave::runner(&s, &thread, &started_on);
      if runner != s.state().node.as_str() && from != "stitch" {
        // A message: sent on to it, its reply still to `report_to`.
        let handed = s.thread_place(&thread).and_then(|(root, _)| {
          crate::weave::forward(
            &s,
            &root,
            &runner,
            &thread,
            &message,
            report_to.as_ref(),
            keep,
            // A chat goes on as one (its follower here hears it is done,
            // and reads the turn once synced).
            served.is_some(),
            served.is_some().then_some(crate::weave::AsChat {
              attachments: &attachments,
              model: picked.as_deref(),
              spoken,
              room_turn: room_turn.as_ref(),
            }),
            (from == crate::loose::FROM).then_some(crate::loose::FROM),
            scope.as_ref().map_or(&[][..], |sc| sc.targets.as_slice()),
          )
        });
        match handed {
          Ok(_) => eprintln!("thread {thread}: a message passed on to {runner}, which runs it"),
          Err(e) => eprintln!("thread {thread}: a message for {runner} not passed on: {e}"),
        }
        return;
      }
      if runner != s.state().node.as_str() && woken_by_reports {
        // Its reports are delivered there.
        let mut delivered = s.detached.delivered.lock().expect("delivered lock");
        for r in &reports {
          delivered.remove(r.as_str());
        }
        eprintln!("thread {thread}: its reports are for {runner}, which runs it");
        return;
      }
    }
    let result = async {
      let (root, place) = s.thread_place(&thread)?;
      // A project with no checkout here (a new chat the phone put on this
      // node): found, or cloned once the user says yes; else the turn
      // fails before its model call, saying so (never a silent start in
      // the home directory).
      let checkout = crate::checkout::ensure(&s, &place, Some(&thread), Some(&root)).await;
      let _folder_turn = crate::folders::turn(&s, &place, Some(&thread)).await;
      let mut record = root.read_record(&thread)?;
      let detached = record.field_bool("detached") == Some(true);
      let limits = if let Some(b) = budget {
        loop_limits(&s, b)
      } else if detached {
        loop_limits(&s, crate::weave::remaining(&s, &thread))
      } else {
        Limits::default()
      };
      let mut limits = if served.is_some() {
        with_chat_steps(limits, s.chat_steps())
      } else {
        limits
      };
      if detached && let Some(why) = limits.spent() {
        // Kept in the thread too, not only in the report to its spawner:
        // the thread may be the one the user talks to.
        keep_unrun(
          &s,
          &thread,
          &from,
          &message,
          &format!(
            "[Stitch: no turn ran for this: {why} (its budget as a spawned thread). It is \
             kept here, and said in this thread's report to its spawner; a message from the \
             user continues the thread.]"
          ),
        );
        // Reports from the thread it reports to (the replies to what it
        // sent there while it ran) are not said back to it: they would
        // come back to their writer as new reports, which it answers, and
        // so on (issue 2026-11-11T154611Z-beast-5vp5). Kept here only.
        let to = report_to
          .clone()
          .or_else(|| record.refs.first("reports-to").map(|r| r.id.clone()));
        let message = match &to {
          Some(to) if woken_by_reports => {
            let to = crate::fork::current(&s, to);
            let writers: Vec<Option<RecordId>> = {
              let index = s.state().index.lock().expect("index lock");
              reports
                .iter()
                .map(|r| {
                  index
                    .edges_from(r)
                    .ok()
                    .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
                    .map(|e| e.dst.id)
                })
                .collect()
            };
            let others: Vec<RecordId> = reports
              .iter()
              .zip(writers)
              .filter(|(_, w)| w.as_ref().is_none_or(|w| crate::fork::current(&s, w) != to))
              .map(|(r, _)| r.clone())
              .collect();
            if others.is_empty() {
              return Ok(None);
            }
            if others.len() == reports.len() {
              message
            } else {
              crate::weave::reports_message(&s, &thread, &others)
            }
          }
          _ => message,
        };
        let text = format!("Not run: {why}. What came for it:\n\n{message}");
        if let Err(e) = write_report_to(
          &s,
          &root,
          &thread,
          "over-budget",
          &text,
          report_to.clone(),
          Delivery::of(
            true,
            keep || (report_to.is_none() && record.field_bool("wake") == Some(false)),
          ),
        ) {
          eprintln!("detached {thread}: no report: {e}");
        }
        return Ok(None);
      }
      // It works again, as every node reads it from now (`chat::turn_open`)
      // until this turn's report.
      if detached && let Err(e) = mark_turn(&s, &root, &thread, report_to.as_ref(), keep) {
        eprintln!("thread {thread}: its turn not marked as started: {e}");
      }
      // Woken by reports alone after it reported: it may have nothing new
      // to say, and then says so instead of reporting again (design §2.7).
      let quiet_ok =
        detached && woken_by_reports && report_to.is_none() && has_reported(&s, &thread);
      let message = if quiet_ok {
        format!("{message}\n\n{NOTHING_NEW_HINT}")
      } else {
        message
      };
      let no_tools = record.field_bool("tools") == Some(false);
      // The thread's own (design §2.7, "A thread's model"): what its
      // client last picked, else what it was spawned on.
      let model = crate::meta::thread_model(&s.state().index.lock().expect("index lock"), &thread);
      let cwd = record.field_str("cwd").map(str::to_owned);
      // The model the client picked, when this node runs it (and the
      // thread's from now); else the thread's, said to whoever follows the
      // turn.
      let chosen = match picked.as_deref() {
        Some(m) => {
          let resolved = crate::modelset::named(&s, Some(&place), Some(m)).await;
          match s
            .config
            .resolve_authenticated(&s.data, Some(&resolved))
            .await
          {
            Ok(r) => {
              let full = format!("{}/{}", r.1, r.2);
              let kept = if crate::modelset::ALIASES.contains(&m) {
                m
              } else {
                &full
              };
              if let Err(e) = crate::meta::keep_model(&s, &thread, kept) {
                eprintln!("thread {thread}: its model not kept: {e}");
              }
              Some(r)
            }
            Err(e) => {
              let mine = model.clone().unwrap_or_else(|| "its default".into());
              let note = format!(
                "[{} does not run {m} ({e}): this turn runs on {mine} instead.]",
                s.state().node
              );
              eprintln!("thread {thread}: {note}");
              s.live
                .push(thread.as_str(), crate::live::LiveEvent::Note(note));
              None
            }
          }
        }
        None => None,
      };
      let resolved = match (checkout, chosen) {
        (Err(e), _) => Err(e),
        (Ok(()), Some(r)) => Ok(r),
        (Ok(()), None) => {
          let model = crate::modelset::named(&s, Some(&place), model.as_deref()).await;
          s.config.resolve_authenticated(&s.data, Some(&model)).await
        }
      };
      let state = s.state();
      let ctx = ChatContext {
        node: &state.node,
        root: &root,
        index: &state.index,
        blobs: &state.blobs,
      };
      let since = jiff::Timestamp::now();
      // Askers of a try compacted midway: answered with the turn.
      let mut earlier_askers = Vec::new();
      // Refused as too long midway with nothing left of its limits: the
      // report that limit makes (`left_after`).
      let mut limit_said = None;
      // A room's participant woken by what the room did not say (a
      // report, a schedule, the node): its reply is written into the room
      // too (`rooms::post_reply`). Not a room's turn (the room writes that),
      // nor what the user or a thread said to it alone, which it answers
      // there. Where its turn starts: the thread and its messages so far.
      let to_room = (room_turn.is_none()
        && served.is_none()
        && from != "user"
        && !from.starts_with("thread ")
        && crate::rooms::room_of(&s, &thread).is_some())
      .then(|| {
        let n = chat::thread_messages(&ctx, &thread).map_or(0, |m| m.len());
        (thread.clone(), n)
      });
      let (result, steering) = match resolved {
        Ok((provider, provider_name, model)) => {
          let full_model = format!("{provider_name}/{model}");
          // The turn's message, its sender and what came with it; after a
          // compaction midway, the note it goes on with (`MIDWAY_NOTE`).
          let mut sent = (message, from, attachments.clone(), spoken, scope.clone());
          let mut compacted = false;
          loop {
            let start = s.start_dir(&place, cwd.as_deref().map(Path::new), Some(&thread));
            let mut selection = s.selection(&place, start.as_deref(), Some(&thread));
            selection.profiles.extend(profile.clone());
            let context = s.context(&place, start.as_deref(), &selection, Some(&thread));
            let read_only = s.read_only(&thread);
            // A room's turn relayed here by the node that runs the room: a
            // read-only participant asks the user (in the room) what the
            // mode refuses, as one whose room runs here does (design §2.27).
            let room_ask = room_turn
              .as_ref()
              .filter(|_| read_only)
              .and_then(|p| crate::rooms::relayed_ask(&state.index.lock().expect("index lock"), p));
            let tools = tools_for_room(
              &s,
              &root,
              &place,
              read_only,
              &thread,
              start.as_deref(),
              no_tools,
              &selection,
              &full_model,
              room_ask,
              // The user's own typed chat asks them for more steps. Not a
              // room turn, which a thread's `send` can start too: locally
              // it does not ask either (rooms.rs `turn_here`).
              served.is_some() && room_turn.is_none(),
            );
            let steer = chat::Steer::open();
            let steering = Steering::new(&s, &thread, steer.clone());
            // Running from here (the first try): no longer queued.
            if let Some(q) = queued.take() {
              drop(q);
              if let Some(sv) = &served {
                s.live.serve(&sv.thread, sv.assignment.as_str());
              }
            }
            let (text, from, attachments, spoken, scope) = sent.clone();
            let input = ChatInput {
              id: None,
              thread: Some(thread.clone()),
              text,
              attachments,
              provider: provider.clone(),
              model: model.clone(),
              system: None,
              from: Some(from),
              detached: None,
              limits,
              recalled: None,
              instruction: None,
              context: Some(context),
              folder: None,
              steer: Some(steer),
              spoken,
              scope,
              room_turn: room_turn.clone(),
            };
            let key = thread.to_string();
            let head = chat::line_head(&ctx, &thread).ok().flatten();
            let result = chat::send(&ctx, input, &*tools, &mut |e| s.live.event(&key, &e)).await;
            drop(tools);
            // Too long for the model after all, the turn grown past its
            // window midway (its start's check saw it under the share):
            // compacted, and tried again once in the new thread, as a
            // chat is (node.rs). Unwritten, the failed step is not in the
            // thread (`chat::failed`).
            match result {
              Err(e)
                if !compacted
                  && may_compact
                  && !matches!(e, Error::Kept { .. })
                  && crate::fork::too_long(&e) =>
              {
                compacted = true;
                // It goes on under what is left of the turn's limits;
                // with nothing left, it reports the limit it reached.
                match left_after(&s, &thread, since, limits) {
                  Ok(left) => limits = left,
                  Err((outcome, text)) => {
                    limit_said = Some((outcome, text));
                    break (Err(e), Some(steering));
                  }
                }
                // Steps of it written (its message with the first), read
                // before the compaction writes its summary's spend there.
                let wrote = chat::line_head(&ctx, &thread).ok().flatten() != head;
                // Compacted only when what is left pays for its summary;
                // what the summary spent is taken off what is left.
                let (new, spent_on) = match compact_midway(&s, &thread, &full_model, limits).await {
                  Compacted::Into(new, left) => {
                    limits = left;
                    (new, None)
                  }
                  Compacted::SpentOn(new, outcome, text) => (new, Some((outcome, text))),
                  Compacted::Unaffordable(outcome, text) => {
                    limit_said = Some((outcome, text));
                    break (Err(e), Some(steering));
                  }
                  Compacted::Failed => break (Err(e), Some(steering)),
                };
                eprintln!("thread {thread}: too long for {full_model} midway: goes on in {new}");
                earlier_askers.extend(steering.steer.askers());
                drop(steering);
                registered.moved_to(&new);
                _newer.push(s.detached.locks.of(&new).lock_owned().await);
                // Steps of it written (its message with the first): the
                // new thread has them in its summary and last messages,
                // and goes on from them; else the message is sent there.
                if wrote {
                  sent = (
                    MIDWAY_NOTE.to_owned(),
                    "stitch".to_owned(),
                    Vec::new(),
                    false,
                    None,
                  );
                }
                thread = new;
                if let Ok(r) = root.read_record(&thread) {
                  record = r;
                }
                // Its summary took what was left: no step on it, the
                // limit's report its.
                if let Some(said) = spent_on {
                  limit_said = Some(said);
                  break (Err(e), None);
                }
              }
              result => break (result, Some(steering)),
            }
          }
        }
        // Failed before its model call (no provider here: an OAuth
        // refresh refused, a key missing): written with why, as a failed
        // model call is, and ended (reported) as one; its follower, the
        // assignment done (`Served`), reads it there.
        Err(e) => {
          eprintln!("thread {thread}: its turn failed before its model call: {e}");
          let e = chat::failed_before_call(&ctx, &thread, &from, &message, &attachments, e);
          (Err(e), None)
        }
      };
      if let (Some((began, n)), Ok(o)) = (&to_room, &result)
        && o.end != End::Interrupted
      {
        let msgs = chat::thread_messages(&ctx, &o.thread).unwrap_or_default();
        // Compacted midway: its new thread, from the last message it was
        // sent (or the note it went on with).
        let start = if o.thread == *began {
          (*n).min(msgs.len())
        } else {
          msgs
            .iter()
            .rposition(|m| {
              m.role == stitch_records::Role::User && m.from.as_deref() != Some("tool")
            })
            .map_or(0, |i| i + 1)
        };
        crate::rooms::post_reply(&s, &o.thread, &msgs[start..]).await;
      }
      // Those whose messages it took hear its reply, but whoever its own
      // report goes to.
      let reports = report_to.is_some() || detached;
      let main = report_to
        .clone()
        .or_else(|| record.refs.first("reports-to").map(|r| r.id.clone()));
      let mut askers = earlier_askers;
      askers.extend(
        steering
          .as_ref()
          .map(|st| st.steer.askers())
          .unwrap_or_default(),
      );
      // One it reports to asked something meanwhile: its reply is news.
      let quiet_ok = quiet_ok
        && !askers
          .iter()
          .any(|a| a.thread.is_some() && a.thread == main);
      answer_askers(
        &s,
        &root,
        &thread,
        &result,
        askers,
        main.as_ref().filter(|_| reports),
      );
      // Its report first: what it leaves untaken then wakes it only as
      // `wakes` says.
      let ended = if reports {
        let said = match limit_said {
          Some((outcome, text)) => (outcome, text, true),
          None => outcome_of(&result, quiet_ok),
        };
        finish_said(&s, &root, &thread, said, report_to.clone(), keep).await;
        None
      } else {
        Some(result)
      };
      drop(steering);
      Ok::<_, Error>(ended.map(|result| (root, result, since)))
    }
    .await;
    match result {
      Ok(None) => {}
      Ok(Some((root, Ok(o), since))) => remember(&s, &root, &thread, since, &o.text),
      Ok(Some((_, Err(e), _))) | Err(e) => eprintln!("thread {thread}: {e}"),
    }
  });
  register(&s, &listed, serial, task.abort_handle());
}

/// A turn of a thread that had reported, started here and cut off before
/// its own report (the node stopped): its `claim` (`mark_turn`).
struct Reopened {
  at: jiff::Timestamp,
  to: Option<RecordId>,
  keep: bool,
}

/// The turn this node started on a thread since its last report of its
/// own, which nothing reported yet.
fn reopened(
  index: &stitch_store::Index,
  me: &str,
  thread: &RecordId,
  reports: &[stitch_store::IndexedRecord],
) -> Option<Reopened> {
  let own = reports
    .iter()
    .filter(|r| !chat::is_woven_reply(r))
    .map(|r| r.created)
    .max()?;
  let c = index
    .sources(kinds::CLAIM, "thread", thread, None)
    .ok()?
    .into_iter()
    .rfind(|c| c.node == me && c.created > own)?;
  let to = index
    .edges_from(&c.id)
    .ok()
    .and_then(|e| e.into_iter().find(|e| e.rel == "reports-to"))
    .map(|e| e.dst.id);
  Some(Reopened {
    at: c.created,
    to,
    keep: c.fields.get("wake").and_then(Value::as_bool) == Some(false),
  })
}

/// Resumes the detached threads this node started and that have no report,
/// and the turns this node started on threads that had reported and that
/// it stopped before their report (`Reopened`): every thread read as
/// working is at work, or comes to report (`chat::turn_open`).
pub(crate) async fn resume_all(s: &Arc<Shared>) {
  let state = s.state();
  let mut pending = Vec::new();
  {
    let index = state.index.lock().expect("index lock");
    let Ok(threads) = index.records(Some(kinds::THREAD)) else {
      return;
    };
    for t in threads {
      let detached = t.fields.get("detached").and_then(Value::as_bool) == Some(true);
      let ours = t.node == state.node.as_str();
      let Ok(reports) = index.sources(kinds::REPORT, "thread", &t.id, None) else {
        continue;
      };
      let reported = reports.iter().any(|r| !chat::is_woven_reply(r));
      // Compacted into another thread: that one resumes, not this.
      let compacted = index
        .sources(kinds::THREAD, "continues", &t.id, None)
        .map(|c| {
          c.iter()
            .any(|c| c.fields.get("compacted").and_then(Value::as_bool) == Some(true))
        })
        .unwrap_or(false);
      if !detached || compacted {
        continue;
      }
      if ours && !reported {
        pending.push((t, None));
      } else if let Some(r) = reopened(&index, state.node.as_str(), &t.id, &reports) {
        pending.push((t, Some(r)));
      }
    }
  }
  for (t, reopened) in pending {
    let id = t.id.clone();
    let (serial, registered, listed) = registration(s, &id);
    let detachments = s.clone();
    // Its loop takes notes from now, before its task first runs: what is
    // told at start (`tell_cut_shells`, right after this returns) is
    // woven into the step it replays, not a turn of its own that writes a
    // message first, so the step kept is moot and its call is never
    // replayed (issue svf3). Notes it does not take wake it after.
    let steer = chat::Steer::open();
    let steering = Steering::new(s, &id, steer.clone());
    let s = s.clone();
    let task = tokio::spawn(async move {
      let _registered = registered;
      let lock = s.detached.locks.of(&t.id);
      let _held = lock.lock().await;
      // A node just started has not synced yet: what was written elsewhere
      // meanwhile first, so it does not continue from an older step.
      crate::weave::catch_up_at_start(&s, &t.id).await;
      let Ok((root, place)) = s.thread_place(&t.id) else {
        return;
      };
      let _folder_turn = crate::folders::turn(&s, &place, Some(&t.id)).await;
      // Its report goes to whoever started it (or the turn's sender);
      // those whose messages it took hear its reply too.
      let to = reopened.as_ref().and_then(|r| r.to.clone());
      let keep = reopened.as_ref().is_some_and(|r| r.keep);
      let parent = to.clone().or_else(|| {
        root
          .read_record(&t.id)
          .ok()
          .and_then(|r| r.refs.first("reports-to").map(|r| r.id.clone()))
      });
      let answer = |result: &Result<ChatOutcome>| {
        answer_askers(
          &s,
          &root,
          &t.id,
          result,
          steering.steer.askers(),
          parent.as_ref(),
        )
      };
      let model = crate::meta::thread_model(&s.state().index.lock().expect("index lock"), &t.id);
      let cwd = t
        .fields
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_owned);
      let model = crate::modelset::named(&s, Some(&place), model.as_deref()).await;
      let resolved = s.config.resolve_authenticated(&s.data, Some(&model)).await;
      // What it may outgrow midway (`goes_on_compacted`).
      let outgrows = model;
      // What is left: it spent some before the node stopped.
      let limits = loop_limits(&s, crate::weave::remaining(&s, &t.id));
      // What its steps spend from here (`left_after`).
      let since = jiff::Timestamp::now();
      // Refused as too long midway: it goes on in a compacted thread, or
      // reports the limit it reached, else what came.
      let ended = |result: Result<ChatOutcome>, to: Option<RecordId>| {
        let (s, root, t, outgrows, answer) = (&s, &root, &t, &outgrows, &answer);
        async move {
          let midway = goes_on_compacted(
            s,
            &t.id,
            &result,
            outgrows,
            since,
            limits,
            to.clone(),
            keep,
            None,
          )
          .await;
          if matches!(midway, Midway::GoesOn) {
            return;
          }
          answer(&result);
          match midway {
            Midway::Spent(outcome, text) => {
              finish_said(s, root, &t.id, (outcome, text, true), to, keep).await
            }
            _ => finish_to(s, root, &t.id, result, to, false, keep).await,
          }
        }
      };
      let result = match resolved {
        Err(e) => Err(e),
        Ok((provider, provider_name, model)) => {
          let start = s.start_dir(&place, cwd.as_deref().map(Path::new), Some(&t.id));
          let selection = s.selection(&place, start.as_deref(), Some(&t.id));
          let context = s.context(&place, start.as_deref(), &selection, Some(&t.id));
          let no_tools = t.fields.get("tools").and_then(Value::as_bool) == Some(false);
          let tools = tools_for(
            &s,
            &root,
            &place,
            s.read_only(&t.id),
            &t.id,
            start.as_deref(),
            no_tools,
            &selection,
            &format!("{provider_name}/{model}"),
            false,
          );
          let state = s.state();
          let ctx = ChatContext {
            node: &state.node,
            root: &root,
            index: &state.index,
            blobs: &state.blobs,
          };
          let started = !state
            .index
            .lock()
            .expect("index lock")
            .thread_segments(&t.id)
            .map(|s| s.is_empty())
            .unwrap_or(true);
          // Its first step not written but kept (its calls were running):
          // that step is replayed as any other.
          let pending = chat::pending_step(&ctx, &t.id).is_some_and(|p| p.prev.is_none());
          if !started && !pending {
            // It stopped before its first step was written, and nothing of
            // it was kept (an older version's loop): it starts over from
            // its goal, and asking for the same command finds the same
            // question.
            let goal = t
              .fields
              .get("goal")
              .and_then(Value::as_str)
              .unwrap_or("")
              .to_owned();
            let input = ChatInput {
              id: None,
              thread: Some(t.id.clone()),
              text: goal,
              attachments: Vec::new(),
              provider,
              model,
              system: None,
              from: None,
              detached: None,
              limits,
              recalled: None,
              instruction: None,
              context: Some(context),
              folder: None,
              steer: Some(steer),
              spoken: false,
              // Started over from its goal: what was attached to it is
              // not kept anywhere to give again.
              scope: None,
              room_turn: None,
            };
            let key = t.id.to_string();
            let result = chat::send(&ctx, input, &*tools, &mut |e| s.live.event(&key, &e)).await;
            ended(result, to).await;
            return;
          }
          let key = t.id.to_string();
          let mut quiet = |e: ChatEvent| s.live.event(&key, &e);
          let resumed = chat::resume(
            &ctx,
            &t.id,
            &provider,
            &model,
            limits,
            &*tools,
            Some(steer),
            &mut quiet,
          );
          match resumed.await {
            Ok(Some(o)) => Ok(o),
            // It had answered; only its report was missing.
            Ok(None) => {
              let last = chat::transcript(&ctx, &t.id)
                .ok()
                .and_then(|entries| {
                  entries.into_iter().rev().find(|(who, _)| {
                    !matches!(
                      who.as_str(),
                      "you" | "tool" | "output" | "error" | "refusal" | "failed" | "note"
                    )
                  })
                })
                .map(|(_, text)| text)
                .unwrap_or_default();
              // A turn cut off before its message was written: nothing of
              // it ran, and its last answer is an older turn's.
              let lost = reopened.as_ref().is_some_and(|r| {
                chat::last_step_at(&ctx, &t.id)
                  .ok()
                  .flatten()
                  .is_none_or(|at| at < r.at)
              });
              let (outcome, text) = if lost {
                (
                  "stopped",
                  format!(
                    "Its node ({}) stopped before this turn's first step was written: what was \
                     sent to it did not run. Send it again.",
                    state.node
                  ),
                )
              } else {
                ("done", last)
              };
              finish_with(&s, &root, &t.id, outcome, &text, to, keep).await;
              return;
            }
            Err(e) => Err(e),
          }
        }
      };
      ended(result, to).await;
    });
    register(&detachments, &listed, serial, task.abort_handle());
  }
}

/// A detached thread's report, when its loop had nothing left to do; what
/// it left running ends, as in `finish_to`.
async fn finish_with(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  outcome: &str,
  text: &str,
  to: Option<RecordId>,
  keep: bool,
) {
  s.shells.end_jobs(thread);
  let delivery = Delivery::of(true, keep);
  if let Err(e) = write_report_to(s, root, thread, outcome, text, to, delivery) {
    eprintln!("detached {thread}: no report: {e}");
  }
  if !keeps_shells(s) {
    s.shells.close_sessions(thread).await;
  }
}

/// A loop's tools: the node's, or none (an asked model answering); `model`
/// (`provider/model`) is the one the loop runs on, which they name.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tools_for(
  s: &Arc<Shared>,
  root: &RecordRoot,
  place: &crate::place::Place,
  read_only: bool,
  thread: &RecordId,
  start: Option<&Path>,
  none: bool,
  selection: &crate::toolset::Selection,
  model: &str,
  asks_more_steps: bool,
) -> Box<dyn crate::tools::ToolRunner> {
  tools_for_room(
    s,
    root,
    place,
    read_only,
    thread,
    start,
    none,
    selection,
    model,
    None,
    asks_more_steps,
  )
}

/// `tools_for`, for a room participant in its room's turn: read-only, it
/// asks the user what the mode refuses (`room_ask`, design §2.27).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tools_for_room(
  s: &Arc<Shared>,
  root: &RecordRoot,
  place: &crate::place::Place,
  read_only: bool,
  thread: &RecordId,
  start: Option<&Path>,
  none: bool,
  selection: &crate::toolset::Selection,
  model: &str,
  room_ask: Option<crate::rules::RoomAsk>,
  asks_more_steps: bool,
) -> Box<dyn crate::tools::ToolRunner> {
  if none {
    Box::new(crate::tools::NoTools)
  } else {
    let mut tools = s.shell_tools_with(
      root,
      place,
      read_only,
      Some(thread),
      start,
      headless(s.clone(), root.clone()),
      None,
      selection,
      room_ask,
    );
    tools.model = model.to_owned();
    // A typed chat: at its step limit it asks the user for more.
    tools.asks_more_steps = asks_more_steps;
    Box::new(tools)
  }
}

/// A thread's model calls so far (its usage events), with what they cost
/// when known.
pub(crate) fn steps_of(s: &Arc<Shared>, thread: &RecordId) -> Option<(usize, Option<f64>)> {
  let (root, _) = s.thread_place(thread).ok()?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let mut steps = 0;
  let mut usd: Option<f64> = None;
  for e in chat::thread_events(&ctx, thread).ok()? {
    if let stitch_records::Event::Usage(u) = e {
      steps += 1;
      if let Some(d) = u.usd {
        *usd.get_or_insert(0.0) += d;
      }
    }
  }
  Some((steps, usd))
}

/// The compaction chain `thread` is in, oldest first (itself alone when
/// never compacted): one conversation, one budget (design §2.15).
pub(crate) fn chain_of(s: &Shared, thread: &RecordId) -> Vec<RecordId> {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .compaction_lineage(thread)
}

/// What `thread`'s whole compaction chain spent (since `since`): a
/// thread's total budget and a schedule's caps hold across its
/// compactions, whatever made them (issue 2026-11-19T193029Z-beast-ijcq).
pub(crate) fn spent_by_chain(
  s: &Arc<Shared>,
  thread: &RecordId,
  since: Option<jiff::Timestamp>,
) -> Option<(stitch_providers::Usage, Option<f64>)> {
  spent_by_chains(s, std::slice::from_ref(thread), since)
}

/// What the compaction chains of `threads` spent (since `since`), each
/// thread of them counted once however many of `threads` share a chain.
/// Each reads only its own steps (what a compaction takes of the one it
/// continues is read there, not copied), so nothing is counted twice.
/// `None` when none of them is here.
pub(crate) fn spent_by_chains(
  s: &Arc<Shared>,
  threads: &[RecordId],
  since: Option<jiff::Timestamp>,
) -> Option<(stitch_providers::Usage, Option<f64>)> {
  let mut seen = std::collections::HashSet::new();
  let mut total: Option<(stitch_providers::Usage, Option<f64>)> = None;
  for t in threads {
    for m in chain_of(s, t) {
      if !seen.insert(m.to_string()) {
        continue;
      }
      let Some((u, d)) = spent_by(s, &m, since) else {
        continue;
      };
      let (usage, usd) = total.get_or_insert_with(Default::default);
      usage.input_tokens += u.input_tokens;
      usage.output_tokens += u.output_tokens;
      usage.cache_read_tokens += u.cache_read_tokens;
      usage.cache_write_tokens += u.cache_write_tokens;
      if let Some(d) = d {
        *usd.get_or_insert(0.0) += d;
      }
    }
  }
  total
}

/// What a thread spent, from its usage events (since `since`): its own,
/// not its compaction chain's (`spent_by_chain`).
pub(crate) fn spent_by(
  s: &Arc<Shared>,
  thread: &RecordId,
  since: Option<jiff::Timestamp>,
) -> Option<(stitch_providers::Usage, Option<f64>)> {
  let (root, _) = s.thread_place(thread).ok()?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let mut usage = stitch_providers::Usage::default();
  let mut usd: Option<f64> = None;
  for e in chat::thread_events(&ctx, thread).ok()? {
    if let stitch_records::Event::Usage(u) = e
      && since.is_none_or(|t| u.at >= t)
    {
      usage.input_tokens += u.input_tokens;
      usage.output_tokens += u.output_tokens;
      usage.cache_read_tokens += u.cache_read_tokens.unwrap_or(0);
      usage.cache_write_tokens += u.cache_write_tokens.unwrap_or(0);
      if let Some(d) = u.usd {
        *usd.get_or_insert(0.0) += d;
      }
    }
  }
  Some((usage, usd))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn questions_from_before_the_rename_still_grant_otter_tools() {
    let fields = |t: &str| t.parse::<toml::Table>().unwrap();
    assert!(grants_otter(&fields("grant_otter = true")));
    assert!(grants_otter(&fields("grant_shards = true")));
    assert!(!grants_otter(&fields("grant_server = \"mail\"")));
    assert!(!grants_otter(&fields("grant_otter = false")));
  }

  /// A wake that carries a report another wake had already (handed again
  /// while the first waited for the thread's lock) runs for the new ones
  /// only, and says only them; one with nothing new runs nothing (issue
  /// 2026-11-11T154611Z-beast-5vp5: the same reports said again and again).
  #[tokio::test]
  async fn a_wake_says_only_the_reports_not_had_already() {
    use stitch_records::{Author, Draft, Ref};
    let dir = tempfile::tempdir().unwrap();
    let (data, home) = (dir.path().join("data"), dir.path().join("home"));
    crate::config::Config {
      node: Some("mbp".into()),
      discovery: Some("local".into()),
      sync_interval_secs: Some(0),
      // A test node never takes the fixed port 7177.
      apps: Some(crate::config::AppsSettings { port: Some(0) }),
      ..Default::default()
    }
    .save(&data)
    .unwrap();
    let root = RecordRoot::init(&home, "home".parse().unwrap(), stitch_store::Role::Home).unwrap();
    stitch_store::Index::open(&data.join("index.sqlite"))
      .unwrap()
      .refresh(&root)
      .unwrap();
    let node = crate::start(&data).await.unwrap();
    let s = node.shared().clone();
    let me = s.state().node.clone();
    let publish = |d: Draft| {
      let id = root.publish(&me, d).unwrap().id;
      s.state().index.lock().unwrap().refresh(&root).unwrap();
      id
    };
    let thread = |title: &str| {
      publish(
        Draft::new(kinds::THREAD, Author::user(me.clone()))
          .unwrap()
          .field("title", title)
          .unwrap(),
      )
    };
    let (parent, child) = (thread("Parent"), thread("Child"));
    let report = |words: &str| {
      publish(
        Draft::new(kinds::REPORT, Author::node(me.clone()))
          .unwrap()
          .field("outcome", "done")
          .unwrap()
          .reference("thread", Ref::to(child.clone()))
          .unwrap()
          .reference("reports-to", Ref::to(parent.clone()))
          .unwrap()
          .body(format!("# Child: done\n\n{words}\n")),
      )
    };
    let (old, new) = (report("OLD-WORDS"), report("NEW-WORDS"));
    let both = crate::weave::reports_message(&s, &parent, &[old.clone(), new.clone()]);
    assert!(
      both.contains("OLD-WORDS") && both.contains("NEW-WORDS"),
      "{both}"
    );
    // The first wake had the old one.
    let (said, taken) =
      take_reports(&s, &parent, "OLD".into(), std::slice::from_ref(&old)).unwrap();
    assert_eq!((said.as_str(), taken), ("OLD", vec![old.clone()]));
    // The second, handed both meanwhile, says the new one only.
    let (said, taken) = take_reports(&s, &parent, both, &[old.clone(), new.clone()]).unwrap();
    assert_eq!(taken, vec![new.clone()]);
    assert!(said.contains(&format!("[Report {new}")), "{said}");
    assert!(said.contains("NEW-WORDS"), "{said}");
    assert!(!said.contains("OLD-WORDS"), "said again: {said}");
    // A third with nothing new runs nothing.
    assert!(take_reports(&s, &parent, "again".into(), &[old, new]).is_none());
    node.stop().await.unwrap();
  }

  /// A loop stopped here is unlisted as `stop_here` returns, not when its
  /// aborted task is next taken up (later, under load): else `threads`
  /// right after `cancel` said it working (issue
  /// 2026-11-12T220548Z-gamer-uc98). On this one-thread runtime the task
  /// is not taken up before the next await, so the order is fixed.
  #[tokio::test]
  async fn a_stopped_loop_is_unlisted_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    crate::config::Config {
      node: Some("mbp".into()),
      discovery: Some("local".into()),
      sync_interval_secs: Some(0),
      apps: Some(crate::config::AppsSettings { port: Some(0) }),
      ..Default::default()
    }
    .save(&data)
    .unwrap();
    let node = crate::start(&data).await.unwrap();
    let s = node.shared().clone();
    let thread: RecordId = "2026-11-17T180000Z-mbp-uc98".parse().unwrap();
    let (s2, t2) = (s.clone(), thread.clone());
    let task = tokio::spawn(async move {
      let _steering = Steering::new(&s2, &t2, chat::Steer::default());
      std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    assert!(running_steer(&s, &thread).is_some(), "its loop runs");
    stop_here(&s, &thread, vec![task.abort_handle()], || ());
    assert!(running_steer(&s, &thread).is_none(), "still listed");
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(running_steer(&s, &thread).is_none());
    node.stop().await.unwrap();
  }
}

#[cfg(test)]
mod chat_steps_tests {
  use super::*;

  #[test]
  fn a_typed_chat_takes_the_chat_steps_and_keeps_the_threads_budget() {
    let budget = Limits {
      usd: Some(1.5),
      tokens: Some(1000),
      steps: Some(7),
      ..Default::default()
    };
    let l = with_chat_steps(budget, 200);
    assert_eq!(l.steps, Some(200));
    assert_eq!((l.usd, l.tokens), (Some(1.5), Some(1000)));
  }
}
