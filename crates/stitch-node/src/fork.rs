//! Forks of a thread (design §2.9): a new thread that continues another
//! from one of its messages, read from it, never copied; with or without
//! its attachments and tool calls, all of it or its last messages, and
//! summarized: "summarize and continue" (a summary stands for all of it)
//! and "summarize and truncate" (a summary, then its last messages
//! verbatim), as edge talk had them. A thread that grows past its model's
//! context window continues the same way, in a new thread (`compact`).

use std::sync::Arc;

use stitch_providers::{Content, Message, Request};
use stitch_records::{Author, Draft, Event, RecordId, kinds};

use crate::api::Compaction;
use crate::chat::{ChatContext, Continues, Shape, Upto};
use crate::live::LiveEvent;
use crate::node::Shared;
use crate::{Error, Result};

/// What a fork takes.
#[derive(Clone, Debug)]
pub struct ForkSpec {
  pub thread: RecordId,
  /// Up to this message (`m12`); the thread's latest by default.
  pub upto: Option<String>,
  /// Only its last messages.
  pub last: Option<usize>,
  pub no_attachments: bool,
  pub no_tools: bool,
  /// A summary stands for what is not kept (all of it, without `last`).
  pub summarize: bool,
}

const NOTHING_TO_SUMMARIZE: &str = "nothing to summarize";

/// How each kind of fork is titled (edge talk's prefixes).
fn prefix(spec: &ForkSpec) -> &'static str {
  match (spec.summarize, spec.last) {
    (true, Some(_)) => "✂",
    (true, None) => "↻",
    (false, _) => "⑂",
  }
}

/// The instruction edge talk summarized with, for agent work as well.
const SUMMARY_SYSTEM: &str =
  "You are a helpful assistant that generates a summary of a conversation.";

const SUMMARY_PROMPT: &str = "Generate a comprehensive summary of the conversation above, \
for continuing it in a new conversation that sees only this summary (and, when kept, its last \
messages). Structure as follows:
**[TOPICS:]** $comma_separated_keywords
*(Identify 3-5 concrete nouns/phrases representing core subjects)*

**Executive Context:**
In 2-3 sentences, state the conversation's central focus using specific terminology from the discussion. Begin with \"This dialogue examined/explored...\" followed by:
- Primary subjects of analysis
- Critical relationships between concepts
- Concrete examples discussed

---

**Structured Breakdown:**

1. **Foundational Elements**
   - Essential definitions/parameters
   - Frameworks/models referenced
   - Baseline assumptions established

2. **Concrete Particulars**
   - Specific examples/case studies
   - Structural relationships mapped
   - Comparative analyses performed

3. **Resolution Matrix**
   - Definitive conclusions reached
   - Chosen frameworks/methods
   - Identified next-phase requirements

---

**Memorable Anchors:**
- Verbatim critical statements
- Signature analogies/metaphors
- Decision rationale quotes
- Tools used, if any

**Referenced files: (optional)**
- List of files referenced in the conversation if any
- List of relevant folders if any

**Work state: (when work was done)**
- What was changed (files, commits, branches) and what was run, with results
- Threads spawned and what they reported
- What is in progress, what is next, and what the user asked for that is not done

---

**Continuity Cheat Sheet:**
Any relevant details that will be required to continue the conversation.

**Outcome Synthesis:**
2-3 sentences distilling operational conclusions using domain-specific terminology from the conversation. No commentary.";

/// Starts a fork: writes its thread record (no message yet; the first
/// one the user sends continues it) and returns its ID. Summarizing asks
/// a model (`[threads] summary_model`, else quick, with a thread-model
/// fallback for context/completeness when quick was chosen automatically).
pub(crate) async fn fork(s: &Arc<Shared>, spec: ForkSpec) -> Result<RecordId> {
  let from = spec.thread.clone();
  let id = fork_with(s, spec, Ok, &mut SummarySpent::default()).await?;
  // A fork of an imported thread has tools (design §2.31).
  crate::share::forked(s, &from, &id)?;
  Ok(id)
}

