//! Calls: an agent rings the user's phone (`call_user`) for what is
//! urgent, with a real incoming call (design §2.3, "Calls").
//!
//! The node sends every phone that registered for calls (a PushKit token,
//! `apns::Device::voip`) a VoIP push, sealed as a notification is: the
//! call's ID, the thread, this node, the caller line and when. The app
//! reports it to CallKit at once, as an incoming call from "Stitch: <the
//! reason>"; answered, it opens a voice call with the thread (its own
//! voice call mode, through the chat). Whatever happens, the phone tells
//! this node how it went (`Request::RingOutcome` there, passed on as
//! `Question::Ring`), and the tool, which waited for it, says so to the
//! model:
//!
//! - answered: the user is on a voice call with the thread; what the
//!   model says next is read aloud;
//! - declined: no more calls for a while (`AFTER_DECLINE`);
//! - missed (no answer within the phone's 30 seconds, or no word from it
//!   at all): a notification instead, so it stays visible.
//!
//! A phone that cannot ring (an app from before calls, a relay from before
//! them) gets a notification instead. Calls are limited per node, whatever
//! thread asks: one every `GAP`, none for `AFTER_DECLINE` after a decline,
//! so an agent cannot ring at 3am in a loop. Nothing is written as a
//! record: the tool call in the transcript is the call's history.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ring::rand::SecureRandom;
use serde::Deserialize;
use serde_json::json;
use stitch_providers::ToolDef;
use stitch_records::RecordId;
use tokio::sync::oneshot;

use crate::node::Shared;
use crate::{Error, Result};

/// The least time between two calls from this node.
pub(crate) const GAP: Duration = Duration::from_secs(5 * 60);
/// No call from this node for this long after the user declined one.
pub(crate) const AFTER_DECLINE: Duration = Duration::from_secs(30 * 60);
/// How long the node waits for the phone to say how the call went: the
/// phone gives up ringing after 30 seconds and says so; answered, it says
/// so at once (before it syncs or sets anything up). This is for a phone
/// that never says.
pub(crate) const WAIT: Duration = Duration::from_secs(60);
/// Answered, how long the tool waits on for the user's first words (the
/// phone sends them into this turn once its voice call is on and the
/// thread synced, over a link that may be slow), so the model's next reply
/// answers them and is read aloud. Past this, the tool says they are on
/// their way: they come woven in later, or as a turn of their own.
pub(crate) const GREET: Duration = Duration::from_secs(30);
/// The longest reason: it is the caller line on a lock screen.
const MAX_REASON: usize = 80;

/// How a call went, as the phone says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
  /// The user answered: the call's voice call is on.
  Answered,
  /// The user declined it.
  Declined,
  /// Nobody answered it within the phone's time.
  Missed,
  /// The phone could not ring for it (a stale push, another call on).
  Failed,
}

impl std::str::FromStr for Outcome {
  type Err = Error;
  fn from_str(s: &str) -> Result<Self> {
    Ok(match s {
      "answered" => Outcome::Answered,
      "declined" => Outcome::Declined,
      "missed" => Outcome::Missed,
      "failed" => Outcome::Failed,
      _ => {
        return Err(Error::Config(format!(
          "a call's outcome is answered, declined, missed or failed, not {s:?}"
        )));
      }
    })
  }
}

/// The calls this node made that wait for their outcome, and its limits.
#[derive(Default)]
pub(crate) struct Rings {
  pending: Mutex<HashMap<String, oneshot::Sender<Outcome>>>,
  limit: Mutex<Limit>,
}

#[derive(Default)]
struct Limit {
  /// When the last call rang.
  last: Option<Instant>,
  /// When the user last declined one.
  declined: Option<Instant>,
}

impl Rings {
  /// Counts one more call now, or says why it may not ring yet.
  fn admit(&self, now: Instant) -> Result<()> {
    let mut l = self.limit.lock().expect("rings lock");
    let mins = |d: Duration| d.as_secs().div_ceil(60).max(1);
    if let Some(t) = l.declined
      && now.duration_since(t) < AFTER_DECLINE
    {
      let wait = AFTER_DECLINE - now.duration_since(t);
      return Err(Error::Config(format!(
        "not called: the user declined a call {} minutes ago; no call goes for {} minutes \
         after a decline (the next in {} minutes). Tell them with notify, or in your report.",
        now.duration_since(t).as_secs() / 60,
        AFTER_DECLINE.as_secs() / 60,
        mins(wait)
      )));
    }
    if let Some(t) = l.last
      && now.duration_since(t) < GAP
    {
      let wait = GAP - now.duration_since(t);
      return Err(Error::Config(format!(
        "not called: this node rang the phone less than {} minutes ago (the next call can go in \
         {} minutes). Tell them with notify instead.",
        GAP.as_secs() / 60,
        mins(wait)
      )));
    }
    l.last = Some(now);
    Ok(())
  }

