//! Memory on the node (design §2.9): engram over EmbeddingGemma 2, folded
//! from `memory` records, with edge talk's field-tested policy.
//!
//! - **Facts are records.** An exchange's credit and dismissals and its
//!   messages' observations go into one `memory` record in the thread's
//!   repository; a thread leaving or rejoining memory too. A sleep (once
//!   every twenty hours, fleet-wide) and a gist go into the home workspace.
//! - **The graph is a fold** (stitch-memory): every node folds every memory
//!   record in ID order and gets the same graph and log.
//! - **A node observes what happens in front of it:** the turns it runs,
//!   and the threads it is told to bring into memory. A thread's messages
//!   not observed yet (a crash, a thread from before memory) are observed
//!   on its next exchange here.
//! - **Unasked recall** (edge talk's ambient channel): what observations
//!   surfaced parks per thread; before a turn, the user's message is the
//!   cue (embedded as it will be stored), its novelty sets how close a
//!   memory must be, the cortex proposes too, and every candidate is
//!   resolved, floored, deduplicated against what the thread was already
//!   given and verified against the cue. Only a cue with a topic gets a
//!   note; a thread with an unanswered note gets none; whatever was not
//!   used waits for the next turn. The note is part of the message (not
//!   the system prompt) and invites an invisible dismissal.
//! - **Asked recall:** the `recall` tool (episodic memory, then a keyword
//!   search over every thread).
//! - **Feedback:** what helped is credited, what was dismissed (by marker,
//!   or by citing it without using it) is knocked; sleep knocks what is
//!   shown again and again without helping; quiet clusters become gists.
//! - **Follow-ups:** when an exchange surfaces something new, the model may
//!   add one afterthought, unprompted, at most once per message of the
//!   user's and a minute apart; it may decline with an invisible marker.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::{Value, json};
use stitch_memory::{
  AMBIENT_BUDGET, AMBIENT_BUFFER, ANSWER_WORDS, Applied, At, Encoder, Fact, Fold, Folding,
  ModelFiles, PROPOSER_TOP, QUOTE_WORDS, SAME_THREAD_DUP_OVERLAP, Stats,
};
use stitch_net::NodeState;
use stitch_providers::{Content, Message, Request, ToolDef};
use stitch_records::{Author, Draft, Event, Msg, Part, RecordId, Role, kinds};
use stitch_store::RecordRoot;

use crate::chat::{ChatContext, thread_events};
use crate::config::{Config, MemorySettings};
use crate::tools::ToolOutcome;

/// Hours between sleeps, fleet-wide.
const SLEEP_HOURS: i64 = 20;
/// Only what happened this recently feeds unasked recall: catching up on
/// a thread's history is ingestion, not experience.
const RECENT_MINUTES: i64 = 30;
/// Messages observed per record when catching up.
const HEAL_BATCH: usize = 256;
/// What catching up leaves free to embed an image: the vision projector's
/// compute buffers (an image is at most 256 tokens) and room to spare.
/// Where the OS caps an app's memory (iOS) and less is left, the image
/// waits for a later catch-up: a background backfill must not be what an
/// out-of-memory crash comes from. Live observations are not held back.
const HEAL_IMAGE_HEADROOM: u64 = 1 << 30;

/// Whether catching up may embed an image, given how much more memory the
/// process may take (`None`: no OS limit to mind).
fn heal_may_embed_image(available: Option<u64>) -> bool {
  available.is_none_or(|a| a >= HEAL_IMAGE_HEADROOM)
}
/// The least time between two follow-ups in a thread.
const FOLLOW_UP_FLOOR: Duration = Duration::from_secs(60);
/// The longest text encoded, in tokens, unless `[memory] context` says:
/// the encoder's compute buffers grow with it (about 1.5 GB at 8192), too
/// much for a phone, where a longer message is encoded by its beginning.
#[cfg(not(any(target_os = "ios", target_os = "visionos")))]
const CONTEXT: u32 = 8192;
#[cfg(any(target_os = "ios", target_os = "visionos"))]
const CONTEXT: u32 = 2048;
/// Keyword-search hits a recall adds, and their words.
const ARCHIVE_TOP: usize = 5;
const ARCHIVE_WORDS: usize = 80;
/// Words of a gist's fragment; the gist writer's answer at most.
const GIST_FRAGMENT_WORDS: usize = 80;
const GIST_MAX_TOKENS: u32 = 256;

const NOTE_HEADER: &str = "Associative memory surfaced these past messages as possibly relevant. \
  Use them only if they actually help; otherwise ignore them silently. ";
const NOTE_WANDER: &str = "The current topic looks like new territory (no established memories \
  nearby), so associative memory wandered: what follows is a DISTANT association — a stray \
  thought, not a match. Use it only if it sparks something; otherwise ignore it silently. ";

const GIST_SYSTEM: &str = "You distill episodic memory fragments from past conversations into \
  one durable long-term memory. You are precise and never invent details.";
const GIST_INSTRUCTION: &str = "\n\n---\n\nWrite the durable memory these numbered fragments \
  amount to: ONE short paragraph, at most 60 words. Keep concrete specifics (names, numbers, \
  files, decisions, outcomes); leave out conversational filler and anything transient. Output \
  ONLY the paragraph — no preamble, no quotes, no markdown.\n\nAdditionally: if any fragments \
  are PURE conversational chaff (status noise, fillers, standby lines) containing nothing \
  durable at all, add one final line exactly of the form `DROP: 2, 5` listing their numbers. \
  Never list a fragment that mentions a fact, name, number, file, or decision. If none \
  qualify, add no such line.";

#[derive(Default)]
struct State {
  /// Everything folded, the fold, and its checkpoints (on disk, under the
  /// data directory's `memory-fold`: a local cache, never synced).
  folding: Folding,
  folded: BTreeSet<String>,
  last_sleep: Option<RecordId>,
  /// Threads the user deleted: what was observed of them is not folded.
  deleted: BTreeSet<String>,
}

impl State {
  fn fold(&self) -> &Fold {
    self.folding.fold()
  }
}

/// Where the fold's checkpoints are kept.
fn checkpoints_dir(data: &Path) -> PathBuf {
  data.join("memory-fold")
}

/// An observation of a message of one of `threads`.
fn observes(f: &Fact, threads: &BTreeSet<String>) -> bool {
  matches!(f, Fact::Observe { place, .. }
    if stitch_memory::thread_of(place).is_some_and(|t| threads.contains(t)))
}

/// Whether a memory record holds only what came from `threads`: their
/// messages' observations, the feedback of their notes, their leaving or
/// rejoining memory (an exchange's record, written in the thread's
/// repository).
fn only_of(fields: &Value, threads: &BTreeSet<String>) -> bool {
  let of = |place: Option<&str>| {
    place
      .and_then(stitch_memory::thread_of)
      .is_some_and(|t| threads.contains(t))
  };
  let Some(facts) = fields.get("facts").and_then(Value::as_array) else {
    return false;
  };
  !facts.is_empty()
    && facts.iter().all(|f| {
      if let Some(t) = f.get("thread").and_then(Value::as_str) {
        threads.contains(t)
      } else if f.get("at").is_some() {
        of(f.get("at").and_then(Value::as_str))
      } else if f.get("credit").is_some() || f.get("penalize").is_some() {
        of(f.get("note").and_then(Value::as_str))
      } else {
        false
      }
    })
}

/// A thread's cue: the user's last message, as stored, and its novelty.
#[derive(Clone)]
struct Cue {
  raw: Vec<f32>,
  novelty: f32,
  text: String,
}

/// What this node keeps about a thread between turns (edge talk kept it
/// in its globals: it is attention, not memory).
#[derive(Default)]
struct Session {
  /// Surfaced memories waiting for a turn, newest first.
  buffer: Vec<u64>,
  cue: Option<Cue>,
  /// The user spoke since the last follow-up.
  armed: bool,
  last_follow_up: Option<Instant>,
  /// What the `recall` tool showed this exchange.
  tool_shown: Vec<u64>,
  /// Embeddings made for a cue, kept to observe its message with.
  embedded: HashMap<String, Vec<f32>>,
  /// Where the last note went (its message's place), for the feedback on
  /// it to name.
  note_place: Option<String>,
}

/// A memory as shown to the model.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
  pub handle: u64,
  /// What the model read (a question with its answer).
  pub text: String,
  /// The memory's own text, which the substance floor judges.
  pub own: String,
}

/// An unasked note: the block the model reads, the handles it holds and
/// what it showed.
#[derive(Clone, Debug)]
pub(crate) struct Note {
  pub block: String,
  pub handles: Vec<String>,
  pub shown: Vec<Shown>,
  /// How it was chosen, written with it (see `Part::Recalled`).
  pub measures: Value,
}

pub(crate) struct Memory {
  encoder: Option<Arc<Encoder>>,
  state: Mutex<State>,
  sessions: Mutex<HashMap<String, Session>>,
  archive: Mutex<Archive>,
}

fn tag() -> String {
  format!("{:016x}", stitch_memory::center().tag_u64)
}

fn hex(h: u64) -> String {
  format!("{h:016x}")
}

fn unhex(s: &str) -> Option<u64> {
  (s.len() == 16)
    .then(|| u64::from_str_radix(s, 16).ok())
    .flatten()
}