/// What a fork of `spec` continues, and the events it takes of it.
fn continues_of(ctx: &ChatContext<'_>, spec: &ForkSpec) -> Result<(Continues, Vec<Event>)> {
  let (head, open) = crate::chat::line_end(ctx, &spec.thread)?;
  let head = head.ok_or_else(|| {
    Error::Config(format!(
      "thread {} has nothing to fork yet: no message is written",
      spec.thread
    ))
  })?;
  let shape = Shape {
    // Its line ends at a rewind: up to the message it goes on from, not
    // what its head holds after it.
    upto: spec.upto.clone().or_else(|| open.and_then(|r| r.upto)),
    no_attachments: spec.no_attachments,
    no_tools: spec.no_tools,
    last: spec.last,
    summary: None,
  };
  let continues = Continues {
    thread: spec.thread.clone(),
    at: Upto::Segment(head),
    shape,
  };
  // What it would take, read now: a message not there fails here.
  let taken = crate::chat::inherited_from(ctx, &continues)?;
  Ok((continues, taken))
}

/// What a summary of a fork stands for: everything before the kept
/// messages (`taken`).
fn summarized(
  ctx: &ChatContext<'_>,
  continues: &Continues,
  spec: &ForkSpec,
  taken: &[Event],
) -> Result<Vec<Event>> {
  let mut whole = continues.clone();
  whole.shape.last = None;
  let all = crate::chat::inherited_from(ctx, &whole)?;
  let before: Vec<Event> = match spec.last {
    Some(_) => {
      let kept = taken.len();
      all[..all.len().saturating_sub(kept)].to_vec()
    }
    None => all,
  };
  if before.is_empty() {
    return Err(Error::Config(format!(
      "{NOTHING_TO_SUMMARIZE}: the messages kept are all of it"
    )));
  }
  Ok(before)
}

/// The model a summary of `thread` is asked of (`[threads] summary_model`,
/// else quick), and the thread's model it may give way to (only for the
/// automatic quick choice: an explicitly configured summary model must
/// either work or say why).
async fn summary_models(s: &Arc<Shared>, thread: &RecordId) -> Result<(String, Option<String>)> {
  let place = s.thread_place(thread)?.1;
  let configured = s
    .config
    .threads
    .as_ref()
    .and_then(|t| t.summary_model.as_deref());
  let model = crate::modelset::named(s, Some(&place), Some(configured.unwrap_or("quick"))).await;
  let fallback = if configured.is_none() {
    Some(crate::modelset::chat_model(s, Some(thread), &place, None).await)
  } else {
    None
  };
  Ok((model, fallback))
}

/// `fork`, with `more` added to the new thread's record; what its
/// summary's model calls spent is added to `spent`, whether it is made or
/// fails after them.
async fn fork_with(
  s: &Arc<Shared>,
  spec: ForkSpec,
  more: impl FnOnce(Draft) -> Result<Draft>,
  spent: &mut SummarySpent,
) -> Result<RecordId> {
  let (root, _) = s.thread_place(&spec.thread)?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let original = root.read_record(&spec.thread)?;
  let (mut continues, taken) = continues_of(&ctx, &spec)?;
  if taken.is_empty() && !spec.summarize {
    return Err(Error::Config("nothing to fork: no message is kept".into()));
  }
  if spec.summarize {
    let before = summarized(&ctx, &continues, &spec, &taken)?;
    let (model, fallback) = summary_models(s, &spec.thread).await?;
    let summary = summarize(
      s,
      &ctx,
      &spec.thread,
      &before,
      &model,
      fallback.as_deref(),
      spent,
    )
    .await?;
    continues.shape.summary = Some(summary);
    // Summarized, it is all of what came before: the last messages are
    // counted again from what was taken (`shaped` keeps them).
    continues.shape.last = Some(spec.last.unwrap_or(0));
  }
  let title = state
    .index
    .lock()
    .expect("index lock")
    .title(&spec.thread)?
    .unwrap_or_else(|| "Chat".into());
  let title = title.trim_start_matches(['⑂', '↻', '✂', ' ']).to_owned();
  let mut draft = Draft::new(kinds::THREAD, Author::user(state.node.clone()))?
    .body(format!("# {} {title}\n", prefix(&spec)));
  // Named, not read: the blob may be on another node only (design §5,
  // milestone 4: a phone's fork of a thread a Mac started).
  draft = crate::chat::with_system_of(draft, &original)?;
  if let Some(folder) = original.fields.get("folder") {
    draft = draft.field("folder", folder.clone())?;
  }
  let draft = more(continues.write(draft)?)?;
  // Its tools and skills as the thread had them changed.
  let id = RecordId::random(jiff::Timestamp::now(), &state.node);
  let (tools, skills) =
    crate::toolset::thread_changes(&state.index.lock().expect("index lock"), &spec.thread);
  crate::toolset::write_changes(&root, &state.index, &state.node, &id, &tools, &skills)?;
  let record = draft.into_record(id, jiff::Timestamp::now());
  root.publish_record(&record)?;
  state.index.lock().expect("index lock").refresh(&root)?;
  // Untrusted events in its history: the fork (a compaction too) is held
  // as the thread was, its summary included (docs/events.md).
  let taint = crate::profile::taint_of(&state.index.lock().expect("index lock"), &spec.thread);
  if !taint.is_empty() {
    crate::profile::write(s, &record.id, &taint)?;
  }
  Ok(record.id)
}

