//! Chats (design §2.3, §2.11): a `thread` record and its transcript
//! segments. A message runs the agent loop (the model, its tool calls and
//! their results, until it answers), and each step is written as a segment
//! chained to the previous one, so a long loop is checkpointed and any node
//! holding the repository continues the thread from its records.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_providers::{
  Content, Message, Native, Provider, Request, Role as PRole, StopReason, Usage,
};
use stitch_records::{
  Attachment, Author, BlobHash, Draft, Event, Msg, NodeName, Part, RecordId, Role, Segment,
  ToolCall, ToolResult, kinds,
};
use stitch_store::{BlobStore, Index, ItemPath, RecordRoot};

use crate::tools::ToolRunner;
use crate::{Error, Result, pricing};

/// Text attachments up to this size are inlined into the prompt.
const MAX_INLINE_TEXT: u64 = 512 * 1024;
/// Audio and video attachments up to this size are given to providers
/// that read them (Gemini, which uploads the large ones to its Files API,
/// whose limit this is); a larger one is named only.
const MAX_MEDIA: u64 = 2 << 30;
/// Model calls per message, at most, unless the loop's limits say
/// otherwise (a detached thread's, design §2.3).
pub const MAX_STEPS: u64 = 50;
/// Model calls a turn of a chat the user is in takes before it asks them
/// for as many again (`[chat] max_steps`).
pub const CHAT_STEPS: u64 = 200;

/// Said to the model when its turn reached its step limit and no more were
/// granted: one last step, to tell the user where the work is.
fn wrap_up(taken: u64) -> String {
  format!(
    "[Stitch: this turn has reached its step limit ({taken} model calls), and no more were \
     granted. Do not call any tool now: answer in a few lines with what you got done, where the \
     work stands and what is left, and say that you stopped at the step limit and that a \
     message such as \"go on\" continues.]"
  )
}

/// A model's turn with nothing in it: no text, no call, and natively at
/// most empty text (a provider refuses it sent back, design §2.30).
fn said_nothing(m: &Message) -> bool {
  m.role == PRole::Assistant
    && m
      .content
      .iter()
      .all(|c| matches!(c, Content::Text(t) if t.is_empty()))
    && m.native.as_ref().is_none_or(|n| {
      n.content.as_array().is_some_and(|blocks| {
        blocks
          .iter()
          .all(|b| b["type"] == "text" && b["text"].as_str().is_some_and(str::is_empty))
      })
    })
}

/// The sender of `NO_REPLY`: Stitch's own, never shown in a transcript (a
/// user's message there would end the turn's preset answers, §2.30).
pub const NO_REPLY_FROM: &str = "stitch:no-reply";
/// Said to the model, once a turn, when it ended a turn of tool calls with
/// no text, neither with its last calls nor after them (design §2.30).
pub const NO_REPLY: &str = "[Stitch: your turn ended without a reply: the user sees only your \
                            tool calls. Write your reply to them now, then end the turn.]";

/// A message the model reads but no transcript shows: `NO_REPLY`.
pub fn unshown(m: &Msg) -> bool {
  matches!(
    m.from.as_deref(),
    Some(NO_REPLY_FROM | crate::share::NOTE_FROM)
  )
}

/// A message for a loop while it runs: given to the model between its
/// steps, after the tool results. The user's, from a client, or a note
/// from the node (a background job ended, a report came).
#[derive(Clone, Debug)]
pub struct Interjection {
  /// The client's ID for it, said back when it is woven in; `None` for a
  /// note from the node.
  pub id: Option<u64>,
  /// Who it is from, as written; `None`: the user on this node.
  pub from: Option<String>,
  pub text: String,
  pub attachments: Vec<Attachment>,
  /// The reports it delivers.
  pub reports: Vec<RecordId>,
  /// Not worth waking the thread for: when its loop ends without taking
  /// it, this gives it back (a person's typing goes back to its session,
  /// for the agent's next call there to hear).
  pub untold: Option<Untold>,
  /// A message sent to the thread (`send` with `interject`, design §2.7):
  /// taken, its sender gets the loop's last reply when it ends; not taken,
  /// it starts a turn of its own.
  pub asker: Option<Asker>,
  /// A client's message relayed to this loop by a chat waiting for it
  /// (`Steer::relay`): the waiting chat hears when it is taken.
  pub relayed: Option<Relayed>,
  /// Threads and efforts attached to it (`Part::Scope`, after its text).
  pub scope: Option<crate::scopes::Scoped>,
}

/// The mark of a message relayed to another loop: its key there (to take
/// it back), and what tells the waiting chat it was taken.
#[derive(Clone)]
pub struct Relayed {
  key: u64,
  /// The client's ID for it.
  id: Option<u64>,
  taken: Arc<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for Relayed {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Relayed({})", self.key)
  }
}

/// What became of the messages a waiting chat relayed to the loop that
/// ran (`Steer::unrelay`).
#[derive(Debug, Default)]
pub struct RelayOutcome {
  /// Those taken, by the client's ID (`None`: the chat's own message).
  pub taken: Vec<Option<u64>>,
  /// Those it ended without taking, or taken back: the client's own.
  pub back: Vec<Interjection>,
}

/// A chat waiting for another loop on its thread, relaying to it what its
/// client says.
struct Relay {
  to: Steer,
  /// What went to it, by the client's ID and its key there.
  sent: Vec<(Option<u64>, u64)>,
  outcome: Arc<Mutex<RelayOutcome>>,
  on_taken: Arc<dyn Fn(Option<u64>) + Send + Sync>,
}

static RELAY_KEYS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Relay {
  /// Sends a message on to the loop; `false` when it ended (the message
  /// is then had back).
  fn forward(&mut self, i: Interjection) -> bool {
    let key = RELAY_KEYS.fetch_add(1, Ordering::SeqCst);
    let id = i.id;
    let original = i.clone();
    let (outcome, on_taken) = (self.outcome.clone(), self.on_taken.clone());
    let (back, given) = (self.outcome.clone(), i.clone());
    let relayed = Interjection {
      id: None,
      from: i.from,
      text: i.text,
      attachments: i.attachments,
      reports: Vec::new(),
      // Not taken before that loop ends: had back here.
      untold: Some(Untold(Arc::new(move || {
        back.lock().expect("relay lock").back.push(given.clone());
      }))),
      asker: None,
      relayed: Some(Relayed {
        key,
        id,
        taken: Arc::new(move || {
          outcome.lock().expect("relay lock").taken.push(id);
          on_taken(id);
        }),
      }),
      scope: i.scope,
    };
    let pushed = self.to.push(relayed);
    if pushed {
      self.sent.push((id, key));
    } else {
      self.outcome.lock().expect("relay lock").back.push(original);
    }
    pushed
  }
}

impl Interjection {
  /// The client's message again, as it was before it was relayed.
  fn unrelayed(self) -> Interjection {
    let id = self.relayed.as_ref().map_or(self.id, |r| r.id);
    Interjection {
      id,
      untold: None,
      relayed: None,
      ..self
    }
  }
}

/// Who sent a message woven into a loop, and how its reply goes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asker {
  /// The thread the reply is reported to; `None`: the user, who reads it
  /// in the thread.
  pub thread: Option<RecordId>,
  /// The reply is kept for the user, waking nothing (`wake = false`).
  pub keep: bool,
}

/// What gives a note back that its loop did not take.
#[derive(Clone)]
pub struct Untold(pub Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for Untold {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("Untold")
  }
}

/// What the user says to a running loop: messages woven in between its
/// steps, and a stop. A loop takes messages until it ends; what it did not
/// take stays with the client, which sends it as the next message.
#[derive(Clone, Default)]
pub struct Steer(Arc<SteerInner>);

#[derive(Default)]
struct SteerInner {
  /// `None` once the loop ended.
  queue: Mutex<Option<Vec<Interjection>>>,
  /// The senders of the messages the loop took, who wait for its reply.
  askers: Mutex<Vec<Asker>>,
  stopped: AtomicBool,
  /// Stopped because the app hosting the node is being suspended (iOS):
  /// the turn is to be resumed, not ended by the user.
  suspended: AtomicBool,
  stop: tokio::sync::Notify,
  /// The client that sent the message left (its connection closed).
  left: AtomicBool,
  gone: tokio::sync::Notify,
  /// Woken when something is queued for the loop.
  arrived: tokio::sync::Notify,
  /// Waiting for another loop on the thread: what comes goes to it.
  relay: Mutex<Option<Relay>>,
}

impl Steer {
  /// One for a loop about to run.
  pub fn open() -> Self {
    let s = Steer::default();
    *s.0.queue.lock().expect("steer lock") = Some(Vec::new());
    s
  }

  /// Queues a message for the loop; `false` once it ended. While this
  /// chat waits relaying to another loop (`relay`), it goes to that one.
  pub fn push(&self, i: Interjection) -> bool {
    {
      let mut relay = self.0.relay.lock().expect("steer lock");
      if let Some(r) = relay.as_mut() {
        r.forward(i);
        return true;
      }
    }
    let pushed = match self.0.queue.lock().expect("steer lock").as_mut() {
      Some(q) => {
        q.push(i);
        true
      }
      None => false,
    };
    if pushed {
      self.0.arrived.notify_waiters();
    }
    pushed
  }

  /// Whether something waits to be woven in (a message, a note).
  pub fn has_queued(&self) -> bool {
    self
      .0
      .queue
      .lock()
      .expect("steer lock")
      .as_ref()
      .is_some_and(|q| !q.is_empty())
  }

  /// Resolves when something is queued for the loop after this is
  /// created (enable it before looking at `has_queued`).
  pub fn arrival(&self) -> tokio::sync::futures::Notified<'_> {
    self.0.arrived.notified()
  }

  /// Takes back a message not woven yet; `false` when it was (or never
  /// came).
  pub fn retract(&self, id: u64) -> bool {
    {
      let mut relay = self.0.relay.lock().expect("steer lock");
      if let Some(r) = relay.as_mut()
        && let Some(at) = r.sent.iter().position(|(c, _)| *c == Some(id))
      {
        let (_, key) = r.sent.remove(at);
        return r.to.withdraw(key).is_some();
      }
    }
    let mut q = self.0.queue.lock().expect("steer lock");
    let Some(q) = q.as_mut() else { return false };
    let before = q.len();
    q.retain(|i| i.id != Some(id));
    q.len() != before
  }

  /// Stops the loop: what runs is cut short, and it ends.
  pub fn interrupt(&self) {
    self.0.stopped.store(true, Ordering::SeqCst);
    self.0.stop.notify_waiters();
  }

  pub fn interrupted(&self) -> bool {
    self.0.stopped.load(Ordering::SeqCst)
  }

  /// Stops the loop as `interrupt` does, because the app that hosts the
  /// node is being suspended (design §5, milestone 4): the stop is
  /// recorded as `why = "suspended"`, so the turn is resumed later.
  pub fn suspend(&self) {
    self.0.suspended.store(true, Ordering::SeqCst);
    self.interrupt();
  }

  pub fn suspended(&self) -> bool {
    self.0.suspended.load(Ordering::SeqCst)
  }

  /// Ends taking messages; returns what was not taken (the client's own
  /// it sends again itself).
  pub fn close(&self) -> Vec<Interjection> {
    self
      .0
      .queue
      .lock()
      .expect("steer lock")
      .take()
      .unwrap_or_default()
  }

  /// The same steer (clones share one).
  pub fn same(&self, other: &Steer) -> bool {
    Arc::ptr_eq(&self.0, &other.0)
  }

  /// The senders of the messages the loop took (see `Interjection::asker`).
  pub fn askers(&self) -> Vec<Asker> {
    self.0.askers.lock().expect("steer lock").clone()
  }

  pub(crate) fn take(&self) -> Vec<Interjection> {
    let taken = self
      .0
      .queue
      .lock()
      .expect("steer lock")
      .as_mut()
      .map(std::mem::take)
      .unwrap_or_default();
    self
      .0
      .askers
      .lock()
      .expect("steer lock")
      .extend(taken.iter().filter_map(|i| i.asker.clone()));
    for r in taken.iter().filter_map(|i| i.relayed.as_ref()) {
      (r.taken)();
    }
    taken
  }

  /// Takes back a message relayed here (by its key), not taken yet.
  fn withdraw(&self, key: u64) -> Option<Interjection> {
    let mut q = self.0.queue.lock().expect("steer lock");
    let q = q.as_mut()?;
    let at = q
      .iter()
      .position(|i| i.relayed.as_ref().is_some_and(|r| r.key == key))?;
    Some(q.remove(at))
  }

  /// This chat waits for the loop `to` (another turn on its thread): its
  /// own message `first` goes to that loop, woven in between its steps as
  /// the user's typing is, and so does what its client sends meanwhile
  /// (queued here already, or coming). `on_taken` hears each one taken,
  /// by the client's ID (`None`: `first`). `false`, nothing relayed: that
  /// loop ended already.
  pub fn relay(
    &self,
    to: Steer,
    first: Interjection,
    on_taken: impl Fn(Option<u64>) + Send + Sync + 'static,
  ) -> bool {
    let mut relay = self.0.relay.lock().expect("steer lock");
    if relay.is_some() {
      return true;
    }
    let mut r = Relay {
      to,
      sent: Vec::new(),
      outcome: Arc::default(),
      on_taken: Arc::new(on_taken),
    };
    if !r.forward(first) {
      return false;
    }
    if let Some(q) = self.0.queue.lock().expect("steer lock").as_mut() {
      for i in std::mem::take(q) {
        r.forward(i);
      }
    }
    *relay = Some(r);
    true
  }

  /// Ends relaying (the chat has its thread's lock, or stops waiting):
  /// what the other loop did not take is taken back from it.
  pub fn unrelay(&self) -> Option<RelayOutcome> {
    let r = self.0.relay.lock().expect("steer lock").take()?;
    for (_, key) in &r.sent {
      if let Some(i) = r.to.withdraw(*key) {
        r.outcome
          .lock()
          .expect("relay lock")
          .back
          .push(i.unrelayed());
      }
    }
    Some(std::mem::take(&mut *r.outcome.lock().expect("relay lock")))
  }

  /// Whether this chat relays to `to`'s loop and its own message was taken.
  pub fn relayed_first(&self) -> bool {
    self
      .0
      .relay
      .lock()
      .expect("steer lock")
      .as_ref()
      .is_some_and(|r| r.outcome.lock().expect("relay lock").taken.contains(&None))
  }

  /// The loop this chat relays to.
  pub fn relaying_to(&self) -> Option<Steer> {
    self
      .0
      .relay
      .lock()
      .expect("steer lock")
      .as_ref()
      .map(|r| r.to.clone())
  }

  /// The client that sent the message left. A turn already running goes
  /// on to its end; a message still waiting for another turn on its
  /// thread is not sent (`Reply::Unsent`), since no one is there to see it
  /// go or to take it back.
  pub fn leave(&self) {
    self.0.left.store(true, Ordering::SeqCst);
    self.0.gone.notify_waiters();
  }

  pub(crate) async fn left(&self) {
    loop {
      let notified = self.0.gone.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      if self.0.left.load(Ordering::SeqCst) {
        return;
      }
      notified.await;
    }
  }

  pub(crate) async fn stopped(&self) {
    loop {
      let notified = self.0.stop.notified();
      tokio::pin!(notified);
      notified.as_mut().enable();
      if self.interrupted() {
        return;
      }
      notified.await;
    }
  }
}

/// Marks where a loop was stopped: by the user, or (`suspended`) by the
/// app hosting the node going to sleep, a stop to resume from.
fn interrupted(suspended: bool) -> Event {
  if suspended {
    Event::Other(json!({"t": "interrupted", "at": now().to_string(), "why": SUSPENDED}))
  } else {
    Event::Other(json!({"t": "interrupted", "at": now().to_string()}))
  }
}

/// Why a turn stopped when its app was suspended (`interrupted`'s `why`,
/// and the outcome's stop reason).
pub const SUSPENDED: &str = "suspended";

/// Model calls cut off by the network (a dropped connection, a stream that
/// ended early) are sent again this many times, after these waits.
const NETWORK_RETRIES: [u64; 3] = [2, 5, 10];

/// Resolves when `steer` is interrupted; never without one.
async fn stopped(steer: Option<&Steer>) {
  match steer {
    Some(s) => s.stopped().await,
    None => std::future::pending().await,
  }
}

/// What a chat works on.
pub struct ChatContext<'a> {
  pub node: &'a NodeName,
  pub root: &'a RecordRoot,
  pub index: &'a Mutex<Index>,
  pub blobs: &'a BlobStore,
}

/// Where a folder is: a node, and its path there (`~/notes`, the same
/// folder on every node).
pub type FolderAt = (String, String);

pub struct ChatInput {
  /// `None` starts a thread.
  pub thread: Option<RecordId>,
  /// The ID a new thread takes, when the caller chose it (memory names
  /// the message before it is written).
  pub id: Option<RecordId>,
  pub text: String,
  pub attachments: Vec<Attachment>,
  pub provider: Provider,
  pub model: String,
  /// The system prompt for a new thread; a thread keeps the one it started
  /// with (changing it would invalidate the provider's earlier turns).
  pub system: Option<String>,
  /// Who the message is from (`user@<node>` when `None`).
  pub from: Option<String>,
  /// A new thread runs detached (design §2.3).
  pub detached: Option<Detached>,
  pub limits: Limits,
  /// Memories recalled for this message unasked: the note and the
  /// handles; ones the thread already got are left out by the caller.
  pub recalled: Option<(String, Vec<String>, Option<Value>)>,
  /// Said to the model with this message and not written: memory's
  /// follow-up instruction.
  pub instruction: Option<String>,
  /// The thread's instructions and skills as they are now (see `context`);
  /// sent along when they changed since it started.
  pub context: Option<String>,
  /// The folder a new thread belongs to; a project is the repository it
  /// is written in.
  pub folder: Option<FolderAt>,
  /// What the user says to the loop while it runs.
  pub steer: Option<Steer>,
  /// The message was said in a voice call (`Part::Spoken`): its reply is
  /// read aloud.
  pub spoken: bool,
  /// Threads and efforts attached to the message (design §2.27): written
  /// with it as `Part::Scope`, its briefing after the text.
  pub scope: Option<crate::scopes::Scoped>,
  /// A room's messages relayed to this participant (`Part::RoomTurn`,
  /// design §2.27): its note goes after the text (and the scope's
  /// briefing), written as the part, so it is replayed the same.
  pub room_turn: Option<Part>,
}

/// What a new detached thread records about itself.
#[derive(Clone, Debug, Default)]
pub struct Detached {
  /// The thread that started it, which its report goes to.
  pub parent: Option<RecordId>,
  pub budget: Limits,
  /// `provider/model`, kept so the thread resumes on the same model.
  pub model: Option<String>,
  /// Where its shell sessions start.
  pub cwd: Option<String>,
  /// The assignment it runs (delegated by another node, design §2.7).
  pub assignment: Option<RecordId>,
  /// The conversation it starts from.
  pub continues: Option<Continues>,
  /// It runs without tools (an asked model answering).
  pub no_tools: bool,
  /// Its report to `parent` is kept for the user and wakes nothing
  /// (`wake = false`, design §2.7).
  pub keep: bool,
  /// Read-only (`approval = "read-only"`): what the rules do not allow is
  /// refused, never asked.
  pub read_only: bool,
  /// A fixer of Stitch (`fixer = true`): its shells and tools in the
  /// fixers' mode, whatever the node's (`selfimprove::is_fixer`).
  pub fixer: bool,
  /// The schedule that started it (`scheduled-by`, design §2.3).
  pub scheduled_by: Option<RecordId>,
  /// Whom its reports go to when no thread started it (`reports-to`).
  pub reports_to: Option<RecordId>,
  /// The app whose thread it is (`app`, `app_name`: its conversation and
  /// name, design §2.22).
  pub app: Option<(RecordId, String)>,
}

/// Where a spawned thread picks up a conversation: the thread, up to a
/// segment (the last one written when it was delegated), or up to a tool
/// call (the one that delegated it, written with its step).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Continues {
  pub thread: RecordId,
  pub at: Upto,
  /// What of it a fork takes (all of it by default).
  pub shape: Shape,
}

/// What a fork takes of the conversation it continues (design §2.9): the
/// conversation is read from the thread it continues, never copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape {
  /// Up to this message of the thread (`m12`), with the tool calls that
  /// followed it, and not past it.
  pub upto: Option<String>,
  /// Attachments and tool images left out (each said as a line).
  pub no_attachments: bool,
  /// Tool calls and their results left out: what was said only.
  pub no_tools: bool,
  /// Only the last this many messages (from a message of the user's).
  pub last: Option<usize>,
  /// Stands for everything before what is kept (`last`, none without
  /// it): summarize and continue, summarize and truncate.
  pub summary: Option<String>,
}

impl Shape {
  fn read(record: &stitch_records::Record) -> Shape {
    Shape {
      upto: record.field_str("continues_msg").map(str::to_owned),
      no_attachments: record.field_bool("continues_attachments") == Some(false),
      no_tools: record.field_bool("continues_tools") == Some(false),
      last: record
        .fields
        .get("continues_last")
        .and_then(toml::Value::as_integer)
        .and_then(|n| usize::try_from(n).ok()),
      summary: record.field_str("summary").map(str::to_owned),
    }
  }

  fn write(&self, mut draft: Draft) -> Result<Draft> {
    if let Some(m) = &self.upto {
      draft = draft.field("continues_msg", m.as_str())?;
    }
    if self.no_attachments {
      draft = draft.field("continues_attachments", false)?;
    }
    if self.no_tools {
      draft = draft.field("continues_tools", false)?;
    }
    if let Some(n) = self.last {
      draft = draft.field("continues_last", n as i64)?;
    }
    if let Some(s) = &self.summary {
      draft = draft.field("summary", s.as_str())?;
    }
    Ok(draft)
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Upto {
  Segment(RecordId),
  Call(String),
}

impl Continues {
  /// From a record: its `continues` reference and `continues_at` or
  /// `continues_call` field.
  pub fn of(record: &stitch_records::Record) -> Option<Continues> {
    let thread = record.refs.first("continues")?.id.clone();
    let at = match (
      record.field_str("continues_at"),
      record.field_str("continues_call"),
    ) {
      (Some(s), _) => Upto::Segment(s.parse().ok()?),
      (None, Some(c)) => Upto::Call(c.to_owned()),
      (None, None) => return None,
    };
    Some(Continues {
      thread,
      at,
      shape: Shape::read(record),
    })
  }

  /// Writes itself into a draft.
  pub fn write(&self, draft: Draft) -> Result<Draft> {
    let draft = draft.reference("continues", stitch_records::Ref::to(self.thread.clone()))?;
    let draft = match &self.at {
      Upto::Segment(s) => draft.field("continues_at", s.as_str())?,
      Upto::Call(c) => draft.field("continues_call", c.as_str())?,
    };
    self.shape.write(draft)
  }
}

/// Limits on a loop: a detached thread's budget.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
  pub usd: Option<f64>,
  pub tokens: Option<u64>,
  /// Wall-clock deadline.
  pub until: Option<Timestamp>,
  /// Model calls one turn takes at most (`MAX_STEPS` when `None`); a turn's
  /// own, not spent across turns like the rest.
  pub steps: Option<u64>,
}

impl Limits {
  /// The budget as a record field.
  pub fn to_toml(&self) -> Option<toml::Value> {
    let mut t = toml::Table::new();
    if let Some(u) = self.usd {
      t.insert("usd".into(), u.into());
    }
    if let Some(n) = self.tokens {
      t.insert("tokens".into(), (n as i64).into());
    }
    if let Some(at) = self.until {
      t.insert(
        "until".into(),
        toml::Value::Datetime(at.to_string().parse().ok()?),
      );
    }
    if let Some(n) = self.steps {
      t.insert("steps".into(), (n.min(i64::MAX as u64) as i64).into());
    }
    (!t.is_empty()).then_some(toml::Value::Table(t))
  }

  pub fn from_json(v: Option<&Value>) -> Limits {
    let Some(v) = v else { return Limits::default() };
    Limits {
      usd: v.get("usd").and_then(Value::as_f64),
      tokens: v.get("tokens").and_then(Value::as_u64),
      until: v
        .get("until")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok()),
      steps: v.get("steps").and_then(Value::as_u64),
    }
  }

  /// Why a loop with these limits must not start: nothing is left of them.
  pub fn spent(&self) -> Option<String> {
    if self.usd.is_some_and(|u| u <= 0.0) {
      return Some("no money left of its budget".into());
    }
    if self.tokens == Some(0) {
      return Some("no tokens left of its budget".into());
    }
    if let Some(until) = self.until
      && Timestamp::now() >= until
    {
      return Some(format!("past its deadline, {until}"));
    }
    None
  }

