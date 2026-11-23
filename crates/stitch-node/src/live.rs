//! The live feed of the turns running on this node (design §2.6, the
//! first piece of the live channel; §2.7 "Continue on another node"): what
//! each loop streams (text, thinking, tool calls and results, notes), kept
//! in memory for a while, by thread, numbered, for another node to follow
//! (`Question::Follow`, a long poll over the question stream). Nothing of
//! it is stored: a turn's steps are still sealed into its segment when it
//! ends, and synced.
//!
//! Messages typed on another node for a thread that runs here (a chat
//! passed on as an `assignment` with `chat = true`) are followed by their
//! assignment: `serve` ties one to the turn that takes it, and that turn's
//! end marks it done, so the follower knows when its turn is over.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::api::Compaction;
use crate::chat::{ChatEvent, call_summary};

/// What a turn streamed, as a follower is told it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LiveEvent {
  /// A loop began on the thread.
  Started,
  Text(String),
  Thinking(String),
  /// A tool call, as one line (`$ command`).
  ToolCall(String),
  ToolResult {
    ok: bool,
    output: String,
  },
  Note(String),
  /// The model call was cut off and is sent again: what it streamed since
  /// the last step is dropped.
  Retry(String),
  /// The loop ended.
  Ended,
  /// The thread was compacted (design §2.15): it continues in `to` from
  /// now. Pushed on both threads' feeds, so a follower of the old one
  /// hears it and follows the new one (`to`'s feed from its start).
  Compacted(Compaction),
}

impl LiveEvent {
  /// A chat's event as followers see it; `None` for what only the local
  /// client needs (the thread's ID, its own messages woven, apps).
  pub(crate) fn of(e: &ChatEvent) -> Option<LiveEvent> {
    Some(match e {
      ChatEvent::Text(t) => LiveEvent::Text(t.clone()),
      ChatEvent::Thinking(t) => LiveEvent::Thinking(t.clone()),
      ChatEvent::ToolCall { name, input } => LiveEvent::ToolCall(call_summary(name, input)),
      ChatEvent::ToolResult { ok, output, .. } => LiveEvent::ToolResult {
        ok: *ok,
        output: output.clone(),
      },
      ChatEvent::Note(n) => LiveEvent::Note(n.clone()),
      ChatEvent::Retry(r) => LiveEvent::Retry(r.clone()),
      ChatEvent::Thread(_) | ChatEvent::Woven(_) | ChatEvent::App { .. } => return None,
    })
  }
}

/// The answer to a follower (`Live::follow`).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Followed {
  /// The number to ask from next.
  pub next: u64,
  /// What came from the number asked (consecutive text joined).
  pub events: Vec<LiveEvent>,
  /// Some of what was asked for is no longer kept (a follower too slow,
  /// or the node started since): the transcript has it once synced.
  pub gap: bool,
  /// The assignment followed was taken here (woven into a loop, or a
  /// turn of its own started).
  pub taken: bool,
  /// The turn that took it ended (or it ran no turn: passed on, refused).
  pub done: bool,
  /// A loop runs on the thread here now.
  pub running: bool,
}

/// Events kept per thread; older ones are dropped (a follower that far
/// behind reads the transcript once synced).
const KEPT: usize = 4000;
/// Assignments remembered as done, per thread.
const DONE_KEPT: usize = 256;
/// A thread's feed with no loop is dropped after this long.
const IDLE: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct Feed {
  /// The number of `events[0]`.
  first: u64,
  events: VecDeque<LiveEvent>,
  /// Loops running on the thread (begun and not ended).
  loops: usize,
  /// Turns accepted for the thread whose loop has not begun yet (its
  /// preamble runs, or it waits for the loop before it): `Live::queue`.
  queued: usize,
  /// The assignments the running loop took.
  serving: Vec<String>,
  /// Those whose turn ended.
  done: VecDeque<String>,
  last: Option<Instant>,
}

impl Feed {
  fn next(&self) -> u64 {
    self.first + self.events.len() as u64
  }

  fn push(&mut self, e: LiveEvent) {
    self.events.push_back(e);
    while self.events.len() > KEPT {
      self.events.pop_front();
      self.first += 1;
    }
    self.last = Some(Instant::now());
  }

  fn finish(&mut self, assignment: String) {
    if !self.done.contains(&assignment) {
      self.done.push_back(assignment);
    }
    while self.done.len() > DONE_KEPT {
      self.done.pop_front();
    }
  }
}

/// The feeds of this node's threads.
#[derive(Default)]
pub(crate) struct Live {
  feeds: Mutex<HashMap<String, Feed>>,
  changed: Notify,
}

impl Live {
  /// A chat event of a loop on `thread`.
  pub(crate) fn event(&self, thread: &str, e: &ChatEvent) {
    if let Some(e) = LiveEvent::of(e) {
      self.push(thread, e);
    }
  }