/// What a summary's model calls spent: their usage and, where a price is
/// known, their money (`counted`). Every compaction writes it as usage on
/// the thread compacted (`calls`), made or failed, so the chain's budget
/// counts it once (`detached::spent_by_chain`); a turn compacted midway
/// also takes it off its own limits (`detached::goes_on_compacted`).
#[derive(Clone, Debug, Default)]
pub(crate) struct SummarySpent {
  pub(crate) usage: stitch_providers::Usage,
  pub(crate) usd: Option<f64>,
  /// Each call, as a `usage` event (empty for a bound, `compaction_bound`).
  pub(crate) calls: Vec<stitch_records::Usage>,
}

impl SummarySpent {
  fn add(&mut self, usage: &stitch_providers::Usage, usd: Option<f64>) {
    self.usage.input_tokens += usage.input_tokens;
    self.usage.output_tokens += usage.output_tokens;
    self.usage.cache_read_tokens += usage.cache_read_tokens;
    self.usage.cache_write_tokens += usage.cache_write_tokens;
    self.usage.cache_write_1h_tokens += usage.cache_write_1h_tokens;
    if let Some(u) = usd {
      *self.usd.get_or_insert(0.0) += u;
    }
  }

  /// Its tokens, as a turn's limit counts them.
  pub(crate) fn tokens(&self) -> u64 {
    self.usage.input_tokens + self.usage.output_tokens
  }
}

/// The money a model call of `full` (`provider/model`) counts against a
/// turn's limit: what the provider billed, else its price; nothing on a
/// subscription (as a turn's own steps count, `chat`).
fn counted(
  provider: &stitch_providers::Provider,
  full: &str,
  model: &str,
  usage: &stitch_providers::Usage,
  cost: Option<f64>,
) -> Option<f64> {
  if cost.is_none() && provider.kind() == "anthropic" && provider.is_subscription() {
    return None;
  }
  crate::pricing::priced(model, usage, cost).or_else(|| crate::pricing::priced(full, usage, None))
}