  /// Why the loop must stop, if it must.
  fn exceeded(&self, usd: Option<f64>, usage: &Usage) -> Option<String> {
    if let (Some(max), Some(spent)) = (self.usd, usd)
      && spent >= max
    {
      return Some(format!("spent ${spent:.4} of ${max}"));
    }
    let tokens = usage.input_tokens + usage.output_tokens;
    if let Some(max) = self.tokens
      && tokens >= max
    {
      return Some(format!("used {tokens} of {max} tokens"));
    }
    if let Some(until) = self.until
      && Timestamp::now() >= until
    {
      return Some(format!("past its deadline, {until}"));
    }
    None
  }
}

/// How a loop ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
  /// The model answered without asking for more tools.
  Answered,
  /// A limit was reached; why.
  OverBudget(String),
  /// It took the most steps a message may take.
  Steps,
  /// The user stopped it.
  Interrupted,
}

/// What a chat shows while it runs.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatEvent {
  Text(String),
  Thinking(String),
  ToolCall {
    name: String,
    input: Value,
  },
  ToolResult {
    name: String,
    ok: bool,
    output: String,
  },
  /// A tool showed an app (`show_app`): its version and name, for a
  /// client to open (the tool result alone says nothing a desktop can open).
  App {
    manifest: BlobHash,
    name: String,
  },
  /// The thread's ID, known before anything is written.
  Thread(RecordId),
  /// Messages the user sent while it ran, now given to the model (by the
  /// client's IDs).
  Woven(Vec<u64>),
  /// A note from the node, now given to the model.
  Note(String),
  /// The model call was cut off by the network and is sent again: what
  /// it streamed so far is dropped (it was never written).
  Retry(String),
}

#[derive(Clone, Debug)]
pub struct ChatOutcome {
  pub thread: RecordId,
  pub segment: RecordId,
  /// The model that answered last (a fallback may differ from the one asked).
  pub model: String,
  pub stop: StopReason,
  /// Summed over the loop's model calls.
  pub usage: Usage,
  pub usd: Option<f64>,
  pub steps: usize,
  /// The thread had two continuations; the latest was followed.
  pub forked: bool,
  pub end: End,
  /// The last reply's text.
  pub text: String,
  /// The tool calls the loop made, and how many of them failed (a
  /// report's outcome minds a turn whose every call failed).
  pub calls: usize,
  pub failed_calls: usize,
}

/// A thread's transcript as provider messages.
pub struct History {
  pub messages: Vec<Message>,
  pub last_segment: Option<RecordId>,
  pub msg_count: usize,
  pub forked: bool,
  pub attachments: Vec<Attachment>,
  /// The latest instructions and skills sent as a context part.
  pub context: Option<String>,
  /// When the latest message was written, for the next one's time note.
  pub last_at: Option<Timestamp>,
  /// What the history leaves out: continuations off its main line
  /// (`fork_note`).
  pub forks: Option<String>,
  /// Handles of memories already recalled into this thread.
  pub recalled: std::collections::BTreeSet<String>,
  /// The line ends at this rewind (its record, the message it goes on
  /// from): the next segment starts its branch.
  pub rewind: Option<(RecordId, Option<String>)>,
  /// Action buttons pressed since the user's last message, not waking the
  /// model (design §2.30): told with the next one (`pending_presses`).
  pub presses: Vec<String>,
}

fn is_text(media_type: &str) -> bool {
  media_type.starts_with("text/")
    || media_type == "application/json"
    || media_type == "application/xml"
    || media_type.ends_with("+json")
    || media_type.ends_with("+xml")
}

/// What a provider gets for an attachment.
pub fn content_for(blobs: &BlobStore, a: &Attachment) -> Result<Content> {
  let name = a.name.clone().unwrap_or_else(|| "attachment".into());
  let read = || -> Result<Vec<u8>> {
    let path = blobs.path(&a.hash);
    std::fs::read(&path).map_err(Error::io(path.display()))
  };
  let media_type = stored_type(blobs, a);
  Ok(match media_type {
    t if is_image(t) => image_content(blobs, a, t)?.0,
    // An app's manifest says nothing to a model: its files are where it
    // was made.
    crate::apps::MEDIA => Content::Text(format!(
      "[App `{name}`, version {}: shown to the user]",
      &a.hash.hex()[..8]
    )),
    "application/pdf" => Content::Document {
      media_type: a.media_type.clone(),
      name: Some(name),
      data: read()?,
    },
    // Left in the blob store: read only by an adapter that takes it
    // (Gemini, which uploads a large one once); every other one sends a
    // note naming it without reading it.
    t if (t.starts_with("audio/") || t.starts_with("video/")) && a.bytes <= MAX_MEDIA => {
      Content::File {
        media_type: a.media_type.clone(),
        name: Some(name),
        path: blobs.path(&a.hash),
        bytes: a.bytes,
      }
    }
    t if is_text(t) && a.bytes <= MAX_INLINE_TEXT => {
      let text = String::from_utf8_lossy(&read()?).into_owned();
      Content::Text(format!("Attached file `{name}` ({t}):\n\n{text}"))
    }
    t => Content::Text(format!(
      "[Attached `{name}` ({t}, {} bytes): the model cannot read this type.]",
      a.bytes
    )),
  })
}

/// An image to send as one: the types every adapter reads, and HEIC and
/// HEIF, which Gemini reads and every other adapter turns into a note
/// saying so (`stitch_providers::portable`).
fn is_image(media_type: &str) -> bool {
  matches!(
    media_type,
    "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/heic" | "image/heif"
  )
}

/// An attachment's media type as its bytes say, for an image or a file
/// of no known type (a `Photo.jpg` may be PNG; records stored before
/// types were read from the bytes keep the name's), else as stored.
fn stored_type<'a>(blobs: &BlobStore, a: &'a Attachment) -> &'a str {
  let t = a.media_type.as_str();
  if !(t.starts_with("image/") || t == "application/octet-stream") {
    return t;
  }
  use std::io::Read;
  let mut head = [0u8; 64];
  let n = std::fs::File::open(blobs.path(&a.hash))
    .and_then(|f| f.take(head.len() as u64).read(&mut head))
    .unwrap_or(0);
  stitch_providers::media::image_type(t, &head[..n])
}

/// A stored image as providers take it (fitted: older records and the
/// user's attachments hold larger ones); the original stays for zooming.
fn image_content(
  blobs: &BlobStore,
  a: &Attachment,
  media_type: &str,
) -> Result<(Content, Option<crate::images::Scaled>)> {
  let path = blobs.path(&a.hash);
  let bytes = std::fs::read(&path).map_err(Error::io(path.display()))?;
  let f = crate::images::fit(bytes, media_type, Some(&a.hash.hex()));
  Ok((
    Content::Image {
      media_type: f.media_type,
      data: f.data,
    },
    f.scaled,
  ))
}

/// An attachment as the model gets it; an image with its short ID first,
/// for `zoom_image`, and how it was scaled to be sent.
fn attachment_content(blobs: &BlobStore, a: &Attachment) -> Result<Vec<Content>> {
  let media_type = stored_type(blobs, a);
  if !is_image(media_type) {
    return Ok(vec![content_for(blobs, a)?]);
  }
  let (image, scaled) = image_content(blobs, a, media_type)?;
  Ok(vec![
    Content::Text(format!(
      "[image {}{}{}]",
      crate::images::short_id(&a.hash),
      a.name
        .as_deref()
        .map(|n| format!(" `{n}`"))
        .unwrap_or_default(),
      scaled
        .map(|s| format!(", {} to be sent", s.describe()))
        .unwrap_or_default()
    )),
    image,
  ])
}

/// The images in a model's reply, stored as blobs and given as the
/// reply's attachments (`image-m3-1.png`, by message and order).
fn generated_images(blobs: &BlobStore, content: &[Content], msg_n: usize) -> Result<Vec<Part>> {
  let images = content.iter().filter_map(|c| match c {
    Content::Image { media_type, data } => Some((media_type, data)),
    _ => None,
  });
  let mut parts = Vec::new();
  for (i, (media_type, data)) in images.enumerate() {
    let media_type = stitch_providers::media::image_type(media_type, data);
    let ext = match media_type {
      "image/jpeg" => "jpg",
      "image/webp" => "webp",
      "image/gif" => "gif",
      _ => "png",
    };
    let hash = blobs.put_bytes(data)?;
    parts.push(Part::Attachment {
      att: Attachment {
        hash,
        media_type: media_type.to_owned(),
        name: Some(format!("image-m{msg_n}-{}.{ext}", i + 1)),
        bytes: data.len() as u64,
      },
    });
  }
  Ok(parts)
}

fn parts_content(parts: &[Part], blobs: &BlobStore) -> Result<Vec<Content>> {
  let mut content = Vec::new();
  for p in parts {
    match p {
      Part::Text { text } => content.push(Content::Text(text.clone())),
      Part::Attachment { att } => content.extend(attachment_content(blobs, att)?),
      Part::Context { context } => content.push(Content::Text(context_note(context))),
      Part::Recalled { recalled, .. } => content.push(Content::Text(recalled_note(recalled))),
      Part::Spoken { spoken: true, note } => content.push(Content::Text(spoken_note(*note).into())),
      // As it was rendered when the message was written: the same bytes
      // at every step and turn, however the scoped threads moved since.
      Part::Scope { briefing, .. } => content.push(Content::Text(briefing.clone())),
      // The note it was relayed with, by its number: the same bytes at
      // every later turn, however the wording moves on.
      Part::RoomTurn {
        slug,
        roster,
        can_pass,
        note,
        ..
      } => content.push(Content::Text(crate::rooms::room_note(
        *note, slug, roster, *can_pass,
      ))),
      Part::Spoken { spoken: false, .. } | Part::Other(_) => {}
    }
  }
  Ok(content)
}

/// A pause after which a message is told the time (design §2.3, "The
/// time"): within it, the thread's start in the system prompt and `date`
/// do.
const PAUSE: std::time::Duration = std::time::Duration::from_secs(3600);

/// What a message that comes after a pause starts with: the local time,
/// and how long since the message before it. Also when the local date
/// changed since, so "today" stays right. Derived from the messages' own
/// times when the history is built, never written into the message, so
/// it reads the same at each step (the prompt cache holds) and clients
/// do not show it.
pub(crate) fn time_note(prev: Option<Timestamp>, at: Timestamp) -> Option<String> {
  time_note_in(prev, at, &jiff::tz::TimeZone::system())
}

fn time_note_in(prev: Option<Timestamp>, at: Timestamp, tz: &jiff::tz::TimeZone) -> Option<String> {
  let prev = prev?;
  let gap = at.duration_since(prev);
  if gap.is_negative() {
    return None;
  }
  let (then, now) = (prev.to_zoned(tz.clone()), at.to_zoned(tz.clone()));
  if gap.unsigned_abs() < PAUSE && then.date() == now.date() {
    return None;
  }
  Some(format!(
    "[{}: {} since the previous message]",
    now.strftime("%a %Y-%m-%d %H:%M %Z"),
    spoken_gap(gap.unsigned_abs())
  ))
}

/// A pause in words, to the minute (hours and days for long ones).
fn spoken_gap(d: std::time::Duration) -> String {
  let m = d.as_secs() / 60;
  let unit = |n: u64, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
  match m {
    0..60 => unit(m, "minute"),
    60..1440 => match m % 60 {
      0 => unit(m / 60, "hour"),
      r => format!("{} {}", unit(m / 60, "hour"), unit(r, "minute")),
    },
    _ => match m / 60 % 24 {
      0 => unit(m / 1440, "day"),
      h => format!("{} {}", unit(m / 1440, "day"), unit(h, "hour")),
    },
  }
}

/// The time a message counts as for the next one's pause: none for the
/// summary a compacted thread starts with (stamped when the history is
/// built, not when anything was said).
fn msg_time(m: &Msg) -> Option<Timestamp> {
  (m.id != "summary").then_some(m.at)
}

/// The time note of a message in the history: one that comes from
/// outside (the user, a report, a note, a wake-up), not the images the
/// tools returned, nor the model's.
fn time_note_for(m: &Msg, prev: Option<Timestamp>) -> Option<String> {
  if m.role != Role::User || m.from.as_deref() == Some("tool") || m.id == "summary" {
    return None;
  }
  time_note(prev, m.at)
}

/// The latest message's time, for the next message's pause.
fn last_msg_time<'e>(events: impl IntoIterator<Item = &'e Event>) -> Option<Timestamp> {
  events
    .into_iter()
    .filter_map(|e| match e {
      Event::Msg(m) => msg_time(m),
      _ => None,
    })
    .last()
}

/// How memories recalled unasked reach the model: the note as memory
/// wrote it (notes from before it carried its own header get one).
pub fn recalled_note(memories: &str) -> String {
  if memories.starts_with("<recalled-memories>") {
    memories.to_owned()
  } else {
    format!(
      "[Stitch: from earlier conversations, possibly relevant; use them only if they help, \
       and say so when you do:]\n{memories}"
    )
  }
}

/// What a message said in a voice call carries (`Part::Spoken`, design
/// §2.26), after its text: written with the message as a part, so the
/// thread's later turns send the same prefix and the provider's cache
/// holds (the system prompt is never touched). Never changed in place:
/// a new note is a new constant and number (`SPOKEN_NOTE_NUMBER`), written
/// with new messages' parts (`Part::Spoken::note`), and an old message
/// is replayed with the note it was sent with (`spoken_note`), so old
/// threads' history stays byte for byte what it was.
pub const SPOKEN_NOTE: &str = SPOKEN_NOTE_1;
/// The note new spoken messages are written with (`SPOKEN_NOTE`'s).
pub const SPOKEN_NOTE_NUMBER: u32 = 1;

/// The second note (2026-11-13, with the user): the text is a
/// transcription, so read it for its likely meaning (he speaks Italian,
/// English and Japanese, and a word of one is written as another's).
pub const SPOKEN_NOTE_1: &str = "[Stitch: the user said this in a voice call; the text is an \
  automatic on-device transcription, so it may have misheard words, words of another language \
  written as the call's language, wrong homophones and missing punctuation. Read it for the most \
  plausible meaning, and ask only when it truly matters. Your reply will be read aloud. Answer as \
  you would speak: short and conversational, in plain sentences, in the language the user spoke. \
  No Markdown (no headings, lists, tables or bold), no emoji, and nothing that cannot be said: no \
  code, commands, file paths or URLs read out. When code or a long list is needed, say in a \
  sentence that it is on screen and put it after your spoken answer. Tools work as usual.]";

/// The note a spoken message was sent with, by its number.
pub fn spoken_note(n: u32) -> &'static str {
  match n {
    0 => SPOKEN_NOTE_0,
    _ => SPOKEN_NOTE_1,
  }
}

/// The first note (2026-11-12), replayed for the messages sent with it.
pub const SPOKEN_NOTE_0: &str = "[Stitch: the user said this in a voice call, and your reply will \
  be read aloud. Answer as you would speak: short and conversational, in plain sentences, in the \
  language the user spoke. No Markdown (no headings, lists, tables or bold), no emoji, and nothing \
  that cannot be said: no code, commands, file paths or URLs read out. When code or a long list \
  is needed, say in a sentence that it is on screen and put it after your spoken answer. Tools \
  work as usual.]";

/// How changed instructions and skills reach the model.
fn context_note(context: &str) -> String {
  if context.trim().is_empty() {
    "[Stitch: the instructions and skills this thread started with no longer apply.]".into()
  } else {
    format!(
      "[Stitch: the instructions and skills for this thread changed; these replace the earlier ones.]\n\n{context}"
    )
  }
}

fn result_text(output: &Value) -> String {
  output
    .get("text")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .unwrap_or_else(|| output.to_string())
}

/// What the user wrote into a step not written yet (a turn's first
/// message, messages woven in), by thread, while that step's tool calls
/// run: steps are written whole, after their calls.
type ByThread = std::collections::HashMap<String, Vec<String>>;
static UNWRITTEN: std::sync::LazyLock<std::sync::Mutex<ByThread>> =
  std::sync::LazyLock::new(Default::default);

/// Whether a message is the user's own (typed, not a report, a note or
/// another thread's message). A turn's first message is `user@<node>`; one
/// the user wove into a running turn is `user`.
pub(crate) fn by_user(m: &Msg) -> bool {
  m.role == Role::User
    && m
      .from
      .as_deref()
      .is_none_or(|f| f == "user" || f.starts_with("user@"))
}

