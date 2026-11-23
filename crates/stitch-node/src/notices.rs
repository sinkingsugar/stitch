//! Notices: what an agent (the `notify` tool) or the user's scripts
//! (`stitch notify`) tell the user now, on their phones (design §2.3,
//! "Notices"). A schedule's thread that found something says so here,
//! rather than in a report the user reads whenever they look.
//!
//! Pushed to every phone, sealed as questions are (`apns`), naming the
//! thread so opening it opens that chat. A phone's own threads cannot push
//! to the phone itself: their notices wait in a queue its app takes
//! (`NodeHandle::take_notices`) and shows as local notifications. Nothing
//! is written as a record: the tool call in the transcript is the history.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use stitch_providers::ToolDef;
use stitch_records::RecordId;

use crate::node::Shared;
use crate::{Error, Result};

/// At most this many notices from one thread in [`WINDOW`]: a loop that
/// goes wrong must not buzz the phone all night.
pub(crate) const PER_THREAD: usize = 10;
pub(crate) const WINDOW: Duration = Duration::from_secs(3600);
/// The longest body sent; a push holds about 4 KB, sealed.
const MAX_BODY: usize = 1500;
const MAX_TITLE: usize = 120;
/// The most notices kept for the app (a desktop's are never taken).
const QUEUE: usize = 32;

/// A notice, as the app shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
  pub title: String,
  /// The thread's title, when it has one.
  pub subtitle: Option<String>,
  pub body: String,
  /// The thread it is from: opening it opens that chat.
  pub thread: Option<String>,
}

/// What `notify` takes.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
  pub title: Option<String>,
  pub body: String,
}