/// A summary of `events` by a model; what its calls spent is added to
/// `spent`, those of a summary that then fails too.
async fn summarize(
  s: &Arc<Shared>,
  ctx: &ChatContext<'_>,
  thread: &RecordId,
  events: &[Event],
  model: &str,
  fallback: Option<&str>,
  spent: &mut SummarySpent,
) -> Result<String> {
  let (mut messages, _) = crate::chat::messages_from(events.iter(), ctx.blobs)?;
  messages.push(Message::user(vec![Content::Text(SUMMARY_PROMPT.into())]));
  // Deliberately conservative (bytes, including attachment payloads, not
  // a tokenizer). Never truncate history to squeeze it into quick.
  let input_bound = summary_input_bound(&messages);
  let fallback = fallback.filter(|f| *f != model);
  let mut candidates = vec![model];
  if let Some(f) = fallback {
    candidates.push(f);
  }
  for (i, full) in candidates.iter().enumerate() {
    let can_fallback = i + 1 < candidates.len();
    let window = crate::window::of(s, full).await;
    if can_fallback && !summary_fits(input_bound, window) {
      eprintln!("summary: {full} may not fit the conversation; using the thread model");
      continue;
    }
    let (provider, _, model) = s.config.resolve_authenticated(&s.data, Some(full)).await?;
    let mut request = Request::new(model, messages.clone());
    request.system = Some(SUMMARY_SYSTEM.into());
    request.max_tokens = summary_max_tokens(window);
    let response = match provider.stream(&request, &mut |_| {}).await {
      Ok(r) => r,
      Err(e) if can_fallback && too_long_said(&e.to_string()) => continue,
      Err(e) => return Err(e.into()),
    };
    let usd = counted(
      &provider,
      full,
      &response.model,
      &response.usage,
      response.cost,
    );
    spent.add(&response.usage, usd);
    spent.calls.push(stitch_records::Usage {
      at: jiff::Timestamp::now(),
      model: response.model.clone(),
      input_tokens: response.usage.input_tokens,
      output_tokens: response.usage.output_tokens,
      cache_read_tokens: Some(response.usage.cache_read_tokens),
      cache_write_tokens: Some(response.usage.cache_write_tokens),
      usd,
    });
    // An app's thread: the summary is its family's spend too (design
    // §2.22), at its price, or at the thread model's when unknown.
    if let Some(owner) = crate::apps::threads::owner_of(s, thread) {
      let usd = crate::pricing::priced(&response.model, &response.usage, response.cost)
        .or_else(|| crate::pricing::priced(full, &response.usage, None))
        .unwrap_or(0.0);
      s.apps.family.ledgers.charge(s, &owner, usd).await;
    }
    if !summary_complete(&response) {
      if can_fallback {
        eprintln!("summary: {full} returned an incomplete summary; using the thread model");
        continue;
      }
      return Err(Error::Config(
        "the summary was empty or incomplete; nothing was forked".into(),
      ));
    }
    return Ok(response.text().trim().to_owned());
  }
  unreachable!("at least one summary model")
}

fn summary_input_bound(messages: &[Message]) -> u64 {
  messages
    .iter()
    .fold(SUMMARY_SYSTEM.len() as u64, |total, m| {
      let content = m.content.iter().fold(64u64, |n, c| {
        n.saturating_add(match c {
          Content::Text(t) => t.len() as u64,
          Content::ToolCall { id, name, input } => {
            (id.len() + name.len() + input.to_string().len()) as u64
          }
          Content::ToolResult { id, output, .. } => (id.len() + output.len()) as u64,
          // Native media tokenization differs by provider. Counting bytes
          // conservatively avoids trusting a small quick window with it.
          Content::Image { data, .. } | Content::Document { data, .. } => data.len() as u64,
          Content::File { bytes, .. } => *bytes,
        })
        .saturating_add(64)
      });
      let native = m
        .native
        .as_ref()
        .map_or(0, |n| n.content.to_string().len() as u64);
      total.saturating_add(content.max(native))
    })
}

/// The most a summary by a model of `window` tokens may write: a tenth of
/// its window, at least 8192, at most what a turn asks for (64k, which
/// every provider takes). A long chain's summary carries more each time;
/// cut off, it is thrown away (`summary_complete`), so a tight cap fails
/// every compaction of it.
fn summary_max_tokens(window: u64) -> u32 {
  (window / 10).clamp(8192, 64_000) as u32
}

fn summary_fits(input_bound: u64, window: u64) -> bool {
  input_bound.saturating_add(u64::from(summary_max_tokens(window))) < window
}

fn summary_complete(response: &stitch_providers::Response) -> bool {
  response.stop == stitch_providers::StopReason::EndTurn && !response.text().trim().is_empty()
}