/// Downloads EmbeddingGemma 2 (and, with `images`, its vision projector)
/// into the data directory's `models/`; the files there.
pub async fn install(
  data: &Path,
  images: bool,
  mut progress: impl FnMut(&str, u64, u64),
) -> crate::Result<Vec<PathBuf>> {
  let dir = models_dir(data);
  std::fs::create_dir_all(&dir).map_err(crate::Error::io(dir.display()))?;
  let http = crate::web::http_client();
  let mut names = vec![ModelFiles::MODEL];
  if images {
    names.push(ModelFiles::PROJECTOR);
  }
  let mut out = Vec::new();
  for name in names {
    let path = dir.join(name);
    if !path.is_file() {
      let url = format!("{}/{name}", ModelFiles::SOURCE);
      let mut response = http
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| crate::Error::Config(format!("{url}: {e}")))?;
      let total = response.content_length().unwrap_or(0);
      let partial = path.with_extension("partial");
      let mut file =
        std::fs::File::create(&partial).map_err(crate::Error::io(partial.display()))?;
      let mut got = 0u64;
      while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| crate::Error::Config(e.to_string()))?
      {
        std::io::Write::write_all(&mut file, &chunk)
          .map_err(crate::Error::io(partial.display()))?;
        got += chunk.len() as u64;
        progress(name, got, total);
      }
      std::fs::rename(&partial, &path).map_err(crate::Error::io(path.display()))?;
    }
    out.push(path);
  }
  Ok(out)
}

/// Removes the model files `install` downloads (memory is off from the
/// node's next start; its facts stay, synced); `images`: only the vision
/// projector.
pub fn uninstall(data: &Path, images: bool) -> crate::Result<()> {
  let dir = models_dir(data);
  let mut names = vec![ModelFiles::PROJECTOR];
  if !images {
    names.push(ModelFiles::MODEL);
  }
  for name in names {
    let path = dir.join(name);
    match std::fs::remove_file(&path) {
      Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
        return Err(crate::Error::io(path.display())(e));
      }
      _ => {}
    }
  }
  Ok(())
}

/// The models installed: the text model, and the vision projector.
pub fn installed(data: &Path) -> (bool, bool) {
  let dir = models_dir(data);
  (
    dir.join(ModelFiles::MODEL).is_file(),
    dir.join(ModelFiles::PROJECTOR).is_file(),
  )
}

/// The models directory in the data directory.
pub(crate) fn models_dir(data: &Path) -> PathBuf {
  data.join("models")
}

/// What the model is told when memory follows up unprompted.
pub(crate) fn follow_up_instruction() -> String {
  format!(
    "(These memories surfaced associatively after your previous reply — the user has NOT sent \
     a new message. Continue naturally: if a memory adds a worthwhile connection, mention it \
     briefly as an afterthought; keep it short either way. If nothing is worth adding, reply \
     with only the invisible character \"{}\" and nothing else — the user will see no message \
     at all.)",
    stitch_memory::DISMISS
  )
}

/// A message's own text: its text parts, without notes Stitch added.
fn message_text(parts: &[Part]) -> String {
  parts
    .iter()
    .filter_map(|p| match p {
      Part::Text { text } => Some(text.as_str()),
      _ => None,
    })
    .collect::<Vec<_>>()
    .join("\n")
}

/// At most `max` words (split at spaces, as edge talk did), and `more`
/// after them when cut.
fn cap_words(text: &str, max: usize, more: &str) -> String {
  let words: Vec<&str> = text.split(' ').collect();
  if words.len() <= max {
    text.to_owned()
  } else {
    format!("{}{more}", words[..max].join(" "))
  }
}

/// A thread's messages with, for each, whether it is a final answer (an
/// assistant message with text that asked for no tools).
fn messages_and_answers(events: &[Event]) -> Vec<(&Msg, bool)> {
  let mut out: Vec<(&Msg, bool)> = Vec::new();
  for (i, e) in events.iter().enumerate() {
    if let Event::Msg(m) = e {
      let calls_tools = events[i + 1..]
        .iter()
        .take_while(|n| !matches!(n, Event::Msg(_)))
        .any(|n| matches!(n, Event::ToolCall(_)));
      let text = message_text(&m.parts);
      let answer = m.role == Role::Assistant
        && !calls_tools
        && !text.trim().is_empty()
        && !stitch_memory::marker_only(&text);
      out.push((m, answer));
    }
  }
  out
}

/// Whether the thread's latest note is still waiting for an answer (its
/// exchange was cancelled): one unanswered note at most.
fn note_unanswered(events: &[Event]) -> bool {
  let msgs = messages_and_answers(events);
  let Some(last) = msgs
    .iter()
    .rposition(|(m, _)| m.parts.iter().any(|p| matches!(p, Part::Recalled { .. })))
  else {
    return false;
  };
  !msgs[last + 1..].iter().any(|(_, answer)| *answer)
}

/// Handles a thread was already given: its notes', and those its `recall`
/// results cite.
fn noted(events: &[Event]) -> BTreeSet<u64> {
  let mut out = BTreeSet::new();
  for e in events {
    match e {
      Event::Msg(m) => {
        for p in &m.parts {
          if let Part::Recalled { handles, .. } = p {
            out.extend(handles.iter().filter_map(|h| unhex(h)));
          }
        }
      }
      Event::ToolResult(r) => {
        out.extend(cited(
          r.output.get("text").and_then(Value::as_str).unwrap_or(""),
        ));
      }
      _ => {}
    }
  }
  out
}

/// Handles cited as `[id <16 hex>` in a text.
fn cited(text: &str) -> Vec<u64> {
  text
    .match_indices("[id ")
    .filter_map(|(i, _)| text.get(i + 4..i + 20).and_then(unhex))
    .collect()
}

/// A resolved memory.
struct Recollection {
  handle: u64,
  sender: String,
  at: Timestamp,
  title: String,
  /// Its own text.
  text: String,
  /// For a user's message, the answer that followed.
  answer: Option<String>,
  stats: Option<Stats>,
}

impl Recollection {
  /// The text the model reads: a question with its answer.
  fn paired(&self) -> String {
    match &self.answer {
      Some(a) => format!(
        "{} ↳ answered: {}",
        self.text,
        cap_words(
          a,
          ANSWER_WORDS,
          " (…answer truncated; recall retrieves it in full)"
        )
      ),
      None => self.text.clone(),
    }
  }

  /// One note line: the bracket is provenance for the model and the mark
  /// the thread's dedup reads.
  fn line(&self, text: &str) -> String {
    let days = Timestamp::now().duration_since(self.at).as_secs().max(0) / 86_400;
    let title = if self.title.is_empty() {
      String::new()
    } else {
      format!(", in \"{}\"", self.title)
    };
    let s = self.stats.unwrap_or(Stats {
      strength: 1.0,
      count: 1,
      used: false,
    });
    format!(
      "- [id {}, {days}d ago, {}{title}, seen {}x, strength {:.1}{}]: {text}\n",
      hex(self.handle),
      self.sender,
      s.count,
      s.strength,
      if s.used { ", helped before" } else { "" }
    )
  }
}

impl Memory {
  pub fn new(data: &Path, settings: &MemorySettings) -> Memory {
    let encoder = (settings.enabled != Some(false))
      .then(|| ModelFiles::in_dir(&models_dir(data)))
      .flatten()
      .map(|files| {
        Arc::new(Encoder::new(
          files,
          settings.images != Some(false),
          settings.gpu != Some(false),
          settings.context.unwrap_or(CONTEXT),
        ))
      });
    // A checkpoint holds what one build's fold made: a build that does not
    // say what it was built from (a test, a checkout with changes) keeps
    // its checkpoints in memory only.
    let build = crate::build();
    let folding = if build == "unknown" || build.contains("dirty") {
      Folding::new()
    } else {
      Folding::open(&checkpoints_dir(data), build)
    };
    Memory {
      encoder,
      state: Mutex::new(State {
        folding,
        ..State::default()
      }),
      sessions: Mutex::new(HashMap::new()),
      archive: Mutex::new(Archive::default()),
    }
  }

  /// Whether this node has memory (the model is installed, not turned off).
  pub fn available(&self) -> bool {
    self.encoder.is_some()
  }

  /// Suspends the encoder while an app is in the background (unloaded,
  /// nothing encoded: what goes unobserved is caught up on the thread's
  /// next exchange), or resumes it.
  pub fn suspend(&self, on: bool) {
    if let Some(e) = &self.encoder {
      e.suspend(on);
    }
  }

  /// The encoder is suspended: a sleep waits, since its gist could not be
  /// encoded.
  pub fn suspended(&self) -> bool {
    self.encoder.as_ref().is_some_and(|e| e.suspended())
  }

  /// Facts folded and episodes in the graph.
  pub fn size(&self) -> (usize, usize) {
    let s = self.state.lock().expect("memory lock");
    (s.fold().len(), s.fold().episodes())
  }

  /// Whether a thread is out of memory.
  pub fn is_off(&self, thread: &RecordId) -> bool {
    self
      .state
      .lock()
      .expect("memory lock")
      .fold()
      .is_off(thread.as_str())
  }

  /// Whether a thread may search memory itself (the `recall` tool): in
  /// memory, or out of it with recall kept.
  pub fn may_recall(&self, thread: &RecordId) -> bool {
    self
      .state
      .lock()
      .expect("memory lock")
      .fold()
      .may_recall(thread.as_str())
  }

  /// Whether a thread's memory was ever switched, on or off (so a place's
  /// default no longer applies to it).
  pub fn is_switched(&self, thread: &RecordId) -> bool {
    self
      .state
      .lock()
      .expect("memory lock")
      .fold()
      .is_switched(thread.as_str())
  }

  fn with_session<T>(&self, thread: &RecordId, f: impl FnOnce(&mut Session) -> T) -> T {
    let mut s = self.sessions.lock().expect("sessions lock");
    f(s.entry(thread.to_string()).or_default())
  }