  fn declined(&self, now: Instant) {
    self.limit.lock().expect("rings lock").declined = Some(now);
  }

  /// Delivers a phone's word on call `call`: whether a call here waited
  /// for it (the first word wins; another phone's later one is dropped).
  pub(crate) fn deliver(&self, call: &str, outcome: Outcome) -> bool {
    let waiting = self.pending.lock().expect("rings lock").remove(call);
    waiting.is_some_and(|tx| tx.send(outcome).is_ok())
  }

  fn wait_for(self: &Arc<Self>, call: &str) -> Waiting {
    let (tx, rx) = oneshot::channel();
    self
      .pending
      .lock()
      .expect("rings lock")
      .insert(call.to_owned(), tx);
    Waiting {
      rings: self.clone(),
      call: call.to_owned(),
      rx: Some(rx),
    }
  }
}

/// A call waiting for its outcome; dropped (the tool call interrupted),
/// it no longer waits.
struct Waiting {
  rings: Arc<Rings>,
  call: String,
  rx: Option<oneshot::Receiver<Outcome>>,
}

impl Waiting {
  async fn outcome(mut self, within: Duration) -> Option<Outcome> {
    let rx = self.rx.take()?;
    tokio::time::timeout(within, rx).await.ok()?.ok()
  }
}

impl Drop for Waiting {
  fn drop(&mut self) {
    self
      .rings
      .pending
      .lock()
      .expect("rings lock")
      .remove(&self.call);
  }
}

/// What `call_user` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
  reason: String,
}

/// A call's ID: a random UUID (version 4), as CallKit names calls.
fn call_id() -> Result<String> {
  let mut b = [0u8; 16];
  ring::rand::SystemRandom::new()
    .fill(&mut b)
    .map_err(|_| Error::Config("no randomness for a call".into()))?;
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
  Ok(format!(
    "{}-{}-{}-{}-{}",
    &h[..8],
    &h[8..12],
    &h[12..16],
    &h[16..20],
    &h[20..]
  ))
}

/// The caller line: "Stitch: <reason>", cut to fit.
fn caller(reason: &str) -> String {
  let reason = reason.trim();
  let line = if reason.to_ascii_lowercase().starts_with("stitch") {
    reason.to_owned()
  } else {
    format!("Stitch: {reason}")
  };
  if line.chars().count() <= MAX_REASON {
    return line;
  }
  let mut out: String = line.chars().take(MAX_REASON - 1).collect();
  out.push('…');
  out
}

/// How a call went, as the tool says it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rang {
  /// No phone can ring: a notification went instead.
  CannotRing(Vec<String>),
  /// The user answered; `greeted`: their first words came within `GREET`
  /// (woven in after this), else they are on their way.
  Answered {
    greeted: bool,
  },
  Declined,
  /// Not answered (or no word from the phone): a notification went too.
  Missed(Vec<String>),
}

impl Rang {
  pub(crate) fn said(&self) -> String {
    match self {
      Rang::CannotRing(lines) => format!(
        "No phone could ring (no phone has the app with calls, or the push did not go): a \
         notification was sent instead.\n{}",
        lines.join("\n")
      ),
      Rang::Answered { greeted: true } => "The user answered: they are on a voice call with \
        you in this thread now. What you say next is read aloud to them: short spoken \
        sentences, no tables or code, starting with why you called."
        .into(),
      Rang::Answered { greeted: false } => "The user answered; their phone is still joining \
        the call (a slow link). Their first words, \"(I picked up your call.)\", come as a \
        message in this thread: your reply to that one is read aloud to them, short spoken \
        sentences starting with why you called. Until it comes, say nothing to them: end this \
        turn with one short line (not read aloud), or go on with other work."
        .into(),
      Rang::Declined => format!(
        "The user declined the call. No call goes from this node for the next {} minutes; if \
         it still matters, tell them with notify or in your report.",
        AFTER_DECLINE.as_secs() / 60
      ),
      Rang::Missed(lines) => format!(
        "No answer: the phone rang and nobody picked up (or its answer did not reach this \
         node in time). A notification was sent instead, so it stays on the phone. Should they \
         pick up late, their words come as a message in this thread, and your reply to it is \
         read aloud.\n{}",
        lines.join("\n")
      ),
    }
  }
}