/// What a compacted thread continues in: the latest thread compacted from
/// it (`compacted = true`, continuing it), followed to the newest.
pub(crate) fn current(s: &Shared, thread: &RecordId) -> RecordId {
  lineage(s, thread).pop().expect("one")
}

/// A thread and those compacted from it, oldest first: a report from any
/// of them is the thread's.
pub(crate) fn lineage(s: &Shared, thread: &RecordId) -> Vec<RecordId> {
  let mut out = vec![thread.clone()];
  loop {
    let last = out.last().expect("one").clone();
    let next = current_step(s, &last);
    match next {
      Some(n) if !out.contains(&n) && out.len() < 64 => out.push(n),
      _ => break,
    }
  }
  out
}

/// The thread a thread's conversation starts in: what it continues
/// (forks and compactions read it, never copied), followed to the first.
pub(crate) fn origin(s: &Shared, thread: &RecordId) -> RecordId {
  let index = s.state().index.lock().expect("index lock");
  let mut out = thread.clone();
  for _ in 0..64 {
    let prev = index
      .edges_from(&out)
      .unwrap_or_default()
      .into_iter()
      .find(|e| e.rel == "continues")
      .map(|e| e.dst.id);
    match prev {
      Some(p) if p != out => out = p,
      _ => break,
    }
  }
  out
}

fn current_step(s: &Shared, thread: &RecordId) -> Option<RecordId> {
  let index = s.state().index.lock().expect("index lock");
  index.compacted_into(thread)
}

/// The share of the window past which a thread is compacted (0: never).
fn compact_percent(s: &Shared) -> u8 {
  s.config
    .threads
    .as_ref()
    .and_then(|t| t.compact_at)
    .unwrap_or(80)
    .min(100)
}

fn compact_keep(s: &Shared) -> usize {
  s.config
    .threads
    .as_ref()
    .and_then(|t| t.compact_keep)
    .unwrap_or(10)
}

/// Below this many tokens sent, a thread is near no model's window, and
/// its model's window is not looked up.
const NEAR_NO_WINDOW: u64 = 2_048;

/// Whether a thread's last step sent `model` more than the share of its
/// window `[threads] compact_at` allows.
pub(crate) async fn needs_compacting(s: &Shared, thread: &RecordId, model: &str) -> bool {
  let pct = compact_percent(s);
  if pct == 0 {
    return false;
  }
  let Ok((root, _)) = s.thread_place(thread) else {
    return false;
  };
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let Ok(Some(used)) = crate::chat::last_prompt_tokens(&ctx, thread) else {
    return false;
  };
  if used < NEAR_NO_WINDOW && crate::window::configured(s, model).is_none() {
    return false;
  }
  used * 100 >= crate::window::of(s, model).await * u64::from(pct)
}

/// Whether a provider's error says the prompt did not fit.
pub(crate) fn too_long(e: &Error) -> bool {
  too_long_said(&e.to_string())
}

/// `too_long`, from what the error says.
pub(crate) fn too_long_said(said: &str) -> bool {
  let text = said.to_ascii_lowercase();
  [
    "prompt is too long",
    "context_length_exceeded",
    "context window",
    "maximum context length",
    "too many tokens",
    "input is too long",
    // llama.cpp.
    "available context size",
  ]
  .iter()
  .any(|m| text.contains(m))
}

/// Continues a thread in a new one, from a summary and its last messages
/// (`[threads] compact_keep`), marked `compacted`: what is sent to the
/// thread, its reports and its loops go there from now on. A detached
/// thread's new one is detached as it was (goal, budget, model, who it
/// reports to); out of memory, it stays out.
///
/// `outgrew` is the model whose window it outgrew: the new record keeps it,
/// with that window and what the last step sent (`compacted_model`,
/// `compacted_window`, `compacted_tokens`), for the divider the lineage
/// transcript shows. Every path that compacts comes here, so here clients
/// are told (design §2.15, "One conversation"): a note on the old
/// thread's live feed while the summary is written, then
/// `LiveEvent::Compacted` on both feeds.
pub(crate) async fn compact(
  s: &Arc<Shared>,
  thread: &RecordId,
  outgrew: &str,
) -> Result<(RecordId, Compaction)> {
  compact_counted(s, thread, outgrew)
    .await
    .map(|(id, said, _)| (id, said))
}