  /// Folds the memory records not folded yet (after a write, a sync, a
  /// start); what each new record's observations did.
  pub fn refresh(&self, state: &NodeState) -> HashMap<String, Vec<Applied>> {
    let records = state
      .index
      .lock()
      .expect("index lock")
      .records(Some(kinds::MEMORY))
      .unwrap_or_default();
    let mut s = self.state.lock().expect("memory lock");
    let mut new: Vec<(At, Fact)> = Vec::new();
    for r in records {
      let id = r.id.to_string();
      if s.folded.contains(&id) {
        continue;
      }
      s.folded.insert(id.clone());
      if r.fields.get("model").and_then(Value::as_str) != Some(tag().as_str()) {
        continue;
      }
      let secs = r.id.timestamp().as_second();
      for (i, f) in r
        .fields
        .get("facts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
      {
        if let Some(fact) = parse(f) {
          if observes(&fact, &s.deleted) {
            continue;
          }
          if fact == Fact::Sleep && s.last_sleep.as_ref().is_none_or(|l| &r.id > l) {
            s.last_sleep = Some(r.id.clone());
          }
          new.push((At::new(id.clone(), i as u32, secs), fact));
        }
      }
    }
    s.folding.add(new)
  }

  /// The memory records holding only what came from `threads` (see
  /// `only_of`), to go with them.
  pub fn records_only_of(&self, state: &NodeState, threads: &BTreeSet<RecordId>) -> Vec<RecordId> {
    let threads: BTreeSet<String> = threads.iter().map(ToString::to_string).collect();
    state
      .index
      .lock()
      .expect("index lock")
      .records(Some(kinds::MEMORY))
      .unwrap_or_default()
      .into_iter()
      .filter(|r| only_of(&r.fields, &threads))
      .map(|r| r.id)
      .collect()
  }

  /// Forgets deleted threads (design §2.5, "Deleting a thread"): the
  /// graph is folded again without `records` (gone with them) and without
  /// any observation of their messages, here and in what comes later; what
  /// this node kept of them between turns and in the keyword search goes.
  pub fn forget(&self, threads: &BTreeSet<RecordId>, records: &[RecordId]) {
    let ids: BTreeSet<String> = threads.iter().map(ToString::to_string).collect();
    let records: BTreeSet<&str> = records.iter().map(RecordId::as_str).collect();
    {
      let mut s = self.state.lock().expect("memory lock");
      s.deleted.extend(ids.iter().cloned());
      let deleted = s.deleted.clone();
      s.folding
        .retain(|at, f| !records.contains(at.record.as_str()) && !observes(f, &deleted));
    }
    let mut sessions = self.sessions.lock().expect("sessions lock");
    for t in &ids {
      sessions.remove(t);
    }
    drop(sessions);
    let mut archive = self.archive.lock().expect("archive lock");
    for t in threads {
      archive.threads.remove(t);
    }
  }

  /// Writes facts as a `memory` record into `root` and folds it; what its
  /// observations did.
  pub fn write(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    facts: Vec<Fact>,
  ) -> crate::Result<Vec<Applied>> {
    self.write_noted(state, root, facts, None)
  }

  /// `write`, the feedback facts naming the note they judge (its
  /// message's place), so a note's measures meet its outcome.
  fn write_noted(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    facts: Vec<Fact>,
    note: Option<&str>,
  ) -> crate::Result<Vec<Applied>> {
    if facts.is_empty() {
      return Ok(Vec::new());
    }
    let rows: Vec<toml::Value> = facts
      .iter()
      .map(|f| {
        let mut row = to_toml(f);
        if let (Some(place), Fact::Credit { .. } | Fact::Penalize { .. }, toml::Value::Table(t)) =
          (note, f, &mut row)
        {
          t.insert("note".into(), place.into());
        }
        row
      })
      .collect();
    let record = Draft::new(kinds::MEMORY, Author::node(state.node.clone()))?
      .field("model", tag())?
      .field("facts", toml::Value::Array(rows))?
      .into_record(self.next_id(state), Timestamp::now());
    root.publish_record(&record)?;
    state.index.lock().expect("index lock").refresh(root)?;
    Ok(
      self
        .refresh(state)
        .remove(record.id.as_str())
        .unwrap_or_default(),
    )
  }

  /// An ID for this node's next memory record that sorts after everything
  /// folded, so its own writes fold on top: IDs are to the second, and one
  /// written in the same second could sort before the last and refold it
  /// all.
  fn next_id(&self, state: &NodeState) -> RecordId {
    let last: Option<RecordId> = self
      .state
      .lock()
      .expect("memory lock")
      .fold()
      .last_record()
      .and_then(|r| r.parse().ok());
    let mut at = Timestamp::now();
    loop {
      if let Some(l) = &last
        && at < l.timestamp()
      {
        at = l.timestamp();
      }
      for _ in 0..16 {
        let id = RecordId::random(at, &state.node);
        if last.as_ref().is_none_or(|l| &id > l) {
          return id;
        }
      }
      at += jiff::SignedDuration::from_secs(1);
    }
  }

  /// A sleep, when the fleet has not slept for twenty hours and there is
  /// memory to consolidate; whether one was written.
  pub fn maybe_sleep(&self, state: &NodeState) -> crate::Result<bool> {
    let due = {
      let s = self.state.lock().expect("memory lock");
      let since = s.last_sleep.as_ref().map(RecordId::timestamp).or_else(|| {
        s.folding
          .facts()
          .first()
          .and_then(|(at, _)| at.record.parse::<RecordId>().ok())
          .map(|r| r.timestamp())
      });
      !s.fold().is_empty()
        && since.is_some_and(|t| {
          Timestamp::now().duration_since(t) >= jiff::SignedDuration::from_hours(SLEEP_HOURS)
        })
    };
    if due && let Some(home) = state.home() {
      self.write(state, &home, vec![Fact::Sleep])?;
      return Ok(true);
    }
    Ok(false)
  }

  async fn embed(&self, text: &str, query: bool) -> Option<Vec<f32>> {
    let enc = self.encoder.clone()?;
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || {
      if query {
        enc.query(&text).ok()
      } else {
        enc.document(&text).ok()
      }
    })
    .await
    .ok()
    .flatten()
  }

  /// A handle's memory: its text, who said it, when, in which thread.
  fn resolve(&self, state: &NodeState, h: u64) -> Option<Recollection> {
    let (place, stats, gist, secs) = {
      let s = self.state.lock().expect("memory lock");
      let place = s.fold().resolve(h)?.to_owned();
      let gist = place
        .strip_prefix("gist:")
        .and_then(unhex)
        .and_then(|g| s.fold().gist(g).map(str::to_owned));
      let secs = place
        .strip_prefix("gist:")
        .and_then(unhex)
        .and_then(|g| s.fold().entry(g))
        .map_or(0, |e| e.secs);
      (place, s.fold().stats(h), gist, secs)
    };
    if let Some(text) = gist {
      return Some(Recollection {
        handle: h,
        sender: "gist".into(),
        at: Timestamp::from_second(secs).ok()?,
        title: String::new(),
        text,
        answer: None,
        stats,
      });
    }
    let (head, msg) = place.split_once('#')?;
    let thread: RecordId = stitch_memory::thread_of(&place)?.parse().ok()?;
    let (msg, image) = match msg.split_once(':') {
      Some((m, a)) => (m, Some(a)),
      None => (msg, None),
    };
    // The thread's repository now (a project split moves it).
    let repo = state
      .index
      .lock()
      .expect("index lock")
      .record(&thread)
      .ok()
      .flatten()
      .map(|r| r.repo)
      .or_else(|| head.rsplit_once(':').and_then(|(r, _)| r.parse().ok()))?;
    let root = state.root(&repo)?;
    let events = Self::events(state, &root, &thread);
    let msgs = messages_and_answers(&events);
    let at = msgs.iter().position(|(m, _)| m.id == msg)?;
    let m = msgs[at].0;
    let text = match image {
      Some(a) => {
        let i: usize = a.strip_prefix('a')?.parse().ok()?;
        match m.parts.get(i)? {
          Part::Attachment { att } => format!(
            "[an image{}]",
            att
              .name
              .as_deref()
              .map(|n| format!(" `{n}`"))
              .unwrap_or_default()
          ),
          _ => return None,
        }
      }
      None => message_text(&m.parts),
    };
    // A question's information lives in its answer: pair them.
    let answer = (m.role == Role::User && image.is_none())
      .then(|| {
        msgs[at + 1..]
          .iter()
          .find(|(_, answer)| *answer)
          .map(|(n, _)| message_text(&n.parts))
      })
      .flatten();
    let title = state
      .index
      .lock()
      .expect("index lock")
      .title(&thread)
      .ok()
      .flatten()
      .unwrap_or_default();
    Some(Recollection {
      handle: h,
      sender: match m.role {
        Role::User => "user".into(),
        _ => m.model.clone().unwrap_or_else(|| "assistant".into()),
      },
      at: m.at,
      title,
      text,
      answer,
      stats,
    })
  }

  fn events(state: &NodeState, root: &RecordRoot, thread: &RecordId) -> Vec<Event> {
    let ctx = ChatContext {
      node: &state.node,
      root,
      index: &state.index,
      blobs: &state.blobs,
    };
    thread_events(&ctx, thread).unwrap_or_default()
  }

  /// Before a turn: the user's message `text` (to be written at `place`)
  /// is the cue. Observes it on a copy of the graph (its novelty, what it
  /// surfaces), proposes by similarity, verifies every candidate; the note
  /// to send with it, if any.
  pub async fn before_turn(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    place: &str,
    text: &str,
  ) -> Option<Note> {
    if !self.available() || self.is_off(thread) {
      return None;
    }
    let mut candidates = self.with_session(thread, |s| {
      s.armed = true;
      std::mem::take(&mut s.buffer)
    });
    let mut fresh = false;
    if !text.trim().is_empty()
      && let Some(raw) = self.embed(text, false).await
    {
      let novelty = if stitch_memory::skip_marked(text) {
        // Not memory-worthy, still a cue: nothing is observed.
        0.8
      } else {
        let a = self.state.lock().expect("memory lock").fold().peek(
          place,
          &raw,
          stitch_memory::heuristic_demand(text),
        );
        for h in a.surfaced {
          if !candidates.contains(&h) {
            candidates.push(h);
          }
        }
        a.novelty
      };
      self.with_session(thread, |s| {
        s.embedded.insert(place.to_owned(), raw.clone());
        s.cue = Some(Cue {
          raw,
          novelty,
          text: text.to_owned(),
        });
      });
      fresh = true;
    }
    self.drain(state, root, thread, candidates, fresh)
  }