/// Rings the user's phones for `thread` (or for nobody: a script's
/// `stitch` call), and waits for how it went.
pub(crate) async fn ring(s: &Arc<Shared>, thread: Option<&RecordId>, reason: &str) -> Result<Rang> {
  if reason.trim().is_empty() {
    return Err(Error::Config(
      "a call needs a reason: the caller line, a few words".into(),
    ));
  }
  if cfg!(any(target_os = "ios", target_os = "visionos")) {
    return Err(Error::Config(
      "this node is the phone: it cannot ring itself. Use notify.".into(),
    ));
  }
  // A thread compacted into another is that one now.
  let thread = thread.map(|t| crate::fork::current(s, t));
  let caller = caller(reason);
  let here = s.state().node.to_string();
  let devices: Vec<_> = crate::apns::devices(&s.state().index.lock().expect("index lock"))
    .into_iter()
    .filter(|d| d.node != here && d.voip.is_some() && d.key.is_some())
    .collect();
  let instead = |s: Arc<Shared>, thread: Option<RecordId>, title: &'static str, caller: String| async move {
    crate::notices::send(&s, thread.as_ref(), Some(title), &caller)
      .await
      .unwrap_or_else(|e| vec![e.to_string()])
  };
  if devices.is_empty() {
    return Ok(Rang::CannotRing(
      instead(s.clone(), thread.clone(), "Stitch tried to call", caller).await,
    ));
  }
  s.rings.admit(Instant::now())?;
  let call = call_id()?;
  let title = thread.as_ref().and_then(|t| {
    s.state()
      .index
      .lock()
      .expect("index lock")
      .title(t)
      .ok()
      .flatten()
  });
  let thread_id = thread.as_ref().map(|t| t.to_string());
  // The phone syncs this repository alone from here, answering.
  let repo = thread
    .as_ref()
    .and_then(|t| s.thread_place(t).ok())
    .map(|(root, _)| root.repo().to_string());
  let waiting = s.rings.wait_for(&call);
  let settings = crate::config::Config::load(&s.data)
    .ok()
    .and_then(|c| c.apns)
    .unwrap_or_default();
  let lines = s
    .pusher
    .ring(
      &s.data,
      &settings,
      &devices,
      &crate::apns::Ring {
        call: &call,
        thread: thread_id.as_deref(),
        repo: repo.as_deref(),
        node: &here,
        caller: &caller,
        title: title.as_deref(),
        at: jiff::Timestamp::now().as_second(),
      },
    )
    .await;
  eprintln!(
    "call {call} ({}): {}",
    thread_id.as_deref().unwrap_or("no thread"),
    lines.join("; ")
  );
  if !lines.iter().any(|l| l.ends_with(": ringing")) {
    let mut out = lines;
    out.extend(instead(s.clone(), thread.clone(), "Stitch tried to call", caller).await);
    return Ok(Rang::CannotRing(out));
  }
  let outcome = waiting.outcome(WAIT).await;
  eprintln!("call {call}: {outcome:?}");
  Ok(match outcome {
    Some(Outcome::Answered) => {
      let greeted = match &thread {
        Some(t) => first_words(s, t, GREET).await,
        None => false,
      };
      eprintln!("call {call}: the user's first words came: {greeted}");
      Rang::Answered { greeted }
    }
    Some(Outcome::Declined) => {
      s.rings.declined(Instant::now());
      Rang::Declined
    }
    Some(Outcome::Missed | Outcome::Failed) | None => {
      Rang::Missed(instead(s.clone(), thread, "Missed call from Stitch", caller).await)
    }
  })
}

/// Waits (`within` at most) for something to be woven into `thread`'s
/// running turn: a call's first words, which the phone sends once its
/// voice call is on. Whether they came.
async fn first_words(s: &Shared, thread: &RecordId, within: Duration) -> bool {
  let Some(steer) = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread.as_str())
    .cloned()
  else {
    return false;
  };
  let until = tokio::time::Instant::now() + within;
  loop {
    let arrival = steer.arrival();
    tokio::pin!(arrival);
    arrival.as_mut().enable();
    if steer.has_queued() {
      return true;
    }
    if tokio::time::timeout_at(until, arrival).await.is_err() {
      return steer.has_queued();
    }
  }
}

/// A phone's word on a call: `Request::RingOutcome` on the phone's own
/// node, for the node that called (`node`): delivered here, or passed on
/// to it (`Question::Ring`), tried a few times.
pub(crate) async fn tell(s: &Arc<Shared>, node: &str, call: &str, outcome: &str) -> Result<()> {
  let parsed: Outcome = outcome.parse()?;
  if node == s.state().node.as_str() {
    s.rings.deliver(call, parsed);
    return Ok(());
  }
  let mut last = String::new();
  for attempt in 0..3 {
    if attempt > 0 {
      tokio::time::sleep(Duration::from_secs(2)).await;
    }
    match crate::elsewhere::ring_outcome(s, node, call, outcome).await {
      Ok(()) => return Ok(()),
      Err(e) => last = e.to_string(),
    }
  }
  eprintln!("call {call}: its outcome ({outcome}) did not reach {node}: {last}");
  Err(Error::Config(last))
}