/// `compact`, with what its summary's model calls spent.
pub(crate) async fn compact_counted(
  s: &Arc<Shared>,
  thread: &RecordId,
  outgrew: &str,
) -> Result<(RecordId, Compaction, SummarySpent)> {
  compact_as(s, thread, outgrew, compact_keep(s), false).await
}

/// The most a compaction of `thread` now (`compact`) may spend on its
/// summary, for a turn with a limit to say beforehand whether it can pay
/// for it. Every summary model it may ask (`summary_models`; the quick one
/// and the thread's, when quick is not enough, both paid) is counted with
/// all it may read, bounded by the bytes it would send (a token is at
/// least a byte: `summary_input_bound`) and by that model's window (more
/// is refused, unpaid), and the most it may write (`summary_max_tokens`). `usd` is `None`
/// when a model's price is not known beforehand.
pub(crate) async fn compaction_bound(s: &Arc<Shared>, thread: &RecordId) -> Result<SummarySpent> {
  let (root, _) = s.thread_place(thread)?;
  let before = {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    let mut spec = compaction_spec(thread, compact_keep(s));
    let before = loop {
      let (continues, taken) = continues_of(&ctx, &spec)?;
      match summarized(&ctx, &continues, &spec, &taken) {
        // A thread shorter than what is kept is summarized whole.
        Err(Error::Config(e)) if e.starts_with(NOTHING_TO_SUMMARIZE) && spec.last.is_some() => {
          spec.last = None;
        }
        other => break other?,
      }
    };
    let (mut messages, _) = crate::chat::messages_from(before.iter(), ctx.blobs)?;
    messages.push(Message::user(vec![Content::Text(SUMMARY_PROMPT.into())]));
    summary_input_bound(&messages)
  };
  let (model, fallback) = summary_models(s, thread).await?;
  let mut candidates = vec![model.clone()];
  candidates.extend(fallback.filter(|f| *f != model));
  let mut bound = SummarySpent {
    usd: Some(0.0),
    ..SummarySpent::default()
  };
  let mut unpriced = false;
  for full in &candidates {
    let Ok((provider, _, name)) = s.config.resolve_authenticated(&s.data, Some(full)).await else {
      // Not asked: the summary fails before its call.
      continue;
    };
    let window = crate::window::of(s, full).await;
    let usage = stitch_providers::Usage {
      input_tokens: before.min(window),
      output_tokens: u64::from(summary_max_tokens(window)),
      ..Default::default()
    };
    let usd = counted(&provider, full, &name, &usage, None);
    unpriced |= usd.is_none() && !(provider.kind() == "anthropic" && provider.is_subscription());
    bound.add(&usage, usd);
  }
  if unpriced {
    bound.usd = None;
  }
  Ok(bound)
}

/// Whether a fork asked for is a compaction (design §2.15, "One
/// conversation"): "summarize and truncate" (a summary, then its last
/// messages) at the thread's latest message, dropping nothing, is the
/// same conversation going on, as the app's button and the TUI's
/// `/compact` mean it. A plain fork, a summary of all of it (`/summarize`)
/// and a fork up to a message or without attachments or tool calls stay
/// forks the user made.
fn is_compaction(spec: &ForkSpec) -> bool {
  spec.summarize
    && spec.last.is_some()
    && spec.upto.is_none()
    && !spec.no_attachments
    && !spec.no_tools
}

/// A fork as `Request::Fork` asks it: a compaction when it is one
/// (`is_compaction`), else a fork.
pub(crate) async fn requested(s: &Arc<Shared>, spec: ForkSpec) -> Result<RecordId> {
  match spec.last {
    Some(keep) if is_compaction(&spec) => compact_by_user(s, &spec.thread, keep)
      .await
      .map(|(id, _)| id),
    _ => fork(s, spec).await,
  }
}