  /// The candidates through the note pipeline against the thread's cue;
  /// parked again when no note may be written.
  fn drain(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    mut candidates: Vec<u64>,
    fresh: bool,
  ) -> Option<Note> {
    let cue = self.with_session(thread, |s| s.cue.clone());
    let events = Self::events(state, root, thread);
    // Notes ride only cues with a topic, one unanswered note at most.
    let allowed = cue
      .as_ref()
      .is_some_and(|c| stitch_memory::substantive(&c.text))
      && !note_unanswered(&events);
    let Some(cue) = cue.filter(|_| allowed) else {
      self.with_session(thread, |s| {
        for h in std::mem::take(&mut s.buffer) {
          if !candidates.contains(&h) {
            candidates.push(h);
          }
        }
        candidates.truncate(AMBIENT_BUFFER);
        s.buffer = candidates;
      });
      return None;
    };
    let (ceiling, wandering) = stitch_memory::ambient_ceiling(cue.novelty);
    let t = thread.as_str();
    if fresh {
      // The cortex proposes too (a mixed-topic cue never seeds the graph),
      // eligible memories only, filtered before its limit; first in line.
      let proposed: Vec<u64> = {
        let s = self.state.lock().expect("memory lock");
        s.fold()
          .cortex(&cue.raw, ceiling, PROPOSER_TOP, &|h| {
            s.fold().eligible(h, Some(t))
          })
          .into_iter()
          .map(|(h, _)| h)
          .collect()
      };
      candidates.retain(|h| !proposed.contains(h));
      candidates.splice(0..0, proposed);
    }
    let already = noted(&events);
    let mut shown: Vec<Shown> = Vec::new();
    let mut measured: Vec<Value> = Vec::new();
    let mut lines = String::new();
    let mut quoted: Vec<String> = Vec::new();
    for h in candidates {
      if shown.len() >= AMBIENT_BUDGET || (wandering && !shown.is_empty()) {
        break;
      }
      // Verified against the cue: the graph proposes on activation alone.
      let (ok, raw, centered) = {
        let s = self.state.lock().expect("memory lock");
        let raw = s.fold().distance(&cue.raw, h);
        let centered = s.fold().centered_distance(&cue.raw, h);
        (
          s.fold().eligible(h, Some(t))
            && raw.is_some_and(|d| d <= ceiling)
            && centered.is_some_and(|d| d <= stitch_memory::AMBIENT_CENTERED_DISTANCE),
          raw.unwrap_or(f32::NAN),
          centered.unwrap_or(f32::NAN),
        )
      };
      if !ok || already.contains(&h) {
        continue;
      }
      let Some(r) = self.resolve(state, h) else {
        continue;
      };
      // The floor judges the memory's own words, not its paired answer.
      if !stitch_memory::substantial(&r.text) {
        continue;
      }
      let text = r.paired();
      if quoted.contains(&text) {
        continue;
      }
      quoted.push(text.clone());
      lines.push_str(&r.line(&cap_words(
        &text,
        QUOTE_WORDS,
        " (…long memory truncated; recall retrieves it in full)",
      )));
      shown.push(Shown {
        handle: h,
        text,
        own: r.text,
      });
      measured.push(json!({"handle": hex(h), "raw": round3(raw), "centered": round3(centered)}));
    }
    let place = format!(
      "{}:{t}#m{}",
      root.repo(),
      events.iter().filter(|e| matches!(e, Event::Msg(_))).count() + 1
    );
    self.with_session(thread, |s| {
      s.note_place = (!shown.is_empty()).then(|| place.clone());
    });
    if shown.is_empty() {
      return None;
    }
    let measures = json!({
      "cue": {
        "distinctive": stitch_memory::distinctive_count(&cue.text),
        "chars": cue.text.chars().count(),
        "novelty": round3(cue.novelty),
        "ceiling": ceiling,
        "wandering": wandering,
        "centered_max": stitch_memory::AMBIENT_CENTERED_DISTANCE,
        "fresh": fresh,
      },
      "memories": measured,
    });
    let block = format!(
      "<recalled-memories>\n{}Bracket diagnostics: memory id, age, speaker, source conversation, \
       times seen, memory strength, whether it helped before. If none of these memories \
       actually help, append the invisible character \"{}\" (U+2060) to your reply: it silently \
       dismisses them, invisible to the user. Do not mention irrelevant memories aloud.\n\
       {lines}</recalled-memories>",
      if wandering { NOTE_WANDER } else { NOTE_HEADER },
      stitch_memory::DISMISS,
    );
    Some(Note {
      block,
      handles: shown.iter().map(|s| hex(s.handle)).collect(),
      shown,
      measures,
    })
  }

  /// Whether the thread may get a follow-up now: in memory, the user spoke
  /// since the last one, a minute apart.
  pub fn may_follow_up(&self, thread: &RecordId) -> bool {
    self.available()
      && !self.is_off(thread)
      && self.with_session(thread, |s| {
        s.armed
          && s
            .last_follow_up
            .is_none_or(|t| t.elapsed() >= FOLLOW_UP_FLOOR)
      })
  }

  /// A follow-up's note: what the exchange surfaced, verified against the
  /// thread's cue; it consumes the user's message until the next one.
  pub fn follow_up(&self, state: &NodeState, root: &RecordRoot, thread: &RecordId) -> Option<Note> {
    if !self.may_follow_up(thread) {
      return None;
    }
    let candidates = self.with_session(thread, |s| std::mem::take(&mut s.buffer));
    let note = self.drain(state, root, thread, candidates, false)?;
    self.with_session(thread, |s| {
      s.armed = false;
      s.last_follow_up = Some(Instant::now());
    });
    Some(note)
  }

  /// After an exchange in `thread` (its messages written from `since`):
  /// credits what helped the `answer`, knocks what it dismissed, observes
  /// the messages, parks what they surfaced; how many memories surfaced.
  /// Older messages never observed are caught up in the background.
  pub async fn after_turn(
    self: &Arc<Self>,
    state: &Arc<NodeState>,
    root: &RecordRoot,
    thread: &RecordId,
    since: Timestamp,
    shown: Vec<Shown>,
    answer: &str,
  ) -> crate::Result<usize> {
    if !self.available() || self.is_off(thread) {
      return Ok(0);
    }
    let note = self.with_session(thread, |s| s.note_place.take());
    let mut facts = self.feedback(state, thread, shown, answer);
    let events = Self::events(state, root, thread);
    let (observed, recent) = self
      .observations(state, root, thread, &events, false, |m| m.at >= since)
      .await?;
    facts.extend(observed);
    let applied = self.write_noted(state, root, facts, note.as_deref())?;
    let fresh = self.park(thread, &applied, &recent);
    if events
      .iter()
      .any(|e| matches!(e, Event::Msg(m) if m.at < since))
    {
      let (m, st, root, t) = (self.clone(), state.clone(), root.clone(), thread.clone());
      tokio::spawn(async move {
        let _ = m.heal(&st, &root, &t, since).await;
      });
    }
    Ok(fresh)
  }

  /// Observes a thread's messages from before `before` never observed, in
  /// batches; nothing it surfaces is parked (history is ingestion, not
  /// experience). Later messages are their own exchange's to observe.
  pub async fn heal(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    before: Timestamp,
  ) -> crate::Result<()> {
    if !self.available() || self.is_off(thread) {
      return Ok(());
    }
    loop {
      let events = Self::events(state, root, thread);
      let (facts, _) = self
        .observations(state, root, thread, &events, true, |m| m.at < before)
        .await?;
      let n = facts.len();
      self.write(state, root, facts)?;
      if n < HEAL_BATCH {
        return Ok(());
      }
    }
  }

  /// Merges what recent observations surfaced into the thread's buffer
  /// (living memories, newest first); how many they brought.
  fn park(&self, thread: &RecordId, applied: &[Applied], recent: &BTreeSet<u64>) -> usize {
    let mut merged: Vec<u64> = Vec::new();
    {
      let s = self.state.lock().expect("memory lock");
      for a in applied.iter().filter(|a| recent.contains(&a.handle)) {
        for h in &a.surfaced {
          if !merged.contains(h) && s.fold().resolve(*h).is_some() {
            merged.push(*h);
          }
        }
      }
    }
    let fresh = merged.len();
    self.with_session(thread, |s| {
      for h in std::mem::take(&mut s.buffer) {
        if !merged.contains(&h) {
          merged.push(h);
        }
      }
      merged.truncate(AMBIENT_BUFFER);
      s.buffer = merged;
    });
    fresh
  }