/// Who wrote `thread`'s latest message from the user's side: the user
/// (`user`, `user@<node>`) or loose ends in their name (`loose-ends`);
/// reports, notes and tool output are not. `None` when there is none.
pub(crate) fn last_prompt_from(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<String>> {
  let (segments, _) = chain_to(ctx, thread, None)?;
  Ok(
    segments
      .iter()
      .rev()
      .flat_map(|s| s.events.iter().rev())
      .filter_map(|e| match e {
        Event::Msg(m) if m.role == Role::User => Some(m),
        _ => None,
      })
      .find(|m| {
        m.from
          .as_deref()
          .is_none_or(|f| f == "user" || f.starts_with("user@") || f == crate::loose::FROM)
      })
      .map(|m| m.from.clone().unwrap_or_else(|| "user".into())),
  )
}

/// The user's texts in `events`.
pub(crate) fn user_texts(events: &[Event]) -> Vec<String> {
  events
    .iter()
    .filter_map(|e| match e {
      Event::Msg(m) if by_user(m) => Some(m),
      _ => None,
    })
    .flat_map(|m| &m.parts)
    .filter_map(|p| match p {
      Part::Text { text } => Some(text.clone()),
      _ => None,
    })
    .collect()
}

/// What the user wrote in `thread`'s step whose calls run now.
pub(crate) fn unwritten_user_texts(thread: &RecordId) -> Vec<String> {
  UNWRITTEN
    .lock()
    .expect("unwritten lock")
    .get(thread.as_str())
    .cloned()
    .unwrap_or_default()
}

/// Holds a step's user texts in `UNWRITTEN` until dropped.
struct Unwritten(String);

impl Unwritten {
  fn set(thread: &RecordId, events: &[Event]) -> Self {
    UNWRITTEN
      .lock()
      .expect("unwritten lock")
      .insert(thread.to_string(), user_texts(events));
    Unwritten(thread.to_string())
  }
}

impl Drop for Unwritten {
  fn drop(&mut self) {
    UNWRITTEN.lock().expect("unwritten lock").remove(&self.0);
  }
}

/// A thread's events, in order, as written (the latest continuation).
pub fn thread_events(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Vec<Event>> {
  let (segments, _) = chain(ctx, thread)?;
  Ok(segments.into_iter().flat_map(|s| s.events).collect())
}

/// A `failed` event: a turn's model call that failed (`crate::failed`).
pub(crate) fn is_failed(e: &Event) -> bool {
  matches!(e, Event::Other(v) if v.get("t").and_then(Value::as_str) == Some("failed"))
}

/// A thread's last turn that never ended (`unfinished`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unfinished {
  /// `suspended` (stopped as the app hosting its node was suspended) or
  /// `cut off` (its last step left the model to answer).
  pub why: &'static str,
  /// The node whose loop has the turn, when it is not this node's chats':
  /// another node, which may be running it now (its records cannot say),
  /// or a detached loop at work (here too). Resuming it would put a second
  /// loop on the thread: it is forked instead (design §5 milestone 4).
  pub elsewhere: Option<String>,
}

/// Whether a thread's last turn never ended, as its records say, why, and
/// whose it is: the writer of its last step's segment (a fork nothing has
/// continued yet is its forker's), or a detached thread's node while it
/// has no report (its loop runs or resumes there). `cut off`: its last
/// step left the model to answer (the user's message, tool results or a
/// note after them, as when the node was killed or the loop failed
/// mid-turn). `None` when it answered, the user stopped it, or the model
/// declined. Says nothing of a chat running here now: its caller knows.
pub fn unfinished(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<Unfinished>> {
  // Who wrote its record, and whether it is a detached loop at work.
  let record = {
    let index = ctx.index.lock().expect("index lock");
    match index.record(thread)? {
      Some(t) => {
        let detached = matches!(
          detached_state(&index, &t)?.as_deref(),
          Some("running" | "waiting")
        );
        Some((t.node, detached))
      }
      None => None,
    }
  };
  let line = line_to(ctx, thread, None)?;
  let rewinder = line.open.as_ref().map(|r| r.node.clone());
  let segments = line.segments;
  // A fork not continued yet: what it took, its forker's to continue.
  let taken = match (segments.is_empty(), &record) {
    (true, Some(_)) => inherited(ctx, thread)?,
    _ => Vec::new(),
  };
  let forker = record.as_ref().map_or("", |(n, _)| n.as_str());
  let steps: Vec<(&str, &[Event])> = std::iter::once((forker, taken.as_slice()))
    .chain(segments.iter().enumerate().map(|(i, s)| {
      // Rewound to it: what follows is the rewinder's to continue.
      let node = match &rewinder {
        Some(n) if i + 1 == segments.len() => n.as_str(),
        _ => s.header.node.as_str(),
      };
      (node, s.events.as_slice())
    }))
    .collect();
  let mut last = None;
  'found: for (node, events) in steps.into_iter().rev() {
    for e in events.iter().rev() {
      let why = match e {
        Event::Usage(_) => continue,
        Event::Other(v) => match v.get("t").and_then(Value::as_str) {
          Some("interrupted") => {
            let why = v.get("why").and_then(Value::as_str);
            (why == Some(SUSPENDED)).then_some(SUSPENDED)
          }
          Some("refusal") => None,
          _ => continue,
        },
        Event::Msg(m) => match m.role {
          Role::Assistant => None,
          Role::User | Role::System => Some(CUT_OFF),
        },
        Event::ToolCall(_) | Event::ToolResult(_) => Some(CUT_OFF),
      };
      last = why.map(|why| (why, node.to_owned()));
      break 'found;
    }
  }
  let Some((why, mut node)) = last else {
    return Ok(None);
  };
  // A detached loop at work has it, here too.
  let detached = matches!(record, Some((_, true)));
  if let Some((runner, true)) = record {
    node = runner;
  }
  Ok(Some(Unfinished {
    why,
    elsewhere: (detached || node != ctx.node.as_str()).then_some(node),
  }))
}

/// A turn whose last step left the model to answer (`unfinished`).
pub const CUT_OFF: &str = "cut off";

/// A thread's messages, in order, as written.
pub fn thread_messages(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Vec<Msg>> {
  let (segments, _) = chain(ctx, thread)?;
  Ok(
    segments
      .into_iter()
      .flat_map(|s| s.events)
      .filter_map(|e| match e {
        Event::Msg(m) => Some(m),
        _ => None,
      })
      .collect(),
  )
}

/// The head of a thread's main line, from its segments (ID, prev) in the
/// order written (design §2.4): the end of its longest chain, the first
/// written of those as long. A node that continued the thread from an
/// older step (its copy was behind) starts a branch shorter than the line
/// it missed, so it does not take the thread over, as it did when the
/// latest head was followed; a continuation that died off (two nodes at
/// once, one gave way) is left behind too. Every node holding the same
/// segments finds the same line.
pub fn main_head<'a>(
  segments: impl IntoIterator<Item = (&'a RecordId, Option<&'a RecordId>)>,
) -> Option<RecordId> {
  let segments: Vec<(&RecordId, Option<&RecordId>)> = segments.into_iter().collect();
  let at: std::collections::HashMap<&RecordId, usize> = segments
    .iter()
    .enumerate()
    .map(|(i, (id, _))| (*id, i))
    .collect();
  // Each one's length from the first segment (one whose predecessor is
  // not here yet starts a line of its own).
  let mut length: Vec<Option<usize>> = vec![None; segments.len()];
  for start in 0..segments.len() {
    let mut path = Vec::new();
    let mut i = start;
    let base = loop {
      if let Some(l) = length[i] {
        break l;
      }
      path.push(i);
      match segments[i].1.and_then(|p| at.get(p)) {
        // A cycle (it cannot be, but the files are data): cut there.
        Some(&p) if !path.contains(&p) => i = p,
        _ => break 0,
      }
    };
    for (n, &i) in path.iter().rev().enumerate() {
      length[i] = Some(base + n + 1);
    }
  }
  let prevs: std::collections::HashSet<&RecordId> =
    segments.iter().filter_map(|(_, p)| *p).collect();
  segments
    .iter()
    .enumerate()
    .filter(|(_, (id, _))| !prevs.contains(id))
    // Longest; of those as long, the first written.
    .max_by_key(|(i, _)| (length[*i].unwrap_or(0), std::cmp::Reverse(*i)))
    .map(|(_, (id, _))| (*id).clone())
}

/// What a thread's history leaves out when it forked: each continuation
/// off its main line (`main_head`), where it branched, on which node and
/// when, and the last of what was said there; `None` when it never forked.
/// Told to the model with its context, so work done there is not unknown
/// to it (a woken turn on a stale copy ran tools, committed, spawned).
pub fn fork_note(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<String>> {
  let (mut segments, open) = segments_of(ctx, thread)?;
  // After a rewind, only its branch: what it left behind is not told.
  {
    let index = ctx.index.lock().expect("index lock");
    let end = crate::rewind::main_line(
      &crate::rewind::rows_in(&index, thread, open.as_ref()),
      &crate::rewind::rewinds_in(&index, thread),
    );
    if let Some(pool) = &end.pool {
      segments.retain(|(id, _, _)| pool.contains(id));
    }
  }
  let Some(main) = main_head(segments.iter().map(|(id, prev, _)| (id, prev.as_ref()))) else {
    return Ok(None);
  };
  let prev_of = |id: &RecordId| {
    segments
      .iter()
      .find(|(s, _, _)| s == id)
      .and_then(|(_, p, _)| p.clone())
  };
  let mut on_main = std::collections::HashSet::new();
  let mut cur = Some(main);
  while let Some(id) = cur {
    cur = prev_of(&id);
    on_main.insert(id);
  }
  let prevs: std::collections::HashSet<&RecordId> =
    segments.iter().filter_map(|(_, p, _)| p.as_ref()).collect();
  let mut branches = Vec::new();
  for (head, _, _) in segments.iter().filter(|(id, _, _)| !prevs.contains(id)) {
    if on_main.contains(head) {
      continue;
    }
    // Back to where it left the main line.
    let mut ids = Vec::new();
    let mut cur = Some(head.clone());
    let mut from = None;
    while let Some(id) = cur {
      if on_main.contains(&id) {
        from = Some(id);
        break;
      }
      cur = prev_of(&id);
      ids.push(id);
    }
    ids.reverse();
    let mut said = Vec::new();
    for id in &ids {
      let item = ItemPath::Segment {
        thread: thread.clone(),
        segment: id.clone(),
      };
      let Ok(text) = ctx.root.read(&item) else {
        continue;
      };
      let Ok(seg) = Segment::parse(&String::from_utf8_lossy(&text)) else {
        continue;
      };
      for e in seg.events {
        if let Event::Msg(m) = e {
          let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
              Part::Text { text } => Some(text.as_str()),
              _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
          let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
          if text.is_empty() {
            continue;
          }
          let who = match m.role {
            Role::Assistant => "assistant".to_owned(),
            _ => m.from.unwrap_or_else(|| "user".into()),
          };
          let short: String = text.chars().take(300).collect();
          let cut = if short.len() < text.len() { "…" } else { "" };
          said.push(format!("  - {who}: {short}{cut}"));
        }
      }
    }
    let (Some(first), Some(last)) = (ids.first(), ids.last()) else {
      continue;
    };
    let shown = said.len().min(6);
    let mut b = format!(
      "- on {}, from {} to {} ({} steps), branching off after step {}:",
      first.node(),
      first.timestamp().strftime("%Y-%m-%d %H:%M UTC"),
      last.timestamp().strftime("%Y-%m-%d %H:%M UTC"),
      ids.len(),
      from.map_or_else(|| "(not here)".to_owned(), |f| f.to_string()),
    );
    if said.len() > shown {
      b.push_str(&format!(
        "\n  ({} messages before these)",
        said.len() - shown
      ));
    }
    for line in &said[said.len() - shown..] {
      b.push('\n');
      b.push_str(line);
    }
    branches.push(b);
  }
  if branches.is_empty() {
    return Ok(None);
  }
  Ok(Some(format!(
    "# Other continuations of this thread\n\nThis thread was also continued from an older step, \
     by a node whose copy was behind; those steps are not in this history (it follows the \
     longest continuation), but what they did happened (commands ran, files and commits \
     may have changed):\n{}\n",
    branches.join("\n")
  )))
}

/// Loads a thread's segments in chain order: its main line (`main_head`).
/// The name a thread's system prompt has among its record's attachments.
const SYSTEM_PROMPT_NAME: &str = "system-prompt.md";

/// A thread's system prompt: a blob by its hash (`system_prompt`, the
/// record's attachment named `system-prompt.md`, so it syncs as one and
/// threads with the same prompt share it), or, in a record written before,
/// its `system` field.
pub fn system_of(ctx: &ChatContext<'_>, record: &stitch_records::Record) -> Result<Option<String>> {
  if let Some(s) = record.field_str("system") {
    return Ok(Some(s.to_owned()));
  }
  let Some(hash) = record.field_str("system_prompt") else {
    return Ok(None);
  };
  let Some(att) = record
    .attachments
    .iter()
    .find(|a| a.hash.to_string() == hash)
  else {
    return Err(Error::Config(format!(
      "thread {}: its system prompt {hash} is not among its attachments",
      record.id
    )));
  };
  if !ctx.blobs.contains(&att.hash) {
    return Err(Error::MissingBlobs(vec![att.hash]));
  }
  let mut text = String::new();
  std::io::Read::read_to_string(&mut ctx.blobs.open_blob(&att.hash)?, &mut text)
    .map_err(Error::io(att.hash))?;
  Ok(Some(text))
}

/// Names in `draft` the system prompt `record` has, as `record` names it
/// (inline, or a blob by its hash), without reading it: a node that does
/// not hold the blob (a phone, for a thread another node started) passes
/// it on all the same, and fetches it when a turn needs it.
pub fn with_system_of(mut draft: Draft, record: &stitch_records::Record) -> Result<Draft> {
  if let Some(s) = record.field_str("system") {
    return Ok(draft.field("system", s)?);
  }
  let Some(hash) = record.field_str("system_prompt") else {
    return Ok(draft);
  };
  let Some(att) = record
    .attachments
    .iter()
    .find(|a| a.hash.to_string() == hash)
  else {
    return Err(Error::Config(format!(
      "thread {}: its system prompt {hash} is not among its attachments",
      record.id
    )));
  };
  draft.attachments.push(att.clone());
  Ok(draft.field("system_prompt", hash)?)
}

/// Puts a system prompt in the blob store and names it in `draft`.
pub fn with_system(blobs: &BlobStore, mut draft: Draft, system: &str) -> Result<Draft> {
  let hash = blobs.put_bytes(system.as_bytes())?;
  draft.attachments.push(Attachment {
    hash,
    media_type: "text/markdown".into(),
    name: Some(SYSTEM_PROMPT_NAME.into()),
    bytes: system.len() as u64,
  });
  Ok(draft.field("system_prompt", hash.to_string())?)
}

/// How many tokens the thread's latest step sent the model (its prompt,
/// cached or not): how full its context window is.
pub(crate) fn last_prompt_tokens(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<u64>> {
  let (segments, _) = chain(ctx, thread)?;
  Ok(
    segments
      .iter()
      .rev()
      .flat_map(|s| s.events.iter().rev())
      .find_map(|e| match e {
        Event::Usage(u) => Some(
          u.input_tokens + u.cache_read_tokens.unwrap_or(0) + u.cache_write_tokens.unwrap_or(0),
        ),
        _ => None,
      }),
  )
}

/// The head of a thread's main line: its latest segment there.
pub(crate) fn line_head(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<RecordId>> {
  Ok(
    chain(ctx, thread)?
      .0
      .last()
      .map(|s| s.header.segment.clone()),
  )
}

/// The main line's head, and the rewind it ends at when nothing continues
/// that yet (its head then cut after the rewind's message).
pub(crate) fn line_end(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
) -> Result<(Option<RecordId>, Option<crate::rewind::Rewind>)> {
  let line = line_to(ctx, thread, None)?;
  Ok((
    line.segments.last().map(|s| s.header.segment.clone()),
    line.open,
  ))
}

/// The segment of the main line holding message `m`.
pub(crate) fn segment_of_message(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  m: &str,
) -> Result<Option<RecordId>> {
  Ok(
    line_to(ctx, thread, None)?
      .segments
      .iter()
      .find(|s| {
        s.events
          .iter()
          .any(|e| matches!(e, Event::Msg(x) if x.id == m))
      })
      .map(|s| s.header.segment.clone()),
  )
}

fn chain(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<(Vec<Segment>, bool)> {
  chain_to(ctx, thread, None)
}

/// A thread's segments in chain order, ending at `head` (the main line's
/// when `None`); and whether it has two continuations.
fn chain_to(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<(Vec<Segment>, bool)> {
  let line = line_to(ctx, thread, head)?;
  Ok((line.segments, line.forked))
}

/// A line of a thread: its segments in order, each cut where a rewind
/// branched off it (design §2.15, "Rewinding a thread").
pub(crate) struct Line {
  pub segments: Vec<Segment>,
  /// Two continuations among the segments the line was chosen from.
  pub forked: bool,
  /// It ends at this rewind, nothing continuing it yet.
  pub open: Option<crate::rewind::Rewind>,
  /// The thread has rewinds.
  pub rewound: bool,
}

/// The line ending at `head` (the main line, `crate::rewind::main_line`,
/// when `None`).
pub(crate) fn line_to(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<Line> {
  let (segments, open) = segments_of(ctx, thread)?;
  let (rows, rewinds) = {
    let index = ctx.index.lock().expect("index lock");
    (
      crate::rewind::rows_in(&index, thread, open.as_ref()),
      crate::rewind::rewinds_in(&index, thread),
    )
  };
  let end = match head {
    Some(h) => {
      if !segments.iter().any(|(s, _, _)| s == h) {
        return Err(Error::Config(format!(
          "segment {h} of thread {thread} is not here yet"
        )));
      }
      crate::rewind::LineEnd {
        head: Some(h.clone()),
        ..Default::default()
      }
    }
    None => crate::rewind::main_line(&rows, &rewinds),
  };
  let in_pool = |id: &RecordId| end.pool.as_ref().is_none_or(|p| p.contains(id));
  let prevs: std::collections::HashSet<&RecordId> = segments
    .iter()
    .filter(|(id, _, _)| in_pool(id))
    .filter_map(|(_, p, _)| p.as_ref())
    .collect();
  let heads = segments
    .iter()
    .filter(|(id, _, _)| in_pool(id) && !prevs.contains(id))
    .count();
  let mut out: Vec<Segment> = Vec::new();
  let mut cur = end.head.clone();
  while let Some(id) = cur {
    let Some((_, prev, _)) = segments.iter().find(|(s, _, _)| *s == id) else {
      break;
    };
    match open.as_ref().filter(|o| o.header.segment == id) {
      Some(o) => out.push(o.clone()),
      None => {
        let item = ItemPath::Segment {
          thread: thread.clone(),
          segment: id.clone(),
        };
        let text =
          String::from_utf8(ctx.root.read(&item)?).map_err(|e| Error::Config(e.to_string()))?;
        out.push(Segment::parse(&text)?);
      }
    }
    cur = prev.clone();
  }
  out.reverse();
  // A segment starting a rewind's branch cuts the one before it.
  for i in 1..out.len() {
    if out[i].header.rewind.is_some()
      && let Some(u) = out[i].header.upto.clone()
    {
      crate::rewind::cut_after(&mut out[i - 1].events, &u);
    }
  }
  if let Some(r) = &end.open
    && let (Some(u), Some(last)) = (&r.upto, out.last_mut())
  {
    crate::rewind::cut_after(&mut last.events, u);
  }
  Ok(Line {
    segments: out,
    forked: heads > 1,
    open: end.open,
    rewound: !rewinds.is_empty(),
  })
}

/// The next message number of a thread: past every message it holds, on
/// any branch, so that a message off the line and one on it never share
/// an ID (memory and search name messages by it).
pub(crate) fn next_msg_n(ctx: &ChatContext<'_>, thread: &RecordId, line: &Line) -> Result<usize> {
  let on_line = line
    .segments
    .iter()
    .flat_map(|s| &s.events)
    .filter(|e| matches!(e, Event::Msg(_)))
    .count();
  if !line.rewound {
    return Ok(on_line);
  }
  let (segments, open) = segments_of(ctx, thread)?;
  let mut top = on_line;
  for (id, _, _) in &segments {
    let seg = match open.as_ref().filter(|o| o.header.segment == *id) {
      Some(o) => o.clone(),
      None => {
        let item = ItemPath::Segment {
          thread: thread.clone(),
          segment: id.clone(),
        };
        let Ok(bytes) = ctx.root.read(&item) else {
          continue;
        };
        let Ok(seg) = Segment::parse(&String::from_utf8_lossy(&bytes)) else {
          continue;
        };
        seg
      }
    };
    top = top.max(crate::rewind::highest_msg(&seg.events));
  }
  Ok(top)
}

/// A segment as the index lists it: its ID, the one before, its path.
pub(crate) type SegmentRow = (RecordId, Option<RecordId>, String);

/// A thread's segments as this node has them: the sealed ones the index
/// lists (ID, previous, path), and its open one, whose steps the turn
/// running here wrote and no other node has yet (`RecordRoot::append_step`).
/// The open one is listed with the others, its path empty.
pub(crate) fn segments_of(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
) -> Result<(Vec<SegmentRow>, Option<Segment>)> {
  // Read together under the index's lock, which sealing holds until the
  // index has the sealed segment: never neither of the two.
  let index = ctx.index.lock().expect("index lock");
  let mut segments = index.thread_segments(thread)?;
  let open = ctx.root.open_segment(thread)?.map(|(s, _)| s);
  drop(index);
  match &open {
    // Sealed and indexed meanwhile: the index has it.
    Some(o) if segments.iter().any(|(id, _, _)| *id == o.header.segment) => {
      return Ok((segments, None));
    }
    Some(o) => segments.push((
      o.header.segment.clone(),
      o.header.prev.clone(),
      String::new(),
    )),
    None => {}
  }
  Ok((segments, open))
}

/// Seals a thread's open segment and indexes it, under the index's lock.
pub(crate) fn seal(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<RecordId>> {
  let mut index = ctx.index.lock().expect("index lock");
  let sealed = ctx.root.seal(thread)?;
  if sealed.is_some() {
    index.refresh(ctx.root)?;
  }
  Ok(sealed)
}

/// When a thread's latest step was written here (its open segment's last
/// step, else its latest sealed segment); `None` before its first.
pub(crate) fn last_step_at(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Option<Timestamp>> {
  let index = ctx.index.lock().expect("index lock");
  let sealed = index.last_segment_at(thread)?;
  let open = ctx.root.open_segment(thread)?.map(|(_, at)| at);
  Ok(sealed.max(open))
}

/// An assistant message's provider-native content as written: for
/// Anthropic, text blocks that are the message's own text become ranges of
/// it (`{"type": "text", "at": [from, to]}`, bytes) and tool calls that are
/// its step's calls become references (`{"type": "tool_use", "call":
/// id}`); `unpack_native` makes it whole again. The thinking blocks and
/// their signatures stay as they came. About a tenth of a transcript was
/// this copy.
fn pack_native(mut native: Value, text: &str, content: &[Content]) -> Value {
  if native.get("provider").and_then(Value::as_str) != Some("anthropic") {
    return native;
  }
  let Some(blocks) = native.get_mut("content").and_then(Value::as_array_mut) else {
    return native;
  };
  let mut at = 0usize;
  for b in blocks.iter_mut() {
    let packed = {
      let Some(obj) = b.as_object() else { continue };
      match obj.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" if obj.len() == 2 => obj
          .get("text")
          .and_then(Value::as_str)
          .filter(|t| text.get(at..).is_some_and(|rest| rest.starts_with(*t)))
          .map(|t| {
            let range = json!({"type": "text", "at": [at, at + t.len()]});
            at += t.len();
            range
          }),
        "tool_use" if obj.len() == 4 => {
          match (
            obj.get("id").and_then(Value::as_str),
            obj.get("name").and_then(Value::as_str),
            obj.get("input"),
          ) {
            (Some(id), Some(name), Some(input))
              if content.iter().any(|c| {
                matches!(c, Content::ToolCall { id: i, name: n, input: x }
                  if i == id && n == name && x == input)
              }) =>
            {
              Some(json!({"type": "tool_use", "call": id}))
            }
            _ => None,
          }
        }
        _ => None,
      }
    };
    if let Some(p) = packed {
      *b = p;
    }
  }
  native
}

/// The native content `pack_native` wrote, whole again: from the message's
/// text and its step's tool calls (`calls`, by ID).
fn unpack_native(
  mut native: Value,
  parts: &[Part],
  calls: &std::collections::HashMap<&str, &ToolCall>,
) -> Option<Value> {
  let text: String = parts
    .iter()
    .filter_map(|p| match p {
      Part::Text { text } => Some(text.as_str()),
      _ => None,
    })
    .collect();
  if let Some(blocks) = native.get_mut("content").and_then(Value::as_array_mut) {
    for b in blocks.iter_mut() {
      if let Some(at) = b.get("at").and_then(Value::as_array) {
        let (from, to) = (
          at.first()?.as_u64()? as usize,
          at.get(1)?.as_u64()? as usize,
        );
        *b = json!({"type": "text", "text": text.get(from..to)?});
      } else if let Some(id) = b.get("call").and_then(Value::as_str) {
        let c = calls.get(id)?;
        *b = json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.input});
      }
    }
  }
  Some(native)
}

/// Builds provider messages from transcript events: an assistant message
/// takes the tool calls that follow it; consecutive results form one user
/// message. A call left without a result (a loop that stopped mid-step)
/// gets an error result, so the history stays valid.
pub(crate) fn messages_from<'e>(
  events: impl IntoIterator<Item = &'e Event>,
  blobs: &BlobStore,
) -> Result<(Vec<Message>, usize)> {
  let events: Vec<&Event> = events.into_iter().collect();
  // The calls each packed native content names.
  let calls: std::collections::HashMap<&str, &ToolCall> = events
    .iter()
    .filter_map(|e| match e {
      Event::ToolCall(c) => Some((c.id.as_str(), c)),
      _ => None,
    })
    .collect();
  let mut messages: Vec<Message> = Vec::new();
  let mut pending: Vec<String> = Vec::new();
  let mut count = 0;
  let close_pending = |messages: &mut Vec<Message>, pending: &mut Vec<String>| {
    if pending.is_empty() {
      return;
    }
    let results = pending
      .drain(..)
      .map(|id| Content::ToolResult {
        id,
        output: "interrupted: no result was recorded".into(),
        is_error: true,
      })
      .collect();
    messages.push(Message::user(results));
  };
  let mut last_at = None;
  // Action buttons the user pressed that did not wake the model (design
  // §2.30): told with the user's next message.
  let mut presses: Vec<String> = Vec::new();
  // A shared conversation's reply (design §2.31): its calls ran on
  // another user's machines, so they and their results are text.
  let mut shared_turn = false;
  let mut shared_calls: std::collections::HashSet<&str> = Default::default();
  // Shared messages come after the import's note, or a fence of their own.
  let mut fenced = false;
  for e in events {
    {
      match e {
        Event::Msg(m) => {
          shared_turn = m.role == Role::Assistant && crate::share::is_shared(m.from.as_deref());
          close_pending(&mut messages, &mut pending);
          if m.from.as_deref() == Some(crate::share::NOTE_FROM) {
            fenced = true;
          } else if crate::share::is_shared(m.from.as_deref()) && !fenced {
            fenced = true;
            messages.push(Message::user(vec![Content::Text(
              crate::share::FENCE.into(),
            )]));
          }
          let mut content = parts_content(&m.parts, blobs)?;
          if let Some(note) = time_note_for(m, last_at) {
            content.insert(0, Content::Text(note));
          }
          if m.role == Role::User && !presses.is_empty() {
            content.insert(
              0,
              Content::Text(crate::actions::presses_note(&std::mem::take(&mut presses))),
            );
          }
          last_at = msg_time(m).or(last_at);
          messages.push(match m.role {
            Role::Assistant => Message {
              role: PRole::Assistant,
              content,
              native: m
                .native
                .clone()
                .and_then(|v| unpack_native(v, &m.parts, &calls))
                .and_then(|v| serde_json::from_value::<Native>(v).ok()),
            },
            Role::User | Role::System => Message::user(content),
          });
          count += 1;
        }
        Event::ToolCall(c) if shared_turn => {
          shared_calls.insert(c.id.as_str());
          let text = Content::Text(crate::share::call_text(c));
          match messages.last_mut() {
            Some(last) if last.role == PRole::Assistant => last.content.push(text),
            _ => messages.push(Message {
              role: PRole::Assistant,
              content: vec![text],
              native: None,
            }),
          }
        }
        Event::ToolResult(r) if shared_calls.contains(r.id.as_str()) => {
          let text = Content::Text(crate::share::result_text(r));
          match messages.last_mut() {
            Some(last) if last.role == PRole::User => last.content.push(text),
            _ => messages.push(Message::user(vec![text])),
          }
        }
        Event::ToolCall(c) => {
          if let Some(last) = messages.last_mut()
            && last.role == PRole::Assistant
          {
            last.content.push(Content::ToolCall {
              id: c.id.clone(),
              name: c.name.clone(),
              input: c.input.clone(),
            });
            pending.push(c.id.clone());
          }
        }
        Event::ToolResult(r) => {
          pending.retain(|id| id != &r.id);
          let content = Content::ToolResult {
            id: r.id.clone(),
            output: result_text(&r.output),
            is_error: !r.ok,
          };
          match messages.last_mut() {
            Some(last)
              if last.role == PRole::User
                && last
                  .content
                  .iter()
                  .all(|c| matches!(c, Content::ToolResult { .. })) =>
            {
              last.content.push(content)
            }
            _ => messages.push(Message::user(vec![content])),
          }
        }
        Event::Other(_) => {
          // One that woke it is the user's message itself.
          if let Some(p) = crate::actions::Press::of(e)
            && !p.wake
          {
            presses.push(p.noted());
          }
        }
        _ => {}
      }
    }
  }
  close_pending(&mut messages, &mut pending);
  // A model's turn that said and called nothing (a reply left empty, design
  // §2.30) is not sent: providers refuse a message with no content.
  messages.retain(|m| !said_nothing(m));
  Ok((messages, count))
}

/// The notes of the action buttons pressed after the last user message,
/// which did not wake the model (`messages_from` tells them with the next
/// one).
fn pending_presses<'e>(events: impl IntoIterator<Item = &'e Event>) -> Vec<String> {
  let mut out = Vec::new();
  for e in events {
    match e {
      Event::Msg(m) if m.role == Role::User => out.clear(),
      e => {
        if let Some(p) = crate::actions::Press::of(e)
          && !p.wake
        {
          out.push(p.noted());
        }
      }
    }
  }
  out
}

/// The conversation a spawned thread starts from, as events: its
/// parent's (and what that one continued, in turn), up to where it was
/// delegated. Empty for any other thread.
pub fn inherited(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Vec<Event>> {
  let Some(c) = ctx
    .root
    .read_record(thread)
    .ok()
    .and_then(|r| Continues::of(&r))
  else {
    return Ok(Vec::new());
  };
  inherited_from(ctx, &c)
}

/// Whether the conversation up to `c` is here.
pub fn inherited_from_ready(ctx: &ChatContext<'_>, c: &Continues) -> Result<()> {
  inherited_from(ctx, c).map(|_| ())
}

/// The conversation up to `c`, as events.
pub(crate) fn inherited_from(ctx: &ChatContext<'_>, c: &Continues) -> Result<Vec<Event>> {
  // What it continues was deleted (design §2.5): the conversation starts
  // here, with the summary standing for what came before, if it has one.
  if continues_deleted(ctx, c) {
    return Ok(shaped(Vec::new(), &c.shape));
  }
  let events = continued_events(ctx, c)?;
  let mut all = inherited(ctx, &c.thread)?;
  all.extend(events);
  Ok(shaped(without_context(all), &c.shape))
}

/// The events of the thread `c` continues, its own (not what it inherits),
/// up to where `c` leaves it, unshaped.
pub(crate) fn continued_events(ctx: &ChatContext<'_>, c: &Continues) -> Result<Vec<Event>> {
  let head = match &c.at {
    Upto::Segment(s) => Some(s),
    Upto::Call(_) => None,
  };
  let (segments, _) = chain_to(ctx, &c.thread, head)?;
  let mut events: Vec<Event> = segments.into_iter().flat_map(|s| s.events).collect();
  if let Upto::Call(call) = &c.at {
    // Up to the delegating call: the request and what led to it, not the
    // call itself (its result is the delegation).
    let cut = events
      .iter()
      .position(|e| matches!(e, Event::ToolCall(t) if &t.id == call))
      .ok_or_else(|| {
        Error::Config(format!(
          "the step of thread {} that delegated is not here yet",
          c.thread
        ))
      })?;
    events.truncate(cut);
    // The message that made the call is cut too, with the calls its step
    // made before it: what the spawner said as it delegated ("I'll hand
    // this to the Mac") read as the spawned thread's own intent, and it
    // delegated again (2026-10-19, `…mbp-grpx`). A line stands for it, so
    // the conversation still alternates; the spawned thread's first
    // message says who it is (`spawned_header`).
    if let Some(at) = events.iter().rposition(|e| matches!(e, Event::Msg(_)))
      && let Event::Msg(m) = &mut events[at]
      && m.role == Role::Assistant
    {
      m.native = None;
      m.parts = vec![Part::Text {
        text: "[Stitch: here this thread called `spawn` to start the thread that reads this; \
               what it said with the call is left out.]"
          .into(),
      }];
      events.truncate(at + 1);
    }
  }
  if let Some(m) = &c.shape.upto {
    // The message, and the tool calls and results that followed it.
    let at = events
      .iter()
      .position(|e| matches!(e, Event::Msg(x) if &x.id == m))
      .ok_or_else(|| Error::Config(format!("thread {} has no message {m}", c.thread)))?;
    let end = events[at + 1..]
      .iter()
      .position(|e| matches!(e, Event::Msg(_)))
      .map_or(events.len(), |n| at + 1 + n);
    events.truncate(end);
  }
  Ok(events)
}

/// Whether the thread `c` continues was deleted by the user.
fn continues_deleted(ctx: &ChatContext<'_>, c: &Continues) -> bool {
  ctx
    .index
    .lock()
    .expect("index lock")
    .is_deleted(&c.thread)
    .unwrap_or(false)
}

/// Writes into a thread what came for it when no turn runs to take it (a
/// report or a message for a thread past its budget, or cancelled): the
/// message as it would have been woven in, then `note`, Stitch's, as the
/// reply, so the thread shows it, the user's next message continues from
/// it, and a report so written counts as had (2026-10-19: a blocked report
/// to a thread past its deadline went only into that thread's own
/// over-budget report to its spawner, and the thread the user was talking
/// to never showed it). No model call.
pub(crate) fn write_unrun(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  from: &str,
  text: &str,
  note: &str,
) -> Result<RecordId> {
  let line = line_to(ctx, thread, None)?;
  let n = next_msg_n(ctx, thread, &line)?;
  let prev = line.segments.last().map(|s| s.header.segment.clone());
  let rewind = line.open.map(|r| (r.id, r.upto));
  let events = vec![
    Event::Msg(Msg {
      id: format!("m{}", n + 1),
      at: now(),
      role: Role::User,
      from: Some(from.to_owned()),
      model: None,
      parts: vec![Part::Text {
        text: text.to_owned(),
      }],
      native: None,
    }),
    Event::Msg(Msg {
      id: format!("m{}", n + 2),
      at: now(),
      role: Role::Assistant,
      from: None,
      model: None,
      parts: vec![Part::Text {
        text: note.to_owned(),
      }],
      native: None,
    }),
  ];
  let segment = ctx
    .root
    .append_step_from(ctx.node, thread, prev, rewind, events)?;
  seal(ctx, thread)?;
  ctx.index.lock().expect("index lock").refresh(ctx.root)?;
  Ok(segment)
}

/// Appends messages to a thread with no model call (a room's, design
/// §2.27): `build` gets the number of the thread's last message and makes
/// them; their IDs.
pub(crate) fn append_messages(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  build: impl FnOnce(usize) -> Vec<Msg>,
) -> Result<Vec<String>> {
  let line = line_to(ctx, thread, None)?;
  let n = next_msg_n(ctx, thread, &line)?;
  let msgs = build(n);
  let ids = msgs.iter().map(|m| m.id.clone()).collect();
  append_line(
    ctx,
    thread,
    line,
    msgs.into_iter().map(Event::Msg).collect(),
  )?;
  Ok(ids)
}

/// Appends events to a thread with no model call, as a step of their own
/// (a press of an action button, design §2.30). The caller holds the
/// thread's lock, so no turn writes beside it.
pub(crate) fn append_events(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  events: Vec<Event>,
) -> Result<()> {
  let line = line_to(ctx, thread, None)?;
  append_line(ctx, thread, line, events)
}

fn append_line(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  line: Line,
  events: Vec<Event>,
) -> Result<()> {
  let prev = line.segments.last().map(|s| s.header.segment.clone());
  let rewind = line.open.map(|r| (r.id, r.upto));
  ctx
    .root
    .append_step_from(ctx.node, thread, prev, rewind, events)?;
  seal(ctx, thread)?;
  ctx.index.lock().expect("index lock").refresh(ctx.root)?;
  Ok(())
}

/// A turn on `thread` that failed before its model call (`e`: its provider
/// not resolved), for a message no client keeps (a chat typed on another
/// node, a report or message that woke it): the message goes into the
/// thread with a `failed` event saying why, as `Turn::failed` writes a
/// failed model call, so it is not lost, the thread shows why on every
/// node, and the turn can be resumed (2026-11-10: an OAuth refresh that
/// failed on the Mac was only logged there, and the phone that followed
/// the chat showed nothing). `Error::Kept` with what the user is told;
/// `e` itself when nothing could be written.
pub(crate) fn failed_before_call(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  from: &str,
  text: &str,
  attachments: &[Attachment],
  e: Error,
) -> Error {
  let failed = crate::failed::before_call(&e);
  let said = crate::failed::shown(&failed);
  let written = (|| {
    let line = line_to(ctx, thread, None)?;
    let n = next_msg_n(ctx, thread, &line)?;
    let prev = line.segments.last().map(|s| s.header.segment.clone());
    let rewind = line.open.map(|r| (r.id, r.upto));
    let mut parts = Vec::new();
    if !text.is_empty() {
      parts.push(Part::Text {
        text: text.to_owned(),
      });
    }
    parts.extend(
      attachments
        .iter()
        .cloned()
        .map(|att| Part::Attachment { att }),
    );
    let events = vec![
      Event::Msg(Msg {
        id: format!("m{}", n + 1),
        at: now(),
        role: Role::User,
        from: Some(from.to_owned()),
        model: None,
        parts,
        native: None,
      }),
      failed,
    ];
    ctx
      .root
      .append_step_from(ctx.node, thread, prev, rewind, events)?;
    seal(ctx, thread)?;
    ctx.index.lock().expect("index lock").refresh(ctx.root)?;
    Ok::<_, Error>(())
  })();
  match written {
    Ok(()) => Error::Kept {
      said,
      source: Box::new(e),
    },
    Err(w) => {
      eprintln!("thread {thread}: the failed turn was not written: {w}");
      e
    }
  }
}

/// Writes `events` (a compaction's summary calls, as `usage`) as a step of
/// their own at the end of `thread`'s line, sealed: spent on it, so its
/// compaction chain's budget counts them (`detached::spent_by_chain`).
/// The caller holds the thread's lock.
pub(crate) fn write_spent(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  events: Vec<Event>,
) -> Result<()> {
  if events.is_empty() {
    return Ok(());
  }
  let line = line_to(ctx, thread, None)?;
  let prev = line.segments.last().map(|s| s.header.segment.clone());
  let rewind = line.open.map(|r| (r.id, r.upto));
  ctx
    .root
    .append_step_from(ctx.node, thread, prev, rewind, events)?;
  seal(ctx, thread)?;
  ctx.index.lock().expect("index lock").refresh(ctx.root)?;
  Ok(())
}

/// The conversation taken from another thread, without the notes of where
/// its turns ran and under which instructions (`Part::Context`): those
/// were that thread's, and the latest of them, read last, told a spawned
/// thread on the Mac that it ran on the phone with no shells, so it
/// spawned its own goal again (2026-10-19, `…mbp-grpx`). The thread that
/// takes the conversation has its own, in its prompt and its turns.
fn without_context(mut events: Vec<Event>) -> Vec<Event> {
  for e in &mut events {
    if let Event::Msg(m) = e {
      m.parts.retain(|p| !matches!(p, Part::Context { .. }));
    }
  }
  events.retain(|e| !matches!(e, Event::Msg(m) if m.role == Role::User && m.parts.is_empty()));
  events
}

/// What a spawned thread is told with its goal, in its first message
/// (after the goal, so it is read last): its own ID and node, who spawned
/// it, that a conversation it continues is its spawner's (which ended by
/// spawning it), and that the goal is its own to do (design §2.7, "Who a
/// spawned thread is").
pub fn spawned_header(
  me: &RecordId,
  node: &str,
  parent: &RecordId,
  continues: Option<&Continues>,
  no_tools: bool,
) -> String {
  let mut out = format!(
    "[Stitch: you are thread {me}, running on node {node}, spawned by thread {parent}, and the \
     goal above is yours. "
  );
  if let Some(c) = continues {
    let whose = if &c.thread == parent {
      "your spawner's".to_owned()
    } else {
      format!("thread {}'s", c.thread)
    };
    out.push_str(&format!(
      "The conversation before it is {whose}, copied so you have its context; it is not yours. \
       Its turns ran as that thread, on its node, with its tools: where they ran, what they \
       did and what they spawned are not yours. "
    ));
    if &c.thread == parent && matches!(c.at, Upto::Call(_)) {
      out.push_str(
        "It ends where your spawner called `spawn` to start you: the thread it started for \
         this goal is you, and nobody else is doing it. ",
      );
    }
  }
  if no_tools {
    out.push_str("Answer it yourself.]");
  } else {
    out.push_str(
      "Do it yourself, here, with your own tools. Do not spawn a thread to do it for you: it \
       was handed to you to do. Spawn only for a separate part of it, or for work another node \
       must do; if you cannot do it here, say so in your report (`Blocked:`).]",
    );
  }
  out
}

/// The conversation as a fork takes it (`Shape`).
pub fn shaped(mut events: Vec<Event>, shape: &Shape) -> Vec<Event> {
  // A summary of a shared conversation (design §2.31) is foreign too.
  let foreign = events
    .iter()
    .any(|e| matches!(e, Event::Msg(m) if crate::share::is_shared(m.from.as_deref())));
  if shape.no_tools {
    events.retain(|e| !matches!(e, Event::ToolCall(_) | Event::ToolResult(_)));
    for e in &mut events {
      if let Event::Msg(m) = e {
        // Its provider-native content holds the calls.
        m.native = None;
      }
    }
  }
  if shape.no_attachments {
    for e in &mut events {
      if let Event::Msg(m) = e {
        for p in &mut m.parts {
          if let Part::Attachment { att } = p {
            *p = Part::Text {
              text: format!(
                "[attachment left out of this fork: {} {}]",
                att.media_type,
                att.name.as_deref().unwrap_or("")
              ),
            };
          }
        }
        m.native = None;
      }
    }
  }
  if shape.no_tools || shape.no_attachments {
    events.retain(|e| !matches!(e, Event::Msg(m) if m.parts.iter().all(|p| matches!(p, Part::Text { text } if text.trim().is_empty()))));
  }
  let keep = match (shape.last, &shape.summary) {
    (Some(n), _) => Some(n),
    // The summary stands for all of it.
    (None, Some(_)) => Some(0),
    (None, None) => None,
  };
  if let Some(n) = keep {
    let msgs: Vec<usize> = events
      .iter()
      .enumerate()
      .filter(|(_, e)| matches!(e, Event::Msg(_)))
      .map(|(i, _)| i)
      .collect();
    let mut start = if n == 0 {
      events.len()
    } else if msgs.len() > n {
      msgs[msgs.len() - n]
    } else {
      0
    };
    // From a message of the user's: never a tool result or a reply whose
    // question is gone.
    while start < events.len() && !matches!(&events[start], Event::Msg(m) if m.role == Role::User) {
      start += 1;
    }
    events.drain(..start);
  }
  if let Some(summary) = &shape.summary {
    events.insert(
      0,
      Event::Msg(Msg {
        id: "summary".into(),
        at: now(),
        role: Role::User,
        from: None,
        model: None,
        parts: vec![Part::Text {
          text: format!(
            "{}[This conversation continues an earlier one. Summary of what came before{}:]\n\n{summary}",
            if foreign {
              format!("{}\n\n", crate::share::FENCE)
            } else {
              String::new()
            },
            if keep.unwrap_or(0) > 0 {
              " the messages that follow"
            } else {
              ""
            }
          ),
        }],
        native: None,
      }),
    );
  }
  events
}

pub fn history(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<History> {
  let line = line_to(ctx, thread, None)?;
  let msg_count = next_msg_n(ctx, thread, &line)?;
  let rewind = line.open.as_ref().map(|r| (r.id.clone(), r.upto.clone()));
  let (segments, forked) = (line.segments, line.forked);
  let before = inherited(ctx, thread)?;
  let mut attachments = Vec::new();
  for e in before.iter().chain(segments.iter().flat_map(|s| &s.events)) {
    if let Event::Msg(m) = e {
      for p in &m.parts {
        if let Part::Attachment { att } = p {
          attachments.push(att.clone());
        }
      }
    }
  }
  let missing = missing_blobs(ctx.blobs, &attachments);
  if !missing.is_empty() {
    return Err(Error::MissingBlobs(missing));
  }
  let context = segments
    .iter()
    .rev()
    .flat_map(|s| s.events.iter().rev())
    .filter_map(|e| match e {
      Event::Msg(m) => Some(m.parts.iter().rev()),
      _ => None,
    })
    .flatten()
    .find_map(|p| match p {
      Part::Context { context } => Some(context.clone()),
      _ => None,
    });
  let recalled = segments
    .iter()
    .flat_map(|s| &s.events)
    .filter_map(|e| match e {
      Event::Msg(m) => Some(&m.parts),
      _ => None,
    })
    .flatten()
    .filter_map(|p| match p {
      Part::Recalled { handles, .. } => Some(handles.iter().cloned()),
      _ => None,
    })
    .flatten()
    .collect();
  let own: Vec<&Event> = segments.iter().flat_map(|s| &s.events).collect();
  let last_at = last_msg_time(before.iter().chain(own.iter().copied()));
  let presses = pending_presses(before.iter().chain(own.iter().copied()));
  let (messages, _) = messages_from(before.iter().chain(own), ctx.blobs)?;
  Ok(History {
    messages,
    presses,
    last_at,
    last_segment: segments.last().map(|s| s.header.segment.clone()),
    msg_count,
    forked,
    attachments,
    context,
    recalled,
    forks: if forked {
      fork_note(ctx, thread)?
    } else {
      None
    },
    rewind,
  })
}

pub fn missing_blobs(blobs: &BlobStore, attachments: &[Attachment]) -> Vec<BlobHash> {
  let mut out: Vec<BlobHash> = attachments
    .iter()
    .map(|a| a.hash)
    .filter(|h| !blobs.contains(h))
    .collect();
  out.dedup();
  out
}

pub(crate) fn title_of(text: &str) -> String {
  let line = text
    .lines()
    .find(|l| !l.trim().is_empty())
    .unwrap_or("Chat")
    .trim();
  let mut t: String = line.chars().take(80).collect();
  if line.chars().count() > 80 {
    t.push('…');
  }
  t
}

fn now() -> Timestamp {
  stitch_records::millis(Timestamp::now())
}

fn add_usage(total: &mut Usage, u: &Usage) {
  total.input_tokens += u.input_tokens;
  total.output_tokens += u.output_tokens;
  total.cache_read_tokens += u.cache_read_tokens;
  total.cache_write_tokens += u.cache_write_tokens;
  total.cache_write_1h_tokens += u.cache_write_1h_tokens;
}

/// Sends a message and runs the loop until the model answers; streams what
/// happens into `on_event`. Each step is written as a segment; a new thread
/// is written with its first step. A failure before the model call writes
/// nothing; a model call that fails for good writes the step's message with
/// the failure (`Run::failed`), but for a request refused as it is.
pub async fn send(
  ctx: &ChatContext<'_>,
  input: ChatInput,
  tools: &dyn ToolRunner,
  on_event: &mut (dyn FnMut(ChatEvent) + Send),
) -> Result<ChatOutcome> {
  let (thread, system, new_thread) = match &input.thread {
    Some(t) => {
      let record = ctx.index.lock().expect("index lock").record(t)?;
      if !record.as_ref().is_some_and(|r| r.kind == kinds::THREAD) {
        return Err(Error::Config(format!("no thread {t} here")));
      }
      let system = system_of(ctx, &ctx.root.read_record(t)?)?;
      (t.clone(), system, false)
    }
    None => (
      input
        .id
        .clone()
        .unwrap_or_else(|| RecordId::random(Timestamp::now(), ctx.node)),
      input.system.clone(),
      true,
    ),
  };
  on_event(ChatEvent::Thread(thread.clone()));
  let missing = missing_blobs(ctx.blobs, &input.attachments);
  if !missing.is_empty() {
    return Err(Error::MissingBlobs(missing));
  }
  let history = if new_thread {
    // A spawned thread starts with the conversation it continues.
    let before = match input.detached.as_ref().and_then(|d| d.continues.as_ref()) {
      Some(c) => inherited_from(ctx, c)?,
      None => Vec::new(),
    };
    let attachments: Vec<Attachment> = before
      .iter()
      .filter_map(|e| match e {
        Event::Msg(m) => Some(&m.parts),
        _ => None,
      })
      .flatten()
      .filter_map(|p| match p {
        Part::Attachment { att } => Some(att.clone()),
        _ => None,
      })
      .collect();
    let missing = missing_blobs(ctx.blobs, &attachments);
    if !missing.is_empty() {
      return Err(Error::MissingBlobs(missing));
    }
    History {
      messages: messages_from(&before, ctx.blobs)?.0,
      last_at: last_msg_time(&before),
      last_segment: None,
      msg_count: 0,
      forked: false,
      attachments,
      context: None,
      recalled: Default::default(),
      forks: None,
      rewind: None,
      presses: Vec::new(),
    }
  } else {
    history(ctx, &thread)?
  };
  // Instructions and skills that changed since the thread last saw them
  // go before the message (its system prompt stays as it started).
  // What a fork left out goes with them.
  let with_forks = match (&input.context, &history.forks) {
    (Some(c), Some(f)) => Some(format!("{c}\n{f}")),
    (c, _) => c.clone(),
  };
  let changed = with_forks.as_ref().filter(|now| match &history.context {
    Some(last) => last != *now,
    None => !new_thread && !system.as_deref().unwrap_or("").contains(now.as_str()),
  });
  let mut user_content = Vec::new();
  let mut user_parts = Vec::new();
  if let Some(c) = changed {
    user_content.push(Content::Text(context_note(c)));
    user_parts.push(Part::Context { context: c.clone() });
  }
  if let Some((note, handles, measures)) = &input.recalled {
    user_content.push(Content::Text(recalled_note(note)));
    user_parts.push(Part::Recalled {
      recalled: note.clone(),
      handles: handles.clone(),
      measures: measures.clone(),
    });
  }
  // A spawned thread is told who it is with its goal (after it, read
  // last): the conversation it continues is its spawner's, which ended by
  // spawning it (design §2.7, "Who a spawned thread is").
  let header = match (&input.detached, new_thread) {
    (Some(d), true) => d.parent.as_ref().map(|p| {
      spawned_header(
        &thread,
        ctx.node.as_str(),
        p,
        d.continues.as_ref(),
        d.no_tools,
      )
    }),
    // One that stopped before its first step starts over from its goal.
    (_, false) if history.last_segment.is_none() => {
      ctx.root.read_record(&thread).ok().and_then(|r| {
        // A room's participant has no goal: the room's messages are its.
        if r.field_str("in_room").is_some() {
          return None;
        }
        let parent = r.refs.first("spawned-by")?.id.clone();
        Some(spawned_header(
          &thread,
          ctx.node.as_str(),
          &parent,
          Continues::of(&r).as_ref(),
          r.field_bool("tools") == Some(false),
        ))
      })
    }
    _ => None,
  };
  if !input.text.is_empty() {
    user_content.push(Content::Text(input.text.clone()));
  }
  // Right after the text, where `Part::Spoken` is written: the history
  // built again from the parts sends the same.
  let spoken = input.spoken && !input.text.is_empty();
  if spoken {
    user_content.push(Content::Text(SPOKEN_NOTE.into()));
  }
  // Where `Part::Scope` is written, after the text (and the spoken note):
  // replayed from the part as the same text.
  if let Some(sc) = &input.scope {
    user_content.push(Content::Text(sc.briefing.clone()));
  }
  // Where `Part::RoomTurn` is written, after the scope: its note, as
  // `parts_content` renders it from the part.
  let room_turn = input
    .room_turn
    .clone()
    .filter(|p| matches!(p, Part::RoomTurn { .. }));
  if let Some(p) = &room_turn {
    user_content.extend(parts_content(std::slice::from_ref(p), ctx.blobs)?);
  }
  if let Some(h) = &header {
    user_content.push(Content::Text(h.clone()));
  }
  if let Some(i) = &input.instruction {
    user_content.push(Content::Text(i.clone()));
  }
  for a in &input.attachments {
    user_content.extend(attachment_content(ctx.blobs, a)?);
  }
  // Its time note, from the same time it is written with, so the history
  // built from it at the next step reads the same.
  let at = now();
  if input.from.as_deref() != Some("tool")
    && let Some(note) = time_note(history.last_at, at)
  {
    user_content.insert(0, Content::Text(note));
  }
  // Buttons pressed since, as `messages_from` puts them when the history
  // is built again.
  if !history.presses.is_empty() {
    user_content.insert(
      0,
      Content::Text(crate::actions::presses_note(&history.presses)),
    );
  }
  let mut messages = history.messages;
  messages.push(Message::user(user_content));
  if !input.text.is_empty() {
    user_parts.push(Part::Text {
      text: input.text.clone(),
    });
  }
  if spoken {
    user_parts.push(Part::Spoken {
      spoken: true,
      note: SPOKEN_NOTE_NUMBER,
    });
  }
  if let Some(sc) = &input.scope {
    user_parts.push(sc.part());
  }
  if let Some(p) = room_turn {
    user_parts.push(p);
  }
  if let Some(h) = header {
    user_parts.push(Part::Text { text: h });
  }
  user_parts.extend(
    input
      .attachments
      .iter()
      .cloned()
      .map(|att| Part::Attachment { att }),
  );
  let mut msg_n = history.msg_count;
  // The user's message goes into the first step's segment.
  msg_n += 1;
  let events = vec![Event::Msg(Msg {
    id: format!("m{msg_n}"),
    at,
    role: Role::User,
    from: Some(
      input
        .from
        .clone()
        .unwrap_or_else(|| format!("user@{}", ctx.node)),
    ),
    model: None,
    parts: user_parts,
    native: None,
  })];
  let new_record = if new_thread {
    let mut draft = Draft::new(kinds::THREAD, Author::user(ctx.node.clone()))?
      .body(format!("# {}\n", title_of(&input.text)));
    if let Some(s) = &system {
      draft = with_system(ctx.blobs, draft, s)?;
    }
    if let Some((node, path)) = &input.folder {
      let mut f = toml::Table::new();
      f.insert("node".into(), node.as_str().into());
      f.insert("path".into(), path.as_str().into());
      draft = draft.field("folder", toml::Value::Table(f))?;
    }
    if let Some(d) = &input.detached {
      draft = draft
        .field("detached", true)?
        .field("goal", input.text.as_str())?;
      if d.no_tools {
        draft = draft.field("tools", false)?;
      }
      if d.keep {
        draft = draft.field("wake", false)?;
      }
      if d.fixer {
        draft = draft.field("fixer", true)?;
      }
      if d.read_only {
        draft = draft.field("approval", "read-only")?;
      }
      if let Some((t, name)) = &d.app {
        draft = draft
          .field("app", t.as_str())?
          .field("app_name", name.as_str())?;
      }
      if let Some(b) = d.budget.to_toml() {
        draft = draft.field("budget", b)?;
      }
      if let Some(m) = &d.model {
        draft = draft.field("model", m.as_str())?;
      }
      if let Some(c) = &d.cwd {
        draft = draft.field("cwd", c.as_str())?;
      }
      if let Some(p) = &d.parent {
        draft = draft
          .reference("spawned-by", stitch_records::Ref::to(p.clone()))?
          .reference("reports-to", stitch_records::Ref::to(p.clone()))?;
      } else if let Some(to) = &d.reports_to {
        draft = draft.reference("reports-to", stitch_records::Ref::to(to.clone()))?;
      }
      if let Some(sc) = &d.scheduled_by {
        draft = draft.reference("scheduled-by", stitch_records::Ref::to(sc.clone()))?;
      }
      if let Some(a) = &d.assignment {
        draft = draft.reference("assignment", stitch_records::Ref::to(a.clone()))?;
      }
      if let Some(c) = &d.continues {
        draft = c.write(draft)?;
      }
    }
    let record = draft.into_record(thread.clone(), Timestamp::now());
    if input.detached.is_some() {
      // A detached thread exists from the start, so a failure before its
      // first step still has a thread to report on.
      ctx.root.publish_record(&record)?;
      ctx.index.lock().expect("index lock").refresh(ctx.root)?;
      None
    } else {
      Some(record)
    }
  } else {
    None
  };
  let run = Run {
    ctx,
    thread: thread.clone(),
    system,
    provider: &input.provider,
    model: &input.model,
    tools,
    limits: input.limits,
    new_record,
    prev: history.last_segment,
    rewind: history.rewind,
    msg_n,
    last_at: Some(at),
    forked: history.forked,
    steer: input.steer.clone(),
    replay: None,
  };
  run.steps(messages, events, on_event).await
}

/// Continues a thread whose loop stopped (a node restarted mid-loop):
/// `None` when its last message is already an answer.
#[allow(clippy::too_many_arguments)]
pub async fn resume(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  provider: &Provider,
  model: &str,
  limits: Limits,
  tools: &dyn ToolRunner,
  steer: Option<Steer>,
  on_event: &mut (dyn FnMut(ChatEvent) + Send),
) -> Result<Option<ChatOutcome>> {
  let history = history(ctx, thread)?;
  // A step whose calls were running when the loop stopped (waiting on an
  // approval, say): replayed as it was, its question with it, rather than
  // asked of the model again (which may ask something else, and the
  // user's answer to the first question would be lost).
  let replay = pending_step(ctx, thread)
    .filter(|p| p.prev == history.last_segment)
    .map(|p| p.replay(thread, ctx.blobs))
    .transpose()?;
  let answered = history.messages.last().is_some_and(|m| {
    m.role == PRole::Assistant
      && !m
        .content
        .iter()
        .any(|c| matches!(c, Content::ToolCall { .. }))
  });
  if replay.is_none() {
    // One left from an older line of the thread is moot.
    clear_pending(ctx, thread);
    if answered || history.messages.is_empty() {
      return Ok(None);
    }
  }
  let system = system_of(ctx, &ctx.root.read_record(thread)?)?;
  let run = Run {
    ctx,
    thread: thread.clone(),
    system,
    provider,
    model,
    tools,
    limits,
    new_record: None,
    prev: history.last_segment,
    rewind: history.rewind,
    msg_n: history.msg_count,
    last_at: history.last_at,
    forked: history.forked,
    steer,
    replay,
  };
  run
    .steps(history.messages, Vec::new(), on_event)
    .await
    .map(Some)
}

/// One loop over a thread: model calls and tool calls until it answers.
struct Run<'a> {
  ctx: &'a ChatContext<'a>,
  thread: RecordId,
  system: Option<String>,
  provider: &'a Provider,
  model: &'a str,
  tools: &'a dyn ToolRunner,
  limits: Limits,
  /// A new thread's record, written with its first step.
  new_record: Option<stitch_records::Record>,
  prev: Option<RecordId>,
  /// The rewind its first segment starts a branch from.
  rewind: Option<(RecordId, Option<String>)>,
  msg_n: usize,
  /// When the latest message was written: a message woven in after a
  /// pause is told the time, as the history built later tells it.
  last_at: Option<Timestamp>,
  forked: bool,
  steer: Option<Steer>,
  /// A step to replay first instead of asking the model (`resume`).
  replay: Option<Replay>,
}

/// A step written only once its calls ran, kept meanwhile for this node
/// alone (the index's local values): what the step holds so far, from
/// which segment. A loop stopped mid-step (the node stopped, crashed or
/// its machine rebooted, while a call waited on the user's approval) is
/// resumed from it: the same calls, the same questions, so an answer
/// given meanwhile still counts.
pub(crate) struct PendingStep {
  pub(crate) prev: Option<RecordId>,
  msg_n: usize,
  events: Vec<Event>,
  /// The call that waited on the user when the loop stopped
  /// (`waiting_on_user`).
  waiting: Option<String>,
  /// The call replayed after an earlier stop (`Replay::replaying`).
  replayed: Option<String>,
}

/// A pending step made ready to run again: its events, the messages they
/// add (up to the model's reply), its calls and the results some had.
struct Replay {
  events: Vec<Event>,
  msg_n: usize,
  messages: Vec<Message>,
  model: String,
  calls: Vec<(String, String, Value)>,
  done: std::collections::HashMap<String, (bool, String)>,
  /// The call the loop stopped in while it executed, run again now: kept
  /// as replayed (`keep_pending`), so a stop in it again does not replay
  /// it a third time.
  replaying: Option<String>,
}

/// The result of a call the node stopped in twice, the second time as it
/// was replayed: not run a third time (2026-11-02, n6rx: a call that
/// killed its node was replayed on every start, in a loop).
const STOPPED_TWICE: &str = "not run again: the node stopped while this call ran, twice (it may \
                             be what stops the node); call it again only if you are sure it is \
                             safe";

impl PendingStep {
  fn replay(mut self, thread: &RecordId, blobs: &BlobStore) -> Result<Replay> {
    // The call the loop stopped in: the first without a result (calls run
    // in turn). Whether the node was killed or stopped cleanly (SIGTERM,
    // a Stop: the loop's future dropped clears `running`), one that
    // waited on the user is replayed as often as it takes, its question
    // with it; one that executed is replayed once, and the node stopping
    // in it again, it gets an error result instead (it may be what stops
    // the node: issue n6rx).
    let unresolved = self.events.iter().find_map(|e| match e {
      Event::ToolCall(c)
        if !self
          .events
          .iter()
          .any(|r| matches!(r, Event::ToolResult(r) if r.id == c.id)) =>
      {
        Some(c.id.clone())
      }
      _ => None,
    });
    let mut replaying = unresolved.filter(|id| self.waiting.as_ref() != Some(id));
    if let Some(id) = &replaying
      && self.replayed.as_ref() == Some(id)
    {
      let name = self
        .events
        .iter()
        .find_map(|e| match e {
          Event::ToolCall(c) if c.id == *id => Some(c.name.as_str()),
          _ => None,
        })
        .unwrap_or("?");
      eprintln!(
        "thread {thread}: call {id} ({name}) not run again: the node stopped while it ran, twice"
      );
      self.events.push(Event::ToolResult(ToolResult {
        id: id.clone(),
        at: now(),
        ok: false,
        output: json!({"text": STOPPED_TWICE}),
      }));
      replaying = None;
    }
    let unanswered: Vec<&Event> = self
      .events
      .iter()
      .filter(|e| !matches!(e, Event::ToolResult(_)))
      .collect();
    let (mut messages, _) = messages_from(unanswered, blobs)?;
    // The results `messages_from` gives calls without one: run instead.
    if messages.last().is_some_and(|m| {
      m.role == PRole::User
        && m
          .content
          .iter()
          .all(|c| matches!(c, Content::ToolResult { .. }))
    }) {
      messages.pop();
    }
    let model = self
      .events
      .iter()
      .rev()
      .find_map(|e| match e {
        Event::Msg(m) if m.role == Role::Assistant => m.model.clone(),
        _ => None,
      })
      .unwrap_or_default();
    let calls = self
      .events
      .iter()
      .filter_map(|e| match e {
        Event::ToolCall(c) => Some((c.id.clone(), c.name.clone(), c.input.clone())),
        _ => None,
      })
      .collect();
    let done = self
      .events
      .iter()
      .filter_map(|e| match e {
        Event::ToolResult(r) => Some((
          r.id.clone(),
          (
            r.ok,
            r.output
              .get("text")
              .and_then(Value::as_str)
              .unwrap_or_default()
              .to_owned(),
          ),
        )),
        _ => None,
      })
      .collect();
    Ok(Replay {
      events: self.events,
      msg_n: self.msg_n,
      messages,
      model,
      calls,
      done,
      replaying,
    })
  }
}

fn pending_key(thread: &RecordId) -> String {
  format!("pending-step:{thread}")
}

/// The step of `thread` its loop had not written when it stopped.
pub(crate) fn pending_step(ctx: &ChatContext<'_>, thread: &RecordId) -> Option<PendingStep> {
  let text = ctx
    .index
    .lock()
    .expect("index lock")
    .local(&pending_key(thread))
    .ok()
    .flatten()?;
  let v: Value = serde_json::from_str(&text).ok()?;
  let prev = match v.get("prev") {
    Some(Value::String(p)) => Some(p.parse().ok()?),
    _ => None,
  };
  let msg_n = v.get("msg_n")?.as_u64()? as usize;
  let id_at = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
  let events = v
    .get("events")?
    .as_array()?
    .iter()
    .map(event_of)
    .collect::<Option<Vec<_>>>()?;
  Some(PendingStep {
    prev,
    msg_n,
    events,
    waiting: id_at("/waiting"),
    replayed: id_at("/replayed"),
  })
}

/// An event from its JSON (`Event::to_value`'s), as segments read it.
fn event_of(v: &Value) -> Option<Event> {
  let mut obj = v.as_object()?.clone();
  let tag = obj.remove("t")?;
  let body = Value::Object(obj);
  Some(match tag.as_str()? {
    "msg" => Event::Msg(serde_json::from_value(body).ok()?),
    "tool_call" => Event::ToolCall(serde_json::from_value(body).ok()?),
    "tool_result" => Event::ToolResult(serde_json::from_value(body).ok()?),
    "usage" => Event::Usage(serde_json::from_value(body).ok()?),
    _ => Event::Other(v.clone()),
  })
}

/// Keeps the step in progress (`pending_step`); `running`: the call that
/// starts now, which clients following the thread show in flight
/// (`step_in_flight`), with this process's ID to tell a live loop's from
/// one a stopped node left behind; `replayed`: the call of a replayed
/// step that was running when the node stopped (`Replay::replaying`).
fn keep_pending(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  prev: Option<&RecordId>,
  msg_n: usize,
  events: &[Event],
  running: Option<&str>,
  replayed: Option<&str>,
) {
  let mut v = json!({
    "prev": prev.map(|p| p.to_string()),
    "msg_n": msg_n,
    "events": events.iter().map(Event::to_value).collect::<Vec<_>>(),
    "pid": std::process::id(),
  });
  if let Some(id) = running {
    v["running"] = json!({"id": id, "since": now().to_string()});
  }
  if let Some(id) = replayed {
    v["replayed"] = json!(id);
  }
  let kept = ctx
    .index
    .lock()
    .expect("index lock")
    .set_local(&pending_key(thread), Some(&v.to_string()));
  if let Err(e) = kept {
    eprintln!("thread {thread}: its step in progress not kept: {e}");
  }
}

/// The call `thread`'s loop runs waits on the user (an approval asked, a
/// `wait`): marked in its kept step while it waits (`on`), unmarked when
/// the wait ends. Cleared only so, never by a guard: a node stopping
/// during the wait, cleanly or not, leaves the mark, and the call is
/// replayed as often as it stops there (`PendingStep::replay`).
pub(crate) fn waiting_on_user(index: &Mutex<Index>, thread: &RecordId, on: bool) {
  let index = index.lock().unwrap_or_else(|e| e.into_inner());
  let key = pending_key(thread);
  let Some(mut v) = index
    .local(&key)
    .ok()
    .flatten()
    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
  else {
    return;
  };
  let running = v
    .pointer("/running/id")
    .and_then(Value::as_str)
    .map(str::to_owned);
  let Some(o) = v.as_object_mut() else {
    return;
  };
  match (on, running) {
    (true, Some(id)) => {
      o.insert("waiting".into(), json!(id));
    }
    (false, _) if o.remove("waiting").is_some() => {}
    _ => return,
  }
  let _ = index.set_local(&key, Some(&v.to_string()));
}

/// A call kept as running (`keep_pending`'s `running`) until dropped:
/// however its loop ends (an error, a cancel dropping the loop's future),
/// the call is no longer shown in flight. The step itself stays kept, for
/// `resume`.
struct InFlightGuard<'a, 'b> {
  ctx: &'a ChatContext<'b>,
  thread: &'a RecordId,
  id: String,
}

impl Drop for InFlightGuard<'_, '_> {
  fn drop(&mut self) {
    let index = self.ctx.index.lock().unwrap_or_else(|e| e.into_inner());
    let key = pending_key(self.thread);
    let Some(mut v) = index
      .local(&key)
      .ok()
      .flatten()
      .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
      return;
    };
    let ours = v
      .pointer("/running/id")
      .and_then(Value::as_str)
      .is_some_and(|id| id == self.id);
    if ours && let Some(o) = v.as_object_mut() {
      o.remove("running");
      let _ = index.set_local(&key, Some(&v.to_string()));
    }
  }
}

/// A tool call of a step whose calls run now (`step_in_flight`).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CallInFlight {
  pub id: String,
  pub name: String,
  /// Its input, whole.
  #[cfg_attr(feature = "ts", ts(type = "unknown"))]
  pub input: Value,
  /// One line, as a turn's own client shows its calls (`call_summary`:
  /// `[deploy] $ cargo build`).
  pub summary: String,
  /// When it began running (only the running call has it).
  #[cfg_attr(feature = "ts", ts(type = "string | null"))]
  pub since: Option<Timestamp>,
  /// Its result's outcome, once it had one.
  pub ok: Option<bool>,
}

/// The step a loop runs the calls of, not written yet (a step is written
/// whole, once its calls returned): what clients following the thread
/// show meanwhile, its running call with a spinner and its time.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StepInFlight {
  pub calls: Vec<CallInFlight>,
  /// The call running now, an index into `calls`.
  pub running: Option<usize>,
}

impl StepInFlight {
  pub fn running(&self) -> Option<&CallInFlight> {
    self.calls.get(self.running?)
  }

  /// As `Request::InFlight` gives it.
  pub fn from_json(text: &str) -> Option<StepInFlight> {
    serde_json::from_str(text).ok()
  }
}

impl CallInFlight {
  /// What runs, in a few words: `shell_run in `deploy` on devbox-1`.
  pub fn headline(&self) -> String {
    let mut line = self.name.clone();
    let field = |k: &str| self.input.get(k).and_then(Value::as_str);
    if self.name.starts_with("shell_") || self.name == "job_stop" {
      if let Some(s) = field("session") {
        line.push_str(&format!(" in `{s}`"));
      }
      if let Some(h) = field("host") {
        line.push_str(&format!(" on {h}"));
      }
    }
    line
  }
}

/// The events of the step `thread`'s loop runs the calls of now (its
/// reply, its calls, the results so far, what was woven in), kept by
/// `keep_pending` until the step is written: what a client opening the
/// thread mid-turn shows of it. Only a live loop's (`pid`, as in
/// `step_in_flight`): a step a stopped node left behind is not shown.
pub fn step_in_progress(index: &Mutex<Index>, thread: &RecordId, pid: u32) -> Vec<Event> {
  let Some(v) = index
    .lock()
    .expect("index lock")
    .local(&pending_key(thread))
    .ok()
    .flatten()
    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
  else {
    return Vec::new();
  };
  if v.get("pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
    return Vec::new();
  }
  v.get("events")
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(event_of).collect())
    .unwrap_or_default()
}

/// The step `thread`'s loop runs the calls of now, if one does: read from
/// what the loop keeps (`keep_pending`), the calls and which one runs.
/// `pid`: the process that runs the node (`crate::node_pid`); a step kept
/// by another process (a node that stopped, crashed) is not in flight.
pub fn step_in_flight(
  index: &Mutex<Index>,
  thread: &RecordId,
  pid: Option<u32>,
) -> Option<StepInFlight> {
  let text = index
    .lock()
    .expect("index lock")
    .local(&pending_key(thread))
    .ok()
    .flatten()?;
  let v: Value = serde_json::from_str(&text).ok()?;
  if v.get("pid").and_then(Value::as_u64) != Some(u64::from(pid?)) {
    return None;
  }
  let running = v.get("running")?;
  let running_id = running.get("id")?.as_str()?;
  let since: Option<Timestamp> = running
    .get("since")
    .and_then(Value::as_str)
    .and_then(|s| s.parse().ok());
  let events: Vec<Event> = v
    .get("events")?
    .as_array()?
    .iter()
    .filter_map(event_of)
    .collect();
  let mut calls: Vec<CallInFlight> = events
    .iter()
    .filter_map(|e| match e {
      Event::ToolCall(c) => Some(CallInFlight {
        id: c.id.clone(),
        name: c.name.clone(),
        summary: call_summary(&c.name, &c.input),
        input: c.input.clone(),
        since: None,
        ok: None,
      }),
      _ => None,
    })
    .collect();
  for e in &events {
    if let Event::ToolResult(r) = e
      && let Some(c) = calls.iter_mut().find(|c| c.id == r.id)
    {
      c.ok = Some(r.ok);
    }
  }
  let at = calls.iter().position(|c| c.id == running_id)?;
  calls[at].since = since;
  Some(StepInFlight {
    calls,
    running: Some(at),
  })
}

fn clear_pending(ctx: &ChatContext<'_>, thread: &RecordId) {
  let _ = ctx
    .index
    .lock()
    .expect("index lock")
    .set_local(&pending_key(thread), None);
}

impl Run<'_> {
  /// The loop's steps; the turn's open segment is sealed when they end,
  /// however they end (one aborted is sealed by the next sync, or when the
  /// node starts again).
  async fn steps(
    self,
    messages: Vec<Message>,
    events: Vec<Event>,
    on_event: &mut (dyn FnMut(ChatEvent) + Send),
  ) -> Result<ChatOutcome> {
    let (ctx, thread) = (self.ctx, self.thread.clone());
    let outcome = self.run_steps(messages, events, on_event).await;
    let sealed = seal(ctx, &thread);
    match (outcome, sealed) {
      (Err(e), _) => Err(e),
      (Ok(_), Err(e)) => Err(e),
      (Ok(o), Ok(_)) => Ok(o),
    }
  }

  /// A message into the turn (woven in, or Stitch's own), as the model
  /// reads it (its time note after a pause) and as it is written.
  fn user_msg(&mut self, from: String, parts: Vec<Part>) -> Result<(Message, Event)> {
    let at = now();
    let mut content = parts_content(&parts, self.ctx.blobs)?;
    let msg = Msg {
      id: String::new(),
      at,
      role: Role::User,
      from: Some(from),
      model: None,
      parts,
      native: None,
    };
    if let Some(note) = time_note_for(&msg, self.last_at) {
      content.insert(0, Content::Text(note));
    }
    self.last_at = Some(at);
    self.msg_n += 1;
    let msg = Msg {
      id: format!("m{}", self.msg_n),
      ..msg
    };
    Ok((Message::user(content), Event::Msg(msg)))
  }

  async fn run_steps(
    mut self,
    mut messages: Vec<Message>,
    mut events: Vec<Event>,
    on_event: &mut (dyn FnMut(ChatEvent) + Send),
  ) -> Result<ChatOutcome> {
    let ctx = self.ctx;
    let mut total = Usage::default();
    let mut usd_total: Option<f64> = None;
    let mut last_segment: Option<RecordId>;
    let mut steps = 0;
    // The steps this turn may take: more when the user grants them; at the
    // limit without them, one last step to wrap up (`wrapping`).
    let per_grant = self.limits.steps.unwrap_or(MAX_STEPS);
    let mut allowed = per_grant;
    let mut wrapping = false;
    let (mut calls_made, mut calls_failed) = (0usize, 0usize);
    // Whether the step that made the last calls said something, and whether
    // the model was told its turn had no reply (`NO_REPLY`, once a turn).
    let (mut said_with_calls, mut told_no_reply) = (true, false);
    let mut end = End::Answered;
    let (model, stop, text) = loop {
      steps += 1;
      // The tools may stop the loop before any model call (an app's
      // thread past its grant or its family's money, design §2.22).
      if let Some(why) = self.tools.admit(&self.thread).await {
        on_event(ChatEvent::Note(format!("[stopped: {why}]")));
        events.push(Event::Other(json!({
          "t": "stopped",
          "at": now().to_string(),
          "why": why,
        })));
        last_segment = self.write_step(std::mem::take(&mut events))?;
        end = End::OverBudget(why.clone());
        break (self.model.to_owned(), StopReason::Other(why), String::new());
      }
      // The call of a replayed step that ran when the node stopped.
      let mut replayed: Option<String> = None;
      let (response, calls, text, done) = if let Some(replay) = self.replay.take() {
        // The step the loop stopped in, as it was: no model call, its
        // calls run again (those that had their result keep it).
        tokio::select! {
          () = self.tools.prepare() => {}
          () = stopped(self.steer.as_ref()) => {}
        }
        events = replay.events;
        replayed = replay.replaying;
        self.msg_n = replay.msg_n;
        self.last_at = last_msg_time(&events).or(self.last_at);
        messages.extend(replay.messages);
        let content = messages
          .last()
          .filter(|m| m.role == PRole::Assistant)
          .map(|m| m.content.clone())
          .unwrap_or_default();
        let native = messages.last().and_then(|m| m.native.clone());
        let response = stitch_providers::Response {
          model: if replay.model.is_empty() {
            self.model.to_owned()
          } else {
            replay.model
          },
          content,
          native,
          stop: StopReason::ToolUse,
          usage: Usage::default(),
          cost: None,
        };
        let text = response.text();
        (response, replay.calls, text, replay.done)
      } else {
        let mut request = Request::new(self.model.to_owned(), messages.clone());
        request.system = self.system.clone();
        request.cache_key = Some(self.thread.to_string());
        // Servers the tools need start here; a stop cuts that short, and the
        // model call below sees it.
        tokio::select! {
          () = self.tools.prepare() => {}
          () = stopped(self.steer.as_ref()) => {}
        }
        request.tools = self.tools.definitions();
        // A call the network cut off (a dropped connection, a stream that
        // ended early: a phone switching networks, an app suspended and
        // woken) is sent again; nothing of it was written.
        let mut retries = NETWORK_RETRIES.iter();
        // A provider that refuses the images as too large for a request with
        // many of them, naming its limit, gets them fitted to it, once.
        let mut fitted_to: Option<u32> = None;
        let response = loop {
          let streamed = {
            let mut on_provider = |e: stitch_providers::Event| match e {
              stitch_providers::Event::Text(t) => on_event(ChatEvent::Text(t)),
              stitch_providers::Event::Thinking(t) => on_event(ChatEvent::Thinking(t)),
              _ => {}
            };
            tokio::select! {
              r = self.provider.stream(&request, &mut on_provider) => Some(r),
              () = stopped(self.steer.as_ref()) => None,
            }
          };
          match streamed {
            None => break None,
            Some(Ok(r)) => break Some(r),
            Some(Err(e)) if e.is_network() => {
              let Some(wait) = retries.next() else {
                return Err(self.failed(e, std::mem::take(&mut events), steps == 1));
              };
              on_event(ChatEvent::Retry(e.to_string()));
              tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_secs(*wait)) => {}
                () = stopped(self.steer.as_ref()) => break None,
              }
            }
            Some(Err(e)) => {
              let side = match &e {
                stitch_providers::Error::Api {
                  status: 400,
                  message,
                  ..
                } if fitted_to.is_none() => crate::images::many_image_limit(message),
                _ => None,
              };
              let Some(side) = side else {
                return Err(self.failed(e, std::mem::take(&mut events), steps == 1));
              };
              fitted_to = Some(side);
              // The loop's messages, which the next steps send again.
              crate::images::clamp(&mut messages, side);
              request.messages = messages.clone();
            }
          }
        };
        let Some(response) = response else {
          // Stopped while the model wrote: what it wrote is not kept.
          let suspended = self.steer.as_ref().is_some_and(Steer::suspended);
          events.push(interrupted(suspended));
          last_segment = self.write_step(std::mem::take(&mut events))?;
          end = End::Interrupted;
          let why = if suspended { SUSPENDED } else { "interrupted" };
          break (
            self.model.to_owned(),
            StopReason::Other(why.into()),
            String::new(),
          );
        };
        add_usage(&mut total, &response.usage);
        // What the provider billed (OpenRouter says), else Claude's API
        // at its list prices.
        let usd = response.cost.or_else(|| {
          (self.provider.kind() == "anthropic" && !self.provider.is_subscription())
            .then(|| pricing::estimate(&response.model, &response.usage))
            .flatten()
        });
        if let Some(u) = usd {
          *usd_total.get_or_insert(0.0) += u;
        }
        // The provider's name for the model may not be priced where the one
        // asked for is: then the one asked for (a tool's spend has no such
        // fallback).
        // A provider that bills no tokens (Antigravity's plan) is not
        // priced as the model's own maker would be.
        let priced_as = if !self.provider.metered()
          || pricing::priced(&response.model, &response.usage, usd).is_some()
        {
          response.model.as_str()
        } else {
          self.model
        };
        self
          .tools
          .metered(&self.thread, priced_as, &response.usage, usd);
        let mut calls = Vec::new();
        let text = response.text();
        if let StopReason::Refusal { category } = &response.stop {
          // A declined reply is not part of the conversation; the decline is.
          events.push(Event::Other(json!({
            "t": "refusal",
            "at": now().to_string(),
            "model": response.model,
            "category": category,
          })));
        } else {
          self.msg_n += 1;
          let mut parts = if text.is_empty() {
            Vec::new()
          } else {
            vec![Part::Text { text: text.clone() }]
          };
          // Images the model made (Gemini's image models) are the reply's
          // attachments, as sent: the user gets them as files, and they
          // sync and replay like the user's own.
          parts.extend(generated_images(ctx.blobs, &response.content, self.msg_n)?);
          let at = now();
          self.last_at = Some(at);
          events.push(Event::Msg(Msg {
            id: format!("m{}", self.msg_n),
            at,
            role: Role::Assistant,
            from: None,
            model: Some(response.model.clone()),
            parts,
            native: response.native.as_ref().map(|n| {
              pack_native(
                serde_json::to_value(n).unwrap_or(Value::Null),
                &text,
                &response.content,
              )
            }),
          }));
          for c in &response.content {
            if let Content::ToolCall { id, name, input } = c {
              events.push(Event::ToolCall(ToolCall {
                id: id.clone(),
                at: now(),
                name: name.clone(),
                input: input.clone(),
              }));
              calls.push((id.clone(), name.clone(), input.clone()));
            }
          }
          messages.push(Message {
            role: PRole::Assistant,
            content: response.content.clone(),
            native: response.native.clone(),
          });
        }
        events.push(Event::Usage(stitch_records::Usage {
          at: now(),
          model: response.model.clone(),
          input_tokens: response.usage.input_tokens,
          output_tokens: response.usage.output_tokens,
          cache_read_tokens: Some(response.usage.cache_read_tokens),
          cache_write_tokens: Some(response.usage.cache_write_tokens),
          usd,
        }));
        // A new thread exists once the model answered, before its calls run:
        // a call may start a thread that reports to it.
        if let Some(record) = self.new_record.take() {
          ctx.root.publish_record(&record)?;
          ctx.index.lock().expect("index lock").refresh(ctx.root)?;
        }
        (response, calls, text, std::collections::HashMap::new())
      };
      // The calls run before the step is written, so a step is whole; what
      // the user wrote in it is readable meanwhile (`unwritten_user_texts`).
      let _unwritten = Unwritten::set(&self.thread, &events);
      let mut results = Vec::new();
      let mut tool_images = Vec::new();
      let mut made: Vec<Attachment> = Vec::new();
      let mut cut = false;
      // Kept until the step is written: a loop stopped meanwhile resumes
      // from it (`resume`).
      if !calls.is_empty() {
        keep_pending(
          ctx,
          &self.thread,
          self.prev.as_ref(),
          self.msg_n,
          &events,
          None,
          replayed.as_deref(),
        );
      }
      for (id, name, call_input) in &calls {
        if !cut {
          on_event(ChatEvent::ToolCall {
            name: name.clone(),
            input: call_input.clone(),
          });
        }
        // A replayed step's call that had its result: not run again.
        let had = done.get(id).cloned();
        // The hazard guard's verdicts on what the call ran (design §2.28).
        let mut verdicts: Vec<crate::hazard::Verdict> = Vec::new();
        // What the calls it made spent (a script's `Tool.Call`s, `Decide`).
        let mut also: Vec<crate::tools::Spent> = Vec::new();
        let outcome = if cut {
          None
        } else if let Some((ok, output)) = &had {
          Some(if *ok {
            crate::tools::ToolOutcome::ok(output.clone())
          } else {
            crate::tools::ToolOutcome::err(output.clone())
          })
        } else {
          // Clients following the thread show it in flight meanwhile.
          keep_pending(
            ctx,
            &self.thread,
            self.prev.as_ref(),
            self.msg_n,
            &events,
            Some(id),
            replayed.as_deref(),
          );
          let _in_flight = InFlightGuard {
            ctx,
            thread: &self.thread,
            id: id.clone(),
          };
          // Kept outside the call's future: stopped midway, what it spent
          // and judged is still counted.
          let acc = crate::tools::AlsoSpentCell::default();
          let judged = crate::hazard::VerdictsCell::default();
          let r = tokio::select! {
            o = crate::tools::spending_into(acc.clone(), crate::hazard::recording_into(judged.clone(), self.tools.call(&self.thread, id, name, call_input))) => {
              Some(o)
            }
            () = stopped(self.steer.as_ref()) => {
              // A command it started is stopped too.
              self.tools.interrupt(&self.thread, name, call_input).await;
              None
            }
          };
          verdicts = std::mem::take(&mut *judged.lock().expect("verdicts lock"));
          also = std::mem::take(&mut acc.lock().expect("spent lock").spent);
          r
        };
        // The calls after a stop do not run, but each has its result, so
        // the conversation stays whole.
        let mut outcome = outcome.unwrap_or_else(|| {
          cut = true;
          if self.steer.as_ref().is_some_and(Steer::suspended) {
            crate::tools::ToolOutcome::err(
              "interrupted: the app running this node was suspended; call it again if still needed",
            )
          } else {
            crate::tools::ToolOutcome::err("interrupted by the user")
          }
        });
        // No secret's value reaches the model or a record (design §2.19).
        outcome.output = self.tools.redact(std::mem::take(&mut outcome.output));
        // A model a tool asked counts like this loop's own steps, the
        // guard's decisions too.
        let spents = outcome
          .spent
          .take()
          .into_iter()
          .chain(verdicts.iter_mut().filter_map(|v| v.spent.take()))
          .chain(also.drain(..));
        for spent in spents.collect::<Vec<_>>() {
          self
            .tools
            .metered(&self.thread, &spent.model, &spent.usage, spent.usd);
          add_usage(&mut total, &spent.usage);
          if let Some(u) = spent.usd {
            *usd_total.get_or_insert(0.0) += u;
          }
          events.push(Event::Usage(stitch_records::Usage {
            at: now(),
            model: spent.model,
            input_tokens: spent.usage.input_tokens,
            output_tokens: spent.usage.output_tokens,
            cache_read_tokens: Some(spent.usage.cache_read_tokens),
            cache_write_tokens: Some(spent.usage.cache_write_tokens),
            usd: spent.usd,
          }));
        }
        if !cut {
          calls_made += 1;
          calls_failed += usize::from(!outcome.ok);
        }
        // Its images are stored as the model will see them: scaled down to
        // what every provider takes, the original kept beside them (its
        // blob referenced by the result, so it syncs) for `zoom_image`.
        let mut stored = Vec::new();
        for image in std::mem::take(&mut outcome.images) {
          let shown = image.path.display().to_string();
          let file_name = image
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
          let original = std::fs::read(&image.path).map_err(Error::io(&shown))?;
          let (att, note) = match crate::images::shrink(&original, crate::images::MAX_SIDE) {
            Some(small) => {
              let kept = ctx.blobs.put_bytes(&original)?;
              outcome.blobs.push(kept);
              let hash = ctx.blobs.put_bytes(&small.data)?;
              let note = format!(
                "the image was {} for the model; its original is {}, which zoom_image crops at \
                 full resolution",
                small.scaled.map(|s| s.describe()).unwrap_or_default(),
                crate::images::short_id(&kept)
              );
              let att = Attachment {
                hash,
                bytes: small.data.len() as u64,
                media_type: small.media_type,
                name: file_name,
              };
              (att, Some(note))
            }
            None => {
              let (hash, bytes) = ctx.blobs.put_file(&image.path)?;
              let att = Attachment {
                hash,
                bytes,
                media_type: stitch_providers::media::image_type(&image.media_type, &original)
                  .to_owned(),
                name: file_name,
              };
              (att, None)
            }
          };
          if let Some(n) = &note {
            outcome.output.push_str(&format!("\n[{n}]"));
          }
          stored.push((shown, att, note.is_some()));
        }
        on_event(ChatEvent::ToolResult {
          name: name.clone(),
          ok: outcome.ok,
          output: outcome.output.clone(),
        });
        for att in &outcome.attachments {
          if att.media_type == crate::apps::MEDIA {
            on_event(ChatEvent::App {
              manifest: att.hash,
              name: att.name.clone().unwrap_or_default(),
            });
          }
        }
        let mut output = json!({"text": outcome.output});
        match verdicts.as_slice() {
          [] => {}
          [v] => output["guard"] = v.to_json(),
          vs => output["guard"] = vs.iter().map(crate::hazard::Verdict::to_json).collect(),
        }
        if !outcome.blobs.is_empty() {
          output["blobs"] = outcome
            .blobs
            .iter()
            .map(|b| json!({"blob": b.to_string()}))
            .collect();
        }
        if had.is_none() {
          events.push(Event::ToolResult(ToolResult {
            id: id.clone(),
            at: now(),
            ok: outcome.ok,
            output,
          }));
          if !cut {
            keep_pending(
              ctx,
              &self.thread,
              self.prev.as_ref(),
              self.msg_n,
              &events,
              None,
              replayed.as_deref(),
            );
          }
        }
        results.push(Content::ToolResult {
          id: id.clone(),
          output: outcome.output,
          is_error: !outcome.ok,
        });
        for (shown, att, scaled) in stored {
          let scaled = if scaled {
            " (scaled: see its result)"
          } else {
            ""
          };
          tool_images.push(Part::Text {
            text: format!("From {name} (call {id}), {shown}{scaled}:"),
          });
          tool_images.push(Part::Attachment { att });
        }
        for att in outcome.attachments {
          tool_images.push(Part::Text {
            text: format!("From {name} (call {id}):"),
          });
          tool_images.push(Part::Attachment { att });
        }
        made.extend(outcome.generated);
      }
      // Images a tool made for the reply are this step's reply's
      // attachments, as an image model's own are: the user sees them in
      // the reply, and the next calls (and every later turn) send them
      // with it (`lift_images` moves them where a provider takes them).
      if !made.is_empty() {
        let parts: Vec<Part> = made
          .iter()
          .map(|att| Part::Attachment { att: att.clone() })
          .collect();
        if let Some(Event::Msg(m)) = events
          .iter_mut()
          .rev()
          .find(|e| matches!(e, Event::Msg(m) if m.role == Role::Assistant))
        {
          m.parts.extend(parts.iter().cloned());
        }
        if let Some(last) = messages
          .iter_mut()
          .rev()
          .find(|m| m.role == PRole::Assistant)
        {
          last.content.extend(parts_content(&parts, ctx.blobs)?);
        }
      }
      let image_content = parts_content(&tool_images, ctx.blobs)?;
      if !tool_images.is_empty() {
        self.msg_n += 1;
        let at = now();
        self.last_at = Some(at);
        events.push(Event::Msg(Msg {
          id: format!("m{}", self.msg_n),
          at,
          role: Role::User,
          // Not the user's words: what the tools returned.
          from: Some("tool".into()),
          model: None,
          parts: tool_images,
          native: None,
        }));
      }
      let suspended = self.steer.as_ref().is_some_and(Steer::suspended);
      if cut {
        events.push(interrupted(suspended));
      }
      last_segment = self.write_step(std::mem::take(&mut events))?;
      if cut {
        end = End::Interrupted;
        let why = if suspended { SUSPENDED } else { "interrupted" };
        break (response.model, StopReason::Other(why.into()), text);
      }
      if calls.is_empty() {
        if wrapping {
          end = End::Steps;
        } else if text.trim().is_empty()
          && calls_made > 0
          && !said_with_calls
          && !told_no_reply
          && !matches!(response.stop, StopReason::Refusal { .. })
          && (steps as u64) < allowed
          && self.limits.exceeded(usd_total, &total).is_none()
        {
          // Tool calls, then nothing said (design §2.30): told once to
          // write its reply. The empty step is not sent back (a model's
          // turn with no content is refused); Stitch's note is not shown.
          told_no_reply = true;
          if messages.last().is_some_and(said_nothing) {
            messages.pop();
          }
          let parts = vec![Part::Text {
            text: NO_REPLY.into(),
          }];
          let (message, event) = self.user_msg(NO_REPLY_FROM.into(), parts)?;
          messages.push(message);
          events.push(event);
          continue;
        }
        break (response.model, response.stop, text);
      }
      said_with_calls = !text.trim().is_empty();
      messages.push(Message::user(results));
      if !image_content.is_empty() {
        messages.push(Message::user(image_content));
      }
      if wrapping {
        // Told to answer, it called tools again: the turn ends here.
        end = End::Steps;
        break (response.model, response.stop, text);
      }
      if steps as u64 >= allowed {
        on_event(ChatEvent::Note(format!(
          "[{steps} steps taken: asking you for {per_grant} more]"
        )));
        let granted = tokio::select! {
          g = self.tools.more_steps(&self.thread, steps as u64, per_grant) => g,
          () = stopped(self.steer.as_ref()) => false,
        };
        if granted {
          allowed += per_grant;
          on_event(ChatEvent::Note(format!(
            "[{per_grant} more steps: going on]"
          )));
        } else {
          wrapping = true;
        }
      }
      if let Some(why) = self.limits.exceeded(usd_total, &total) {
        end = End::OverBudget(why);
        break (response.model, response.stop, text);
      }
      // What came meanwhile goes in before the next step: one message per
      // run of the same sender.
      let sent = self.steer.as_ref().map(Steer::take).unwrap_or_default();
      let mut ids = Vec::new();
      let mut runs: Vec<(String, Vec<Part>)> = Vec::new();
      for i in sent {
        let from = i.from.unwrap_or_else(|| format!("user@{}", ctx.node));
        match i.id {
          Some(id) => ids.push(id),
          // Another client's, relayed by its waiting chat: that client
          // hears it went.
          None if i.relayed.is_some() => {}
          None => on_event(ChatEvent::Note(i.text.clone())),
        }
        let mut parts = Vec::new();
        if !i.text.is_empty() {
          parts.push(Part::Text { text: i.text });
        }
        if let Some(sc) = &i.scope {
          parts.push(sc.part());
        }
        parts.extend(
          i.attachments
            .into_iter()
            .map(|att| Part::Attachment { att }),
        );
        match runs.last_mut() {
          Some((f, p)) if *f == from => p.extend(parts),
          _ => runs.push((from, parts)),
        }
      }
      for (from, parts) in runs {
        let (message, event) = self.user_msg(from, parts)?;
        messages.push(message);
        events.push(event);
      }
      if !ids.is_empty() {
        on_event(ChatEvent::Woven(ids));
      }
      // Last, so it is what the model reads before its last step.
      if wrapping {
        let parts = vec![Part::Text {
          text: wrap_up(steps as u64),
        }];
        let (message, event) = self.user_msg("stitch".into(), parts)?;
        messages.push(message);
        events.push(event);
      }
    };
    if end == End::Steps {
      on_event(ChatEvent::Note(format!(
        "[stopped at the step limit ({steps} steps): say \"go on\" to continue]"
      )));
    }
    ctx.index.lock().expect("index lock").refresh(ctx.root)?;
    Ok(ChatOutcome {
      thread: self.thread,
      segment: last_segment.expect("at least one step"),
      model,
      stop,
      usage: total,
      usd: usd_total,
      steps,
      forked: self.forked,
      end,
      text,
      calls: calls_made,
      failed_calls: calls_failed,
    })
  }

  /// The model call failed for good (retries spent, or an error the
  /// provider answered). What the step had to write goes in the thread
  /// with the failure (a `failed` event, `crate::failed`): the user's
  /// message on the first step (a new thread's record with it), messages
  /// and notes woven in since the last step, which their senders were told
  /// went to the model. So no message is lost to an error, the thread
  /// shows why its turn stopped, on every node, and the turn can be
  /// resumed (`unfinished` says `cut off`); `Error::Kept` tells the client
  /// not to give the message back (design §5, milestone 4, 2026-10-22).
  /// Not when the request itself was refused as it is, which writing it
  /// would make every later turn repeat: too long for the model (the
  /// node compacts the thread and sends the message there), or a bad
  /// request on the first step (a 400: an attachment the model does not
  /// take), whose message goes back to the client to be changed. Too long
  /// after the first step, what came since the last step written (messages
  /// woven in, which their senders were told went to the model; Stitch's
  /// notes: `NO_REPLY`, the wrap-up) is written still, without the
  /// failure: it is not the refused request but what led to it, so a turn
  /// compacted midway has it in the new thread (summarized or kept) rather
  /// than losing it.
  fn failed(&mut self, e: stitch_providers::Error, mut events: Vec<Event>, first: bool) -> Error {
    let label = self.provider.label();
    let as_is = matches!(
      e,
      stitch_providers::Error::Api {
        status: 400 | 413 | 422,
        ..
      }
    );
    let too_long = crate::fork::too_long_said(&e.to_string());
    if too_long && !first && !events.is_empty() {
      if let Err(w) = self.write_step(events) {
        eprintln!(
          "thread {}: what came before a call too long was not written: {w}",
          self.thread
        );
      }
      return e.into();
    }
    if too_long || (first && as_is) {
      return e.into();
    }
    events.push(crate::failed::event(&e, &label));
    let said = crate::failed::said(&e, &label);
    let err = Error::Provider(e);
    match self.write_step(events) {
      Ok(_) => Error::Kept {
        said,
        source: Box::new(err),
      },
      Err(w) => {
        eprintln!(
          "thread {}: the failed step was not written: {w}",
          self.thread
        );
        err
      }
    }
  }

  /// Writes a step's events as the next segment (a new thread's record
  /// first); its ID, or the last one when there was nothing to write.
  fn write_step(&mut self, events: Vec<Event>) -> Result<Option<RecordId>> {
    let ctx = self.ctx;
    if let Some(record) = self.new_record.take() {
      ctx.root.publish_record(&record)?;
      ctx.index.lock().expect("index lock").refresh(ctx.root)?;
    }
    if events.is_empty() {
      return Ok(self.prev.clone());
    }
    // On the turn's open segment, sealed when the turn ends (`steps`).
    let segment = ctx.root.append_step_from(
      ctx.node,
      &self.thread,
      self.prev.take(),
      self.rewind.take(),
      events,
    )?;
    // Written: nothing of it is pending any more.
    clear_pending(ctx, &self.thread);
    self.prev = Some(segment.clone());
    Ok(Some(segment))
  }
}