/// Compacts a thread as the user asked, keeping its last `keep`
/// messages: the newest of its compactions (an earlier one is the same
/// conversation), never while a turn runs on it (its lock is held while
/// the summary is written, as a chat's compaction holds it: a turn sent
/// meanwhile waits and then runs on the new thread), nor while another
/// node's turn has it.
pub(crate) async fn compact_by_user(
  s: &Arc<Shared>,
  thread: &RecordId,
  keep: usize,
) -> Result<(RecordId, Compaction)> {
  let thread = current(s, thread);
  let Ok(_held) = s.detached.locks.of(&thread).try_lock_owned() else {
    return Err(Error::Config(format!(
      "a turn runs on thread {thread}: compact it once the turn is over"
    )));
  };
  let (root, place) = s.thread_place(&thread)?;
  {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    if let Some(crate::chat::Unfinished {
      elsewhere: Some(node),
      ..
    }) = crate::chat::unfinished(&ctx, &thread)?
      && node != state.node.as_str()
    {
      return Err(Error::Config(format!(
        "{node} has the turn of thread {thread}: compact it once the turn is over"
      )));
    }
  }
  let model = crate::modelset::chat_model(s, Some(&thread), &place, None).await;
  compact_as(s, &thread, &model, keep, true)
    .await
    .map(|(id, said, _)| (id, said))
}

/// `compact`, keeping the last `keep` messages; `by_user`: the user asked
/// for it (`compact_by_user`), `model` then the thread's own, its window
/// and what its last step sent said as for one that outgrew it, and the
/// record marked `compacted_by_user`.
async fn compact_as(
  s: &Arc<Shared>,
  thread: &RecordId,
  outgrew: &str,
  keep: usize,
  by_user: bool,
) -> Result<(RecordId, Compaction, SummarySpent)> {
  let window = crate::window::of(s, outgrew).await;
  let tokens = {
    let (root, _) = s.thread_place(thread)?;
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    crate::chat::last_prompt_tokens(&ctx, thread).ok().flatten()
  };
  let note = if by_user {
    "[Compacted as asked: this thread continues in a new thread, from a summary of it and its \
     last messages.]"
      .to_owned()
  } else {
    format!(
      "[This thread is near the {}k-token window of {outgrew}: it continues in a new thread, \
       from a summary of it and its last messages.]",
      window / 1000
    )
  };
  s.live.push(thread.as_str(), LiveEvent::Note(note));
  let spec = compaction_spec(thread, keep);
  let (root, _) = s.thread_place(thread)?;
  let original = root.read_record(thread)?;
  // Its budget as it stands: one the user lifted stays lifted.
  let lifted = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .current_user_value(thread, "budget")
    .ok()
    .flatten()
    .is_some();
  // Its model as it stands: the one its client last picked, else the
  // one it was spawned on.
  let model = crate::meta::thread_model(&s.state().index.lock().expect("index lock"), thread);
  let carry = |mut draft: Draft| -> Result<Draft> {
    draft = draft.field("compacted", true)?;
    draft = draft.field("compacted_model", outgrew)?;
    draft = draft.field("compacted_window", window as i64)?;
    if by_user {
      draft = draft.field("compacted_by_user", true)?;
    }
    if let Some(t) = tokens {
      draft = draft.field("compacted_tokens", t as i64)?;
    }
    for key in [
      "detached", "goal", "budget", "cwd", "approval", "wake", "tools", "app", "app_name",
    ] {
      if key == "budget" && lifted {
        continue;
      }
      if let Some(v) = original.fields.get(key) {
        draft = draft.field(key, v.clone())?;
      }
    }
    if let Some(m) = &model {
      draft = draft.field("model", m.as_str())?;
    }
    for rel in ["spawned-by", "reports-to", "assignment"] {
      if let Some(r) = original.refs.first(rel) {
        draft = draft.reference(rel, r.clone())?;
      }
    }
    Ok(draft)
  };
  // A thread shorter than what is kept is summarized whole.
  let mut spent = SummarySpent::default();
  let made = match fork_with(s, spec.clone(), carry, &mut spent).await {
    Err(Error::Config(e)) if e.starts_with(NOTHING_TO_SUMMARIZE) => {
      fork_with(s, ForkSpec { last: None, ..spec }, carry, &mut spent).await
    }
    other => other,
  };
  // What its summary's calls spent, made or not, is spent on the thread
  // compacted (whose lock its caller holds): its chain's budget, a
  // schedule's caps and the cost shown count it, once
  // (`detached::spent_by_chain`).
  write_spent(s, thread, &spent);
  let id = made?;
  if s.memory.is_off(thread) {
    let recall = s.memory.may_recall(thread);
    s.memory.switch(s.state(), &root, &id, false, recall)?;
  } else if s.memory.is_switched(thread) {
    // Brought back by the user (`/memory on`): so is its continuation,
    // whatever its place's `memory = false` says (design §2.9).
    s.memory.switch(s.state(), &root, &id, true, false)?;
  }
  eprintln!("thread {thread}: compacted into {id}");
  let said = {
    let index = s.state().index.lock().expect("index lock");
    compaction_of(&index, &id)
  }
  .unwrap_or_else(|| Compaction {
    from: thread.to_string(),
    to: id.to_string(),
    model: outgrew.to_owned(),
    window,
    tokens,
    ..Compaction::default()
  });
  s.live
    .push(thread.as_str(), LiveEvent::Compacted(said.clone()));
  s.live.push(id.as_str(), LiveEvent::Compacted(said.clone()));
  Ok((id, said, spent))
}