  /// Credit and dismissal for an exchange: what was shown (the note's and
  /// the `recall` tool's), what the answer used, what it dismissed.
  fn feedback(
    &self,
    state: &NodeState,
    thread: &RecordId,
    mut shown: Vec<Shown>,
    answer: &str,
  ) -> Vec<Fact> {
    let tool = self.with_session(thread, |s| std::mem::take(&mut s.tool_shown));
    for h in tool {
      if shown.iter().all(|s| s.handle != h)
        && let Some(r) = self.resolve(state, h)
      {
        shown.push(Shown {
          handle: h,
          text: r.paired(),
          own: r.text,
        });
      }
    }
    if shown.is_empty() {
      return Vec::new();
    }
    // Quoting a memory is not using it: lines citing one earn nothing.
    let lower = answer.to_lowercase();
    let for_credit = lower
      .lines()
      .filter(|l| !l.contains("[id "))
      .collect::<Vec<_>>()
      .join("\n");
    let used: Vec<u64> = shown
      .iter()
      .filter(|s| {
        stitch_memory::substantial(&s.own) && stitch_memory::credit_earned(&s.text, &for_credit)
      })
      .map(|s| s.handle)
      .collect();
    let mut dismissed: Vec<u64> = Vec::new();
    let mut dismiss = |h: u64| {
      if !used.contains(&h) && !dismissed.contains(&h) && shown.iter().any(|s| s.handle == h) {
        dismissed.push(h);
      }
    };
    if stitch_memory::dismiss_marked(answer) {
      for s in &shown {
        dismiss(s.handle);
      }
    }
    // Citing a shown memory without using it is a spoken dismissal.
    for line in lower.lines().filter(|l| l.contains("[id ")) {
      for h in cited(line) {
        dismiss(h);
      }
    }
    let mut facts = vec![Fact::Credit {
      shown: shown.iter().map(|s| s.handle).collect(),
      used,
    }];
    if !dismissed.is_empty() {
      facts.push(Fact::Penalize { handles: dismissed });
    }
    facts
  }