  pub(crate) fn push(&self, thread: &str, e: LiveEvent) {
    {
      let mut feeds = self.feeds.lock().expect("live lock");
      // Feeds of threads long idle go (they are only for following).
      if feeds.len() > 64 {
        feeds
          .retain(|_, f| f.loops > 0 || f.queued > 0 || f.last.is_none_or(|l| l.elapsed() < IDLE));
      }
      let f = feeds.entry(thread.to_owned()).or_default();
      match &e {
        LiveEvent::Started => f.loops += 1,
        LiveEvent::Ended => {
          f.loops = f.loops.saturating_sub(1);
          if f.loops == 0 {
            for a in std::mem::take(&mut f.serving) {
              f.finish(a);
            }
          }
        }
        _ => {}
      }
      f.push(e);
    }
    self.changed.notify_waiters();
  }

  /// A loop began on `thread` (`Steering`).
  pub(crate) fn begin(&self, thread: &str) {
    self.push(thread, LiveEvent::Started);
  }

  /// The loop on `thread` ended.
  pub(crate) fn end(&self, thread: &str) {
    self.push(thread, LiveEvent::Ended);
  }

  /// A turn was accepted for `thread` (`detached::turn_with`): until the
  /// guard `unqueue`s it (its loop began, or it ran none), followers read it as
  /// queued (`status`), so a client shows at once that something comes,
  /// before the turn's preamble (catching up with peers, its model, its
  /// tools) or the loop running before it is over.
  /// `unqueue` ends it.
  pub(crate) fn queue(&self, thread: &str) {
    {
      let mut feeds = self.feeds.lock().expect("live lock");
      let f = feeds.entry(thread.to_owned()).or_default();
      f.queued += 1;
      f.last = Some(Instant::now());
    }
    self.changed.notify_waiters();
  }

  /// A turn `queue` counted began its loop, or runs none.
  pub(crate) fn unqueue(&self, thread: &str) {
    if let Some(f) = self.feeds.lock().expect("live lock").get_mut(thread) {
      f.queued = f.queued.saturating_sub(1);
    }
    self.changed.notify_waiters();
  }

  /// Where `thread`'s turns stand here: a loop running, and turns
  /// accepted that have not begun.
  pub(crate) fn status(&self, thread: &str) -> TurnStatus {
    let feeds = self.feeds.lock().expect("live lock");
    feeds
      .get(thread)
      .map_or_else(TurnStatus::default, |f| TurnStatus {
        running: f.loops > 0,
        queued: f.queued,
      })
  }

  /// Waits until `thread`'s status is other than `seen`, at most `wait`;
  /// the status then. Only the web client's socket (apps::ws, desktop) waits.
  #[cfg(not(mobile))]
  pub(crate) async fn status_change(
    &self,
    thread: &str,
    seen: TurnStatus,
    wait: Duration,
  ) -> TurnStatus {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
      let changed = self.changed.notified();
      tokio::pin!(changed);
      changed.as_mut().enable();
      let now = self.status(thread);
      if now != seen {
        return now;
      }
      tokio::select! {
        () = changed => {}
        () = tokio::time::sleep_until(deadline) => return self.status(thread),
      }
    }
  }

  /// `assignment` (a message for `thread`) is in the loop running on it:
  /// its end ends it. With no loop running, it is done at once.
  pub(crate) fn serve(&self, thread: &str, assignment: &str) {
    {
      let mut feeds = self.feeds.lock().expect("live lock");
      let f = feeds.entry(thread.to_owned()).or_default();
      if f.loops > 0 {
        if !f.serving.iter().any(|a| a == assignment) {
          f.serving.push(assignment.to_owned());
        }
      } else {
        f.finish(assignment.to_owned());
      }
      f.last = Some(Instant::now());
    }
    self.changed.notify_waiters();
  }

  /// `assignment` ran no turn, or its turn is over, whatever runs.
  pub(crate) fn done(&self, thread: &str, assignment: &str) {
    {
      let mut feeds = self.feeds.lock().expect("live lock");
      let f = feeds.entry(thread.to_owned()).or_default();
      f.serving.retain(|a| a != assignment);
      f.finish(assignment.to_owned());
    }
    self.changed.notify_waiters();
  }

  fn look(&self, thread: &str, after: Option<u64>, assignment: Option<&str>) -> Followed {
    let feeds = self.feeds.lock().expect("live lock");
    let Some(f) = feeds.get(thread) else {
      return Followed {
        next: 0,
        gap: after.is_some_and(|a| a > 0),
        ..Followed::default()
      };
    };
    let next = f.next();
    let (events, gap) = match after {
      None => (Vec::new(), false),
      // From a node that started since: what it has is all there is.
      Some(a) if a > next => (f.events.iter().cloned().collect(), true),
      Some(a) => {
        let from = a.max(f.first);
        let skip = (from - f.first) as usize;
        (joined(f.events.iter().skip(skip).cloned()), a < f.first)
      }
    };
    let done = assignment.is_some_and(|a| f.done.iter().any(|d| d == a));
    let taken = done || assignment.is_some_and(|a| f.serving.iter().any(|s| s == a));
    Followed {
      next,
      events,
      gap,
      taken,
      done,
      running: f.loops > 0,
    }
  }

  /// What `thread`'s loops streamed from `after` on, waiting up to `wait`
  /// for something new (an event, or `assignment`'s turn ending); with
  /// `after` `None`, at once, only where its feed stands (`next`).
  pub(crate) async fn follow(
    &self,
    thread: &str,
    after: Option<u64>,
    assignment: Option<&str>,
    wait: Duration,
  ) -> Followed {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
      let changed = self.changed.notified();
      tokio::pin!(changed);
      changed.as_mut().enable();
      let f = self.look(thread, after, assignment);
      if after.is_none() || !f.events.is_empty() || f.done || f.gap {
        return f;
      }
      tokio::select! {
        () = changed => {}
        () = tokio::time::sleep_until(deadline) => return self.look(thread, after, assignment),
      }
    }
  }
}