/// A report replying to a message woven into a turn (`woven = true`): the
/// thread's turn told to a sender, not a report of its own.
pub fn is_woven_reply(report: &stitch_store::IndexedRecord) -> bool {
  report.fields.get("woven").and_then(Value::as_bool) == Some(true)
}

/// Whether a turn of a thread that reported is under way since, as every
/// node can tell from synced records (design §2.7): a turn started on the
/// node that runs it (a `claim` of the thread, written as a turn nobody
/// typed starts) after its last report of its own, or a message sent to
/// it (an `assignment` with `continue = true`) after its last report of
/// any kind, which the turn that takes it ends with (its own, or the reply
/// to a message woven into a running loop).
pub fn turn_open(
  index: &Index,
  thread: &RecordId,
  reports: &[stitch_store::IndexedRecord],
) -> Result<bool> {
  let own = reports
    .iter()
    .filter(|r| !is_woven_reply(r))
    .map(|r| r.created)
    .max();
  let any = reports.iter().map(|r| r.created).max();
  let after = |at: Timestamp, last: Option<Timestamp>| last.is_none_or(|l| at > l);
  if index
    .sources(kinds::CLAIM, "thread", thread, None)?
    .iter()
    .any(|c| after(c.created, own))
  {
    return Ok(true);
  }
  Ok(
    index
      .sources(kinds::ASSIGNMENT, "thread", thread, None)?
      .iter()
      .filter(|a| a.fields.get("continue").and_then(Value::as_bool) == Some(true))
      .any(|a| after(a.created, any)),
  )
}