  /// Observations of a thread's messages that `pick` selects and memory
  /// has not seen (at most `HEAL_BATCH`), each Strengthen into a sibling of
  /// the same thread checked by words; and the handles of the recent ones.
  /// `heal`: catching up, whose images wait while memory is short
  /// (`HEAL_IMAGE_HEADROOM`).
  async fn observations(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    events: &[Event],
    heal: bool,
    pick: impl Fn(&Msg) -> bool,
  ) -> crate::Result<(Vec<Fact>, BTreeSet<u64>)> {
    let Some(enc) = self.encoder.clone() else {
      return Ok((Vec::new(), BTreeSet::new()));
    };
    enum Item {
      Text(String),
      Image(Vec<u8>),
      Embedded(Vec<f32>),
    }
    let recent_from = Timestamp::now() - jiff::SignedDuration::from_mins(RECENT_MINUTES);
    let mut todo: Vec<(String, Item, String)> = Vec::new();
    let mut recent = BTreeSet::new();
    let mut texts: HashMap<String, String> = HashMap::new();
    let cached = self.with_session(thread, |x| x.embedded.clone());
    // A room's messages relayed to this thread, and its replies to them:
    // the room observes them, once (design §2.27).
    let relayed = crate::rooms::relayed(events);
    {
      let s = self.state.lock().expect("memory lock");
      for e in events {
        let Event::Msg(m) = e else { continue };
        let base = format!("{}:{thread}#{}", root.repo(), m.id);
        let text = message_text(&m.parts);
        texts.insert(base.clone(), text.clone());
        if !pick(m) || todo.len() >= HEAL_BATCH || relayed.contains(&m.id) {
          continue;
        }
        // Not anyone's words: tools' output, memory's own notes.
        let said = m.role != Role::System
          && !matches!(m.from.as_deref(), Some("tool" | "memory"))
          && !text.trim().is_empty()
          && !stitch_memory::skip_marked(&text);
        if said && !s.fold().knows(stitch_memory::handle(&base)) {
          let item = match cached.get(&base) {
            Some(v) => Item::Embedded(v.clone()),
            None => Item::Text(text.clone()),
          };
          if m.at >= recent_from {
            recent.insert(stitch_memory::handle(&base));
          }
          todo.push((base.clone(), item, text.clone()));
        }
        for (i, p) in m.parts.iter().enumerate() {
          if let Part::Attachment { att } = p
            && enc.sees()
            && matches!(
              att.media_type.as_str(),
              "image/png" | "image/jpeg" | "image/gif"
            )
          {
            let place = format!("{base}:a{i}");
            if !s.fold().knows(stitch_memory::handle(&place))
              && let Ok(bytes) = std::fs::read(state.blobs.path(&att.hash))
            {
              if m.at >= recent_from {
                recent.insert(stitch_memory::handle(&place));
              }
              todo.push((place, Item::Image(bytes), String::new()));
            }
          }
        }
      }
    }
    if todo.is_empty() {
      return Ok((Vec::new(), recent));
    }
    let embedded = tokio::task::spawn_blocking(move || {
      todo
        .into_iter()
        .filter_map(|(place, item, text)| {
          let v = match item {
            Item::Text(t) => enc.document(&t).ok()?,
            Item::Image(_) if heal && !heal_may_embed_image(stitch_memory::available_memory()) => {
              return None;
            }
            Item::Image(b) => enc.image(&b).ok()?,
            Item::Embedded(v) => v,
          };
          Some((place, v, text))
        })
        .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| crate::Error::Config(e.to_string()))?;
    self.with_session(thread, |s| {
      for (p, _, _) in &embedded {
        s.embedded.remove(p);
      }
    });
    let items: Vec<(String, Vec<f32>, f32)> = embedded
      .iter()
      .map(|(p, v, t)| (p.clone(), v.clone(), stitch_memory::heuristic_demand(t)))
      .collect();
    let batch: HashMap<u64, String> = items
      .iter()
      .map(|(p, _, _)| (stitch_memory::handle(p), p.clone()))
      .collect();
    // Same-thread messages share words, so distinct replies land in the
    // dup band: a Strengthen into a sibling is only a real duplicate (a
    // mutable fact) when they share their distinctive words.
    let probed = {
      let s = self.state.lock().expect("memory lock");
      let mut i = 0;
      s.fold().probe(&items, |a| {
        let mine = &embedded[i].2;
        i += 1;
        let Some(target) = a.target.filter(|_| a.novelty > 0.05) else {
          return false;
        };
        let place = s
          .fold()
          .resolve(target)
          .map(str::to_owned)
          .or_else(|| batch.get(&target).cloned());
        place.is_some_and(|p| {
          stitch_memory::thread_of(&p) == Some(thread.as_str())
            && texts.get(&p).is_some_and(|t| {
              stitch_memory::overlap(t, &mine.to_lowercase()) < SAME_THREAD_DUP_OVERLAP
            })
        })
      })
    };
    let facts = embedded
      .into_iter()
      .zip(probed)
      .map(|((place, v, text), (_, own))| Fact::Observe {
        place,
        embedding: stitch_memory::pack(&v),
        demand: stitch_memory::heuristic_demand(&text),
        own,
        supersedes: stitch_memory::mutable(&text),
      })
      .collect();
    Ok((facts, recent))
  }

  /// Takes a thread out of memory (recalling nothing either, as
  /// `/memory off`), or brings it back; whether that changed anything.
  pub fn remember(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    on: bool,
  ) -> crate::Result<bool> {
    self.switch(state, root, thread, on, false)
  }

  /// Takes a thread out of memory but lets it recall (`recall`: a spawned
  /// thread's default), or brings it back (`on`); whether that changed
  /// anything.
  pub fn switch(
    &self,
    state: &NodeState,
    root: &RecordRoot,
    thread: &RecordId,
    on: bool,
    recall: bool,
  ) -> crate::Result<bool> {
    // Already so, and switched: a thread never switched is written even
    // when on, since its place's `memory = false` would take it out at
    // its next turn (design §2.9).
    let recall = recall && !on;
    if self.is_off(thread) != on
      && (on || self.may_recall(thread) == recall)
      && self.is_switched(thread)
    {
      return Ok(false);
    }
    self.write(
      state,
      root,
      vec![Fact::Remember {
        thread: thread.to_string(),
        on,
        recall,
      }],
    )?;
    if !on {
      self.with_session(thread, |s| *s = Session::default());
    }
    Ok(true)
  }

  /// Search for the user (design §2.9): every thread's messages by their
  /// words (BM25), and, where memory runs, those it holds by meaning
  /// (EmbeddingGemma 2); the two rankings fused by reciprocal rank. Threads
  /// out of memory are found by their words only. `keep` limits the
  /// threads (a place's).
  pub async fn search(
    &self,
    state: &NodeState,
    query: &str,
    keep: &(dyn Fn(&RecordId) -> bool + Sync),
    top: usize,
  ) -> Vec<SearchHit> {
    let words = self
      .archive
      .lock()
      .expect("archive lock")
      .scored(state, query, keep, SEARCH_DEPTH);
    let mut meaning: Vec<(RecordId, String)> = Vec::new();
    if let Some(cue) = self.embed(query, true).await {
      let s = self.state.lock().expect("memory lock");
      let near = s
        .fold()
        .cortex(&cue, SEARCH_MAX_DISTANCE, SEARCH_DEPTH, &|h| {
          s.fold().eligible(h, None)
        });
      for (h, _) in near {
        let Some(place) = s.fold().resolve(h) else {
          continue;
        };
        let (Some(thread), Some(msg)) = (
          stitch_memory::thread_of(place).and_then(|t| t.parse::<RecordId>().ok()),
          place
            .split_once('#')
            .map(|(_, m)| m.split(':').next().unwrap_or(m).to_owned()),
        ) else {
          continue;
        };
        if keep(&thread) && !meaning.iter().any(|(t, m)| *t == thread && *m == msg) {
          meaning.push((thread, msg));
        }
      }
    }
    // Reciprocal rank fusion (k = 60): (thread, message) to its score and
    // whether words and meaning found it.
    type Fused = HashMap<(RecordId, String), (f32, bool, bool)>;
    let mut fused: Fused = HashMap::new();
    for (rank, (_, d)) in words.iter().enumerate() {
      let e = fused
        .entry((d.thread.clone(), d.msg.clone()))
        .or_insert((0.0, false, false));
      e.0 += 1.0 / (60.0 + rank as f32 + 1.0);
      e.1 = true;
    }
    for (rank, key) in meaning.iter().enumerate() {
      let e = fused.entry(key.clone()).or_insert((0.0, false, false));
      e.0 += 1.0 / (60.0 + rank as f32 + 1.0);
      e.2 = true;
    }
    let mut ranked: Vec<_> = fused.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let mut archive = self.archive.lock().expect("archive lock");
    archive.refresh(state);
    let hits: Vec<(SearchHit, bool)> = ranked
      .into_iter()
      .filter_map(|((thread, msg), (_, by_words, by_meaning))| {
        let d = archive.doc(&thread, &msg)?;
        let hit = SearchHit {
          thread: thread.to_string(),
          title: d.title.clone(),
          msg,
          at: d.at.to_string(),
          sender: d.sender.clone(),
          snippet: snippet(&d.text, query),
          found_by: match (by_words, by_meaning) {
            (true, true) => "both",
            (true, false) => "words",
            _ => "meaning",
          }
          .into(),
        };
        Some((hit, d.agent))
      })
      .collect();
    // The user searches: their own conversations first, then what
    // threads among themselves said.
    user_first(hits, |(_, agent)| *agent)
      .into_iter()
      .map(|(h, _)| h)
      .take(top)
      .collect()
  }

  /// The `recall` tool.
  pub fn definition() -> ToolDef {
    ToolDef {
      name: "recall".into(),
      description: "Search memory: what was said and seen in the user's earlier conversations, \
        on every machine (messages, and images by what they show), then a keyword search over \
        every conversation's history. Ask with a phrase about what you are looking for. \
        Returns the closest memories with when, where and who; nothing when nothing is close. \
        The current conversation is not searched."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "query": {"type": "string", "description": "What to remember, as a phrase or question."},
          "scope": {"type": "array", "items": {"type": "string"}, "description": "Search only these threads or efforts: `effort:<id>` (a conversation and every thread spawned under it), `thread:<id>` or a thread's ID. Adds an exact keyword search of their messages and artefacts, which also covers threads memory does not ([memory: off])."}
        },
        "required": ["query"]
      }),
    }
  }

  pub async fn recall_tool(
    &self,
    state: &NodeState,
    thread: &RecordId,
    input: &Value,
    scope: Option<&crate::scopes::ScopeSearch>,
  ) -> ToolOutcome {
    let Some(query) = input
      .get("query")
      .and_then(Value::as_str)
      .filter(|q| !q.trim().is_empty())
    else {
      return ToolOutcome::err("query must be text");
    };
    let mut out = String::new();
    // Kept to a scope's threads (`scope`), said first.
    let in_scope = |t: &str| scope.is_none_or(|sc| sc.threads.contains(t));
    if let Some(sc) = scope {
      out.push_str(&sc.header);
    }
    // What agent-to-agent threads hold comes after the user's own when
    // the user is the one asking (their chat); other threads see one
    // ranking.
    let (agents, for_user) = {
      let mut a = self.archive.lock().expect("archive lock");
      a.refresh(state);
      let for_user = a.agents_only(thread) == Some(false)
        || !crate::chat::unwritten_user_texts(thread).is_empty();
      (a.agent_threads(), for_user)
    };
    if !self.may_recall(thread) {
      out.push_str(
        "(Episodic memory is disabled for this conversation — memory toggle off. The archive \
         is still searchable.)\n",
      );
    } else if let Some(cue) = self.embed(query, true).await {
      let hits = {
        let s = self.state.lock().expect("memory lock");
        let t = thread.as_str();
        let hits = s.fold().recall(&cue, &|h| {
          s.fold().eligible(h, Some(t))
            && (scope.is_none()
              || s
                .fold()
                .resolve(h)
                .and_then(stitch_memory::thread_of)
                .is_some_and(in_scope))
        });
        if for_user {
          user_first(hits, |h| {
            s.fold()
              .resolve(h.handle)
              .and_then(stitch_memory::thread_of)
              .is_some_and(|t| agents.contains(t))
          })
        } else {
          hits
        }
      };
      let mut lines = String::new();
      let mut shown = Vec::new();
      for h in hits {
        if let Some(r) = self.resolve(state, h.handle) {
          lines.push_str(&r.line(&r.paired()));
          shown.push(h.handle);
        }
      }
      if !lines.is_empty() {
        out.push_str(
          "Episodic memory (past messages; bracket diagnostics: age, speaker, source \
           conversation, times seen, memory strength, whether it helped before):\n",
        );
        out.push_str(&lines);
      }
      self.with_session(thread, |s| {
        for h in shown {
          if !s.tool_shown.contains(&h) {
            s.tool_shown.push(h);
          }
        }
      });
    }
    // Threads out of memory stay out of the keyword search too.
    let off: BTreeSet<String> = {
      let s = self.state.lock().expect("memory lock");
      state
        .index
        .lock()
        .expect("index lock")
        .threads()
        .unwrap_or_default()
        .into_iter()
        .map(|(r, _)| r.id.to_string())
        .filter(|t| s.fold().is_off(t))
        .collect()
    };
    let archive = self
      .archive
      .lock()
      .expect("archive lock")
      .search(state, query, &|t| {
        t != thread && !off.contains(t.as_str()) && in_scope(t.as_str())
      });
    let archive = if for_user {
      user_first(archive, |d| d.agent)
    } else {
      archive
    };
    if !archive.is_empty() {
      out.push_str("\nArchive (full history keyword search):\n");
      for a in archive {
        out.push_str(&format!(
          "- [archive, \"{}\", {}, {}]: {}\n",
          a.title,
          a.at.strftime("%Y-%m-%d"),
          a.sender,
          cap_words(&a.text, ARCHIVE_WORDS, " (…)")
        ));
      }
    }
    if let Some(sc) = scope {
      out.push_str(&sc.exact);
      if sc.exact.is_empty() && !out.contains("Episodic memory (") && !out.contains("\nArchive") {
        out.push_str("Nothing in the scope matches: try other words, or read_thread.\n");
      }
    }
    ToolOutcome {
      ok: true,
      output: if out.is_empty() {
        "Nothing related in episodic memory or the archive (episodic memory only spans \
         messages since it was installed; the gate also suppresses weak matches by design)."
          .into()
      } else {
        out
      },
      images: Vec::new(),
      spent: None,
      attachments: Vec::new(),
      blobs: Vec::new(),
      generated: Vec::new(),
    }
  }

  /// After a sleep: condenses the quiet cluster that is due, if any, into
  /// a gist written by `[memory] gist_model` (else the quick model),
  /// resolved by the host; dropping chaff; whether one was written.
  pub async fn gist(
    &self,
    state: &NodeState,
    config: &Config,
    data: &Path,
    model: &str,
  ) -> crate::Result<bool> {
    let members = self
      .state
      .lock()
      .expect("memory lock")
      .fold()
      .gist_due(Timestamp::now().as_second());
    let mut order = Vec::new();
    let mut fragments = String::new();
    for h in members {
      if let Some(r) = self.resolve(state, h) {
        order.push(h);
        fragments.push_str(&format!(
          "{}. [{}, {}] {}\n",
          order.len(),
          r.at.strftime("%Y-%m-%d"),
          r.sender,
          r.text
            .split(' ')
            .take(GIST_FRAGMENT_WORDS)
            .collect::<Vec<_>>()
            .join(" ")
        ));
      }
    }
    if order.len() < stitch_memory::GIST_MIN_MEMBERS {
      return Ok(false);
    }
    let (provider, _, model) = config.resolve_authenticated(data, Some(model)).await?;
    let prompt = format!(
      "These related memory fragments were captured across past conversations:\n\n\
       {fragments}{GIST_INSTRUCTION}"
    );
    let mut request = Request::new(model, vec![Message::user(vec![Content::Text(prompt)])]);
    request.system = Some(GIST_SYSTEM.into());
    request.max_tokens = GIST_MAX_TOKENS;
    let response = provider.stream(&request, &mut |_| {}).await?;
    self.commit_gist(state, &order, &response.text()).await
  }

  /// Writes a gist from the writer's answer over the numbered `members`.
  async fn commit_gist(
    &self,
    state: &NodeState,
    members: &[u64],
    answer: &str,
  ) -> crate::Result<bool> {
    let (text, drop) = parse_gist(answer);
    if text.is_empty() {
      return Ok(false);
    }
    let Some(raw) = self.embed(&text, false).await else {
      return Ok(false);
    };
    // Bounds-checked: a hallucinated number drops nothing.
    let dropped: Vec<u64> = drop
      .into_iter()
      .filter_map(|i| i.checked_sub(1).and_then(|i| members.get(i)).copied())
      .collect();
    let kept: Vec<u64> = members
      .iter()
      .copied()
      .filter(|h| !dropped.contains(h))
      .collect();
    let Some(home) = state.home() else {
      return Ok(false);
    };
    self.write(
      state,
      &home,
      vec![Fact::Gist {
        text,
        embedding: stitch_memory::pack(&raw),
        kept,
        dropped,
      }],
    )?;
    Ok(true)
  }

  /// The drift check and the state of the graph, as lines.
  pub fn audit(&self) -> Vec<String> {
    let s = self.state.lock().expect("memory lock");
    let now = Timestamp::now().as_second();
    let (strengths, forgotten) = s.fold().strengths();
    let mut out = vec![format!(
      "{} facts, {} episodes, {} forgotten",
      s.fold().len(),
      s.fold().episodes(),
      forgotten
    )];
    for (name, since) in [("last 2 days", now - 2 * 86_400), ("all time", 0)] {
      out.push(format!(
        "decisions by novelty, {name}:   ≤{:.3}  ≤{:.3}  <{:.3}  ≥{:.3}",
        stitch_memory::DUP_JACCARD,
        stitch_memory::ESTABLISHED_NOVELTY,
        stitch_memory::NOVEL_NOVELTY,
        stitch_memory::NOVEL_NOVELTY
      ));
      for (kind, b) in s.fold().bands(since) {
        out.push(format!(
          "  {:10} {:>6} {:>6} {:>6} {:>6}",
          kind.as_str(),
          b[0],
          b[1],
          b[2],
          b[3]
        ));
      }
    }
    let mut buckets = [0usize; 5];
    for x in &strengths {
      buckets[match *x {
        x if x < 0.5 => 0,
        x if x < 1.0 => 1,
        x if x < 2.0 => 2,
        x if x < 4.0 => 3,
        _ => 4,
      }] += 1;
    }
    out.push(format!(
      "strengths: <0.5 {}, <1 {}, <2 {}, <4 {}, ≥4 {}",
      buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
    ));
    out
  }
}