/// A peer's `Question::Ring`: a phone says how a call from here went.
pub(crate) fn answer(s: &Shared, call: &str, outcome: &str) -> crate::api::Reply {
  match outcome.parse::<Outcome>() {
    Ok(o) => {
      // A call that no longer waits (it timed out, the turn was stopped)
      // takes nothing: said, not an error.
      let waited = s.rings.deliver(call, o);
      if !waited {
        // Late: the tool said missed already. Answered, the phone's first
        // words still reach the thread (an assignment: woven into a turn
        // still running, else a turn of their own), and the reply to them
        // is read aloud in the call.
        eprintln!("call {call}: {outcome}, after the call stopped waiting");
      }
      crate::api::Reply::Lines(vec![if waited {
        format!("call {call}: {outcome}")
      } else {
        format!("call {call}: no longer waits")
      }])
    }
    Err(e) => crate::api::Reply::Error(e.to_string()),
  }
}

/// The `call_user` tool.
pub(crate) async fn tool(
  s: &Arc<Shared>,
  thread: &RecordId,
  input: serde_json::Value,
) -> Result<String> {
  let i: Input = serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
  Ok(ring(s, Some(thread), &i.reason).await?.said())
}

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: "call_user".into(),
    description: "Ring the user's phone: a real incoming call, for what is urgent and cannot \
      wait for them to look (production down, a run stuck that only they can unblock, \
      something they asked to be called about). Not for news a notification carries: use \
      notify for that. The reason is the caller line on their lock screen. It waits up to a \
      minute for them: answered, they are on a voice call with you in this thread and \
      what you say next is read aloud; declined or missed, it says so (a missed call also \
      sends a notification). Calls are limited: one every 5 minutes, none for 30 minutes \
      after a decline."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "reason": {"type": "string", "description": "Why you call, a few words (the caller line, at most 80 characters): \"the build on beast is stuck\"."}
      },
      "required": ["reason"]
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn calls_are_spaced_and_held_after_a_decline() {
    let r = Rings::default();
    let t0 = Instant::now();
    r.admit(t0).unwrap();
    let e = r.admit(t0 + Duration::from_secs(60)).unwrap_err();
    assert!(e.to_string().contains("in 4 minutes"), "{e}");
    r.admit(t0 + GAP).unwrap();
    r.declined(t0 + GAP);
    let e = r.admit(t0 + GAP * 3).unwrap_err();
    assert!(e.to_string().contains("declined"), "{e}");
    r.admit(t0 + GAP + AFTER_DECLINE).unwrap();
  }

  #[tokio::test]
  async fn the_first_word_on_a_call_wins() {
    let r = Arc::new(Rings::default());
    let w = r.wait_for("c1");
    assert!(!r.deliver("c2", Outcome::Answered), "another call");
    assert!(r.deliver("c1", Outcome::Declined));
    assert!(!r.deliver("c1", Outcome::Missed), "a second phone's, later");
    assert_eq!(
      w.outcome(Duration::from_secs(1)).await,
      Some(Outcome::Declined)
    );
    // No word: none, and the call no longer waits.
    let w = r.wait_for("c3");
    assert_eq!(w.outcome(Duration::from_millis(10)).await, None);
    assert!(!r.deliver("c3", Outcome::Answered));
  }

  #[test]
  fn a_call_is_a_uuid_and_its_caller_line_fits() {
    let id = call_id().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4");
    assert_ne!(id, call_id().unwrap());
    assert_eq!(caller(" the build is stuck "), "Stitch: the build is stuck");
    assert_eq!(caller("Stitch: deploy failed"), "Stitch: deploy failed");
    assert_eq!(caller(&"x".repeat(200)).chars().count(), MAX_REASON);
    assert_eq!("missed".parse::<Outcome>().unwrap(), Outcome::Missed);
    assert!("maybe".parse::<Outcome>().is_err());
  }

  #[test]
  fn each_outcome_tells_the_model_what_to_do() {
    assert!(
      Rang::Answered { greeted: true }
        .said()
        .contains("read aloud")
    );
    let joining = Rang::Answered { greeted: false }.said();
    assert!(joining.contains("still joining") && joining.contains("picked up"));
    assert!(Rang::Declined.said().contains("30 minutes"));
    assert!(
      Rang::Missed(vec!["iphone: pushed".into()])
        .said()
        .contains("iphone: pushed")
    );
    assert!(Rang::CannotRing(vec![]).said().contains("notification"));
  }
}