/// A detached thread's state, from its records: `running` while a turn of
/// it is under way (none reported yet, or one started since its last
/// report, `turn_open`), `waiting` instead while an approval is open, else
/// its last report's outcome (`done`, `failed`, `over-budget`,
/// `cancelled`, `stopped`). `None` for a thread that is not detached.
pub fn detached_state(
  index: &Index,
  thread: &stitch_store::IndexedRecord,
) -> Result<Option<String>> {
  if thread.fields.get("detached").and_then(Value::as_bool) != Some(true) {
    return Ok(None);
  }
  // Compacted into another thread: that one runs now.
  if index
    .sources(kinds::THREAD, "continues", &thread.id, None)?
    .iter()
    .any(|t| t.fields.get("compacted").and_then(Value::as_bool) == Some(true))
  {
    return Ok(Some("compacted".into()));
  }
  // A thread woken again after its report writes another; the latest counts
  // (a reply to a message woven into a turn, `woven = true`, is not its own).
  let reports = index.sources(kinds::REPORT, "thread", &thread.id, None)?;
  if let Some(r) = reports.iter().rfind(|r| !is_woven_reply(r)) {
    let outcome = r
      .fields
      .get("outcome")
      .and_then(Value::as_str)
      .unwrap_or("done");
    if !turn_open(index, &thread.id, &reports)? {
      return Ok(Some(outcome.to_owned()));
    }
  }
  for q in index.sources(kinds::QUESTION, "thread", &thread.id, None)? {
    if index
      .sources(kinds::ANSWER, "answers", &q.id, Some(""))?
      .is_empty()
    {
      return Ok(Some("waiting".into()));
    }
  }
  Ok(Some("running".into()))
}