/// A gist writer's answer: the paragraph, and the fragments it dropped
/// (1-based; a malformed line drops nothing).
fn parse_gist(answer: &str) -> (String, Vec<usize>) {
  let mut text = String::new();
  let mut drop = Vec::new();
  for line in answer.lines() {
    if let Some(rest) = line.trim().strip_prefix("DROP:") {
      drop.extend(rest.split(',').filter_map(|n| {
        n.trim()
          .trim_matches(|c: char| !c.is_ascii_digit())
          .parse::<usize>()
          .ok()
      }));
    } else {
      text.push_str(line);
      text.push('\n');
    }
  }
  (text.trim().to_owned(), drop)
}

fn hexes(v: &[u64]) -> toml::Value {
  toml::Value::Array(v.iter().map(|h| hex(*h).into()).collect())
}

/// A fact as a record writes it.
fn to_toml(f: &Fact) -> toml::Value {
  let mut t = toml::Table::new();
  match f {
    Fact::Observe {
      place,
      embedding,
      demand,
      own,
      supersedes,
    } => {
      t.insert("at".into(), place.as_str().into());
      t.insert(
        "embedding".into(),
        stitch_memory::to_base64(embedding).into(),
      );
      t.insert(
        "demand".into(),
        f64::from((demand * 1000.0).round() / 1000.0).into(),
      );
      if *own {
        t.insert("own".into(), true.into());
      }
      if *supersedes {
        t.insert("supersedes".into(), true.into());
      }
    }
    Fact::Credit { shown, used } => {
      t.insert("shown".into(), hexes(shown));
      t.insert("credit".into(), hexes(used));
    }
    Fact::Penalize { handles } => {
      t.insert("penalize".into(), hexes(handles));
    }
    Fact::Sleep => {
      t.insert("sleep".into(), true.into());
    }
    Fact::Gist {
      text,
      embedding,
      kept,
      dropped,
    } => {
      t.insert("gist".into(), text.as_str().into());
      t.insert(
        "embedding".into(),
        stitch_memory::to_base64(embedding).into(),
      );
      t.insert("kept".into(), hexes(kept));
      t.insert("dropped".into(), hexes(dropped));
    }
    Fact::Remember { thread, on, recall } => {
      t.insert("thread".into(), thread.as_str().into());
      t.insert("remember".into(), (*on).into());
      // Only out of memory, and only when kept: a build from before reads
      // the fact without it, as out of memory altogether.
      if *recall && !*on {
        t.insert("recall".into(), true.into());
      }
    }
  }
  toml::Value::Table(t)
}

fn handles_of(v: Option<&Value>) -> Vec<u64> {
  v.and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|h| unhex(h.as_str()?))
    .collect()
}

/// A measure as kept: three decimals.
fn round3(x: f32) -> f64 {
  (f64::from(x) * 1000.0).round() / 1000.0
}

/// A fact from a record's JSON fields.
fn parse(f: &Value) -> Option<Fact> {
  let flag = |k: &str| f.get(k).and_then(Value::as_bool) == Some(true);
  if flag("sleep") {
    return Some(Fact::Sleep);
  }
  if let Some(text) = f.get("gist").and_then(Value::as_str) {
    return Some(Fact::Gist {
      text: text.to_owned(),
      embedding: stitch_memory::from_base64(f.get("embedding").and_then(Value::as_str)?)?,
      kept: handles_of(f.get("kept")),
      dropped: handles_of(f.get("dropped")),
    });
  }
  if let Some(thread) = f.get("thread").and_then(Value::as_str) {
    let on = f.get("remember").and_then(Value::as_bool).unwrap_or(true);
    return Some(Fact::Remember {
      thread: thread.to_owned(),
      on,
      recall: !on && flag("recall"),
    });
  }
  if f.get("penalize").is_some() {
    return Some(Fact::Penalize {
      handles: handles_of(f.get("penalize")),
    });
  }
  if f.get("credit").is_some() {
    let used = handles_of(f.get("credit"));
    let shown = match f.get("shown") {
      Some(s) => handles_of(Some(s)),
      None => used.clone(),
    };
    return Some(Fact::Credit { shown, used });
  }
  let place = f.get("at").and_then(Value::as_str)?;
  Some(Fact::Observe {
    place: place.to_owned(),
    embedding: stitch_memory::from_base64(f.get("embedding").and_then(Value::as_str)?)?,
    demand: f.get("demand").and_then(Value::as_f64).unwrap_or(0.0) as f32,
    own: flag("own"),
    supersedes: flag("supersedes"),
  })
}

/// A keyword search (BM25) over every thread's messages, kept per thread
/// and refreshed when a thread moves on: what episodic memory does not
/// span (threads from before it, what it let go).
#[derive(Default)]
struct Archive {
  /// Each thread as indexed: its last change, whether it is the agents'
  /// alone (`agents_only`), its messages.
  threads: HashMap<RecordId, (Timestamp, bool, Vec<Doc>)>,
}

/// Whether a thread is agent-to-agent: no message in it is the user's own
/// (a thread spawned or scheduled that the user never wrote into). Its
/// remembered messages rank after the user's own conversations when the
/// user searches (design §2.9); a message of the user's in it later makes
/// it theirs at its next indexing.
fn agents_only(events: &[Event]) -> bool {
  !events
    .iter()
    .any(|e| matches!(e, Event::Msg(m) if crate::chat::by_user(m)))
}

/// `ranked` with what `agent` says is agent-to-agent after the rest, each
/// part in its own order.
fn user_first<T>(ranked: Vec<T>, agent: impl Fn(&T) -> bool) -> Vec<T> {
  let (mut user, agents): (Vec<T>, Vec<T>) = ranked.into_iter().partition(|t| !agent(t));
  user.extend(agents);
  user
}

#[derive(Clone)]
struct Doc {
  thread: RecordId,
  /// The message (`m12`).
  msg: String,
  title: String,
  at: Timestamp,
  sender: String,
  text: String,
  terms: HashMap<String, u32>,
  len: usize,
  /// Its thread is agent-to-agent (`agents_only`).
  agent: bool,
}

impl Archive {
  fn refresh(&mut self, state: &NodeState) {
    let threads = state
      .index
      .lock()
      .expect("index lock")
      .threads()
      .unwrap_or_default();
    let live: BTreeSet<RecordId> = threads.iter().map(|(r, _)| r.id.clone()).collect();
    self.threads.retain(|t, _| live.contains(t));
    for (r, last) in threads {
      if self
        .threads
        .get(&r.id)
        .is_some_and(|(at, _, _)| *at == last)
      {
        continue;
      }
      let Some(root) = state.root(&r.repo) else {
        continue;
      };
      let title = r.title.clone().unwrap_or_default();
      let events = Memory::events(state, &root, &r.id);
      let agent = agents_only(&events);
      let docs = events
        .iter()
        .filter_map(|e| match e {
          Event::Msg(m) if !matches!(m.from.as_deref(), Some("tool" | "memory")) => Some(m),
          _ => None,
        })
        .filter_map(|m| {
          let text = stitch_memory::without_markers(&message_text(&m.parts));
          if text.trim().is_empty() {
            return None;
          }
          let words = stitch_memory::terms(&text);
          let mut terms: HashMap<String, u32> = HashMap::new();
          for w in &words {
            *terms.entry(w.clone()).or_default() += 1;
          }
          Some(Doc {
            thread: r.id.clone(),
            msg: m.id.clone(),
            title: title.clone(),
            at: m.at,
            sender: match m.role {
              Role::User => "user".into(),
              _ => m.model.clone().unwrap_or_else(|| "assistant".into()),
            },
            text,
            len: words.len(),
            terms,
            agent,
          })
        })
        .collect();
      self.threads.insert(r.id.clone(), (last, agent, docs));
    }
  }

  /// Whether a thread as indexed is agent-to-agent; `None` before its
  /// first message is written.
  fn agents_only(&self, thread: &RecordId) -> Option<bool> {
    self.threads.get(thread).map(|(_, a, _)| *a)
  }

  /// The agent-to-agent threads, as indexed.
  fn agent_threads(&self) -> BTreeSet<String> {
    self
      .threads
      .iter()
      .filter(|(_, (_, a, _))| *a)
      .map(|(t, _)| t.to_string())
      .collect()
  }

  /// The best matches for `query` in the threads `keep` allows.
  fn search(
    &mut self,
    state: &NodeState,
    query: &str,
    keep: &dyn Fn(&RecordId) -> bool,
  ) -> Vec<Doc> {
    self
      .scored(state, query, keep, ARCHIVE_TOP)
      .into_iter()
      .map(|(_, d)| d)
      .collect()
  }

  /// A message of a thread, as indexed.
  fn doc(&self, thread: &RecordId, msg: &str) -> Option<&Doc> {
    self
      .threads
      .get(thread)
      .and_then(|(_, _, docs)| docs.iter().find(|d| d.msg == msg))
  }