/// Writes a summary's model calls (`spent.calls`) as `usage` on `thread`.
fn write_spent(s: &Arc<Shared>, thread: &RecordId, spent: &SummarySpent) {
  if spent.calls.is_empty() {
    return;
  }
  let written = s.thread_place(thread).and_then(|(root, _)| {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    let events = spent.calls.iter().cloned().map(Event::Usage).collect();
    crate::chat::write_spent(&ctx, thread, events)
  });
  if let Err(e) = written {
    eprintln!("thread {thread}: its summary's spend not written: {e}");
  }
}

/// The fork a compaction is: summarize & truncate at the latest message,
/// keeping its last `keep` messages.
fn compaction_spec(thread: &RecordId, keep: usize) -> ForkSpec {
  ForkSpec {
    thread: thread.clone(),
    upto: None,
    last: Some(keep),
    no_attachments: false,
    no_tools: false,
    summarize: true,
  }
}

/// The compaction a thread is (`compacted = true`, continuing `from`),
/// as clients are told of it: none for any other thread. A compaction
/// written before its figures were kept has `model` empty and `window` 0.
pub fn compaction_of(index: &stitch_store::Index, thread: &RecordId) -> Option<Compaction> {
  let from = index.compacted_from(thread)?;
  let r = index.record(thread).ok().flatten()?;
  let f = &r.fields;
  Some(Compaction {
    from: from.to_string(),
    to: r.id.to_string(),
    model: f
      .get("compacted_model")
      .and_then(serde_json::Value::as_str)
      .unwrap_or_default()
      .to_owned(),
    window: f
      .get("compacted_window")
      .and_then(serde_json::Value::as_u64)
      .unwrap_or(0),
    tokens: f
      .get("compacted_tokens")
      .and_then(serde_json::Value::as_u64),
    kept: f
      .get("continues_last")
      .and_then(serde_json::Value::as_u64)
      .unwrap_or(0),
    summary: f
      .get("summary")
      .and_then(serde_json::Value::as_str)
      .unwrap_or_default()
      .to_owned(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn summary_writes_a_tenth_of_the_window_within_bounds() {
    assert_eq!(summary_max_tokens(32_000), 8192);
    assert_eq!(summary_max_tokens(200_000), 20_000);
    assert_eq!(summary_max_tokens(1_000_000), 64_000);
    // The reserve for the summary counts in whether a model fits.
    assert!(summary_fits(170_000, 200_000));
    assert!(!summary_fits(185_000, 200_000));
  }
}