/// A thread's transcript for showing: (who, text) per message, tool call,
/// tool result and refusal.
pub fn transcript(ctx: &ChatContext<'_>, thread: &RecordId) -> Result<Vec<(String, String)>> {
  transcript_to(ctx, thread, None)
}

/// `transcript`, up to and with `head` (a segment of the thread): what a
/// chat that ended there showed.
pub fn transcript_to(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<Vec<(String, String)>> {
  Ok(
    transcript_shown(ctx, thread, head)?
      .into_iter()
      .map(|e| (e.who, e.text))
      .collect(),
  )
}

/// A transcript entry, for clients that show more than text.
#[derive(Clone, Debug, Default)]
pub struct Shown {
  /// `you`, a model, `tool`, `output`, `error`, `memory`, `fork`,
  /// `failed` (a turn's model call failed: why), `note` (a room's note,
  /// `rooms::NOTE_EVENT`), …
  pub who: String,
  /// As `transcript` has it: attachments said as lines.
  pub text: String,
  /// Without those lines, for a client that shows the attachments.
  pub said: String,
  /// A message's text parts alone, as written: without the lines said for
  /// what else it carries (`[instructions updated]`, `[memories
  /// recalled]`), for a client matching a message it sent (issue
  /// 2026-11-02T102522Z-mbp-2fkd).
  pub typed: String,
  /// A message's ID (`m12`, what a fork's `upto` takes); tool calls,
  /// results and refusals have none.
  pub id: Option<String>,
  /// When a message was written; `None` for what is not a message.
  pub at: Option<Timestamp>,
  pub attachments: Vec<Attachment>,
  /// A message the user said in a voice call (`Part::Spoken`): the app
  /// and the web client mark it.
  pub spoken: bool,
  /// Threads and efforts attached to the message (`Part::Scope`): clients
  /// show them as chips; their briefing is not in its text.
  pub scopes: Vec<stitch_records::ScopeItem>,
  /// A room's messages relayed to this participant thread
  /// (`Part::RoomTurn`): the room, and the last of its messages relayed.
  pub room_turn: Option<(RecordId, String)>,
  /// A `suggest_replies` call's preset answers (design §2.30), on its
  /// `tool` entry; empty for anything else. Live while no `you` entry
  /// comes after the latest such entry and no turn runs
  /// (`replies::live`).
  pub replies: Vec<crate::replies::Suggested>,
  /// An accepted `offer_actions` call's buttons (design §2.30, step 2), on
  /// its `tool` entry; empty for anything else (a refused call's too).
  /// The latest such entry is live (`actions::live`).
  pub actions: Vec<crate::actions::Action>,
  /// A tool call's ID, on its `tool` entry: a set's, which a press names.
  pub call: Option<String>,
  /// A press of one of those buttons (who `press`; `text` its line).
  pub press: Option<crate::actions::Press>,
  /// A `compacted` divider's compaction (design §2.15).
  pub compaction: Option<crate::api::Compaction>,
  /// A `forked` divider's fork (design §2.15, "A fork shows what it
  /// continues").
  pub fork: Option<ForkPoint>,
  /// Of the history a fork continues, shown before its divider: read-only
  /// here (no `id`), its own thread and message ID (`source`, the ID for
  /// a message only), whether the fork's model reads it (`context`).
  pub ancestor: bool,
  pub source: Option<(RecordId, Option<String>)>,
  pub context: bool,
  /// A message of a conversation shared by another user (design §2.31):
  /// shown as foreign.
  pub shared: bool,
}

impl Shown {
  fn of(who: impl Into<String>, text: String) -> Shown {
    Shown {
      who: who.into(),
      said: text.clone(),
      typed: text.clone(),
      text,
      id: None,
      at: None,
      attachments: Vec::new(),
      spoken: false,
      scopes: Vec::new(),
      room_turn: None,
      replies: Vec::new(),
      actions: Vec::new(),
      call: None,
      press: None,
      ..Default::default()
    }
  }

  /// A user's message, as `replies::live` counts them.
  pub fn is_user(&self) -> bool {
    self.who == "you"
  }
}

/// `transcript_shown` over a compaction chain (design §2.15, "One
/// conversation"): the threads `thread` was compacted from, oldest first,
/// each compaction then said by a `compacted` entry (its divider,
/// `Compaction::said`, as `said`; that and the summary as `text`; the
/// summary alone as `typed`; when it was made as `at`; the compaction as
/// `compaction`) in place of its fork notice, then `thread`'s own entries
/// up to `head`. Messages of the earlier threads have no `id`: IDs are
/// per thread, and a fork or rewind from one would take it for
/// `thread`'s.
///
/// A fork shows what it continues (design §2.15, "A fork shows what it
/// continues"): before the chain, the history of the thread it was forked
/// from up to the fork point (through that thread's own compactions and
/// forks, `fork_past`), each entry `ancestor` with its `source` and no
/// `id`, `context` when the fork's model reads it, a `context` line where
/// that begins when it does not begin with it, then a `forked` divider
/// (`fork`) in place of its fork notice. What the fork inherited is not
/// shown again.
pub fn lineage_shown(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<Vec<Shown>> {
  let mut own = chain_shown(ctx, thread, head)?;
  let root = {
    let index = ctx.index.lock().expect("index lock");
    index.compaction_lineage(thread)[0].clone()
  };
  let (past, context) = fork_past(ctx, &root);
  if past.is_empty() {
    return Ok(own);
  }
  if own.first().is_some_and(|e| e.who == "fork") {
    own.remove(0);
  }
  let mut out = past_shown(ctx, past, &context);
  out.extend(own);
  Ok(out)
}

/// A `compacted` divider.
fn compacted_shown(c: crate::api::Compaction, at: Option<Timestamp>) -> Shown {
  let line = c.said();
  Shown {
    who: "compacted".into(),
    text: format!("{line}\n\n{}", c.summary),
    said: line,
    typed: c.summary.clone(),
    at,
    compaction: Some(c),
    ..Default::default()
  }
}

/// Where a fork leaves the thread it continues, as its divider says it
/// (design §2.15, "A fork shows what it continues").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForkPoint {
  /// The thread it was forked from, and its conversation's title.
  pub from: String,
  pub title: Option<String>,
  /// The fork.
  pub to: String,
  /// When it was forked.
  pub at: Option<Timestamp>,
  /// The message of `from` it was forked at; `None`: its latest.
  pub upto: Option<String>,
  /// `full`, `last` (its last `last` messages), `summary`, or
  /// `summary+last`.
  pub how: String,
  pub last: Option<usize>,
  pub summary: Option<String>,
  pub no_tools: bool,
  pub no_attachments: bool,
  /// `from` is a conversation another user shared (design §2.31).
  pub shared: Option<crate::share::Origin>,
  /// The divider's line.
  pub said: String,
}

/// An item of the history a fork continues (`fork_past`).
pub(crate) enum Past {
  /// An event of `thread`.
  Event { event: Event, thread: RecordId },
  /// `thread` was compacted from the one before it.
  Compacted(RecordId),
  /// A fork starts here.
  Forked(ForkPoint),
}

/// The fork `thread` is (one the user made: not a compaction, not a
/// spawned thread's conversation, not of a thread deleted since): what it
/// continues.
fn user_fork(ctx: &ChatContext<'_>, thread: &RecordId) -> Option<Continues> {
  let record = ctx.root.read_record(thread).ok()?;
  let c = Continues::of(&record)?;
  let compacted = record
    .fields
    .get("compacted")
    .and_then(toml::Value::as_bool)
    == Some(true);
  (!compacted && record.refs.first("spawned-by").is_none() && !continues_deleted(ctx, &c))
    .then_some(c)
}

fn fork_point(ctx: &ChatContext<'_>, fork: &RecordId, c: &Continues) -> ForkPoint {
  let index = ctx.index.lock().expect("index lock");
  let title = index.lineage_title(&c.thread);
  let at = index.record(fork).ok().flatten().map(|r| r.created);
  let shared = crate::share::origin(&index, &c.thread);
  drop(index);
  let summary = c.shape.summary.clone();
  let last = c.shape.last.filter(|n| *n > 0);
  let how = match (&summary, last) {
    (Some(_), Some(_)) => "summary+last",
    (Some(_), None) => "summary",
    (None, Some(_)) => "last",
    (None, None) => "full",
  };
  let mut said = format!(
    "⑂ forked from “{}”{}",
    title.as_deref().unwrap_or("a thread"),
    c.shape
      .upto
      .as_deref()
      .map(|m| format!(" at {m}"))
      .unwrap_or_default()
  );
  said.push_str(match how {
    "summary+last" => ": a summary, then its last messages",
    "summary" => ": a summary of it",
    "last" => ": its last messages",
    _ => "",
  });
  if let Some(n) = last {
    said = said.replace("its last messages", &format!("its last {n} messages"));
  }
  if c.shape.no_tools {
    said.push_str(", without tool calls");
  }
  if c.shape.no_attachments {
    said.push_str(", without attachments");
  }
  if let Some(o) = &shared {
    said.push_str(&format!(
      " (a conversation shared with you, {})",
      if o.own {
        "signed by your own key".to_owned()
      } else if o.signed {
        format!("signed by key {}…", &o.source[..o.source.len().min(16)])
      } else {
        "unsigned".to_owned()
      }
    ));
  }
  ForkPoint {
    from: c.thread.to_string(),
    title,
    to: fork.to_string(),
    at,
    upto: c.shape.upto.clone(),
    how: how.to_owned(),
    last,
    summary,
    no_tools: c.shape.no_tools,
    no_attachments: c.shape.no_attachments,
    shared,
    said,
  }
}

/// The history `c` continues, as shown: the conversation of the thread it
/// forks (its compaction chain, and what that chain's first thread was
/// forked from, recursively), up to the fork point.
fn history_to(ctx: &ChatContext<'_>, c: &Continues, out: &mut Vec<Past>, depth: usize) {
  let chain = {
    let index = ctx.index.lock().expect("index lock");
    let mut chain = index.compaction_lineage(&c.thread);
    if let Some(at) = chain.iter().position(|t| *t == c.thread) {
      chain.truncate(at + 1);
    }
    chain
  };
  if depth < 16
    && let Some(first) = chain.first()
    && let Some(pc) = user_fork(ctx, first)
  {
    history_to(ctx, &pc, out, depth + 1);
    out.push(Past::Forked(fork_point(ctx, first, &pc)));
  }
  for (i, t) in chain.iter().enumerate() {
    if i > 0 {
      out.push(Past::Compacted(t.clone()));
    }
    let events = if *t == c.thread {
      continued_events(ctx, c)
    } else {
      thread_events(ctx, t)
    };
    for event in events.unwrap_or_default() {
      out.push(Past::Event {
        event,
        thread: t.clone(),
      });
    }
  }
}

/// What an event is known by across threads: a message by when it was
/// written and its ID, a call and its result by the call's ID.
pub(crate) fn event_key(e: &Event) -> String {
  match e {
    Event::Msg(m) => format!("m {} {}", m.at, m.id),
    Event::ToolCall(c) => format!("c {}", c.id),
    Event::ToolResult(r) => format!("r {}", r.id),
    other => other.to_value().to_string(),
  }
}

/// The history the fork `root` (a compaction chain's first thread)
/// continues, ending with its `forked` divider; and the keys of the
/// events its model reads of it. Empty when `root` is not a fork the user
/// made.
pub(crate) fn fork_past(
  ctx: &ChatContext<'_>,
  root: &RecordId,
) -> (Vec<Past>, std::collections::HashSet<String>) {
  let Some(c) = user_fork(ctx, root) else {
    return Default::default();
  };
  let mut out = Vec::new();
  history_to(ctx, &c, &mut out, 0);
  out.push(Past::Forked(fork_point(ctx, root, &c)));
  let context = inherited(ctx, root)
    .unwrap_or_default()
    .iter()
    .map(event_key)
    .collect();
  (out, context)
}

/// `fork_past` as transcript entries.
fn past_shown(
  ctx: &ChatContext<'_>,
  past: Vec<Past>,
  context: &std::collections::HashSet<String>,
) -> Vec<Shown> {
  let mut out: Vec<Shown> = Vec::new();
  let mut marked = false;
  let mut outside = false;
  for p in past {
    match p {
      Past::Event { event, thread } => {
        let in_context = context.contains(&event_key(&event));
        let from = out.len();
        push_shown(event, &mut out);
        if from < out.len() && in_context && outside && !marked {
          out.insert(
            from,
            Shown::of(
              "context",
              "── the fork's model reads from here on ──".to_owned(),
            ),
          );
          marked = true;
        }
        for e in out.iter_mut().skip(from) {
          if e.who == "context" {
            continue;
          }
          e.ancestor = true;
          e.context = in_context;
          e.source = Some((thread.clone(), e.id.take()));
        }
        if !in_context && from < out.len() {
          outside = true;
        }
        if in_context {
          marked = true;
        }
      }
      Past::Compacted(t) => {
        let (c, at) = {
          let index = ctx.index.lock().expect("index lock");
          (
            crate::fork::compaction_of(&index, &t).unwrap_or_default(),
            index.record(&t).ok().flatten().map(|r| r.created),
          )
        };
        let mut e = compacted_shown(c, at);
        e.ancestor = true;
        e.source = Some((t, None));
        out.push(e);
      }
      Past::Forked(f) => {
        let text = match &f.summary {
          Some(s) => format!("{}\n\n{s}", f.said),
          None => f.said.clone(),
        };
        out.push(Shown {
          who: "forked".into(),
          text,
          said: f.said.clone(),
          typed: f.summary.clone().unwrap_or_default(),
          at: f.at,
          fork: Some(f),
          ..Default::default()
        });
      }
    }
  }
  out
}

/// The compaction chain alone (`lineage_shown` without a fork's past).
fn chain_shown(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<Vec<Shown>> {
  let chain = {
    let index = ctx.index.lock().expect("index lock");
    let mut chain = index.compaction_lineage(thread);
    if let Some(at) = chain.iter().position(|t| t == thread) {
      chain.truncate(at + 1);
    }
    chain
  };
  if chain.len() < 2 {
    return transcript_shown(ctx, thread, head);
  }
  let mut out = Vec::new();
  for (i, t) in chain.iter().enumerate() {
    let last = i + 1 == chain.len();
    // An earlier thread not readable here (deleted since) is left out.
    let mut shown = match transcript_shown(ctx, t, if last { head } else { None }) {
      Ok(s) => s,
      Err(_) if !last => Vec::new(),
      Err(e) => return Err(e),
    };
    if i > 0 {
      if shown.first().is_some_and(|e| e.who == "fork") {
        shown.remove(0);
      }
      let (c, at) = {
        let index = ctx.index.lock().expect("index lock");
        (
          crate::fork::compaction_of(&index, t).unwrap_or_default(),
          index.record(t).ok().flatten().map(|r| r.created),
        )
      };
      out.push(compacted_shown(c, at));
    }
    if !last {
      for e in &mut shown {
        e.id = None;
      }
    }
    out.extend(shown);
  }
  Ok(out)
}

/// `transcript_to`, each message with its ID and attachments.
pub fn transcript_shown(
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  head: Option<&RecordId>,
) -> Result<Vec<Shown>> {
  let (segments, _) = chain_to(ctx, thread, head)?;
  let mut out = Vec::new();
  // A fork says what it continues, and its summary when it has one: up to
  // a segment too, so that transcript is the whole one's start (the TUI
  // counts what a chat showed by it; without the notice, the last entry
  // of each turn on a fork was printed again).
  if let Ok(record) = ctx.root.read_record(thread)
    && let Some(c) = Continues::of(&record)
    && record.refs.first("spawned-by").is_none()
  {
    let title = ctx
      .index
      .lock()
      .expect("index lock")
      .title(&c.thread)
      .ok()
      .flatten()
      .unwrap_or_default();
    let mut what = if continues_deleted(ctx, &c) {
      format!(
        "⑂ continued thread {}, deleted since: its earlier part is gone",
        c.thread
      )
    } else {
      format!("⑂ continues thread {} \"{title}\"", c.thread)
    };
    if let Some(m) = &c.shape.upto {
      what.push_str(&format!(" up to {m}"));
    }
    match (c.shape.last, &c.shape.summary) {
      (Some(n), Some(_)) if n > 0 => {
        what.push_str(&format!(": a summary, then its last {n} messages"))
      }
      (_, Some(_)) => what.push_str(": from a summary of it"),
      (Some(n), None) => what.push_str(&format!(": its last {n} messages")),
      (None, None) => {}
    }
    if c.shape.no_attachments {
      what.push_str(", without attachments");
    }
    if c.shape.no_tools {
      what.push_str(", without tool calls");
    }
    if let Some(s) = &c.shape.summary {
      what.push_str(&format!("\n\n{s}"));
    }
    out.push(Shown::of("fork", what));
  }
  for s in segments {
    for e in s.events {
      push_shown(e, &mut out);
    }
  }
  Ok(out)
}

/// One event as transcript entries (none, or one), pushed on `out`: a
/// refused action set's buttons on the entry before it are cleared.
fn push_shown(e: Event, out: &mut Vec<Shown>) {
  match e {
    Event::Msg(m) if unshown(&m) => {}
    Event::Msg(m) => {
      let (mut text, mut said, mut typed, mut attachments) =
        (String::new(), String::new(), String::new(), Vec::new());
      let spoken = m
        .parts
        .iter()
        .any(|p| matches!(p, Part::Spoken { spoken: true, .. }));
      let mut scopes = Vec::new();
      let mut room_turn = None;
      for p in &m.parts {
        let line = match p {
          // Two notes woven into one message (a report and another)
          // are shown apart, not run together.
          Part::Text { text: t } => {
            if !text.is_empty() && !text.ends_with('\n') && !t.is_empty() {
              text.push_str("\n\n");
            }
            if !said.is_empty() && !said.ends_with('\n') && !t.is_empty() {
              said.push_str("\n\n");
            }
            if !typed.is_empty() && !typed.ends_with('\n') && !t.is_empty() {
              typed.push_str("\n\n");
            }
            said.push_str(t);
            typed.push_str(t);
            t.clone()
          }
          Part::Attachment { att } => {
            attachments.push(att.clone());
            text.push_str(&format!(
              "\n[{} {}]",
              att.media_type,
              att.name.as_deref().unwrap_or("")
            ));
            continue;
          }
          Part::Context { .. } => "[instructions updated]\n".into(),
          Part::Recalled { .. } => "[memories recalled]\n".into(),
          // Chips in clients (`scopes`); a line in the plain
          // transcript, never the briefing itself.
          Part::Scope { scopes: items, .. } => {
            scopes.extend(items.iter().cloned());
            let named: Vec<String> = items
              .iter()
              .map(|i| match &i.title {
                Some(t) => format!("{} {} \"{t}\"", i.kind.as_str(), i.id),
                None => format!("{} {}", i.kind.as_str(), i.id),
              })
              .collect();
            text.push_str(&format!("\n[scope: {}]", named.join("; ")));
            continue;
          }
          // A participant's private thread: where the relay stood,
          // never the room's note itself.
          Part::RoomTurn { room, upto, .. } => {
            room_turn = Some((room.clone(), upto.clone()));
            text.push_str(&format!("\n[room turn up to {room}#{upto}]"));
            continue;
          }
          // Said, not typed: the message reads the same.
          Part::Spoken { .. } | Part::Other(_) => continue,
        };
        if !matches!(p, Part::Text { .. }) {
          said.push_str(&line);
        }
        text.push_str(&line);
      }
      // A declined follow-up (only memory's markers) is never shown.
      if m.role == Role::Assistant && stitch_memory::marker_only(&text) {
        return;
      }
      let text = stitch_memory::without_markers(&text);
      if text.is_empty() && m.role == Role::Assistant {
        return;
      }
      let said = stitch_memory::without_markers(&said);
      let typed = stitch_memory::without_markers(&typed);
      let who = match m.role {
        Role::User if m.from.as_deref() == Some("tool") => "output".to_owned(),
        Role::User if m.from.as_deref() == Some("memory") => "memory".to_owned(),
        Role::User => "you".to_owned(),
        // A room's reply: who said it, then on what.
        Role::Assistant => match (
          m.from.as_deref().and_then(crate::rooms::speaker_of),
          m.model,
        ) {
          (Some((slug, _)), Some(model)) => format!("{slug} · {model}"),
          (Some((slug, _)), None) => slug,
          (None, model) => model.unwrap_or_else(|| "assistant".into()),
        },
        Role::System => "system".into(),
      };
      out.push(Shown {
        who,
        text,
        said,
        typed,
        shared: crate::share::is_shared(m.from.as_deref()),
        id: Some(m.id),
        at: Some(m.at),
        attachments,
        spoken,
        scopes,
        room_turn,
        ..Default::default()
      });
    }
    Event::ToolCall(c) => {
      let mut e = Shown::of("tool", call_summary(&c.name, &c.input));
      e.replies = crate::replies::of_call(&c.name, &c.input).unwrap_or_default();
      e.actions = crate::actions::of_call(&c.name, &c.input).unwrap_or_default();
      e.call = Some(c.id);
      out.push(e);
    }
    Event::ToolResult(r) => {
      // A refused set offers nothing.
      if !r.ok
        && let Some(e) = out
          .iter_mut()
          .rev()
          .find(|e| e.call.as_deref() == Some(r.id.as_str()))
      {
        e.actions.clear();
      }
      let text = result_text(&r.output);
      let lines: Vec<&str> = text.lines().collect();
      let shown = if lines.len() > 12 {
        format!(
          "{}\n[… {} more lines]",
          lines[..12].join("\n"),
          lines.len() - 12
        )
      } else {
        text.clone()
      };
      out.push(Shown::of(if r.ok { "output" } else { "error" }, shown));
    }
    Event::Other(v) if v.get("t").and_then(Value::as_str) == Some("refusal") => {
      out.push(Shown::of("refusal", v.to_string()));
    }
    // A room's note (design §2.27: a participant's question settled):
    // a quiet line, no message.
    ref e if crate::rooms::note_of(e).is_some() => {
      let mut shown = Shown::of(
        "note",
        crate::rooms::note_of(e).unwrap_or_default().to_owned(),
      );
      if let Event::Other(v) = e {
        shown.at = v
          .get("at")
          .and_then(Value::as_str)
          .and_then(|a| a.parse().ok());
      }
      out.push(shown);
    }
    // A turn whose model call failed: why, as the user was told.
    e @ Event::Other(_) if is_failed(&e) => {
      out.push(Shown::of("failed", crate::failed::shown(&e)));
    }
    // An action button pressed (design §2.30): its line.
    e @ Event::Other(_) if crate::actions::Press::of(&e).is_some() => {
      let p = crate::actions::Press::of(&e).expect("a press");
      // Its line as `said`, with its output's first lines as `text`.
      let mut shown = Shown::of("press", p.said());
      let lines: Vec<&str> = p.output.trim().lines().collect();
      if !lines.is_empty() {
        shown.text = format!("{}\n{}", p.said(), lines[..lines.len().min(12)].join("\n"));
        if lines.len() > 12 {
          shown
            .text
            .push_str(&format!("\n[… {} more lines]", lines.len() - 12));
        }
      }
      shown.press = Some(p);
      out.push(shown);
    }
    _ => {}
  }
}

/// A tool call as one line: `$ command` for the shell.
pub fn call_summary(name: &str, input: &Value) -> String {
  let session = input
    .get("session")
    .and_then(Value::as_str)
    .filter(|s| *s != "main")
    .map(|s| format!("[{s}] "))
    .unwrap_or_default();
  match name {
    "shell_run" => format!(
      "{session}$ {}",
      input.get("command").and_then(Value::as_str).unwrap_or("")
    ),
    "shell_send" => {
      let text = input.get("text").and_then(Value::as_str).unwrap_or("");
      let keys: Vec<&str> = input
        .get("keys")
        .and_then(Value::as_array)
        .map(|k| k.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
      format!(
        "{session}⌨ {text:?}{}",
        if keys.is_empty() {
          String::new()
        } else {
          format!(" {}", keys.join(" "))
        }
      )
    }
    "shell_view" => format!("{session}👁 screen"),
    // Its labels, not its JSON.
    crate::replies::TOOL => crate::replies::summary(input),
    crate::actions::TOOL => crate::actions::summary(input),
    "shell_close" => format!("{session}✕ close"),
    // Otter code as written, on its lines, not escaped in JSON; the
    // rest (a file, an input) on the first line.
    "otter_run" | "otter_check" => {
      let mut rest = input.clone();
      let code = rest
        .as_object_mut()
        .and_then(|o| o.remove("code"))
        .and_then(|c| c.as_str().map(str::to_owned));
      let mut line = name.to_owned();
      if rest.as_object().is_some_and(|o| !o.is_empty()) {
        line.push_str(&format!(" {rest}"));
      }
      match code {
        Some(code) => format!("{line}\n{}", code.trim_end()),
        None => line,
      }
    }
    other => format!("{other} {input}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ts(s: &str) -> Timestamp {
    s.parse().unwrap()
  }

  #[test]
  fn woven_in_messages_are_the_users() {
    let msg = |from: Option<&str>| Msg {
      id: String::new(),
      at: ts("2026-11-11T06:59:30Z"),
      role: Role::User,
      from: from.map(str::to_owned),
      model: None,
      parts: vec![Part::Text {
        text: "maybe ask gpt astra and gemini argon too?".into(),
      }],
      native: None,
    };
    // A turn's first message, one woven in (`user`), and one with no sender.
    for from in [Some("user@gamer"), Some("user"), None] {
      assert!(by_user(&msg(from)), "{from:?}");
    }
    for from in ["thread 2026-11-11T065853Z-gamer-wkwi", "stitch", "tool"] {
      assert!(!by_user(&msg(Some(from))), "{from}");
    }
    let texts = user_texts(&[Event::Msg(msg(Some("user")))]);
    assert!(crate::modelset::mentions(&texts[0], "openai/gpt-6-astra"));
  }

  #[test]
  fn a_message_after_a_pause_is_told_the_time() {
    let tz = jiff::tz::TimeZone::get("Europe/Rome").unwrap();
    let note = |prev: &str, at: &str| time_note_in(Some(ts(prev)), ts(at), &tz);
    // Within the hour, the same day: nothing.
    assert_eq!(note("2026-10-31T08:00:00Z", "2026-10-31T08:59:00Z"), None);
    assert_eq!(
      note("2026-10-31T08:00:00Z", "2026-10-31T13:02:30Z").as_deref(),
      Some("[Sat 2026-10-31 14:02 CET: 5 hours 2 minutes since the previous message]")
    );
    // A short pause across midnight: the new day is said.
    assert_eq!(
      note("2026-10-31T22:50:00Z", "2026-10-31T23:10:00Z").as_deref(),
      Some("[Sun 2026-11-01 00:10 CET: 20 minutes since the previous message]")
    );
    assert_eq!(
      note("2026-10-28T13:00:00Z", "2026-10-31T13:00:00Z").as_deref(),
      Some("[Sat 2026-10-31 14:00 CET: 3 days since the previous message]")
    );
    // Nothing before it, or a clock that went back: nothing.
    assert_eq!(time_note_in(None, ts("2026-10-31T13:00:00Z"), &tz), None);
    assert_eq!(note("2026-10-31T13:00:00Z", "2026-10-31T08:00:00Z"), None);
    assert_eq!(spoken_gap(std::time::Duration::from_secs(3600)), "1 hour");
    assert_eq!(
      spoken_gap(std::time::Duration::from_secs(90_000)),
      "1 day 1 hour"
    );
  }

  /// A spoken message is replayed with the note it was sent with: one
  /// written before the second note (no `note`) with the first, byte for
  /// byte, so an old thread's prompt cache holds; a new one with the
  /// note that says the text is a transcription.
  #[test]
  fn a_spoken_message_replays_the_note_it_was_sent_with() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let old: Part = serde_json::from_str(r#"{"spoken":true}"#).unwrap();
    assert_eq!(
      old,
      Part::Spoken {
        spoken: true,
        note: 0
      }
    );
    let new = Part::Spoken {
      spoken: true,
      note: SPOKEN_NOTE_NUMBER,
    };
    assert_eq!(
      serde_json::to_string(&new).unwrap(),
      r#"{"spoken":true,"note":1}"#
    );
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"spoken":true}"#);
    let text = |p: Part| match &parts_content(&[p], &blobs).unwrap()[..] {
      [Content::Text(t)] => t.clone(),
      other => panic!("{other:?}"),
    };
    let first = text(old);
    assert_eq!(first, SPOKEN_NOTE_0);
    assert_eq!(
      blake3::hash(first.as_bytes()).to_hex()[..16].to_string(),
      "42c0e442ebfb661a",
      "the first note never changes: old threads replay it"
    );
    let second = text(new);
    assert_eq!(second, SPOKEN_NOTE);
    assert!(second.contains("transcription"));
    assert!(SPOKEN_NOTE.len() < SPOKEN_NOTE_0.len() + 300, "kept short");
  }

  /// The note is derived from the messages' times when the history is
  /// built: the user's message after a pause gets it, the model's and the
  /// tools' images never, and the text written is left as it was.
  #[test]
  fn the_history_tells_the_time_after_a_pause() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(dir.path()).unwrap();
    let msg = |id: &str, at: &str, role: Role, from: Option<&str>| {
      Event::Msg(Msg {
        id: id.into(),
        at: ts(at),
        role,
        from: from.map(str::to_owned),
        model: None,
        parts: vec![Part::Text { text: id.into() }],
        native: None,
      })
    };
    let events = [
      msg("m1", "2026-10-31T08:00:00Z", Role::User, Some("user@gamer")),
      msg("m2", "2026-10-31T08:01:00Z", Role::Assistant, None),
      msg("m3", "2026-10-31T11:01:00Z", Role::User, Some("tool")),
      msg("m4", "2026-10-31T14:01:00Z", Role::Assistant, None),
      msg("m5", "2026-10-31T14:30:00Z", Role::User, Some("user@gamer")),
      msg("m6", "2026-10-31T18:30:00Z", Role::User, Some("stitch")),
    ];
    let (messages, _) = messages_from(&events, &blobs).unwrap();
    let texts: Vec<Vec<String>> = messages
      .iter()
      .map(|m| {
        m.content
          .iter()
          .filter_map(|c| match c {
            Content::Text(t) => Some(t.clone()),
            _ => None,
          })
          .collect()
      })
      .collect();
    let noted = |i: usize| texts[i].len() == 2 && texts[i][0].contains("since the previous");
    assert!(!noted(0) && texts[0] == ["m1"]);
    assert!(!noted(1) && !noted(2) && !noted(3), "{texts:?}");
    assert!(!noted(4), "29 minutes after the model's reply: {texts:?}");
    assert!(noted(5), "a report 4 hours on: {texts:?}");
    assert!(texts[5][0].contains("4 hours since"), "{texts:?}");
    assert_eq!(last_msg_time(&events), Some(ts("2026-10-31T18:30:00Z")));
  }

  /// Otter code is shown on its lines, the other arguments before it.
  #[test]
  fn otter_calls_show_their_code() {
    let code = "1 = a\na | Log\n";
    assert_eq!(
      call_summary("otter_run", &serde_json::json!({"code": code})),
      "otter_run\n1 = a\na | Log"
    );
    assert_eq!(
      call_summary(
        "otter_check",
        &serde_json::json!({"code": "2", "json": true})
      ),
      "otter_check {\"json\":true}\n2"
    );
    assert_eq!(
      call_summary("otter_run", &serde_json::json!({"file": "x.otr"})),
      "otter_run {\"file\":\"x.otr\"}"
    );
  }

  fn typed(id: Option<u64>, text: &str) -> Interjection {
    Interjection {
      id,
      from: None,
      text: text.into(),
      attachments: Vec::new(),
      reports: Vec::new(),
      untold: None,
      asker: None,
      relayed: None,
      scope: None,
    }
  }

  /// A waiting chat's messages relayed to the loop that runs: what it
  /// takes is said, what it ends without is had back, one taken back
  /// (edited) leaves it.
  #[test]
  fn a_relay_says_what_was_taken_and_has_the_rest_back() {
    let running = Steer::open();
    let waiting = Steer::open();
    // Queued before the relay began: it goes too.
    waiting.push(typed(Some(1), "queued early"));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let h = heard.clone();
    assert!(
      waiting.relay(running.clone(), typed(None, "first"), move |id| {
        h.lock().unwrap().push(id)
      })
    );
    let taken = running.take();
    assert_eq!(taken.len(), 2);
    assert!(taken.iter().all(|i| i.id.is_none() && i.relayed.is_some()));
    assert_eq!(*heard.lock().unwrap(), vec![None, Some(1)]);
    assert!(waiting.relayed_first());

    waiting.push(typed(Some(2), "taken back"));
    assert!(waiting.retract(2));
    assert!(!running.has_queued());
    waiting.push(typed(Some(3), "too late"));
    // The loop ends without taking it: given back as Steering does.
    for i in running.close() {
      (i.untold.expect("relayed").0)();
    }
    let out = waiting.unrelay().unwrap();
    assert_eq!(out.taken, vec![None, Some(1)]);
    assert_eq!(out.back.len(), 1);
    assert_eq!(out.back[0].id, Some(3));
    assert_eq!(out.back[0].text, "too late");
    // Relaying no more: its own queue again.
    assert!(waiting.push(typed(Some(4), "own")));
    assert!(waiting.has_queued());
  }

  #[test]
  fn a_relay_to_a_loop_that_ended_relays_nothing() {
    let ended = Steer::open();
    ended.close();
    let waiting = Steer::open();
    assert!(!waiting.relay(ended, typed(None, "first"), |_| {}));
    assert!(waiting.unrelay().is_none());
  }

  #[test]
  fn a_spawned_thread_takes_the_conversation_without_its_spawners_context() {
    let msg = |id: &str, role: Role, parts: Vec<Part>| {
      Event::Msg(Msg {
        id: id.into(),
        at: Timestamp::UNIX_EPOCH,
        role,
        from: None,
        model: None,
        parts,
        native: None,
      })
    };
    let text = |t: &str| Part::Text { text: t.into() };
    let events = vec![
      msg(
        "m1",
        Role::User,
        vec![
          Part::Context {
            context: "# Where this turn runs\n\nOn node iphone".into(),
          },
          text("redesign it"),
        ],
      ),
      msg("m2", Role::Assistant, vec![text("ok")]),
      msg(
        "m3",
        Role::User,
        vec![Part::Context {
          context: "On node iphone".into(),
        }],
      ),
    ];
    let kept = without_context(events);
    assert_eq!(kept.len(), 2, "a message of context alone goes: {kept:?}");
    let Event::Msg(m) = &kept[0] else { panic!() };
    assert_eq!(m.parts, vec![text("redesign it")]);
  }

  #[test]
  fn a_spawned_thread_is_told_who_it_is() {
    let me: RecordId = "2026-10-19T140040Z-mbp-grpx".parse().unwrap();
    let parent: RecordId = "2026-10-19T132625Z-mbp-vyvm".parse().unwrap();
    let c = Continues {
      thread: parent.clone(),
      at: Upto::Call("toolu_1".into()),
      shape: Shape::default(),
    };
    let h = spawned_header(&me, "mbp", &parent, Some(&c), false);
    assert!(h.starts_with(&format!(
      "[Stitch: you are thread {me}, running on node mbp, spawned by thread {parent}"
    )));
    assert!(h.contains("The conversation before it is your spawner's"));
    assert!(h.contains("the thread it started for this goal is you"));
    assert!(h.contains("Do not spawn a thread to do it for you"));
    // From no conversation: nothing said of one.
    let h = spawned_header(&me, "mbp", &parent, None, false);
    assert!(!h.contains("conversation"), "{h}");
    // A question to another model only answers.
    assert!(spawned_header(&me, "mbp", &parent, None, true).ends_with("Answer it yourself.]"));
  }

  #[test]
  fn native_content_is_written_packed_and_replays_whole() {
    let native = json!({
      "provider": "anthropic",
      "model": "claude-opus-5-5",
      "content": [
        {"type": "thinking", "thinking": "Let me look.", "signature": "c2ln"},
        {"type": "text", "text": "First, "},
        {"type": "text", "text": "then."},
        {"type": "tool_use", "id": "c1", "name": "shell_run", "input": {"command": "ls"}},
        {"type": "text", "text": "unrelated", "citations": []},
      ],
    });
    let content = vec![
      Content::Text("First, ".into()),
      Content::Text("then.".into()),
      Content::ToolCall {
        id: "c1".into(),
        name: "shell_run".into(),
        input: json!({"command": "ls"}),
      },
    ];
    let packed = pack_native(native.clone(), "First, then.", &content);
    let blocks = packed["content"].as_array().unwrap();
    assert_eq!(blocks[0], native["content"][0], "thinking stays");
    assert_eq!(blocks[1], json!({"type": "text", "at": [0, 7]}));
    assert_eq!(blocks[2], json!({"type": "text", "at": [7, 12]}));
    assert_eq!(blocks[3], json!({"type": "tool_use", "call": "c1"}));
    assert_eq!(
      blocks[4], native["content"][4],
      "not the message's own text"
    );
    let call = ToolCall {
      id: "c1".into(),
      at: now(),
      name: "shell_run".into(),
      input: json!({"command": "ls"}),
    };
    let calls = std::collections::HashMap::from([("c1", &call)]);
    let parts = vec![Part::Text {
      text: "First, then.".into(),
    }];
    assert_eq!(unpack_native(packed, &parts, &calls), Some(native));
    // Another provider's content is written as it came.
    let other =
      json!({"provider": "openai", "model": "m", "content": [{"type": "text", "text": "x"}]});
    assert_eq!(pack_native(other.clone(), "x", &[]), other);
  }

  /// A `Photo.jpg` that is PNG (a WiFi scope app's) goes to the model as
  /// PNG, stored before the type was read from its bytes or after; a
  /// HEIC as an image (Gemini reads it; the other adapters say why not).
  #[test]
  fn an_image_goes_as_its_bytes_say_whatever_it_was_labelled() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 3)
      .write_to(&mut png, image::ImageFormat::Png)
      .unwrap();
    let png = png.into_inner();
    let file = dir.path().join("Photo.jpg");
    std::fs::write(&file, &png).unwrap();
    assert_eq!(crate::node::media_type(&file), "image/jpeg");
    assert_eq!(crate::node::media_type_of(&file), "image/png");
    let labelled = |data: &[u8], media_type: &str| Attachment {
      hash: blobs.put_bytes(data).unwrap(),
      media_type: media_type.into(),
      name: Some("Photo.jpg".into()),
      bytes: data.len() as u64,
    };
    let old = labelled(&png, "image/jpeg");
    let sent = attachment_content(&blobs, &old).unwrap();
    assert!(
      matches!(&sent[1], Content::Image { media_type, data } if media_type == "image/png" && *data == png)
    );
    let Content::Image { media_type, .. } = content_for(&blobs, &old).unwrap() else {
      panic!("not an image");
    };
    assert_eq!(media_type, "image/png");
    // Unknown to the name, known to the bytes.
    let unnamed = labelled(&png, "application/octet-stream");
    assert!(
      matches!(&attachment_content(&blobs, &unnamed).unwrap()[1], Content::Image { media_type, .. } if media_type == "image/png")
    );
    let mut heic = 24u32.to_be_bytes().to_vec();
    heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
    let heic = labelled(&heic, "image/jpeg");
    assert!(
      matches!(&attachment_content(&blobs, &heic).unwrap()[1], Content::Image { media_type, .. } if media_type == "image/heic")
    );
  }

  /// A thread that reported reads as working again, on every node, from
  /// the message sent to it (an assignment) or the turn started on it (a
  /// claim of it) until that turn's report; a reply to a woven message
  /// ends a message's turn, not one the node started.
  #[test]
  fn a_thread_sent_a_message_after_its_report_is_working_until_it_reports() {
    use stitch_records::{Author, Draft, NodeName, Ref};
    use stitch_store::Role;
    let dir = tempfile::tempdir().unwrap();
    let mbp: NodeName = "mbp".parse().unwrap();
    let gamer: NodeName = "gamer".parse().unwrap();
    let root = RecordRoot::init(dir.path(), "home".parse().unwrap(), Role::Home).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    let publish = |node: &NodeName, draft: Draft, index: &mut Index| {
      // Records a moment apart, as a turn's are.
      std::thread::sleep(std::time::Duration::from_millis(3));
      let id = root.publish(node, draft).unwrap().id;
      index.refresh(&root).unwrap();
      id
    };
    let thread = publish(
      &mbp,
      Draft::new(kinds::THREAD, Author::user(mbp.clone()))
        .unwrap()
        .field("detached", true)
        .unwrap(),
      &mut index,
    );
    let state = |index: &Index| {
      let t = index.record(&thread).unwrap().unwrap();
      detached_state(index, &t).unwrap().unwrap()
    };
    let report = |woven: bool, index: &mut Index| {
      let mut d = Draft::new(kinds::REPORT, Author::node(mbp.clone()))
        .unwrap()
        .field("outcome", "done")
        .unwrap()
        .reference("thread", Ref::to(thread.clone()))
        .unwrap();
      if woven {
        d = d.field("woven", true).unwrap();
      }
      publish(&mbp, d, index);
    };
    let message = |index: &mut Index| {
      let d = Draft::new(kinds::ASSIGNMENT, Author::user(gamer.clone()))
        .unwrap()
        .field("node", "mbp")
        .unwrap()
        .field("thread", thread.as_str())
        .unwrap()
        .field("continue", true)
        .unwrap()
        .reference("thread", Ref::to(thread.clone()))
        .unwrap();
      publish(&gamer, d, index)
    };
    let turn = |index: &mut Index| {
      let d = Draft::new(kinds::CLAIM, Author::node(mbp.clone()))
        .unwrap()
        .reference("thread", Ref::to(thread.clone()))
        .unwrap();
      publish(&mbp, d, index);
    };
    assert_eq!(state(&index), "running");
    report(false, &mut index);
    assert_eq!(state(&index), "done");
    // A message sent from gamer: working there at once.
    message(&mut index);
    assert_eq!(state(&index), "running");
    // mbp takes it and runs a turn: still working, until its report.
    turn(&mut index);
    assert_eq!(state(&index), "running");
    report(false, &mut index);
    assert_eq!(state(&index), "done");
    // A turn the node starts (reports woke it): working until it reports;
    // a reply to a message it took on the way does not end it.
    turn(&mut index);
    assert_eq!(state(&index), "running");
    report(true, &mut index);
    assert_eq!(state(&index), "running");
    report(false, &mut index);
    assert_eq!(state(&index), "done");
    // A message woven into a loop that writes no report of its own (the
    // user's chat): its reply ends it.
    message(&mut index);
    assert_eq!(state(&index), "running");
    report(true, &mut index);
    assert_eq!(state(&index), "done");
  }

  /// A mock Anthropic API: `responses` (SSE bodies) in order, one per
  /// connection.
  async fn serve(responses: Vec<String>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
      for response in responses {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
          let n = sock.read(&mut chunk).await.unwrap();
          buf.extend_from_slice(&chunk[..n]);
          if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
            let len: usize = head
              .lines()
              .find_map(|l| {
                l.strip_prefix("content-length:")
                  .map(|v| v.trim().parse().unwrap())
              })
              .unwrap_or(0);
            while buf.len() < end + 4 + len {
              let n = sock.read(&mut chunk).await.unwrap();
              buf.extend_from_slice(&chunk[..n]);
            }
            break;
          }
        }
        let body = format!(
          "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{response}"
        );
        sock.write_all(body.as_bytes()).await.unwrap();
        sock.shutdown().await.unwrap();
      }
    });
    base
  }

  fn sse(events: Vec<Value>) -> String {
    events
      .iter()
      .map(|d| format!("event: {}\ndata: {d}\n\n", d["type"].as_str().unwrap()))
      .collect()
  }

  fn sse_reply(block: Value, delta: Value, stop: &str) -> String {
    sse(vec![
      json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 10}}}),
      json!({"type": "content_block_start", "index": 0, "content_block": block}),
      json!({"type": "content_block_delta", "index": 0, "delta": delta}),
      json!({"type": "content_block_stop", "index": 0}),
      json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 5}}),
      json!({"type": "message_stop"}),
    ])
  }

  /// Tools whose first `hang` calls never return (the node stops in
  /// them): executing, or with `waits`, waiting on the user (marked as an
  /// approval's asking marks it).
  struct Hanging<'i> {
    calls: std::sync::atomic::AtomicUsize,
    hang: usize,
    called: tokio::sync::Notify,
    waits: Option<&'i Mutex<Index>>,
  }

  impl ToolRunner for Hanging<'_> {
    fn definitions(&self) -> Vec<stitch_providers::ToolDef> {
      Vec::new()
    }
    fn call<'a>(
      &'a self,
      thread: &'a RecordId,
      _call: &'a str,
      _name: &'a str,
      _input: &'a Value,
    ) -> crate::tools::BoxFuture<'a, crate::tools::ToolOutcome> {
      let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
      Box::pin(async move {
        if let Some(index) = self.waits {
          waiting_on_user(index, thread, true);
        }
        self.called.notify_one();
        if n <= self.hang {
          std::future::pending::<()>().await;
        }
        if let Some(index) = self.waits {
          waiting_on_user(index, thread, false);
        }
        crate::tools::ToolOutcome::ok("ran")
      })
    }
  }

  /// What a node killed in a call leaves: its step kept with the call
  /// running, by a process that is gone (the loop's future dropped here
  /// clears the running call, as a node that stops cleanly does).
  fn node_died(ctx: &ChatContext<'_>, thread: &RecordId, call: &str) {
    let index = ctx.index.lock().unwrap();
    let key = pending_key(thread);
    let mut v: Value = serde_json::from_str(&index.local(&key).unwrap().unwrap()).unwrap();
    v["running"] = json!({"id": call, "since": now().to_string()});
    v["pid"] = json!(0);
    index.set_local(&key, Some(&v.to_string())).unwrap();
  }

  /// How the node stops in a call: killed (its step left with the call
  /// running), or cleanly (SIGTERM, a Stop: `abort_all` drops the loop's
  /// future, as the tests do, and its guard clears the running call).
  #[derive(Clone, Copy)]
  enum Stop {
    Crash,
    Clean,
  }

  /// A thread whose call the node stops in `stops` times (the tools hang
  /// in it that often), resumed after each: the calls run, the call's
  /// result in the history (its text, whether an error), the last reply.
  async fn stopped_in_a_call(
    stop: Stop,
    waits: bool,
    stops: usize,
  ) -> (usize, (String, bool), String) {
    use stitch_store::Role as StoreRole;
    let dir = tempfile::tempdir().unwrap();
    let node: NodeName = "geek".parse().unwrap();
    let root = RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      StoreRole::Home,
    )
    .unwrap();
    let index = Mutex::new(Index::open_in_memory().unwrap());
    let blobs = BlobStore::open(&dir.path().join("blobs")).unwrap();
    let ctx = ChatContext {
      node: &node,
      root: &root,
      index: &index,
      blobs: &blobs,
    };
    let base = serve(vec![
      sse_reply(
        json!({"type": "tool_use", "id": "t1", "name": "shell_run", "input": {}}),
        json!({"type": "input_json_delta", "partial_json": json!({"command": "pkill -f \"stitch node\""}).to_string()}),
        "tool_use",
      ),
      sse_reply(
        json!({"type": "text", "text": ""}),
        json!({"type": "text_delta", "text": "Done."}),
        "end_turn",
      ),
    ])
    .await;
    let provider = Provider::Anthropic(stitch_providers::Anthropic {
      base_url: base,
      ..stitch_providers::Anthropic::new("k")
    });
    let tools = Hanging {
      calls: Default::default(),
      hang: stops,
      called: Default::default(),
      waits: waits.then_some(&index),
    };
    let thread = RecordId::random(Timestamp::now(), &node);
    let input = ChatInput {
      thread: None,
      id: Some(thread.clone()),
      text: "Restart the node".into(),
      attachments: Vec::new(),
      provider: provider.clone(),
      model: "claude-opus-5-5".into(),
      system: None,
      from: None,
      detached: None,
      limits: Limits::default(),
      recalled: None,
      instruction: None,
      context: None,
      folder: None,
      steer: None,
      spoken: false,
      scope: None,
      room_turn: None,
    };
    let stopped = || {
      if let Stop::Crash = stop {
        node_died(&ctx, &thread, "t1");
      }
    };
    let mut quiet = |_: ChatEvent| {};
    // The node stops in the call.
    tokio::select! {
      r = send(&ctx, input, &tools, &mut quiet) => panic!("the loop ended: {:?}", r.map(|o| o.text)),
      () = tools.called.notified() => {}
    }
    stopped();
    // It starts again, the call is replayed, and the node stops in it
    // again, as often as asked.
    for _ in 1..stops {
      let mut quiet = |_: ChatEvent| {};
      let resumed = resume(
        &ctx,
        &thread,
        &provider,
        "claude-opus-5-5",
        Limits::default(),
        &tools,
        None,
        &mut quiet,
      );
      tokio::select! {
        r = resumed => {
          // Not run again: the loop finished without it.
          let text = r.unwrap().map(|o| o.text).unwrap_or_default();
          return (tools.calls.load(Ordering::SeqCst), call_result(&ctx, &thread), text);
        }
        () = tools.called.notified() => {}
      }
      stopped();
    }
    // Started again: the loop goes on to its end.
    let outcome = tokio::time::timeout(
      std::time::Duration::from_secs(20),
      resume(
        &ctx,
        &thread,
        &provider,
        "claude-opus-5-5",
        Limits::default(),
        &tools,
        None,
        &mut quiet,
      ),
    )
    .await
    .expect("the loop finishes")
    .unwrap()
    .expect("resumed");
    (
      tools.calls.load(Ordering::SeqCst),
      call_result(&ctx, &thread),
      outcome.text,
    )
  }

  /// Call `t1`'s result in `thread`'s history: its text, whether an error.
  fn call_result(ctx: &ChatContext<'_>, thread: &RecordId) -> (String, bool) {
    history(ctx, thread)
      .unwrap()
      .messages
      .iter()
      .flat_map(|m| &m.content)
      .find_map(|c| match c {
        Content::ToolResult {
          id,
          output,
          is_error,
        } if id == "t1" => Some((output.clone(), *is_error)),
        _ => None,
      })
      .expect("its result")
  }

  /// 2026-11-02 (n6rx): a call that stopped its node (`pkill -f "stitch
  /// node"`, a SIGTERM: a clean stop) was replayed on every start,
  /// stopping it again, in a loop. A call executing when the node stopped
  /// is replayed once, however the node stopped; the node stopping again
  /// in it, it gets an error result instead, and the loop goes on.
  #[tokio::test(flavor = "multi_thread")]
  async fn a_call_the_node_stopped_in_twice_is_not_run_a_third_time() {
    for stop in [Stop::Crash, Stop::Clean] {
      let (calls, (output, error), text) = stopped_in_a_call(stop, false, 2).await;
      assert_eq!(calls, 2, "run once, replayed once");
      assert!(error, "an error: {output}");
      assert!(output.contains("not run again"), "{output}");
      assert_eq!(text, "Done.");
    }
  }

  /// A call waiting on the user (an approval, `wait`) when the node stops
  /// is replayed however often it stops, its question with it.
  #[tokio::test(flavor = "multi_thread")]
  async fn a_call_waiting_on_the_user_is_replayed_after_every_stop() {
    for stop in [Stop::Crash, Stop::Clean] {
      let (calls, (output, error), text) = stopped_in_a_call(stop, true, 3).await;
      assert_eq!(calls, 4, "replayed after each stop");
      assert!(!error, "{output}");
      assert_eq!(output, "ran");
      assert_eq!(text, "Done.");
    }
  }
}