  /// The `top` best matches for `query` in the threads `keep` allows,
  /// with their BM25 scores.
  fn scored(
    &mut self,
    state: &NodeState,
    query: &str,
    keep: &dyn Fn(&RecordId) -> bool,
    top: usize,
  ) -> Vec<(f32, Doc)> {
    let q: BTreeSet<String> = stitch_memory::terms(query).into_iter().collect();
    if q.is_empty() {
      return Vec::new();
    }
    self.refresh(state);
    let docs: Vec<&Doc> = self
      .threads
      .values()
      .flat_map(|(_, _, d)| d)
      .filter(|d| keep(&d.thread))
      .collect();
    if docs.is_empty() {
      return Vec::new();
    }
    let n = docs.len() as f32;
    let avg = docs.iter().map(|d| d.len).sum::<usize>() as f32 / n;
    let (k1, b) = (1.2f32, 0.75f32);
    let df: HashMap<&String, f32> = q
      .iter()
      .map(|t| {
        (
          t,
          docs.iter().filter(|d| d.terms.contains_key(t)).count() as f32,
        )
      })
      .collect();
    let mut scored: Vec<(f32, &Doc)> = docs
      .iter()
      .map(|d| {
        let score: f32 = q
          .iter()
          .map(|t| {
            let f = d.terms.get(t).copied().unwrap_or(0) as f32;
            if f == 0.0 {
              return 0.0;
            }
            let idf = ((n - df[t] + 0.5) / (df[t] + 0.5) + 1.0).ln();
            idf * f * (k1 + 1.0) / (f + k1 * (1.0 - b + b * d.len as f32 / avg.max(1.0)))
          })
          .sum();
        (score, *d)
      })
      .filter(|(s, _)| *s > 0.0)
      .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.at.cmp(&a.1.at)));
    scored
      .into_iter()
      .take(top)
      .map(|(s, d)| (s, d.clone()))
      .collect()
  }
}

/// A message found by `Memory::search`.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
  pub thread: String,
  pub title: String,
  /// The message (`m12`): `stitch fork <thread> --upto` takes it.
  pub msg: String,
  pub at: String,
  /// `user`, or the model that wrote it.
  pub sender: String,
  /// Around what matched.
  pub snippet: String,
  /// What found it: `words`, `meaning`, or `both`.
  pub found_by: String,
}

/// How far (cosine distance, raw) a message may be from the query to be
/// found by meaning: a little looser than asked recall's relevance floor
/// (0.340), since a list is skimmed, not put into a prompt.
const SEARCH_MAX_DISTANCE: f32 = 0.36;
/// How deep each ranking goes before they are fused.
const SEARCH_DEPTH: usize = 50;

/// Words around the first term of `query` found in `text`.
fn snippet(text: &str, query: &str) -> String {
  let terms: BTreeSet<String> = stitch_memory::terms(query).into_iter().collect();
  let words: Vec<&str> = text.split_whitespace().collect();
  let hit = words.iter().position(|w| {
    let w: String = w
      .chars()
      .filter(|c| c.is_alphanumeric())
      .collect::<String>()
      .to_lowercase();
    terms.iter().any(|t| w.starts_with(t.as_str()))
  });
  let start = hit.map_or(0, |i| i.saturating_sub(10));
  let end = (start + 30).min(words.len());
  let mut out = words[start..end].join(" ");
  if start > 0 {
    out = format!("…{out}");
  }
  if end < words.len() {
    out.push('…');
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Catching up embeds an image only with room for it where the OS caps
  /// the app's memory (an iPhone crashed embedding one), always elsewhere.
  #[test]
  fn catching_up_embeds_images_only_with_room() {
    assert!(heal_may_embed_image(None));
    assert!(heal_may_embed_image(Some(3 << 30)));
    assert!(heal_may_embed_image(Some(HEAL_IMAGE_HEADROOM)));
    assert!(!heal_may_embed_image(Some(HEAL_IMAGE_HEADROOM - 1)));
    assert!(!heal_may_embed_image(Some(200 << 20)));
    if !cfg!(target_os = "ios") {
      assert_eq!(stitch_memory::available_memory(), None);
    }
  }

  /// A deleted thread is forgotten: the records only of it go with it,
  /// and what was observed of its messages elsewhere leaves the graph.
  #[test]
  fn a_deleted_thread_is_forgotten() {
    let (t1, t2) = ("2026-10-22T100000Z-mac-aaaa", "2026-10-22T100000Z-mac-bbbb");
    let place = |t: &str, m: &str| format!("home:{t}#{m}");
    let observe = |p: String, e: usize| Fact::Observe {
      place: p,
      embedding: (0..stitch_memory::center().dim)
        .map(|i| if i == e { 100 } else { 0 })
        .collect(),
      demand: 0.0,
      own: true,
      supersedes: false,
    };
    let row = |f: &Fact| serde_json::to_value(to_toml(f)).unwrap();
    let doomed: BTreeSet<String> = [t1.to_owned()].into();
    // An exchange's record: its observations and its note's feedback.
    let mut credit = row(&Fact::Credit {
      shown: vec![1],
      used: vec![],
    });
    credit["note"] = place(t1, "m2").into();
    let only = json!({"facts": [row(&observe(place(t1, "m1"), 0)), credit]});
    assert!(only_of(&only, &doomed));
    let mixed =
      json!({"facts": [row(&observe(place(t1, "m1"), 0)), row(&observe(place(t2, "m1"), 1))]});
    assert!(!only_of(&mixed, &doomed));
    assert!(!only_of(&json!({"facts": [row(&Fact::Sleep)]}), &doomed));

    let dir = tempfile::tempdir().unwrap();
    let memory = Memory::new(dir.path(), &MemorySettings::default());
    {
      let mut s = memory.state.lock().unwrap();
      let facts = vec![
        (At::new("r1", 0, 1), observe(place(t1, "m1"), 0)),
        (At::new("r2", 0, 2), observe(place(t2, "m1"), 1)),
        (At::new("r2", 1, 2), observe(place(t1, "m3"), 2)),
      ];
      s.folding.add(facts);
    }
    let knows = |p: String| {
      memory
        .state
        .lock()
        .unwrap()
        .fold()
        .knows(stitch_memory::handle(&p))
    };
    assert!(knows(place(t1, "m1")) && knows(place(t1, "m3")) && knows(place(t2, "m1")));
    memory.forget(&[t1.parse().unwrap()].into(), &[]);
    assert!(!knows(place(t1, "m1")) && !knows(place(t1, "m3")));
    assert!(knows(place(t2, "m1")), "another thread's memory stays");
  }

  /// A thread is agent-to-agent while no message in it is the user's own:
  /// its spawner's goal, its own replies and reports do not count.
  #[test]
  fn threads_without_the_users_messages_are_agent_to_agent() {
    let msg = |role: Role, from: Option<&str>| {
      Event::Msg(Msg {
        id: "m1".into(),
        at: Timestamp::now(),
        role,
        from: from.map(str::to_owned),
        model: None,
        parts: vec![Part::Text { text: "hi".into() }],
        native: None,
      })
    };
    let spawned = vec![
      msg(Role::User, Some("thread 2026-10-29T121225Z-gamer-5u74")),
      msg(Role::Assistant, None),
      msg(Role::User, Some("schedule s")),
    ];
    assert!(agents_only(&spawned));
    let mut stepped_in = spawned.clone();
    stepped_in.push(msg(Role::User, None));
    assert!(!agents_only(&stepped_in), "the user wrote into it");
    assert!(!agents_only(&[msg(Role::User, Some("user@mbp"))]));
  }

  /// The user's own come first, then agent-to-agent ones, each in the
  /// order it was ranked in.
  #[test]
  fn agent_to_agent_results_rank_after_the_users() {
    let ranked = vec![("a1", true), ("u1", false), ("a2", true), ("u2", false)];
    let names = |v: Vec<(&'static str, bool)>| v.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(
      names(user_first(ranked.clone(), |(_, agent)| *agent)),
      ["u1", "u2", "a1", "a2"]
    );
    assert_eq!(
      names(user_first(ranked, |_| false)),
      ["a1", "u1", "a2", "u2"],
      "nothing agent-to-agent: as ranked"
    );
  }

  #[test]
  fn gist_answers_parse() {
    let (t, d) = parse_gist("The rig mists every 15 minutes.\nDROP: 2, 5\n");
    assert_eq!(t, "The rig mists every 15 minutes.");
    assert_eq!(d, vec![2, 5]);
    let (t, d) = parse_gist("  Just a paragraph.  ");
    assert_eq!(t, "Just a paragraph.");
    assert!(d.is_empty());
    let (_, d) = parse_gist("x\nDROP: none");
    assert!(d.is_empty());
  }

  #[test]
  fn citations_and_words() {
    let h = 0x1234_5678_9abc_def0u64;
    let line = format!("I recall [id {}, 3d ago] but it is not relevant", hex(h));
    assert_eq!(cited(&line), vec![h]);
    assert!(cited("[id nothex]").is_empty());
    assert_eq!(cap_words("a b c d", 2, "…"), "a b…");
    assert_eq!(cap_words("a b", 2, "…"), "a b");
  }

  #[test]
  fn facts_round_trip_through_records() {
    let facts = vec![
      Fact::Observe {
        place: "home:t#m1".into(),
        embedding: vec![1, -2, 3],
        demand: 0.25,
        own: true,
        supersedes: true,
      },
      Fact::Credit {
        shown: vec![1, 2],
        used: vec![2],
      },
      Fact::Penalize { handles: vec![1] },
      Fact::Sleep,
      Fact::Gist {
        text: "g".into(),
        embedding: vec![4, 5, 6],
        kept: vec![7],
        dropped: vec![8],
      },
      Fact::Remember {
        thread: "t".into(),
        on: false,
        recall: false,
      },
      Fact::Remember {
        thread: "t".into(),
        on: false,
        recall: true,
      },
    ];
    for f in facts {
      let json = serde_json::to_value(to_toml(&f)).unwrap();
      assert_eq!(parse(&json), Some(f));
    }
  }
}