#[derive(Default)]
pub(crate) struct Notices {
  /// For this node's own app (a phone).
  queue: Mutex<VecDeque<Notice>>,
  /// When each thread's recent notices went.
  sent: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Notices {
  /// Takes what waits for the app.
  pub(crate) fn take(&self) -> Vec<Notice> {
    self.queue.lock().expect("notices lock").drain(..).collect()
  }

  fn keep(&self, n: Notice) {
    let mut q = self.queue.lock().expect("notices lock");
    if q.len() >= QUEUE {
      q.pop_front();
    }
    q.push_back(n);
  }

  /// Counts one more notice from `key`, or says it is one too many.
  fn admit(&self, key: &str, now: Instant) -> Result<()> {
    let mut sent = self.sent.lock().expect("notices lock");
    let times = sent.entry(key.to_owned()).or_default();
    while times
      .front()
      .is_some_and(|t| now.duration_since(*t) >= WINDOW)
    {
      times.pop_front();
    }
    if times.len() >= PER_THREAD {
      let wait = WINDOW.saturating_sub(now.duration_since(*times.front().expect("full")));
      return Err(Error::Config(format!(
        "not sent: this thread sent {PER_THREAD} notices in the last hour, the most it may; \
         the next can go in {} minutes. Gather what is left into one, or into your report.",
        wait.as_secs().div_ceil(60)
      )));
    }
    times.push_back(now);
    Ok(())
  }
}

/// Cuts `s` to at most `max` characters, marking the cut.
fn cut(s: &str, max: usize) -> String {
  let s = s.trim();
  if s.chars().count() <= max {
    return s.to_owned();
  }
  let mut out: String = s.chars().take(max - 1).collect();
  out.push('…');
  out
}

/// Sends a notice to the user's phones, from `thread` (a tool call) or
/// from nobody (`stitch notify` outside a thread). A line for each phone.
pub(crate) async fn send(
  s: &Arc<Shared>,
  thread: Option<&RecordId>,
  title: Option<&str>,
  body: &str,
) -> Result<Vec<String>> {
  if body.trim().is_empty() {
    return Err(Error::Config("a notice needs a body".into()));
  }
  // A thread compacted into another is that one now.
  let thread = thread.map(|t| crate::fork::current(s, t));
  if let Some(t) = &thread {
    s.notices.admit(t.as_str(), Instant::now())?;
  }
  let thread_title = thread.as_ref().and_then(|t| {
    s.state()
      .index
      .lock()
      .expect("index lock")
      .title(t)
      .ok()
      .flatten()
  });
  let title = title
    .map(str::trim)
    .filter(|t| !t.is_empty())
    .map(|t| cut(t, MAX_TITLE))
    .unwrap_or_else(|| "Stitch".into());
  let subtitle = thread_title.map(|t| cut(&t, MAX_TITLE));
  let body = cut(body, MAX_BODY);
  let notice = Notice {
    title,
    subtitle,
    body,
    thread: thread.as_ref().map(|t| t.to_string()),
  };
  let mut lines = crate::node::push(
    s,
    &crate::apns::Push {
      title: &notice.title,
      subtitle: notice.subtitle.as_deref(),
      body: &notice.body,
      badge: None,
      question: None,
      node: s.state().node.as_str(),
      answered: false,
      thread: notice.thread.as_deref(),
    },
  )
  .await;
  if cfg!(any(target_os = "ios", target_os = "visionos")) {
    lines.push(format!("{}: shown on this phone", s.state().node));
  }
  s.notices.keep(notice);
  Ok(lines)
}

/// The `notify` tool.
pub(crate) async fn tool(
  s: &Arc<Shared>,
  thread: &RecordId,
  input: serde_json::Value,
) -> Result<String> {
  let i: Input = serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
  let lines = send(s, Some(thread), i.title.as_deref(), &i.body).await?;
  let pushed = lines.iter().filter(|l| l.ends_with(": pushed")).count();
  let shown = lines
    .iter()
    .any(|l| l.ends_with(": shown on this phone") || l.ends_with(": shown on this desktop"));
  let head = if pushed > 0 || shown {
    "Sent."
  } else {
    "Not delivered to any phone or desktop."
  };
  Ok(format!("{head}\n{}", lines.join("\n")))
}

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: "notify".into(),
    description: "Send the user a notification on their phones now (a push; opening it opens \
      this conversation), shown on this machine's desktop too. For what they would want to know before they next look: a scheduled \
      check that found something, long work finished or stuck, a result they asked to be told \
      of. Not for progress or for every step: at most a few per run, with the news itself in the \
      body (short, concrete), not \"see the thread\". It does not ask, and does not replace your \
      report."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "title": {"type": "string", "description": "A few words (default: Stitch); the thread's title is shown under it."},
        "body": {"type": "string", "description": "The news, a sentence or two (at most 1500 characters)."}
      },
      "required": ["body"]
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_thread_sends_ten_an_hour() {
    let n = Notices::default();
    let t0 = Instant::now();
    for i in 0..PER_THREAD {
      n.admit("a", t0 + Duration::from_secs(i as u64)).unwrap();
    }
    let e = n.admit("a", t0 + Duration::from_secs(60)).unwrap_err();
    assert!(e.to_string().contains("in 59 minutes"), "{e}");
    // Another thread is counted apart.
    n.admit("b", t0 + Duration::from_secs(60)).unwrap();
    // An hour after the first, one more goes.
    n.admit("a", t0 + WINDOW).unwrap();
    assert!(n.admit("a", t0 + WINDOW).is_err());
  }

  #[test]
  fn the_queue_keeps_the_latest() {
    let n = Notices::default();
    for i in 0..QUEUE + 3 {
      n.keep(Notice {
        title: "t".into(),
        subtitle: None,
        body: i.to_string(),
        thread: None,
      });
    }
    let taken = n.take();
    assert_eq!(taken.len(), QUEUE);
    assert_eq!(taken[0].body, "3");
    assert!(n.take().is_empty());
  }

  #[test]
  fn long_text_is_cut() {
    assert_eq!(cut("  hi  ", 5), "hi");
    assert_eq!(cut("abcdefgh", 5), "abcd…");
  }
}