/// Where a thread's turns stand on this node (`Live::status`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TurnStatus {
  /// A loop runs on it.
  pub(crate) running: bool,
  /// Turns accepted that have not begun.
  pub(crate) queued: usize,
}

/// Consecutive text (and thinking) joined: a turn streams them in deltas.
fn joined(events: impl Iterator<Item = LiveEvent>) -> Vec<LiveEvent> {
  let mut out: Vec<LiveEvent> = Vec::new();
  for e in events {
    match (out.last_mut(), e) {
      (Some(LiveEvent::Text(a)), LiveEvent::Text(b)) => a.push_str(&b),
      (Some(LiveEvent::Thinking(a)), LiveEvent::Thinking(b)) => a.push_str(&b),
      (_, e) => out.push(e),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn a_follower_gets_what_came_since_and_when_its_turn_ended() {
    let live = Live::default();
    let start = live.follow("t", None, None, Duration::ZERO).await;
    assert_eq!(start.next, 0);
    live.begin("t");
    live.serve("t", "a1");
    live.push("t", LiveEvent::Text("Hel".into()));
    live.push("t", LiveEvent::Text("lo".into()));
    live.push("t", LiveEvent::ToolCall("$ ls".into()));
    let f = live
      .follow("t", Some(start.next), Some("a1"), Duration::from_secs(5))
      .await;
    assert_eq!(
      f.events,
      vec![
        LiveEvent::Started,
        LiveEvent::Text("Hello".into()),
        LiveEvent::ToolCall("$ ls".into())
      ]
    );
    assert!(f.taken && !f.done && f.running);
    // Waits for what comes next.
    let waiting = live.follow("t", Some(f.next), Some("a1"), Duration::from_secs(5));
    let pushing = async {
      tokio::time::sleep(Duration::from_millis(50)).await;
      live.end("t");
    };
    let (f, ()) = tokio::join!(waiting, pushing);
    assert_eq!(f.events, vec![LiveEvent::Ended]);
    assert!(f.done && !f.running);
    // Nothing new: the wait ends at its deadline, empty.
    let f2 = live
      .follow("t", Some(f.next), None, Duration::from_millis(50))
      .await;
    assert!(f2.events.is_empty() && !f2.gap);
  }

  #[tokio::test]
  async fn an_assignment_with_no_loop_is_done_at_once_and_old_events_are_a_gap() {
    let live = Live::default();
    live.serve("t", "a1");
    let f = live.follow("t", Some(0), Some("a1"), Duration::ZERO).await;
    assert!(f.done && f.taken);
    for i in 0..(KEPT + 10) {
      live.push("u", LiveEvent::Note(i.to_string()));
    }
    let f = live.follow("u", Some(0), None, Duration::ZERO).await;
    assert!(f.gap);
    assert_eq!(f.events.len(), KEPT);
  }

  #[cfg(not(mobile))]
  #[tokio::test]
  async fn a_turn_queued_then_running_is_said_as_it_changes() {
    let live = Live::default();
    let idle = TurnStatus::default();
    assert_eq!(live.status("t"), idle);
    let waiting = live.status_change("t", idle, Duration::from_secs(5));
    let queueing = async {
      tokio::time::sleep(Duration::from_millis(20)).await;
      live.queue("t");
    };
    let (now, ()) = tokio::join!(waiting, queueing);
    assert_eq!(
      now,
      TurnStatus {
        running: false,
        queued: 1
      }
    );
    // Its loop begins: running, nothing queued.
    live.begin("t");
    live.unqueue("t");
    assert_eq!(
      live.status("t"),
      TurnStatus {
        running: true,
        queued: 0
      }
    );
    live.end("t");
    assert_eq!(live.status("t"), idle);
    // Nothing changes: the wait ends at its deadline, as it was.
    assert_eq!(
      live
        .status_change("t", idle, Duration::from_millis(20))
        .await,
      idle
    );
  }
}
