//! Weaving threads (design §2.3, §2.7): every piece of work is a thread,
//! and a thread can start another and talk to any.
//!
//! - **`spawn`** starts a thread toward a goal, on this node or another of
//!   the user's, on a model that node runs, with or without its tools,
//!   starting from no conversation, this one, or any other thread's. It
//!   returns the thread's ID at once.
//! - **`send`** continues any thread with a message, on the node that owns
//!   it (where its shells and checkouts are).
//! - A spawned or messaged thread reports back to whoever started or
//!   messaged it, when its turn ends. The report is delivered on the node
//!   that owns that thread, once: by **`wait`**, which returns reports as
//!   they come in, or by waking the thread with it.
//! - **`models`** lists what a node runs.
//!
//! Work for another node goes as an `assignment` record in the repository
//! of the thread it is about: a new thread (whose ID the assignment
//! names) or a message for an existing one. Naming the node is the
//! placement: only it claims, so no lease. A node is nudged (synced with)
//! right away, and claims what is its own after every sync.

use std::sync::{Arc, Weak};
use std::time::Duration;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use stitch_providers::{ToolDef, Usage};
use stitch_records::{Author, Draft, Event, NodeName, Part, RecordId, Ref, kinds};

use crate::chat::{self, ChatContext, Continues, Limits, Upto};
use crate::node::Shared;
use crate::place::Places;
use crate::tools::{Spent, ToolOutcome};
use crate::{Error, Result};

/// How long a step may take to be written before what waits for it goes
/// without it (the regular sync carries it later).
const STEP_WAIT: Duration = Duration::from_secs(120);
/// How long `wait` waits at most, and by default.
/// The longest a `wait` given minutes waits.
const WAIT_MAX: Duration = Duration::from_secs(24 * 60 * 60);

/// The conversation a spawned thread starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum From {
  /// The goal alone.
  None,
  /// The spawning thread's, so far.
  This,
  /// Another thread's, so far.
  Thread(RecordId),
}

/// What to spawn.
pub(crate) struct Spawn {
  pub goal: String,
  /// The node to run it; this one when `None`.
  pub node: Option<String>,
  /// `provider/model`; that node's default when `None`.
  pub model: Option<String>,
  pub tools: bool,
  pub from: From,
  pub budget: Limits,
  /// Where a thread nobody started lives (as for a chat): its repository,
  /// the client's directory, no folder.
  pub repo: Option<String>,
  pub cwd: Option<String>,
  pub global: bool,
  /// Where it works on the node that runs it: a checkout or folder there
  /// (`~/devel/otter`), placed as a chat started in it is; its sandbox,
  /// instructions and skills are that place's. `None`: with the
  /// conversation it continues, else whoever spawned it.
  pub place: Option<String>,
  /// Changes to its tool and skill sets (design §2.14).
  pub toolset: Vec<crate::toolset::Entry>,
  pub skills: Vec<crate::toolset::Entry>,
  /// Its report wakes the thread it goes to (merged with others that came
  /// while it was idle); `false`: kept for the user, waking nothing.
  pub wake: bool,
  /// Memory remembers it; by default it is out of memory from its start.
  pub remember: bool,
  /// It may search memory (`recall`) even when out of it: what the user
  /// and other threads left. Remembered, it recalls anyway.
  pub recall: bool,
  /// Read-only: what the rules do not allow is refused, never asked. A
  /// read-only thread's spawns are read-only too, whatever they ask.
  pub read_only: bool,
  /// A fixer of Stitch (`selfimprove`): admitted on the node that runs it
  /// (its daily cap, one at a time), under its lock, before it starts.
  pub fix: Option<crate::selfimprove::Ticket>,
  /// Its budget as given, not bounded by what its parent has left (a
  /// fixer the triage starts reports to it, but spends its own).
  pub own_budget: bool,
}

/// `model`: the one this thread's loop runs on (`provider/model`), which
/// the model is told (not the node's default: design §2.7, "A thread's model").
pub(crate) fn definitions(peers: &[String], me: &str, model: &str) -> Vec<ToolDef> {
  let nodes = if peers.is_empty() {
    format!("this one, {me}")
  } else {
    format!("this one, {me}, or {}", peers.join(", "))
  };
  vec![
    ToolDef {
      name: "spawn".into(),
      description: format!(
        "Start a thread toward a goal: work that runs on its own (with tools), a question to \
         another model (tools false), work for another of the user's machines (node), with \
         that machine's shells and checkouts. It returns the thread's ID at once; when its \
         turn ends, its last reply comes back to this thread as a report, which wakes you, or \
         call `wait` to have it before going on. Spawn several and wait once to run them in \
         parallel. A thread can be continued later with `send`. Nodes: {nodes}. This thread \
         runs on {model}. Write the goal so it stands on its own. With `wake` false the \
         work is the user's: its report is kept for them on this thread and never wakes you \
         (`wait` for it still gets it)."
      ),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "goal": {"type": "string", "description": "What to do or answer, and how to tell it is done."},
          "node": {"type": "string", "description": "The machine to run it; this one when omitted."},
          "place": {"type": "string", "description": "Where it works on that machine: a checkout or folder there (`~/devel/otter`). Its sandbox (where it may write), instructions (AGENTS.md) and skills are that place's. When omitted: this thread's place, which may not cover the repository the goal is about; the result says the place it got."},
          "model": {"type": "string", "description": "`quick` for quick, self-contained work (a lookup, a summary, a small check or edit): the user's quick model. Omitted (or `default`): the user's default model. Or a `provider/model` that node runs (`models` lists them; one it cannot run is refused with the list); one the user reserved only when they asked for it by name."},
          "tools": {"type": "boolean", "description": "Let it use that node's tools (shells, web, files); default true. false: it only answers."},
          "context": {"type": "string", "description": "The conversation it starts from: `none` (default: the goal alone), `this` (this conversation so far), or a thread's ID (that thread's conversation)."},
          "budget_usd": {"type": "number", "description": "Stop it after spending this much (it never gets more than this thread has left)."},
          "budget_minutes": {"type": "integer", "description": "Stop it after this long."},
          "max_steps": {"type": "integer", "description": "Model calls each of its turns may take before it stops and reports (default: the node's, 50 unless configured). Raise it for long work: builds, test runs, driving another agent."},
          "toolset": {"type": "array", "items": {"type": "string"}, "description": "Changes to its tools, on top of its place's defaults: `+github` (an MCP server), `-web` (a built-in group: shell, web, viewer, weave, memory), `+github__get_*` (a pattern). It does not get this thread's changes otherwise."},
          "skills": {"type": "array", "items": {"type": "string"}, "description": "Changes to its skills: `-release`, `+mail`."},
          "wake": {"type": "boolean", "description": "Its report wakes this thread (default true; reports that come while it is idle wake it once, together). false: the report is kept for the user, shown when they open this thread, and never wakes it."},
          "recall": {"type": "boolean", "description": "Let it search memory (`recall`): the user's earlier conversations and what remembered threads left (default true). false for work that must not lean on the past (a fresh, independent look) or that only runs given steps."},
          "remember": {"type": "boolean", "description": "Keep this thread's conversation in memory, for you and for later threads. Its report reaches the parent either way; this adds the path: approaches that failed, why a choice was made, gotchas found. True for investigation, debugging and design work whose lessons a later thread would otherwise rediscover; false (the default) for mechanical runs (builds, polling, scripted steps), quick lookups, and work handling secrets. A place whose settings say `memory = false` keeps it out all the same (the result says so)."},
          "read_only": {"type": "boolean", "description": "Read-only (default false): commands and calls the rules do not allow are refused at once instead of waiting for the user's approval. Use it for looking and reporting (surveys, inventories, reviews), so the thread never blocks on the user. Its shells write only temporary directories; in that sandbox anything runs but what acts beyond it (pushing, uploads, agents, git but its reading subcommands), and unsandboxed only the built-in read-only commands run. A read-only thread's own spawns are read-only; only the user can switch a thread's mode."},
          "scopes": {"type": "array", "items": {"type": "string"}, "description": "Threads or efforts to brief it on with its goal: `effort:<id>` (a conversation and every thread spawned under it), `thread:<id>` or a thread's ID. It gets their digests, state and artefacts, and reads further with read_thread."}
        },
        "required": ["goal"]
      }),
    },
    ToolDef {
      name: "send".into(),
      description: "Continue a thread with a message: one you spawned (a follow-up, with all \
        it did), or any other. It runs on the node that owns that thread, with that thread's \
        model and tools; its reply comes back as a report, as from `spawn`. A message to a \
        thread at work waits for its turn to end, unless `interject`: then it is woven into \
        the running turn between its steps, and the turn's last reply comes back to you."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "thread": {"type": "string", "description": "The thread's ID."},
          "message": {"type": "string", "description": "What to say to it."},
          "wake": {"type": "boolean", "description": "Its reply wakes this thread (default true). false: kept for the user, never waking it."},
          "interject": {"type": "boolean", "description": "When a turn runs on that thread, weave the message into it between its steps instead of waiting for it to end (default false): for news, a correction or a stop that cannot wait. Its reply is that turn's last reply. With no turn running it starts one, as without."}
        },
        "required": ["thread", "message"]
      }),
    },
    ToolDef {
      name: "wait".into(),
      description: "Wait for threads you spawned or sent to, and get their reports. Without \
        threads: all of them still out. Returns when they are all in, or as soon as \
        something else comes for this thread (the user writes, a background job ends), or one \
        of them is blocked on the user's approval (once per approval: tell the user), with \
        what is in; the rest wake you when they report. No time to guess: give minutes only \
        when the work is due by then."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "threads": {"type": "array", "items": {"type": "string"}, "description": "The threads to wait for; all when omitted."},
          "minutes": {"type": "number", "description": "A deadline, when the work has one: return then with what is in. Without it, wait until the reports are in or something comes for this thread."}
        }
      }),
    },
    ToolDef {
      name: "threads".into(),
      description: "The threads this one spawned or sent to, newest first: each one's ID \
        (\"(you)\" marks this thread), the node it runs on now (an ID names the node it \
        started on, which may differ) and its place, its mode (read-only or writable), its state (working; \
        blocked: approval <question>: the exact command it waits on the user for; idle: \
        reported, with its outcome; ended), its goal, when it started and last stepped, its \
        model calls and cost so far, and the first lines of what it said last. Answering an \
        approval is the user's alone: tell them what it asks."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "all": {"type": "boolean", "description": "Every detached thread this node knows of, on any of the user's nodes, not only this thread's."},
          "status": {"type": "string", "enum": ["working", "blocked", "idle", "ended"], "description": "Only the threads in this state."}
        }
      }),
    },
    ToolDef {
      name: "cancel".into(),
      description: "Stop a thread's running turn now: its tool calls are cut, its jobs stopped \
        and its sessions closed with what runs in them, on whichever node runs it. Its report \
        says cancelled, with the reason and what it did so far (its last words and calls); \
        the result here says the same. A thread you did not spawn or send to: asks the user. \
        To change its course instead, `send` with interject."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "thread": {"type": "string", "description": "The thread's ID."},
          "reason": {"type": "string", "description": "Why, for its report (and the user)."}
        },
        "required": ["thread"]
      }),
    },
    ToolDef {
      name: "models".into(),
      description: "The models a node runs, `provider/model`, and its default: this node, or \
        another of the user's machines by name, with the user's quick model, prices per million \
        tokens where known, and those reserved for when the user asks. Providers without \
        credentials there say so."
        .into(),
      input_schema: json!({
        "type": "object", "additionalProperties": false,
        "properties": {
          "node": {"type": "string", "description": "Another machine; this one when omitted."}
        }
      }),
    },
  ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnInput {
  goal: String,
  #[serde(default)]
  node: Option<String>,
  #[serde(default)]
  place: Option<String>,
  #[serde(default)]
  model: Option<String>,
  #[serde(default)]
  tools: Option<bool>,
  #[serde(default)]
  context: Option<String>,
  #[serde(default)]
  budget_usd: Option<f64>,
  #[serde(default)]
  budget_minutes: Option<u64>,
  #[serde(default)]
  max_steps: Option<u64>,
  #[serde(default)]
  toolset: Vec<String>,
  #[serde(default)]
  skills: Vec<String>,
  #[serde(default)]
  wake: Option<bool>,
  #[serde(default)]
  remember: Option<bool>,
  #[serde(default)]
  recall: Option<bool>,
  /// Before `remember` and `recall` (2026-10-29): `true` both, `false`
  /// neither. Still read, no longer offered; the new ones win.
  #[serde(default)]
  memory: Option<bool>,
  #[serde(default)]
  read_only: bool,
  #[serde(default)]
  scopes: Vec<String>,
}

impl SpawnInput {
  /// (remember, recall): as asked, else as the old `memory` said, else
  /// out of memory, recalling.
  fn memory(&self) -> (bool, bool) {
    let remember = self.remember.or(self.memory).unwrap_or(false);
    let recall = self.recall.or(self.memory).unwrap_or(true);
    (remember, remember || recall)
  }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendInput {
  thread: String,
  message: String,
  #[serde(default)]
  wake: Option<bool>,
  #[serde(default)]
  interject: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitInput {
  #[serde(default)]
  threads: Vec<String>,
  #[serde(default)]
  minutes: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelsInput {
  #[serde(default)]
  node: Option<String>,
}

/// How long `wait` waits at most, from the minutes the model gave (none:
/// until the reports are in or something comes for the thread), bounded
/// before it is a duration, so a huge number is no panic.
fn wait_max(minutes: Option<f64>) -> Option<Duration> {
  minutes
    .filter(|m| m.is_finite() && *m > 0.0)
    .map(|m| Duration::from_secs_f64((m * 60.0).min(WAIT_MAX.as_secs_f64())))
}

/// A context mode as written: `none`, `this`, or a thread's ID.
pub(crate) fn parse_from(context: Option<&str>) -> Result<From> {
  Ok(match context.map(str::trim) {
    None | Some("") | Some("none") => From::None,
    Some("this") => From::This,
    Some(id) => From::Thread(id.parse()?),
  })
}

pub(crate) fn outcome(r: Result<String>) -> ToolOutcome {
  match r {
    Ok(output) => ToolOutcome {
      ok: true,
      output,
      images: Vec::new(),
      spent: None,
      attachments: Vec::new(),
      blobs: Vec::new(),
      generated: Vec::new(),
    },
    Err(e) => ToolOutcome::err(e.to_string()),
  }
}

/// A tool of this module, called by tool call `call` of `thread`.
pub(crate) async fn tool(
  shared: Weak<Shared>,
  thread: RecordId,
  call: String,
  name: String,
  input: Value,
) -> ToolOutcome {
  let Some(s) = shared.upgrade() else {
    return ToolOutcome::err("the node is stopping");
  };
  match name.as_str() {
    "spawn" => {
      let r = async {
        let i: SpawnInput =
          serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
        // `default` and `quick` are the user's own choices: never refused.
        if let Some(m) = i.model.as_deref()
          && !crate::modelset::ALIASES.contains(&m)
        {
          reserved_asked(&s, &thread, m)?;
        }
        let toolset = crate::toolset::parse_all(&i.toolset)?;
        if let Some(g) = crate::toolset::enables_guarded(&toolset) {
          return Err(Error::Config(format!(
            "`{g}` cannot be handed on: only the user turns it on for a thread (`stitch \
             toolset`)"
          )));
        }
        let (remember, recall) = i.memory();
        let scopes = i
          .scopes
          .iter()
          .map(|t| crate::scopes::parse_target(t))
          .collect::<Result<Vec<_>>>()?;
        let spec = Spawn {
          goal: i.goal,
          node: i.node,
          model: i.model,
          tools: i.tools.unwrap_or(true),
          from: parse_from(i.context.as_deref())?,
          budget: Limits {
            usd: i.budget_usd,
            tokens: None,
            until: i
              .budget_minutes
              .map(|m| Timestamp::now() + jiff::SignedDuration::from_mins(m as i64)),
            steps: i.max_steps.filter(|n| *n > 0),
          },
          repo: None,
          cwd: None,
          global: false,
          place: i.place.filter(|p| !p.trim().is_empty()),
          toolset,
          skills: crate::toolset::parse_all(&i.skills)?,
          wake: i.wake != Some(false),
          remember,
          recall,
          read_only: i.read_only,
          fix: None,
          own_budget: false,
        };
        let wake = spec.wake;
        let node = spec
          .node
          .clone()
          .unwrap_or_else(|| s.state().node.to_string());
        // Said, so the agent knows it before the child's report does.
        let mode = if s.read_only(&thread) {
          ", read-only (inherited from this thread: only the user can switch it)"
        } else if spec.read_only {
          ", read-only"
        } else {
          ""
        };
        // Its own goal handed straight back down is refused (2026-10-19:
        // a spawned thread spawned its goal again, and that one again).
        if spec.tools {
          redelegation(&s, &thread, &spec.goal, &node)?;
        }
        // A thread with nothing left spawns nothing: one that could never
        // run a turn is refused, not started (2026-10-19, `…xed6`: spawned
        // past its spawner's deadline, and over budget at once).
        if let Some(why) = spec.budget.spent() {
          return Err(Error::Config(format!(
            "refused: the budget asked for leaves it nothing ({why}): it could not run"
          )));
        }
        if let Some(why) = tighter(spec.budget, remaining(&s, &thread)).spent() {
          return Err(Error::Config(format!(
            "refused: this thread ({thread}) has nothing left of its budget as a spawned \
             thread ({why}), so a thread it spawns could not run. Do the work yourself, or \
             report to your spawner; a message from the user lifts the budget."
          )));
        }
        let capped = capped_said(spec.budget, remaining(&s, &thread));
        let same_place = spec.tools && !spec.read_only && !s.read_only(&thread);
        let remember = spec.remember;
        let here = node == s.state().node.as_str();
        let Spawned {
          id,
          place,
          busy,
          kept_out,
          unreached,
        } = spawn_scoped(&s, Some(&thread), Some(&call), spec, &scopes).await?;
        let busy = busy_said(&busy);
        let own = if same_place && here {
          own_place_said(&s, &thread, &id)
        } else {
          String::new()
        };
        // Said, so the agent does not count on it being remembered.
        let memory = if kept_out {
          format!(" {KEPT_OUT}")
        } else if remember && !here {
          format!(" {MAYBE_KEPT_OUT}")
        } else {
          String::new()
        };
        let busy = format!("{capped}{own}{busy}{memory}");
        // Said first and at once (2026-11-09): the spawner would otherwise
        // see "start unconfirmed" for an hour.
        if let Some(said) = unreached {
          let report = if wake {
            "its report comes back here then, which wakes you"
          } else {
            "its report is kept for the user on this thread"
          };
          return Ok(format!(
            "{said} (Spawned for {node}, in {place}{mode}; {report}.{busy})"
          ));
        }
        Ok(if wake {
          format!(
            "Spawned thread {id} to run on {node}, in {place}{mode}. Its report comes back here when \
             its turn ends, which wakes you; `wait` to have it now, `send` to continue it \
             later.{busy}"
          )
        } else {
          format!(
            "Spawned thread {id} to run on {node}, in {place}{mode}. Its report is kept for the user \
             on this thread and will not wake you; `wait` for it gets it, `send` continues \
             it.{busy}"
          )
        })
      }
      .await;
      outcome(r)
    }
    "send" => {
      let r = async {
        let i: SendInput =
          serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
        let to: RecordId = i.thread.parse()?;
        let wake = i.wake != Some(false);
        let (node, sent) = send(&s, Some(&thread), &to, &i.message, wake, i.interject).await?;
        let how = match (i.interject, sent) {
          (false, _) => String::new(),
          (true, sent) => format!(": {}", sent.said()),
        };
        Ok(if wake {
          format!(
            "Sent to thread {to}, which runs on {node}{how}. Its reply comes back here as a report; \
             `wait` to have it now."
          )
        } else {
          format!(
            "Sent to thread {to}, which runs on {node}{how}. Its reply is kept for the user on this \
             thread and will not wake you; `wait` for it gets it."
          )
        })
      }
      .await;
      outcome(r)
    }
    "wait" => {
      let i: WaitInput = match serde_json::from_value(input) {
        Ok(i) => i,
        Err(e) => return ToolOutcome::err(e.to_string()),
      };
      let mut threads = Vec::new();
      for t in &i.threads {
        match t.parse::<RecordId>() {
          Ok(id) => threads.push(id),
          Err(e) => return ToolOutcome::err(format!("{t}: {e}")),
        }
      }
      let max = wait_max(i.minutes);
      // A node stopping while it waits replays the call, as often as it
      // takes (`chat::waiting_on_user`).
      crate::chat::waiting_on_user(&s.state().index, &thread, true);
      let waited = wait(&s, &thread, threads, max).await;
      crate::chat::waiting_on_user(&s.state().index, &thread, false);
      ToolOutcome {
        ok: true,
        output: waited.text,
        images: Vec::new(),
        spent: waited.spent.map(|(usage, usd)| Spent {
          model: "threads waited for".into(),
          usage,
          usd,
        }),
        attachments: Vec::new(),
        blobs: Vec::new(),
        generated: Vec::new(),
      }
    }
    "models" => {
      let r = async {
        let i: ModelsInput =
          serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
        let (models, problems, default) = crate::node::models_of(&s, i.node.as_deref()).await?;
        // This thread's place may set its own default and quick model.
        let place = s.thread_place(&thread).ok().map(|(_, p)| p);
        let here = i
          .node
          .as_deref()
          .is_none_or(|n| n == s.state().node.as_str());
        let default = if here {
          crate::modelset::shown(&crate::modelset::default_said(&s, place.as_ref()).await)
        } else {
          default
        };
        Ok(crate::modelset::render(
          &crate::modelset::settings(&s),
          &models,
          &problems,
          &default,
          &crate::modelset::shown(&crate::modelset::quick_said(&s, place.as_ref()).await),
        ))
      }
      .await;
      outcome(r)
    }
    other => ToolOutcome::err(format!("no tool {other}")),
  }
}

/// Whether `node` is this one or one of the user's others.
pub(crate) fn known(s: &Shared, node: &str) -> Result<()> {
  let state = s.state();
  let ok = node == state.node.as_str()
    || state
      .known_peers()
      .unwrap_or_default()
      .iter()
      .any(|p| p.name == node);
  if ok {
    Ok(())
  } else {
    Err(Error::Config(format!(
      "no node `{node}` among the user's nodes"
    )))
  }
}

/// The last written segment of a thread.
fn last_segment(s: &Shared, thread: &RecordId) -> Result<RecordId> {
  let (root, _) = s.thread_place(thread)?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  chat::history(&ctx, thread)
    .ok()
    .and_then(|h| h.last_segment)
    .ok_or_else(|| Error::Config(format!("thread {thread} has no conversation yet")))
}

/// What a thread has left of its budget: its limits less what it spent,
/// over all its turns (its own step limit as it is: that one is per turn).
pub(crate) fn remaining(s: &Arc<Shared>, thread: &RecordId) -> Limits {
  let record = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .record(thread)
    .ok()
    .flatten();
  let limits = budget_of(
    &s.state().index.lock().expect("index lock"),
    thread,
    record.as_ref().and_then(|r| r.fields.get("budget")),
  );
  if limits == Limits::default() {
    return limits;
  }
  let (usage, usd) = crate::detached::spent_by_chain(s, thread, None).unwrap_or_default();
  Limits {
    usd: limits.usd.map(|u| (u - usd.unwrap_or(0.0)).max(0.0)),
    tokens: limits
      .tokens
      .map(|t| t.saturating_sub(usage.input_tokens + usage.output_tokens)),
    until: limits.until,
    steps: limits.steps,
  }
}

/// A thread's budget as it stands: the user's `set` of `budget` when there
/// is one (the user taking a spawned thread over by writing in it lifts its
/// spawn budget, design §2.7), else the one it was spawned with (`record`,
/// its record's `budget` field).
pub(crate) fn budget_of(
  index: &stitch_store::Index,
  thread: &RecordId,
  record: Option<&Value>,
) -> Limits {
  match index.current_user_value(thread, "budget").ok().flatten() {
    Some(set) => Limits::from_json(Some(&set.value)),
    None => Limits::from_json(record),
  }
}

/// Lifts a spawned thread's budget when the user writes in it (decided
/// 2026-10-19): the thread is the user's chat from then on, and chats have
/// no budget; reports and messages that wake it run, and what it spawns
/// gets what it asks for. A user's `set` of `budget` to no limits, written
/// once (nothing when the thread had none, or it was lifted already).
pub(crate) fn lift_budget(s: &Shared, thread: &RecordId) -> Result<bool> {
  let budgeted = {
    let state = s.state();
    let index = state.index.lock().expect("index lock");
    let Some(record) = index.record(thread)? else {
      return Ok(false);
    };
    record.fields.get("detached").and_then(Value::as_bool) == Some(true)
      && budget_of(&index, thread, record.fields.get("budget")) != Limits::default()
  };
  if !budgeted {
    return Ok(false);
  }
  crate::meta::set(
    s,
    thread,
    "budget",
    toml::Value::Table(toml::Table::new()),
    Author::user(s.state().node.clone()),
  )?;
  Ok(true)
}

/// The tighter of two limits, but for `a`'s step limit: steps are a turn's
/// own, nothing a parent spends from.
fn tighter(a: Limits, b: Limits) -> Limits {
  fn min<T: PartialOrd>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
      (Some(x), Some(y)) => Some(if x < y { x } else { y }),
      (x, y) => x.or(y),
    }
  }
  Limits {
    usd: min(a.usd, b.usd),
    tokens: min(a.tokens, b.tokens),
    until: min(a.until, b.until),
    steps: a.steps,
  }
}

/// The spawn tool's result for a thread asked to be remembered that its
/// place keeps out of memory (design §2.9).
pub(crate) const KEPT_OUT: &str = "Not remembered: its place's settings say `memory = false`, \
  which wins over `remember: true` (only the user brings it back, `/memory on`); it may still \
  recall.";

/// The same, for one started on another node, whose place's settings
/// are read there.
pub(crate) const MAYBE_KEPT_OUT: &str = "It is remembered unless its place there says `memory = \
  false`, which wins over `remember: true`.";

/// What `spawn` started.
pub(crate) struct Spawned {
  pub id: RecordId,
  /// Its place's label.
  pub place: String,
  /// The writable threads already at work there.
  pub busy: Vec<(RecordId, String)>,
  /// Its place's `memory = false` keeps it out of memory though it was
  /// asked to be remembered (known when it starts here).
  pub kept_out: bool,
  /// The node named was not reached: what to say of it, and which nodes
  /// reached now could run it instead (`unreached_said`).
  pub unreached: Option<String>,
}

/// Starts a thread (see the module docs); its ID, at once, and the place
/// it works in (a label; `unchecked` when it is another node's to resolve
/// and that node did not answer). `parent` is who it reports to, `call`
/// the tool call that spawned it (the step that is written after it,
/// which `From::This` waits for). Third: the writable threads already at
/// work in that place on that node (`busy_in`), for its result to say.
pub(crate) async fn spawn(
  s: &Arc<Shared>,
  parent: Option<&RecordId>,
  call: Option<&str>,
  spec: Spawn,
) -> Result<Spawned> {
  spawn_scoped(s, parent, call, spec, &[]).await
}

/// `spawn`, with threads and efforts attached to its goal (design §2.27):
/// resolved here first (one that names no thread is refused), written with
/// the goal where the thread starts (`Part::Scope`).
pub(crate) async fn spawn_scoped(
  s: &Arc<Shared>,
  parent: Option<&RecordId>,
  call: Option<&str>,
  spec: Spawn,
  scopes: &[crate::api::ScopeTarget],
) -> Result<Spawned> {
  if spec.goal.trim().is_empty() {
    return Err(Error::Config("goal must say what to do".into()));
  }
  let scope = crate::scopes::scoped(s, scopes).await?;
  let me = s.state().node.to_string();
  let node = spec.node.clone().unwrap_or_else(|| me.clone());
  known(s, &node)?;
  // Another node is asked first whether it is reached at all: one that is
  // not is asked nothing more (each question would wait out its own
  // timeout), its assignment is queued all the same, and the result says
  // so at once, with the nodes that could run it now (2026-11-09).
  let unreached = if node != me
    && matches!(
      crate::elsewhere::reach(s, &node).await,
      crate::elsewhere::Reach::Unreached
    ) {
    let model = spec
      .model
      .as_deref()
      .filter(|m| !crate::modelset::ALIASES.contains(m));
    let want = crate::elsewhere::Wanted {
      tools: spec.tools,
      place: spec.place.as_deref(),
      model,
    };
    Some(crate::elsewhere::alternatives(s, &node, &want).await)
  } else {
    None
  };
  // An alias is resolved where the thread starts, for its place there.
  if let Some(m) = spec.model.as_deref()
    && !crate::modelset::ALIASES.contains(&m)
    && unreached.is_none()
  {
    crate::node::check_model(s, &node, m).await?;
  }
  let continues = match (&spec.from, parent) {
    (From::None, _) => None,
    (From::This, None) => {
      return Err(Error::Config(
        "context `this` needs a thread to start from (--parent)".into(),
      ));
    }
    (From::This, Some(p)) => Some(Continues {
      shape: Default::default(),
      thread: p.clone(),
      at: match call {
        Some(c) => Upto::Call(c.to_owned()),
        None => Upto::Segment(last_segment(s, p)?),
      },
    }),
    (From::Thread(t), _) => Some(Continues {
      shape: Default::default(),
      thread: t.clone(),
      at: Upto::Segment(last_segment(s, t)?),
    }),
  };
  // It lives with the conversation it continues (it reads it), else with
  // whoever started it, else where the client is (`Spec::at` empty).
  let near = continues
    .as_ref()
    .map(|c| c.thread.clone())
    .or_else(|| parent.cloned());
  let at = near.as_ref().map(|t| s.thread_place(t)).transpose()?;
  // The place asked for, resolved where it runs: here, or by asking that
  // node (one that cannot be asked resolves it when it starts the thread).
  let mut here = None;
  let asked = match &spec.place {
    Some(p) if node == me => {
      let r = resolve_place(s, p)?;
      let found = r.found();
      here = Some(r);
      Some(Some(found))
    }
    Some(_) if unreached.is_some() => Some(None),
    Some(p) => Some(crate::node::peer_place(s, &node, p).await?),
    None => None,
  };
  // It reads the conversation it continues from its own repository.
  if let (Some(c), Some(Some(found))) = (&continues, &asked) {
    same_root(s, c, &found.repo)?;
  }
  let label = match &asked {
    Some(Some(found)) => found.label.clone(),
    Some(None) => format!(
      "{} (unchecked: {node} {})",
      spec.place.as_deref().unwrap_or(""),
      if unreached.is_some() {
        "not reached"
      } else {
        "did not answer"
      }
    ),
    None => at
      .as_ref()
      .map_or_else(|| "global".to_owned(), |(_, p)| p.label(&node, &s.places())),
  };
  let budget = match parent {
    Some(p) if !spec.own_budget => tighter(spec.budget, remaining(s, p)),
    _ => spec.budget,
  };
  // Never looser than whoever spawns it.
  let read_only = spec.read_only || parent.is_some_and(|p| s.read_only(p));
  // Writable threads already at work there, said (not refused): two
  // writers in one checkout trip over each other (2026-10-17).
  let busy = if read_only {
    Vec::new()
  } else {
    let (dir, repo) = match (&here, &asked, &at) {
      (Some(r), _, _) => (Some(r.dir.clone()), r.repo.clone()),
      (None, Some(Some(found)), _) => (
        found.dir.as_ref().map(std::path::PathBuf::from),
        found.repo.clone(),
      ),
      (None, _, Some((root, place))) => {
        let dir = (node == me && (spec.cwd.is_some() || *place != crate::place::Place::Global))
          .then(|| {
            crate::place::start_dir(
              s.place_dir(place),
              spec.cwd.as_deref().map(std::path::Path::new),
            )
          })
          .flatten();
        (dir, root.repo().to_string())
      }
      _ => (None, String::new()),
    };
    if dir.is_none() && node == me {
      Vec::new()
    } else {
      let except: Vec<&RecordId> = parent.into_iter().collect();
      busy_in(s, &node, dir.as_deref(), &repo, &except)
    }
  };
  let runner: NodeName = node.parse()?;
  let id = RecordId::random(Timestamp::now(), &runner);
  let unreached = unreached.map(|alts| unreached_said(s, &node, &id, &alts));
  let waits_for_step = matches!(&continues, Some(c) if matches!(c.at, Upto::Call(_)));
  if let Some(p) = parent
    && spec.wake
  {
    expect(s, p, &id);
  }
  let fix = spec.fix.clone();
  // Started here and asked to be remembered, but kept out by its place.
  let mut kept_out = false;
  if node == me {
    // A fixer: admitted here, under the lock, before it starts.
    if let Some(t) = &fix {
      crate::selfimprove::admit(s, t, &id).await?;
    }
    let (at, cwd) = match here {
      Some(r) => (
        Some(s.placement(None, false, &r.here)?),
        Some(r.dir.display().to_string()),
      ),
      None => (at, spec.cwd),
    };
    let detached = crate::detached::Spec {
      goal: spec.goal,
      model: spec.model,
      repo: spec.repo,
      cwd,
      parent: parent.cloned(),
      budget,
      global: spec.global,
      at,
      assignment: None,
      continues,
      id: Some(id.clone()),
      system: None,
      no_tools: !spec.tools,
      tools: spec.toolset,
      skills: spec.skills,
      keep: !spec.wake,
      remember: spec.remember,
      recall: spec.recall,
      read_only,
      scheduled: None,
      // A fixer, or a thread a fixer spawns: the fixers' mode.
      fixer: fix.is_some() || parent.is_some_and(|p| crate::selfimprove::in_fixer_mode(s, p)),
      app: None,
      attachments: Vec::new(),
      scope,
      profile: None,
    };
    kept_out = crate::detached::place_keeps_out(s, &detached);
    if waits_for_step {
      // It reads the spawning step, written once the step's calls return.
      let (p, c) = (
        parent.expect("context this has a parent").clone(),
        call.expect("a call").to_owned(),
      );
      s.detached
        .starting
        .lock()
        .expect("starting lock")
        .insert(id.to_string(), p.to_string());
      let (s, child) = (s.clone(), id.clone());
      tokio::spawn(async move {
        wait_for_step(&s, &p, &c).await;
        let started = crate::detached::start(&s, detached).await;
        s.detached
          .starting
          .lock()
          .expect("starting lock")
          .remove(child.as_str());
        if let Err(e) = started {
          eprintln!("spawning {child}: {e}");
          s.detached.reported.notify_waiters();
        }
      });
    } else if let Err(e) = crate::detached::start(s, detached).await {
      if let Some(t) = &fix {
        crate::selfimprove::release(s, t, &id, &e.to_string());
      }
      return Err(e);
    }
  } else {
    // A fixer the user asks for from the command line has no thread to
    // report to: its assignment lives in the home workspace.
    let at = match (at, &fix) {
      (None, Some(_)) => s.state().home().map(|h| (h, crate::place::Place::Global)),
      (at, _) => at,
    };
    let Some((root, _)) = at else {
      return Err(Error::Config(
        "a thread for another node needs a thread to report to or start from".into(),
      ));
    };
    let mut draft = Draft::new(kinds::ASSIGNMENT, Author::user(s.state().node.clone()))?
      .body(format!("# {}\n", one_line(&spec.goal)))
      .field("node", node.as_str())?
      .field("goal", spec.goal.as_str())?
      .field("thread", id.as_str())?;
    if let Some(p) = parent {
      draft = draft.reference("reports-to", Ref::to(p.clone()))?;
    }
    if let Some(p) = &spec.place {
      draft = draft.field("place", p.as_str())?;
    }
    if let Some(m) = &spec.model {
      if matches!(m.as_str(), "quick" | "default") {
        // Resolved where it starts (`model_as`, the name), and here too, as
        // `model`: a node older than these names (2026-10-26) reads only
        // that, and refused "quick" at every pass, never starting it.
        let resolved = crate::modelset::named(s, None, Some(m)).await;
        draft = draft
          .field("model", resolved.as_str())?
          .field("model_as", m.as_str())?;
      } else {
        draft = draft.field("model", m.as_str())?;
      }
    }
    if let Some(b) = budget.to_toml() {
      draft = draft.field("budget", b)?;
    }
    if !spec.tools {
      draft = draft.field("tools", false)?;
    }
    if !spec.wake {
      draft = draft.field("wake", false)?;
    }
    // `memory = true`: remembered (and so recalling), as every build
    // reads it; `recall = true` out of memory, which a build from before
    // 2026-10-29 ignores (out of memory, no recall: its old default).
    if spec.remember {
      draft = draft.field("memory", true)?;
    } else if spec.recall {
      draft = draft.field("recall", true)?;
    }
    if read_only {
      draft = draft.field("approval", "read-only")?;
    }
    if !scopes.is_empty() {
      draft = draft.field("scopes", crate::scopes::field(scopes))?;
    }
    if let Some(t) = &fix {
      draft = t.write(draft)?;
    }
    if let Some(c) = &continues {
      draft = c.write(draft)?;
    }
    // Its sets are there before the assignment, wherever it starts.
    crate::toolset::write_changes(
      &root,
      &s.state().index,
      &s.state().node,
      &id,
      &spec.toolset,
      &spec.skills,
    )?;
    publish(s, &root, draft)?;
    let step = waits_for_step.then(|| {
      (
        parent.expect("a parent").clone(),
        call.expect("a call").to_owned(),
      )
    });
    let s = s.clone();
    tokio::spawn(async move { nudge(&s, &node, step).await });
  }
  Ok(Spawned {
    id,
    place: label,
    busy,
    kept_out,
    unreached,
  })
}

/// What a spawn's result says when the node named was not reached
/// (design §2.7, "A spawn to a node not reached"): that the work is
/// queued there all the same, when that node was last reached, how to
/// withdraw it, and `alts`, the nodes reached now that could run it.
/// Nothing is rerouted: the model decides.
fn unreached_said(s: &Shared, node: &str, id: &RecordId, alts: &[String]) -> String {
  let seen = s.state().seen();
  let last = match seen.peer.get(node).and_then(|p| p.reached) {
    Some(r) => {
      let now = Timestamp::now().as_second().max(0) as u64;
      format!(
        "last seen {} ago",
        crate::jobs::spoken(Duration::from_secs(now.saturating_sub(r)))
      )
    }
    None => "never reached from here".to_owned(),
  };
  let alts = if alts.is_empty() {
    "No other node is reached now that could run it.".to_owned()
  } else {
    format!(
      "Reached now and able to run it: {}. Leave it queued, or `cancel` it and spawn it on one \
       of those.",
      alts.join("; ")
    )
  };
  format!(
    "{node} not reached ({last}): thread {id} is queued there and starts when {node} is back; \
     nothing runs it until then. `cancel {id}` withdraws it. {alts}"
  )
}

/// How deep spawns go at most: a thread this many spawns below one nobody
/// spawned spawns nothing more (a backstop against delegation that
/// recurses; design §2.7, "Delegation does not recurse").
pub(crate) const MAX_SPAWN_DEPTH: usize = 6;

/// How alike two goals must be (Jaccard similarity of their words) to be
/// the same goal: the goals of the 2026-10-19 chain, reworded a little at
/// each step, were 0.94 to 0.99 alike; a goal and its spawner's distinct
/// part of it, 0.35.
const SAME_GOAL: f64 = 0.8;

/// A goal's words, lowercased, for `same_goal`.
fn goal_words(goal: &str) -> std::collections::BTreeSet<String> {
  goal
    .split(|c: char| !c.is_alphanumeric())
    .filter(|w| !w.is_empty())
    .map(str::to_lowercase)
    .collect()
}

/// Whether two goals are the same or nearly (`SAME_GOAL`).
pub(crate) fn same_goal(a: &str, b: &str) -> bool {
  let (a, b) = (goal_words(a), goal_words(b));
  if a.is_empty() || b.is_empty() {
    return false;
  }
  let common = a.intersection(&b).count();
  let all = a.union(&b).count();
  common as f64 / all as f64 >= SAME_GOAL
}

/// The thread and the threads that spawned it in turn (`spawned-by`), with
/// their goals (none for a chat), nearest first, as far as the records go.
fn lineage_goals(s: &Shared, thread: &RecordId) -> Vec<(RecordId, Option<String>)> {
  let index = s.state().index.lock().expect("index lock");
  let mut out: Vec<(RecordId, Option<String>)> = Vec::new();
  let mut at = Some(thread.clone());
  while let Some(t) = at.take() {
    if out.iter().any(|(o, _)| *o == t) || out.len() > MAX_SPAWN_DEPTH {
      break;
    }
    let goal = index.record(&t).ok().flatten().and_then(|r| {
      r.fields
        .get("goal")
        .and_then(Value::as_str)
        .map(str::to_owned)
    });
    at = index.edges_from(&t).ok().and_then(|edges| {
      edges
        .into_iter()
        .find(|e| e.rel == "spawned-by")
        .map(|e| e.dst.id)
    });
    out.push((t, goal));
  }
  out
}

/// Refuses an agent's spawn on a model the user's model settings reserve
/// (design §2.7) unless the user named it in a message they
/// wrote in `thread` or a thread above it.
fn reserved_asked(s: &Shared, thread: &RecordId, model: &str) -> Result<()> {
  if !crate::modelset::reserved(&crate::modelset::settings(s), model) {
    return Ok(());
  }
  let asked = lineage_goals(s, thread).iter().any(|(t, _)| {
    let Ok((root, _)) = s.thread_place(t) else {
      return false;
    };
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    let mut texts = chat::user_texts(&chat::thread_events(&ctx, t).unwrap_or_default());
    // The step whose calls run now is written after them.
    texts.extend(chat::unwritten_user_texts(t));
    texts
      .iter()
      .any(|text| crate::modelset::mentions(text, model))
  });
  if asked {
    return Ok(());
  }
  Err(Error::Config(format!(
    "refused: {model} is reserved: spawned only when the user asks for it by name, and no \
     message of theirs here does. Pick another model (`models` ranks them), or leave it out \
     for the default."
  )))
}

/// Refuses a spawn (with tools) that hands `thread`'s own goal, or the
/// goal of a thread above it, straight back down, or that goes deeper than
/// `MAX_SPAWN_DEPTH` (design §2.7, "Delegation does not recurse"). One
/// exception: a thread on a node that runs no programs (a phone) hands its
/// goal to another node, which is how work for a machine gets there.
fn redelegation(s: &Shared, thread: &RecordId, goal: &str, node: &str) -> Result<()> {
  let lineage = lineage_goals(s, thread);
  let me = s.state().node.to_string();
  let handing_on = node != me && !stitch_shell::CAN_SPAWN;
  if !handing_on
    && let Some((t, _)) = lineage
      .iter()
      .find(|(_, g)| g.as_deref().is_some_and(|g| same_goal(goal, g)))
  {
    let whose = if t == thread {
      format!("your own goal (you are thread {thread})")
    } else {
      format!("the goal of thread {t}, which spawned you or your spawner")
    };
    return Err(Error::Config(format!(
      "refused: this goal is {whose}, nearly word for word. It was handed to you to do: do it \
       yourself, here, with your own tools. Spawn only for a separate part of it (a goal of \
       its own), or for work another node must do. If you cannot do it here, end your turn \
       with `Blocked:` and say why."
    )));
  }
  // The thread itself and those above it: its depth is one less.
  if lineage.len() > MAX_SPAWN_DEPTH {
    return Err(Error::Config(format!(
      "refused: thread {thread} is {MAX_SPAWN_DEPTH} spawns below the thread that started \
       this work; it spawns nothing more. Do the work yourself, or report `Blocked:` to your \
       spawner."
    )));
  }
  Ok(())
}

/// What a spawn's result says when the budget given was cut to what the
/// spawning thread has left (a child never gets more than its parent has):
/// on 2026-10-19 a two-hour spawn got the six minutes its spawner had left,
/// unsaid, and every later turn of it was refused as past its deadline.
fn capped_said(asked: Limits, left: Limits) -> String {
  let mut said = Vec::new();
  if let Some(until) = left.until
    && asked.until.is_none_or(|a| until < a)
  {
    let now = Timestamp::now();
    let mins = (until.as_second() - now.as_second()).max(0) / 60;
    let at = until
      .to_zoned(jiff::tz::TimeZone::system())
      .strftime("%H:%M")
      .to_string();
    said.push(format!("it must end by {at} ({mins} minutes from now)"));
  }
  if let Some(usd) = left.usd
    && asked.usd.is_none_or(|a| usd < a)
  {
    said.push(format!("it may spend ${usd:.2} at most"));
  }
  if let Some(tokens) = left.tokens
    && asked.tokens.is_none_or(|a| tokens < a)
  {
    said.push(format!("it may use {tokens} tokens at most"));
  }
  if said.is_empty() {
    return String::new();
  }
  format!(
    " Its budget is this thread's remainder: {}. Turns past it do not run (the reports and \
     messages that come for it then are kept in its transcript).",
    said.join(", ")
  )
}

/// What a writable spawn says when a spawned thread starts another in its
/// own place on its own node: allowed (an orchestrator splits a big job),
/// but the usual reason it happens is a thread delegating what it was
/// handed (design §2.7, "Delegation does not recurse").
fn own_place_said(s: &Arc<Shared>, thread: &RecordId, child: &RecordId) -> String {
  let spawned = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .edges_from(thread)
    .ok()
    .is_some_and(|edges| edges.iter().any(|e| e.rel == "spawned-by"));
  if !spawned {
    return String::new();
  }
  let (Ok((_, mine)), Ok((_, theirs))) = (s.thread_place(thread), s.thread_place(child)) else {
    return String::new();
  };
  if mine != theirs {
    return String::new();
  }
  format!(
    " Note: you were spawned to work in this place, and {child} writes in it too. Hand it only \
     a separate part of your goal, never the goal itself, and do your own part yourself."
  )
}

/// What a spawn's result says of the writable threads already at work
/// where it went (`busy_in`); empty when none.
pub(crate) fn busy_said(busy: &[(RecordId, String)]) -> String {
  if busy.is_empty() {
    return String::new();
  }
  let names: Vec<String> = busy
    .iter()
    .map(|(id, title)| match title.trim() {
      "" => format!("thread {id}"),
      t => format!("thread {id} \"{t}\""),
    })
    .collect();
  format!(
    " Note: another writable thread is working in that place on that node ({}): the two may \
     write over each other there. It was not refused; `cancel` one, or `send` to coordinate, if \
     that was not intended.",
    names.join(", ")
  )
}

/// A place a spawn asks for, found on the node that runs it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) struct Found {
  /// The project's name, or the folder.
  pub label: String,
  /// The repository its threads are written in (a project's own; the
  /// home workspace's otherwise).
  pub repo: String,
  /// The directory, on that node (absent from an older node's answer).
  #[serde(default)]
  pub dir: Option<String>,
}

/// A directory as a place, as a chat started in it is placed (`Shared::
/// place_of_dir`), without listing a project seen for the first time.
pub(crate) struct Resolved {
  pub here: (crate::place::Place, Option<crate::place::Checkout>),
  pub dir: std::path::PathBuf,
  label: String,
  repo: String,
}

impl Resolved {
  pub(crate) fn found(&self) -> Found {
    Found {
      label: self.label.clone(),
      repo: self.repo.clone(),
      dir: Some(same_dir(&self.dir).display().to_string()),
    }
  }
}

/// A directory as compared with another: canonical where it exists.
fn same_dir(d: &std::path::Path) -> std::path::PathBuf {
  std::fs::canonicalize(d).unwrap_or_else(|_| d.to_owned())
}

/// Writable threads at work on `node` in `dir` (the directory their shells
/// start in), other than `except`: what a spawn there says (design §2.7,
/// "Spawning into a busy place"), never refused. From the records, as
/// `threads` reads them: a detached thread working or blocked, run by
/// `node`, not read-only. Their directory is computed here for this
/// node's own; for another node's it is their record's `cwd`, else (no
/// `cwd`: started in their place's directory there) the same repository.
pub(crate) fn busy_in(
  s: &Arc<Shared>,
  node: &str,
  dir: Option<&std::path::Path>,
  repo: &str,
  except: &[&RecordId],
) -> Vec<(RecordId, String)> {
  let me = s.state().node.to_string();
  // Canonical here; another node's as it said it (canonical there).
  let dir = dir.map(|d| {
    if node == me {
      same_dir(d)
    } else {
      d.to_owned()
    }
  });
  let mut out = Vec::new();
  for id in listed(s, None, true) {
    if except.contains(&&id) {
      continue;
    }
    let (_, word) = state_of(s, &id);
    if !matches!(word, "working" | "blocked") || s.read_only(&id) {
      continue;
    }
    let (record, title) = {
      let index = s.state().index.lock().expect("index lock");
      let Ok(Some(record)) = index.record(&id) else {
        continue;
      };
      let title = index.title(&id).ok().flatten();
      (record, title)
    };
    if runner(s, &id, record.node.as_str()) != node {
      continue;
    }
    let Ok((root, place)) = s.thread_place(&id) else {
      continue;
    };
    let cwd = record.fields.get("cwd").and_then(Value::as_str);
    let same = if node == me {
      if place == crate::place::Place::Global && cwd.is_none() {
        false
      } else {
        let theirs = crate::place::start_dir(s.place_dir(&place), cwd.map(std::path::Path::new));
        matches!((&dir, theirs), (Some(a), Some(b)) if *a == same_dir(&b))
      }
    } else {
      match (cwd, &dir) {
        (Some(c), Some(d)) => std::path::Path::new(c) == d,
        (None, _) => root.repo().to_string() == repo && place != crate::place::Place::Global,
        (Some(_), None) => false,
      }
    };
    if same {
      let goal = record
        .fields
        .get("goal")
        .and_then(Value::as_str)
        .map(one_line);
      out.push((id, title.or(goal).unwrap_or_default()));
    }
  }
  out
}

/// `path` (`~` for the home directory) on this node as a place; an error
/// when it is not a directory here.
pub(crate) fn resolve_place(s: &Shared, path: &str) -> Result<Resolved> {
  use crate::place::Place;
  let me = s.state().node.to_string();
  let dir = crate::place::expand_home(path.trim());
  if !dir.is_dir() {
    return Err(Error::Config(format!(
      "`{path}` is not a directory on {me}: name a checkout or folder there"
    )));
  }
  let here = s.place_of_dir(Some(&dir.display().to_string()));
  let label = match &here {
    // A project seen for the first time is not listed yet: its checkout's name.
    (Place::Project(_), Some(co)) => co.name(),
    (p, _) => p.label(&me, &s.places()),
  };
  let repo = match &here.0 {
    Place::Project(repo) => repo.to_string(),
    _ => s
      .state()
      .home()
      .map(|h| h.repo().to_string())
      .unwrap_or_default(),
  };
  Ok(Resolved {
    here,
    dir,
    label,
    repo,
  })
}

/// A thread placed in `repo` that continues `c`'s conversation reads it
/// from its own repository: the two must be one.
fn same_root(s: &Arc<Shared>, c: &Continues, repo: &str) -> Result<()> {
  let (root, _) = s.thread_place(&c.thread)?;
  if root.repo().to_string() == repo {
    return Ok(());
  }
  Err(Error::Config(
    "a thread in another place cannot start from this conversation (it reads it from its own \
     place's records): leave context out and put what it needs in the goal"
      .into(),
  ))
}

/// What became of a message `send` sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sent {
  /// It starts a turn here (after the one running, if any).
  Turn,
  /// Woven into the turn running here (`interject`).
  Woven,
  /// Passed to the node that runs the thread, which does either.
  Passed,
}

impl Sent {
  /// As the tool's result says it, for an `interject`.
  pub(crate) fn said(self) -> &'static str {
    match self {
      Sent::Woven => "woven into its running turn",
      Sent::Turn => "no turn runs on it, so it starts a turn",
      Sent::Passed => {
        "passed on to that node, which weaves it into its running turn, or starts a turn when \
         none runs"
      }
    }
  }
}

/// Continues `thread` with `message`, on the node that owns it; its reply
/// is reported to `from`. With `interject`, a loop running on it there
/// takes it between its steps, and reports its last reply to `from` when
/// it ends; with none running it starts a turn, as without. That node's
/// name, and what became of the message.
pub(crate) async fn send(
  s: &Arc<Shared>,
  from: Option<&RecordId>,
  thread: &RecordId,
  message: &str,
  wake: bool,
  interject: bool,
) -> Result<(String, Sent)> {
  send_as(s, from, thread, message, wake, interject, None, &[]).await
}

/// `send`, the message written as from `label` (loose ends' auto-handling,
/// `crate::loose::FROM`) instead of the user or the sending thread, with
/// the threads and efforts `scopes` attached (design §2.27).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_as(
  s: &Arc<Shared>,
  from: Option<&RecordId>,
  thread: &RecordId,
  message: &str,
  wake: bool,
  interject: bool,
  label: Option<&str>,
  scopes: &[crate::api::ScopeTarget],
) -> Result<(String, Sent)> {
  if message.trim().is_empty() && scopes.is_empty() {
    return Err(Error::Config("message is empty".into()));
  }
  // A thread compacted into another goes on there.
  let thread = &crate::fork::current(s, thread);
  // A fixer gets no other thread to act for it (docs/self-improve.md §5).
  if let Some(f) = from {
    crate::selfimprove::may_send(s, f, thread)?;
  }
  // A room: its participants answer there (design §2.27); a round runs
  // on it here, or the one running takes it.
  if crate::rooms::is_room_here(s, thread) {
    let said = crate::rooms::Said {
      text: message.to_owned(),
      attachments: Vec::new(),
      scope: crate::scopes::scoped(s, scopes).await?,
      spoken: false,
      from: match (label, from) {
        (Some(l), _) => Some(l.to_owned()),
        (None, Some(f)) => Some(format!("thread {f}")),
        (None, None) => None,
      },
    };
    let how = crate::rooms::send(s, thread, said).await?;
    let sent = if how.starts_with("taken") {
      Sent::Woven
    } else {
      Sent::Turn
    };
    return Ok((s.state().node.to_string(), sent));
  }
  // The user typing says they read what was there (design §2.25).
  if from.is_none() && label.is_none() {
    crate::meta::read_on_send(s, thread);
  }
  let (owner, root) = whereabouts(s, thread).await?;
  if let Some(f) = from
    && wake
  {
    expect(s, f, thread);
  }
  let as_label = label;
  let label = match label {
    Some(l) => l.to_owned(),
    None => from.map_or_else(|| "user".to_owned(), |f| format!("thread {f}")),
  };
  let asker = chat::Asker {
    thread: from.cloned(),
    keep: !wake,
  };
  // Resolved here first, so a scope that names no thread is refused now
  // (the node that writes the message resolves it again from its targets).
  let scope = crate::scopes::scoped(s, scopes).await?;
  if owner != s.state().node.as_str() {
    forward(
      s, &root, &owner, thread, message, from, !wake, interject, None, as_label, scopes,
    )?;
    return Ok((owner, Sent::Passed));
  }
  if interject
    && crate::detached::interject_with(s, thread, message, &label, asker, Vec::new(), scope.clone())
  {
    return Ok((owner, Sent::Woven));
  }
  crate::detached::turn_with(
    s.clone(),
    thread.clone(),
    message.to_owned(),
    label,
    from.cloned(),
    Vec::new(),
    !wake,
    crate::detached::TurnOpts {
      scope,
      ..Default::default()
    },
  );
  Ok((owner, Sent::Turn))
}

/// A message for a thread that runs on another node (`owner`): an
/// assignment there, which it takes as its next turn, or weaves into the
/// turn running (`interject`); its reply is reported to `from`. `chat`:
/// the user typed it into the thread on this node (`elsewhere::chat`), so
/// its turn is a chat's there, followed live from here, with its files
/// and the model the client picked. The assignment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn forward(
  s: &Arc<Shared>,
  root: &stitch_store::RecordRoot,
  owner: &str,
  thread: &RecordId,
  message: &str,
  from: Option<&RecordId>,
  keep: bool,
  interject: bool,
  chat: Option<AsChat<'_>>,
  label: Option<&str>,
  scopes: &[crate::api::ScopeTarget],
) -> Result<RecordId> {
  let mut draft = Draft::new(kinds::ASSIGNMENT, Author::user(s.state().node.clone()))?
    .body(format!("# {}\n", one_line(message)))
    .field("node", owner)?
    .field("goal", message)?
    .field("thread", thread.as_str())?
    .field("continue", true)?
    // Every node reads the thread as working from it on (`turn_open`).
    .reference("thread", Ref::to(thread.clone()))?;
  if let Some(f) = from {
    draft = draft.reference("reports-to", Ref::to(f.clone()))?;
  }
  if keep {
    draft = draft.field("wake", false)?;
  }
  if interject {
    draft = draft.field("interject", true)?;
  }
  // Who it is from, when neither the user nor a thread (loose ends).
  if let Some(l) = label {
    draft = draft.field("from", l)?;
  }
  // Resolved where its message is written (`scopes_of`).
  if !scopes.is_empty() {
    draft = draft.field("scopes", crate::scopes::field(scopes))?;
  }
  if let Some(c) = chat {
    draft = draft.field("chat", true)?;
    if let Some(m) = c.model {
      draft = draft.field("model", m)?;
    }
    if c.spoken {
      draft = draft.field("spoken", true)?;
    }
    if let Some(p) = c.room_turn
      && let Some(t) = crate::rooms::turn_field(p)
    {
      draft = draft.field("room_turn", t)?;
    }
    for a in c.attachments {
      draft = draft.attach(a.clone());
    }
  }
  let id = publish(s, root, draft)?;
  let (s, o) = (s.clone(), owner.to_owned());
  tokio::spawn(async move { nudge(&s, &o, None).await });
  Ok(id)
}

/// A message the user typed into a thread on one node, for the node it
/// runs on (`forward`): its files, and the model the client picked.
#[derive(Clone, Copy, Default)]
pub(crate) struct AsChat<'a> {
  pub(crate) attachments: &'a [stitch_records::Attachment],
  pub(crate) model: Option<&'a str>,
  /// Said in a voice call (`spoken` on the assignment).
  pub(crate) spoken: bool,
  /// A room's messages relayed to this participant (`room_turn` on the
  /// assignment, design §2.27): written with the message there.
  pub(crate) room_turn: Option<&'a stitch_records::Part>,
}

/// Before a turn on a thread: its repository is synced with the peers, so
/// the turn continues from the latest step written anywhere instead of
/// forking the thread from an older one, and one continued elsewhere
/// since is run there. The turn waits on each peer a short while
/// (`catch_up_budget`: 1 second on the local network, 3 elsewhere, none
/// for one that failed in the last minute); a sync not done by then goes
/// on in the background, and what it brings is taken in when it ends. A
/// chat (`always` false) syncs only for a thread another node has written
/// to; a turn nobody typed always does, since this node cannot tell it
/// was continued elsewhere.
pub(crate) async fn catch_up(s: &Arc<Shared>, thread: &RecordId, always: bool) {
  catch_up_within(s, thread, always, Duration::ZERO).await
}

/// `catch_up` for a thread resuming as its node starts: while no peer is
/// reached (a machine just booted, its network not up yet), tried again
/// for `CATCH_UP_AT_START`, so what was written while the node was down
/// (an answer to the question the thread waited on) is here before the
/// thread goes on, and it does not ask again.
pub(crate) async fn catch_up_at_start(s: &Arc<Shared>, thread: &RecordId) {
  // Only a thread that waited on the user: others go on as before.
  let patience = if open_approvals(s, thread).is_empty() {
    Duration::ZERO
  } else {
    CATCH_UP_AT_START
  };
  catch_up_within(s, thread, true, patience).await
}

/// How long a thread resuming at start tries to reach a peer.
const CATCH_UP_AT_START: Duration = Duration::from_secs(30);

async fn catch_up_within(s: &Arc<Shared>, thread: &RecordId, always: bool, patience: Duration) {
  let me = s.state().node.to_string();
  let elsewhere = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .thread_segments(thread)
    .unwrap_or_default()
    .iter()
    .any(|(id, _, _)| id.node() != me);
  if !elsewhere && !always {
    return;
  }
  let Ok((root, _)) = s.thread_place(thread) else {
    return;
  };
  let state = s.state().clone();
  let peers = tokio::task::spawn_blocking(move || state.known_peers())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();
  let repo = root.repo().clone();
  if patience.is_zero() {
    let seen = s.state().seen();
    let waits = peers.iter().filter_map(|p| {
      let addr = p.addr().ok()?;
      let budget = catch_up_budget(p, seen.peer.get(&p.name), now_secs());
      let running = catching_up(s, p.name.clone(), addr, root.clone());
      Some(running.wait(budget))
    });
    futures_util::future::join_all(waits).await;
    let _ = s.state().index.lock().expect("index lock").refresh(&root);
    return;
  }
  // Waiting on the user's answer: peers are waited for as they always
  // were, and tried again until one is reached.
  let until = tokio::time::Instant::now() + patience;
  loop {
    let syncs = peers.iter().filter_map(|p| p.addr().ok()).map(|addr| {
      let (s, repo) = (s.clone(), repo.clone());
      async move {
        matches!(
          tokio::time::timeout(Duration::from_secs(5), s.net.sync_with(addr, &repo)).await,
          Ok(Ok(_))
        )
      }
    });
    let reached = futures_util::future::join_all(syncs).await;
    if reached.is_empty()
      || reached.iter().any(|r| *r)
      || tokio::time::Instant::now() + Duration::from_secs(2) >= until
    {
      break;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
  }
  let _ = s.state().index.lock().expect("index lock").refresh(&root);
}

/// How long a turn's start waits on a peer on the local network.
const CATCH_UP_LAN: Duration = Duration::from_secs(1);
/// How long it waits on one reached through a relay, a tailnet or the
/// internet.
const CATCH_UP_REMOTE: Duration = Duration::from_secs(3);
/// A peer that failed to be reached this recently is not waited for: its
/// catch-up runs in the background only.
const CATCH_UP_FAILED_WITHIN: Duration = Duration::from_secs(60);
/// A catch-up left to the background: the peer reached within this…
const CATCH_UP_CONNECT: Duration = Duration::from_secs(10);
/// …and the whole of it over within this, else given up (the next sync
/// brings it).
const CATCH_UP_BACKGROUND: Duration = Duration::from_secs(120);

fn now_secs() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

/// How long a turn's start waits on `peer` (what this node last knew of
/// it, `seen`): none when it failed in the last minute, 1 second when it
/// was last reached on the local network (or, never reached, lists a
/// local address), 3 otherwise.
fn catch_up_budget(peer: &stitch_net::Peer, seen: Option<&stitch_net::Seen>, now: u64) -> Duration {
  if let Some(failed) = seen.and_then(|s| s.failed)
    && now.saturating_sub(failed) < CATCH_UP_FAILED_WITHIN.as_secs()
  {
    return Duration::ZERO;
  }
  let lan = seen
    .and_then(|s| s.on_lan())
    .unwrap_or_else(|| peer.addrs.iter().any(|a| stitch_net::is_local_ip(a.ip())));
  if lan { CATCH_UP_LAN } else { CATCH_UP_REMOTE }
}

/// The catch-ups running, by peer and repository: a turn starting while
/// one runs waits on it rather than starting another.
pub(crate) type CatchUps =
  std::sync::Mutex<std::collections::HashMap<(String, String), CatchingUp>>;

/// A catch-up running in the background.
#[derive(Clone)]
pub(crate) struct CatchingUp {
  done: tokio::sync::watch::Receiver<bool>,
  /// A turn stopped waiting on it before it ended: what it brings is
  /// taken in when it does.
  late: Arc<std::sync::atomic::AtomicBool>,
}

impl CatchingUp {
  /// Waits for it to end, `budget` at most (it goes on regardless).
  async fn wait(self, budget: Duration) {
    let mut done = self.done;
    let ended = !budget.is_zero()
      && tokio::time::timeout(budget, done.wait_for(|d| *d))
        .await
        .is_ok_and(|r| r.is_ok());
    if !ended {
      self.late.store(true, std::sync::atomic::Ordering::SeqCst);
    }
  }
}

/// Starts `sync` in the background under `key` (unless one runs there
/// already: that one is returned), then `then` with its result and
/// whether a turn stopped waiting on it.
fn run_once<T, S, F, G>(
  running: &Arc<CatchUps>,
  key: (String, String),
  sync: S,
  then: F,
) -> CatchingUp
where
  T: Send + 'static,
  S: std::future::Future<Output = T> + Send + 'static,
  F: FnOnce(T, bool) -> G + Send + 'static,
  G: std::future::Future<Output = ()> + Send + 'static,
{
  let mut map = running.lock().expect("catch-ups lock");
  if let Some(c) = map.get(&key) {
    return c.clone();
  }
  let (tx, done) = tokio::sync::watch::channel(false);
  let c = CatchingUp {
    done,
    late: Default::default(),
  };
  map.insert(key.clone(), c.clone());
  drop(map);
  let (running, late) = (running.clone(), c.late.clone());
  tokio::spawn(async move {
    let out = sync.await;
    running.lock().expect("catch-ups lock").remove(&key);
    let _ = tx.send(true);
    then(out, late.load(std::sync::atomic::Ordering::SeqCst)).await;
  });
  c
}

/// `root`'s catch-up with peer `name`, at `addr`: what arrived is indexed
/// by the session itself; one a turn stopped waiting on is then taken in
/// as a peer's sync is (`after_sync`). One that could not reach the peer
/// says so (`stitch peer ls`), and the next turns do not wait on it.
fn catching_up(
  s: &Arc<Shared>,
  name: String,
  addr: iroh::EndpointAddr,
  root: stitch_store::RecordRoot,
) -> CatchingUp {
  let key = (name.clone(), root.repo().to_string());
  let s2 = s.clone();
  let repo = root.repo().clone();
  let sync = async move {
    tokio::time::timeout(
      CATCH_UP_BACKGROUND,
      s2.net.sync_within(addr, &repo, CATCH_UP_CONNECT),
    )
    .await
  };
  let s = s.clone();
  run_once(
    &s.detached.catch_ups.clone(),
    key,
    sync,
    move |out, late| async move {
      match out {
        Ok(Ok(report)) if late && !report.received.is_empty() => {
          let st = s.state().clone();
          let _ = tokio::task::spawn_blocking(move || {
            st.index.lock().expect("index lock").refresh(&root)
          })
          .await;
          after_sync(&s).await;
        }
        Ok(Err(stitch_net::AskError::Unreachable(why))) => s.state().note_failed(&name, &why),
        _ => {}
      }
    },
  )
}

/// Where a message for a thread goes: the node that runs it (`runner`)
/// and its repository. A thread just spawned is reachable before its
/// record is here: one about to start here (once the spawning step is
/// written) is waited for a little; one assigned to another node is
/// reached through its assignment (that node takes the message once the
/// thread runs there).
async fn whereabouts(
  s: &Arc<Shared>,
  thread: &RecordId,
) -> Result<(String, stitch_store::RecordRoot)> {
  let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
  loop {
    let record = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .record(thread)?
      .filter(|r| r.kind == kinds::THREAD);
    if let Some(r) = record {
      let (root, _) = s.thread_place(thread)?;
      return Ok((runner(s, thread, &r.node), root));
    }
    let starting = s
      .detached
      .starting
      .lock()
      .expect("starting lock")
      .contains_key(thread.as_str());
    if !starting {
      break;
    }
    if tokio::time::Instant::now() >= deadline {
      return Err(Error::Config(format!(
        "{thread} starts once the step that spawned it is written: send in your next step"
      )));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  let assigned = spawn_assignment(s, thread)?;
  if let Some(a) = assigned
    && let Some(node) = a.fields.get("node").and_then(Value::as_str)
    && let Some(root) = s.state().root(&a.repo)
  {
    return Ok((node.to_owned(), root));
  }
  Err(Error::Config(format!("no thread {thread} here")))
}

/// The original assignment, before its thread record has arrived as well as after.
pub(crate) fn spawn_assignment(
  s: &Shared,
  thread: &RecordId,
) -> Result<Option<stitch_store::IndexedRecord>> {
  Ok(
    s.state()
      .index
      .lock()
      .expect("index lock")
      .records(Some(kinds::ASSIGNMENT))?
      .into_iter()
      .find(|a| {
        a.fields.get("thread").and_then(Value::as_str) == Some(thread.as_str())
          && a.fields.get("continue").and_then(Value::as_bool) != Some(true)
      }),
  )
}

/// A cancellation of an unconfirmed start also names the assignment. It is
/// durable, so a late claim must not launch it, even after a node restarts.
pub(crate) fn assignment_cancelled(s: &Shared, assignment: &RecordId) -> bool {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "assignment", assignment, None)
    .is_ok_and(|reports| {
      reports
        .iter()
        .any(|r| r.fields.get("outcome").and_then(Value::as_str) == Some("cancelled"))
    })
}

/// How long a start may go unconfirmed (no thread record back from the node
/// it was given to) before lists stop counting it as working.
pub(crate) const UNCONFIRMED: std::time::Duration = std::time::Duration::from_secs(3600);

/// What became of a start not confirmed here (no thread record): its report
/// says `cancelled` (withdrawn) or `failed` (the node it went to could not
/// start it); `unconfirmed` once it has waited `UNCONFIRMED`; else none
/// (still starting, as far as anyone here knows).
pub(crate) fn start_outcome(s: &Shared, thread: &RecordId) -> Option<String> {
  let reports = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "thread", thread, None)
    .unwrap_or_default();
  for want in ["cancelled", "failed"] {
    if reports
      .iter()
      .any(|r| r.fields.get("outcome").and_then(Value::as_str) == Some(want))
    {
      return Some(want.into());
    }
  }
  let a = spawn_assignment(s, thread).ok().flatten()?;
  let waited = Timestamp::now().duration_since(a.created);
  (!waited.is_negative() && waited.unsigned_abs() >= UNCONFIRMED).then(|| "unconfirmed".into())
}

/// Why a start failed, as the node it went to said it (its `failed` report).
pub(crate) fn start_failure(s: &Shared, thread: &RecordId) -> Option<String> {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "thread", thread, None)
    .ok()?
    .into_iter()
    .filter(|r| r.fields.get("outcome").and_then(Value::as_str) == Some("failed"))
    .find_map(|r| {
      r.fields
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
    })
}

/// The node a thread was given to, by its spawn assignment.
fn assigned_node(s: &Shared, thread: &RecordId) -> Option<String> {
  spawn_assignment(s, thread)
    .ok()
    .flatten()?
    .fields
    .get("node")
    .and_then(Value::as_str)
    .map(str::to_owned)
}

/// Whether the node an assignment went to gave up starting it (its `failed`
/// report): a later claim must not try again.
pub(crate) fn assignment_failed(s: &Shared, assignment: &RecordId) -> bool {
  s.state()
    .index
    .lock()
    .expect("index lock")
    .sources(kinds::REPORT, "assignment", assignment, None)
    .is_ok_and(|reports| {
      reports
        .iter()
        .any(|r| r.fields.get("outcome").and_then(Value::as_str) == Some("failed"))
    })
}

/// The node a new thread's assignment went to could not start it, again and
/// again for `START_GIVE_UP`: a `failed` report to whoever spawned it, with
/// the error, so the thread ends there instead of "starting" for ever.
pub(crate) fn fail_start(
  s: &Arc<Shared>,
  root: &stitch_store::RecordRoot,
  a: &stitch_store::IndexedRecord,
  thread: &RecordId,
  error: &str,
) -> Result<()> {
  if assignment_failed(s, &a.id) || assignment_cancelled(s, &a.id) {
    return Ok(());
  }
  let assignment = root.read_record(&a.id)?;
  let node = s.state().node.clone();
  let mut draft = Draft::new(kinds::REPORT, Author::node(node.clone()))?
    .field("final", true)?
    .field("outcome", "failed")?
    .field("error", error)?
    .reference("thread", Ref::to(thread.clone()))?
    .reference("assignment", Ref::to(a.id.clone()))?
    .body(format!(
      "# {}: failed\n\nNot started: {node} could not start this thread: {error}\n\nNothing ran. \
       Spawn it again once that is fixed.\n",
      assignment.title().unwrap_or("thread")
    ));
  if let Some(parent) = assignment.refs.first("reports-to") {
    draft = draft.reference("reports-to", parent.clone())?;
  }
  if assignment.field_bool("wake") == Some(false) {
    draft = draft.field("wake", false)?;
  }
  publish(s, root, draft)?;
  deliver(s);
  Ok(())
}

pub(crate) fn cancel_pending(s: &Arc<Shared>, thread: &RecordId, text: &str) -> Result<String> {
  let a = spawn_assignment(s, thread)?
    .ok_or_else(|| Error::Config(format!("no thread {thread} here")))?;
  let node = a
    .fields
    .get("node")
    .and_then(Value::as_str)
    .ok_or_else(|| Error::Config("assignment has no node".into()))?
    .to_owned();
  let root = s
    .state()
    .root(&a.repo)
    .ok_or_else(|| Error::Config(format!("repository {} is not held here", a.repo)))?;
  if !assignment_cancelled(s, &a.id) {
    let assignment = root.read_record(&a.id)?;
    let mut draft = Draft::new(kinds::REPORT, Author::node(s.state().node.clone()))?
      .field("final", true)?.field("outcome", "cancelled")?
      .reference("thread", Ref::to(thread.clone()))?
      .reference("assignment", Ref::to(a.id.clone()))?
      .body(format!("# {}: cancelled\n\n{text}\n\nAssignment withdrawn before its start was confirmed. If it already started on {node}, it stops when this cancellation arrives.\n", assignment.title().unwrap_or("thread")));
    if let Some(parent) = assignment.refs.first("reports-to") {
      draft = draft.reference("reports-to", parent.clone())?;
    }
    if assignment.field_bool("wake") == Some(false) {
      draft = draft.field("wake", false)?;
    }
    publish(s, &root, draft)?;
    crate::detached::stop_reported(s);
    s.detached.reported.notify_waiters();
    deliver(s);
    if let Some(parent) = assignment.refs.first("reports-to") {
      let owner = {
        let index = s.state().index.lock().expect("index lock");
        index
          .record(&parent.id)?
          .map(|r| runner_in(&index, &parent.id, &r.node))
      };
      if let Some(owner) = owner
        && owner != s.state().node.as_str()
        && owner != node
      {
        let s = s.clone();
        tokio::spawn(async move { nudge(&s, &owner, None).await });
      }
    }
  }
  let (s, target) = (s.clone(), node.clone());
  tokio::spawn(async move { nudge(&s, &target, None).await });
  Ok(node)
}

/// The node that runs a thread's turns nobody typed (a message sent to it,
/// reports): the one that continued it last, as far as this node knows
/// (the writer of its latest segment), else the one that started it.
pub(crate) fn runner(s: &Shared, thread: &RecordId, started_on: &str) -> String {
  runner_in(
    &s.state().index.lock().expect("index lock"),
    thread,
    started_on,
  )
}

/// `runner`, with the index at hand.
pub(crate) fn runner_in(
  index: &stitch_store::Index,
  thread: &RecordId,
  started_on: &str,
) -> String {
  let line = crate::rewind::line_in(index, thread);
  // Rewound, nothing after it yet: the node it was rewound on goes on.
  if let Some(r) = &line.open
    && run_choice(index, thread).is_none()
  {
    return r.node.clone();
  }
  let head = line.head;
  match run_choice(index, thread) {
    // The user had it run there (design §2.7, "Continue on another node").
    Some(RunsOn { node: Some(n), .. }) => n,
    // Brought back to `by` ("Continue here"): it runs there until another
    // node continues it.
    Some(RunsOn { node: None, by, at }) => match head {
      Some(h) if h.timestamp() > at => h.node().to_owned(),
      _ => by,
    },
    None => head.map_or_else(|| started_on.to_owned(), |h| h.node().to_owned()),
  }
}

/// The user's choice of where a thread runs (design §2.7, "Continue on
/// another node"): the latest user's `set` of `runs-on`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunsOn {
  /// The node its turns run on, whichever node they are typed into;
  /// `None`: brought back ("Continue here"), chats run where typed again.
  pub(crate) node: Option<String>,
  /// The node that wrote the choice.
  pub(crate) by: String,
  pub(crate) at: jiff::Timestamp,
}

/// The field of a thread's `set` records that says where it runs.
pub(crate) const RUNS_ON: &str = "runs-on";

pub(crate) fn run_choice(index: &stitch_store::Index, thread: &RecordId) -> Option<RunsOn> {
  let set = index.current_user_value(thread, RUNS_ON).ok().flatten()?;
  Some(RunsOn {
    node: set
      .value
      .as_str()
      .filter(|n| !n.is_empty())
      .map(str::to_owned),
    by: set.record.node().to_owned(),
    at: set.record.timestamp(),
  })
}

/// The node a thread was set to run on, when it is another than this one:
/// a chat typed into it here goes there (`elsewhere::chat`).
pub(crate) fn pinned_elsewhere(s: &Shared, thread: &RecordId) -> Option<String> {
  let state = s.state();
  let node = run_choice(&state.index.lock().expect("index lock"), thread)?.node?;
  (node != state.node.as_str()).then_some(node)
}

/// The node that owns a thread the user chose no node for, when it is
/// another than this one: where the user's chats go by default when typed
/// into the web client or the app (`Peer::owner_default`; the TUI's run
/// where typed). "Run here" is a choice (`runs-on` = ""), so it is not this.
pub(crate) fn owner_elsewhere(s: &Shared, thread: &RecordId) -> Option<String> {
  let state = s.state();
  let index = state.index.lock().expect("index lock");
  owner_elsewhere_in(&index, thread, state.node.as_str())
}

/// `owner_elsewhere` from an index, for the node named `me` (the app reads
/// it to say where an unpinned thread's turns run).
pub fn owner_elsewhere_in(
  index: &stitch_store::Index,
  thread: &RecordId,
  me: &str,
) -> Option<String> {
  if run_choice(index, thread).is_some() {
    return None;
  }
  let started = index.record(thread).ok().flatten()?.node.to_string();
  let owner = runner_in(index, thread, &started);
  (owner != me).then_some(owner)
}

/// Has a thread run on `node` from now on, whichever node its messages
/// are typed into (`Some`), or brings it back (`None`, "Continue here"):
/// chats run where they are typed again, and what nobody types runs here
/// until another node continues it. The user's call (a `set` of
/// `runs-on`); what is said back.
pub(crate) async fn run_on(
  s: &Arc<Shared>,
  thread: &RecordId,
  node: Option<&str>,
) -> Result<String> {
  let thread = &crate::fork::current(s, thread);
  let me = s.state().node.to_string();
  let node = node.filter(|n| !n.is_empty() && *n != "here");
  // Only a node that runs programs (shells) runs a thread for the user: a
  // phone would run its turns with no shells. Asked of it, as `models`
  // and places are; one not reached is taken at its word later.
  let mut unasked = false;
  if let Some(n) = node {
    known(s, n)?;
    match crate::elsewhere::runs_programs(s, n).await {
      Ok(Some(())) => {}
      Ok(None) => unasked = true,
      Err(why) => {
        return Err(Error::Config(format!(
          "thread {thread} cannot run on {n}: {why}. Pick a node with shells (`stitch status` \
           lists your nodes)."
        )));
      }
    }
  }
  let (root, _) = s.thread_place(thread)?;
  crate::meta::set(
    s,
    thread,
    RUNS_ON,
    toml::Value::String(node.unwrap_or("").to_owned()),
    Author::user(s.state().node.clone()),
  )?;
  // Every node knows at once, the one named first.
  let repo = root.repo().clone();
  {
    let (s, repo, node) = (s.clone(), repo.clone(), node.map(str::to_owned));
    tokio::spawn(async move {
      if let Some(n) = node.filter(|n| *n != s.state().node.as_str()) {
        reach(&s, &n, &repo).await;
      }
      crate::node::sync_one(&s, &repo).await;
    });
  }
  let unasked = match node {
    Some(n) if unasked => format!(
      " ({n} was not reached to ask whether it runs programs (shells); if it runs none, bring \
       the thread back)"
    ),
    _ => String::new(),
  };
  let said = match node {
    Some(n) if n == me => format!(
      "Thread {thread} runs on {n} (this node): messages typed into it on any node run here, and \
       those nodes follow its turns live. `/run-on here` there (Continue here) takes it back."
    ),
    Some(n) => format!(
      "Thread {thread} now runs on {n}: what you type into it here goes there, and its turns \
       stream here as they run (on {n}'s shells, files and tools). `/run-on here` (Continue \
       here) brings it back."
    ),
    None => format!(
      "Thread {thread} runs here again: messages run on the node they are typed into, and what \
       nobody types (reports, messages) runs on {me}."
    ),
  };
  Ok(format!("{said}{unasked}"))
}

fn publish(s: &Shared, root: &stitch_store::RecordRoot, draft: Draft) -> Result<RecordId> {
  let record = root.publish(&s.state().node, draft)?;
  s.state().index.lock().expect("index lock").refresh(root)?;
  Ok(record.id)
}

/// `parent` now waits on a report from `child`, asked for now.
fn expect(s: &Shared, parent: &RecordId, child: &RecordId) {
  s.detached
    .outstanding
    .lock()
    .expect("outstanding lock")
    .entry(parent.to_string())
    .or_default()
    .push((child.to_string(), Timestamp::now()));
}

fn one_line(s: &str) -> String {
  let line = s.lines().next().unwrap_or("").trim();
  let mut t: String = line.chars().take(80).collect();
  if line.chars().count() > 80 || s.lines().count() > 1 {
    t.push('…');
  }
  t
}

async fn wait_for_step(s: &Arc<Shared>, thread: &RecordId, call: &str) {
  let until = tokio::time::Instant::now() + STEP_WAIT;
  while !step_written(s, thread, call) && tokio::time::Instant::now() < until {
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
}

/// Lets `node` know now: syncs with it (once `step`, a tool call of a
/// thread, is written), or claims here when it is this node.
pub(crate) async fn nudge(s: &Arc<Shared>, node: &str, step: Option<(RecordId, String)>) {
  if let Some((thread, call)) = step {
    wait_for_step(s, &thread, &call).await;
  }
  if node == s.state().node.as_str() {
    claim_all(s).await;
  } else {
    crate::node::sync_all(s, Some(node)).await;
  }
}

/// Syncs `repo` with peer `node` now (5 seconds at most): whether it was
/// reached. This node itself is always reached.
pub(crate) async fn reach(s: &Arc<Shared>, node: &str, repo: &stitch_records::RepoId) -> bool {
  if node == s.state().node.as_str() {
    return true;
  }
  let state = s.state().clone();
  let peer = tokio::task::spawn_blocking(move || state.known_peers())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default()
    .into_iter()
    .find(|p| p.name == node);
  let Some(addr) = peer.and_then(|p| p.addr().ok()) else {
    return false;
  };
  matches!(
    tokio::time::timeout(Duration::from_secs(5), s.net.sync_with(addr, repo)).await,
    Ok(Ok(_))
  )
}

/// Whether tool call `call` of `thread` is written.
pub(crate) fn step_written(s: &Shared, thread: &RecordId, call: &str) -> bool {
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
  chat::thread_events(&ctx, thread)
    .map(|events| {
      events
        .iter()
        .any(|e| matches!(e, Event::ToolCall(c) if c.id == call))
    })
    .unwrap_or(false)
}

/// Takes the assignments for this node it has not taken: new threads,
/// once the conversation they continue is here, and messages.
pub(crate) async fn claim_all(s: &Arc<Shared>) {
  let state = s.state();
  let me = state.node.to_string();
  let mine: Vec<stitch_store::IndexedRecord> = {
    let index = state.index.lock().expect("index lock");
    index
      .records(Some(kinds::ASSIGNMENT))
      .unwrap_or_default()
      .into_iter()
      .filter(|a| a.fields.get("node").and_then(Value::as_str) == Some(me.as_str()))
      .filter(|a| {
        // Taken: a thread started for it, or a claim on it.
        index
          .sources(kinds::THREAD, "assignment", &a.id, None)
          .is_ok_and(|t| t.is_empty())
          && index
            .sources(kinds::CLAIM, "claims", &a.id, None)
            .is_ok_and(|c| c.is_empty())
      })
      .collect()
  };
  // New threads first: a message sent to one before it was here (in the
  // same sync) is taken once it runs.
  let mut mine = mine;
  mine.sort_by_key(|a| a.fields.get("continue").and_then(Value::as_bool) == Some(true));
  for a in mine {
    if s
      .detached
      .claimed
      .lock()
      .expect("claimed lock")
      .contains(a.id.as_str())
    {
      continue;
    }
    if let Err(e) = claim(s, &a).await {
      eprintln!("assignment {}: {e}", a.id);
    }
  }
}

/// Takes one assignment, unless what it needs is not here yet.
async fn claim(s: &Arc<Shared>, a: &stitch_store::IndexedRecord) -> Result<()> {
  let state = s.state();
  let Some(root) = state.root(&a.repo) else {
    return Ok(());
  };
  let record = root.read_record(&a.id)?;
  let to = record.refs.first("reports-to").map(|r| r.id.clone());
  let goal = record.field_str("goal").unwrap_or("").to_owned();
  let Some(thread) = record
    .field_str("thread")
    .and_then(|t| t.parse::<RecordId>().ok())
  else {
    return Err(Error::Config("an assignment without its thread".into()));
  };
  if record.field_bool("continue") == Some(true) {
    // A message for a thread here.
    if state
      .index
      .lock()
      .expect("index lock")
      .record(&thread)?
      .is_none()
    {
      return Ok(());
    }
    if !s
      .detached
      .claimed
      .lock()
      .expect("claimed lock")
      .insert(a.id.to_string())
    {
      return Ok(());
    }
    let claim = Draft::new(kinds::CLAIM, Author::node(state.node.clone()))?
      .reference("claims", Ref::to(a.id.clone()))?
      .body(format!("# taken by {}\n", state.node));
    publish(s, &root, claim)?;
    let label = match record.field_str("from") {
      Some(f) if f == crate::loose::FROM => f.to_owned(),
      _ => to
        .as_ref()
        .map_or_else(|| format!("user@{}", a.node), |t| format!("thread {t}")),
    };
    let keep = record.field_bool("wake") == Some(false);
    // Into the loop running on it, when sent to interject.
    let asker = chat::Asker {
      thread: to.clone(),
      keep,
    };
    // Typed into the thread on another node, which follows it live
    // (design §2.7, "Continue on another node").
    let chat = record.field_bool("chat") == Some(true);
    // Its files, here before its turn reads them (they sync lazily).
    let attachments = record.attachments.clone();
    let missing = chat::missing_blobs(&state.blobs, &attachments);
    if !missing.is_empty() {
      crate::node::fetch_blobs(s, &missing).await;
    }
    let scope = crate::scopes::of_record(s, &record).await;
    if record.field_bool("interject") == Some(true)
      && crate::detached::interject_with(
        s,
        &thread,
        &goal,
        &label,
        asker,
        attachments.clone(),
        scope.clone(),
      )
    {
      if chat {
        s.live.serve(thread.as_str(), a.id.as_str());
      }
      return Ok(());
    }
    crate::detached::turn_with(
      s.clone(),
      thread,
      goal,
      label,
      to,
      Vec::new(),
      keep,
      crate::detached::TurnOpts {
        chat: chat.then(|| a.id.clone()),
        attachments,
        // The model its client picked, when this node runs it.
        model: record
          .field_str("model")
          .filter(|_| chat)
          .map(str::to_owned),
        // Said in a voice call there: its reply is read aloud.
        spoken: chat && record.field_bool("spoken") == Some(true),
        scope,
        // Relayed from a room by the node that runs it (design §2.27).
        room_turn: record
          .fields
          .get("room_turn")
          .filter(|_| chat)
          .and_then(crate::rooms::turn_of_field),
        ..Default::default()
      },
    );
    return Ok(());
  }
  if assignment_cancelled(s, &a.id) || assignment_failed(s, &a.id) {
    return Ok(());
  }
  let continues = Continues::of(&record);
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  if let Some(c) = &continues
    && chat::inherited_from_ready(&ctx, c).is_err()
  {
    // Not synced yet: the next sync brings it.
    return Ok(());
  }
  if state
    .index
    .lock()
    .expect("index lock")
    .record(&thread)?
    .is_some()
  {
    return Ok(());
  }
  let near = continues
    .as_ref()
    .map(|c| c.thread.clone())
    .or_else(|| to.clone());
  let at = near.as_ref().map(|t| s.thread_place(t)).transpose()?;
  // The place it was spawned for, as it is here now; gone since it was
  // asked (or never asked: its spawner could not reach this node), it
  // works where it would have without, and is told so.
  let mut goal = goal;
  let (at, cwd) = match record.field_str("place") {
    Some(p) => {
      let placed = resolve_place(s, p).and_then(|r| {
        if let Some(c) = &continues {
          same_root(s, c, &r.repo)?;
        }
        let dir = r.dir.display().to_string();
        Ok((s.placement(None, false, &r.here)?, dir))
      });
      match placed {
        Ok((at, dir)) => (Some(at), Some(dir)),
        Err(e) => {
          goal = format!(
            "[Stitch: this thread was spawned to work in `{p}`, but {e}. It works in its \
             spawner's place instead: say so in your report if the goal needs that one.]\n\n{goal}"
          );
          (at, None)
        }
      }
    }
    None => (at, None),
  };
  if !s
    .detached
    .claimed
    .lock()
    .expect("claimed lock")
    .insert(a.id.to_string())
  {
    return Ok(());
  }
  let fixer = crate::selfimprove::Ticket::read(&record).is_some()
    || to
      .as_ref()
      .is_some_and(|p| crate::selfimprove::in_fixer_mode(s, p));
  let spec = crate::detached::Spec {
    goal,
    // As named (`quick`, `default`), resolved here; else the model itself.
    model: record
      .field_str("model_as")
      .or_else(|| record.field_str("model"))
      .map(str::to_owned),
    repo: None,
    cwd,
    parent: to,
    budget: Limits::from_json(a.fields.get("budget")),
    global: false,
    at,
    assignment: Some(a.id.clone()),
    continues,
    id: Some(thread.clone()),
    system: None,
    no_tools: record.field_bool("tools") == Some(false),
    tools: Vec::new(),
    skills: Vec::new(),
    keep: record.field_bool("wake") == Some(false),
    remember: record.field_bool("memory") == Some(true),
    // An assignment from before `recall` (no field): out of memory
    // altogether, as it was written.
    recall: record.field_bool("memory") == Some(true) || record.field_bool("recall") == Some(true),
    // The mode it was spawned with (the assignment's): the thread is not
    // written yet, so no switch of the user's applies to it.
    read_only: record.field_str("approval") == Some("read-only"),
    scheduled: None,
    fixer,
    app: None,
    attachments: Vec::new(),
    scope: crate::scopes::of_record(s, &record).await,
    profile: None,
  };
  // A fixer of Stitch: admitted here, the node that runs it (its daily
  // cap, one at a time, the issue's attempts), or not started at all.
  let fix = crate::selfimprove::Ticket::read(&record);
  if let Some(t) = &fix {
    let refused = if crate::issues::find(s, t.issue.as_str()).is_err() {
      // Its issue not synced here yet: the next pass, once it is.
      Some(None)
    } else {
      crate::selfimprove::admit(s, t, &thread)
        .await
        .err()
        .map(Some)
    };
    match refused {
      None => {}
      Some(None) => {
        s.detached
          .claimed
          .lock()
          .expect("claimed lock")
          .remove(a.id.as_str());
        return Ok(());
      }
      Some(Some(e)) => {
        fail_start(s, &root, a, &thread, &e.to_string())?;
        return Err(e);
      }
    }
  }
  if let Err(e) = crate::detached::start(s, spec).await {
    if let Some(t) = &fix {
      crate::selfimprove::release(s, t, &thread, &e.to_string());
    }
    // Tried again at the next pass (a provider's token not refreshed yet,
    // a place being cloned); failing for `START_GIVE_UP`, it is given up,
    // its spawner told why, rather than "starting" there for ever.
    let first = {
      let mut failures = s.detached.start_failures.lock().expect("failures lock");
      let entry = failures
        .entry(a.id.to_string())
        .or_insert((Timestamp::now(), 0));
      entry.1 += 1;
      *entry
    };
    if give_up_start(first.0, first.1, Timestamp::now()) {
      fail_start(s, &root, a, &thread, &e.to_string())?;
      s.detached
        .start_failures
        .lock()
        .expect("failures lock")
        .remove(a.id.as_str());
      return Err(Error::Config(format!(
        "{e}; given up after {} tries, its spawner told",
        first.1
      )));
    }
    s.detached
      .claimed
      .lock()
      .expect("claimed lock")
      .remove(a.id.as_str());
    return Err(e);
  }
  s.detached
    .start_failures
    .lock()
    .expect("failures lock")
    .remove(a.id.as_str());
  Ok(())
}

/// How long a new thread's start may keep failing here, and in how many
/// tries at least, before it is given up (`fail_start`).
pub(crate) const START_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(600);
const START_GIVE_UP_TRIES: u32 = 3;

/// Whether a start failing since `first`, `tries` times, is given up.
fn give_up_start(first: Timestamp, tries: u32, now: Timestamp) -> bool {
  let failing = now.duration_since(first);
  tries >= START_GIVE_UP_TRIES && !failing.is_negative() && failing.unsigned_abs() >= START_GIVE_UP
}

/// After records arrived by sync: memory folds them, approvals waiting
/// here look again, detached threads cancelled elsewhere stop,
/// assignments for this node are taken, and reports for threads here are
/// delivered.
pub(crate) async fn after_sync(s: &Arc<Shared>) {
  // Threads the user deleted elsewhere go first: nothing below sees them.
  crate::delete::honour(s).await;
  // Turns here on threads the user rewound elsewhere since they started.
  crate::rewind::stop_overtaken(s);
  // Apps' state another node wrote: open pages look again.
  s.apps.bump();
  // Secrets sealed for this node opened, removed ones dropped, held ones
  // sealed for nodes paired since; variables read again.
  crate::secrets::after_sync(s).await;
  let (m, st) = (s.memory.clone(), s.state().clone());
  let _ = tokio::task::spawn_blocking(move || m.refresh(&st)).await;
  s.detached.answered.notify_waiters();
  s.detached.asked.notify_waiters();
  crate::detached::stop_reported(s);
  // Threads the user switched to read-only on another node.
  s.close_switched().await;
  claim_all(s).await;
  crate::remote::claim_calls(s);
  deliver(s);
  // Schedules made or changed elsewhere, pinned here.
  s.schedules.changed();
  // A phone's clones of projects, fetched (in the background).
  crate::clones::after_sync(s);
  // Keys asked to be authorized on hosts this node reaches (in the
  // background: a host may be slow to answer).
  {
    let s = s.clone();
    tokio::spawn(async move { crate::ssh::after_sync(&s).await });
  }
  // Folders' changes from other nodes, merged in where no turn runs.
  for (key, p) in crate::folders::pass_all(s).await {
    if p.applied > 0 || !p.conflicts.is_empty() {
      eprintln!(
        "folder {key}: {} file(s) from other nodes{}",
        p.applied,
        if p.conflicts.is_empty() {
          String::new()
        } else {
          format!(", kept both of {}", p.conflicts.join(", "))
        }
      );
    }
  }
}

/// A report delivered this long after it was written says so.
const LATE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// A report as its thread (`to`) reads it: the report's ID first (how it is
/// known as delivered, `has_report`), then the node and the thread it is
/// from, with that thread's title, so it can be sent to (`send`), and who
/// started that thread. A report delivered long after it was written says
/// when it was written and that it was held.
fn report_message(
  s: &Shared,
  report: &stitch_store::IndexedRecord,
  to: Option<&RecordId>,
) -> String {
  let record = s
    .state()
    .root(&report.repo)
    .and_then(|root| root.read_record(&report.id).ok());
  let body = record
    .as_ref()
    .map(|r| r.body.trim().to_owned())
    .unwrap_or_default();
  let late = late_note(report.created, Timestamp::now());
  // A queued call's result names the call.
  if let Some(call) = record.as_ref().and_then(|r| r.refs.first("call")) {
    let what = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .record(&call.id)
      .ok()
      .flatten()
      .map(|c| {
        let f = |k: &str| {
          c.fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
        };
        format!("{}__{}", f("server"), f("tool"))
      })
      .unwrap_or_default();
    return format!(
      "[Report {} from {}: queued call {what} ({})]{late}\n{body}",
      report.id, report.node, call.id
    );
  }
  let from = match record.as_ref().and_then(|r| r.refs.first("thread")) {
    Some(t) => {
      let (title, starter) = {
        let index = s.state().index.lock().expect("index lock");
        let thread = index.record(&t.id).ok().flatten();
        let starter = index
          .edges_from(&t.id)
          .ok()
          .and_then(|edges| edges.into_iter().find(|e| e.rel == "spawned-by"))
          .map(|e| {
            let title = index.record(&e.dst.id).ok().flatten().and_then(|r| r.title);
            (e.dst.id, title)
          });
        (thread.and_then(|r| r.title), starter)
      };
      let started = match starter {
        Some((p, _)) if Some(&p) == to => ", started by this thread".to_owned(),
        Some((p, Some(title))) => format!(", started by thread {p} \"{title}\""),
        Some((p, None)) => format!(", started by thread {p}"),
        None => ", started by the user".to_owned(),
      };
      match title {
        Some(title) => format!(": thread {} \"{title}\"{started}", t.id),
        None => format!(": thread {}{started}", t.id),
      }
    }
    None => String::new(),
  };
  format!(
    "[Report {} from {}{from}]{late}\n{body}",
    report.id, report.node
  )
}

/// Reports as `deliver` tells them to `to`, together: what a wake says when
/// it carries only some of the reports it gathered.
pub(crate) fn reports_message(s: &Shared, to: &RecordId, reports: &[RecordId]) -> String {
  reports
    .iter()
    .filter_map(|r| {
      let report = s
        .state()
        .index
        .lock()
        .expect("index lock")
        .record(r)
        .ok()??;
      Some(report_message(s, &report, Some(to)))
    })
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// What a report delivered `LATE` or later says of it: when it was written,
/// how long ago, and that it was held until now.
fn late_note(written: Timestamp, now: Timestamp) -> String {
  let held = now.duration_since(written);
  if held.is_negative() || held.unsigned_abs() < LATE {
    return String::new();
  }
  let at = written
    .to_zoned(jiff::tz::TimeZone::system())
    .strftime("%Y-%m-%d %H:%M %Z");
  format!(
    "\n[Late: written {at}, {} ago, and held until now: what it says may be out of date.]",
    crate::jobs::spoken(held.unsigned_abs())
  )
}

/// Whether `thread` already has `report` (or, from before reports carried
/// their ID, a report from `child`).
fn has_report(s: &Shared, thread: &RecordId, report: &RecordId, child: Option<&RecordId>) -> bool {
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
  let Ok(events) = chat::thread_events(&ctx, thread) else {
    return false;
  };
  let texts = events.iter().filter_map(|e| match e {
    Event::Msg(m) => Some(
      m.parts
        .iter()
        .filter_map(|p| match p {
          Part::Text { text } => Some(text.as_str()),
          _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n"),
    ),
    Event::ToolResult(r) => r
      .output
      .get("text")
      .and_then(Value::as_str)
      .map(str::to_owned),
    _ => None,
  });
  let mut texts = texts;
  texts.any(|t| {
    t.contains(&format!("[Report {report}"))
      || child.is_some_and(|c| {
        t.contains("[Report from the detached thread") && t.contains(&format!("({c})"))
      })
  })
}

/// The reports for threads this node owns that they have not had: a
/// thread waiting is told, the others are woken with them.
pub(crate) fn deliver(s: &Arc<Shared>) {
  let me = s.state().node.to_string();
  let due: Vec<(RecordId, stitch_store::IndexedRecord)> = {
    let index = s.state().index.lock().expect("index lock");
    let delivered = s.detached.delivered.lock().expect("delivered lock");
    index
      .records(Some(kinds::THREAD))
      .unwrap_or_default()
      .into_iter()
      .filter(|t| runner_in(&index, &t.id, &t.node) == me)
      .flat_map(|t| {
        index
          .sources(kinds::REPORT, "reports-to", &t.id, None)
          .unwrap_or_default()
          .into_iter()
          .filter(|r| !delivered.contains(r.id.as_str()))
          .map(move |r| (t.id.clone(), r))
          .collect::<Vec<_>>()
      })
      .collect()
  };
  let mut woke = false;
  // By the thread they go to, in order: what came together wakes it once.
  let mut to_tell: Vec<(RecordId, Vec<String>, Vec<RecordId>)> = Vec::new();
  for (thread, report) in due {
    let child: Option<RecordId> = s
      .state()
      .index
      .lock()
      .expect("index lock")
      .edges_from(&report.id)
      .ok()
      .and_then(|edges| {
        edges
          .into_iter()
          .find(|e| e.rel == "thread")
          .map(|e| e.dst.id)
      });
    // Had already, nothing new in it (its thread said so), or kept for the
    // user (`wake = false`): not told.
    let quiet = report.fields.get("news").and_then(Value::as_bool) == Some(false)
      || report.fields.get("wake").and_then(Value::as_bool) == Some(false);
    // Had by the thread or by one compacted from it since (where reports
    // to it go): not delivered again.
    let had = || {
      crate::fork::lineage(s, &thread)
        .iter()
        .any(|t| has_report(s, t, &report.id, child.as_ref()))
    };
    if quiet || had() {
      s.detached
        .delivered
        .lock()
        .expect("delivered lock")
        .insert(report.id.to_string());
      continue;
    }
    if s
      .detached
      .waiting
      .lock()
      .expect("waiting lock")
      .contains(thread.as_str())
    {
      woke = true;
      continue;
    }
    let message = report_message(s, &report, Some(&thread));
    match to_tell.iter_mut().find(|(t, ..)| *t == thread) {
      Some((_, texts, ids)) => {
        texts.push(message);
        ids.push(report.id.clone());
      }
      None => to_tell.push((thread, vec![message], vec![report.id.clone()])),
    }
  }
  // Into its loop when one runs, else it wakes it.
  for (thread, texts, ids) in to_tell {
    crate::detached::notify(s, &thread, texts.join("\n\n"), ids);
  }
  if woke {
    s.detached.reported.notify_waiters();
  }
}

/// Open approval questions of a thread: (question, what it asks and
/// where).
fn blocked_on_user(s: &Shared, thread: &RecordId) -> Vec<(String, String)> {
  open_approvals(s, thread)
    .into_iter()
    .map(|q| {
      (
        q.id.to_string(),
        format!(
          "waiting for the user's approval to run {}",
          approval_said(s, thread, &q)
        ),
      )
    })
    .collect()
}

/// A thread's approval questions nobody answered yet.
fn open_approvals(s: &Shared, thread: &RecordId) -> Vec<stitch_store::IndexedRecord> {
  let index = s.state().index.lock().expect("index lock");
  index
    .sources(kinds::QUESTION, "thread", thread, None)
    .unwrap_or_default()
    .into_iter()
    .filter(|q| q.fields.get("approval").and_then(Value::as_bool) == Some(true))
    .filter(|q| {
      index
        .sources(kinds::ANSWER, "answers", &q.id, Some(""))
        .is_ok_and(|a| a.is_empty())
    })
    .collect()
}

/// What an approval question asks, exactly, and where: the command or
/// call (its first 600 characters), the session, the question, the node
/// that asks and the thread's place.
fn approval_said(s: &Shared, thread: &RecordId, q: &stitch_store::IndexedRecord) -> String {
  let field = |k: &str| q.fields.get(k).and_then(Value::as_str).unwrap_or("");
  let command = field("command");
  let short: String = command.chars().take(600).collect();
  let more = if command.chars().count() > 600 {
    "…"
  } else {
    ""
  };
  let session = match field("session") {
    "" => String::new(),
    s if s.starts_with("mcp:") || s.starts_with("otter") => format!("; {s}"),
    s => format!("; in session \"{s}\""),
  };
  let place = s
    .thread_place(thread)
    .map(|(_, p)| format!(", in {}", p.label(s.state().node.as_str(), &s.places())))
    .unwrap_or_default();
  format!(
    "`{short}{more}` (question {}, asked on {}{place}{session})",
    q.id, q.node
  )
}

/// Tells the thread that started `thread` (its `reports-to`) that it is
/// blocked on approval `question`, woven into the loop running on it here
/// (never waking it: the question is on the user's attention list and
/// pushed to their phone already). A thread in `wait` is told by its
/// result instead; one told here is not told again there.
pub(crate) fn tell_blocked(s: &Arc<Shared>, thread: &RecordId, question: &RecordId) {
  let (parent, q, title) = {
    let index = s.state().index.lock().expect("index lock");
    let parent = index.edges_from(thread).ok().and_then(|edges| {
      edges
        .into_iter()
        .find(|e| e.rel == "reports-to")
        .map(|e| e.dst.id)
    });
    let q = index.record(question).ok().flatten();
    let title = index.title(thread).ok().flatten().unwrap_or_default();
    (parent, q, title)
  };
  let (Some(parent), Some(q)) = (parent, q) else {
    return;
  };
  if s
    .detached
    .waiting
    .lock()
    .expect("waiting lock")
    .contains(parent.as_str())
  {
    return;
  }
  let key = format!("{parent} {question}");
  if !s
    .detached
    .told_blocked
    .lock()
    .expect("told lock")
    .insert(key.clone())
  {
    return;
  }
  let text = format!(
    "[Thread {thread} \"{title}\", which this thread started, is blocked: it waits for the \
     user's approval to run {}. Only the user can answer it (their attention list: Ctrl+T in \
     the TUI, `stitch attention`, the app; or `stitch answer {question} y` / `n`): tell them \
     exactly what it asks. Its report still wakes you; `threads` shows it, `cancel` stops it.]",
    approval_said(s, thread, &q)
  );
  let back = s.clone();
  let undo = key.clone();
  let untold = chat::Untold(Arc::new(move || {
    back
      .detached
      .told_blocked
      .lock()
      .expect("told lock")
      .remove(&undo);
  }));
  if !crate::detached::weave(s, &parent, text, untold) {
    // No loop runs on it: `wait` (or `threads`) tells it later.
    s.detached
      .told_blocked
      .lock()
      .expect("told lock")
      .remove(&key);
  }
}

/// The threads `me` spawned or sent to (reports came, or are due): newest
/// first.
fn mine(s: &Arc<Shared>, me: &RecordId) -> Vec<RecordId> {
  let mut out: Vec<(Timestamp, RecordId)> = Vec::new();
  let mut add = |at: Timestamp, t: RecordId| {
    if !out.iter().any(|(_, o)| *o == t) {
      out.push((at, t));
    }
  };
  {
    let index = s.state().index.lock().expect("index lock");
    for t in index
      .sources(kinds::THREAD, "spawned-by", me, None)
      .unwrap_or_default()
    {
      add(t.created, t.id);
    }
    for r in index
      .sources(kinds::REPORT, "reports-to", me, None)
      .unwrap_or_default()
    {
      if let Some(t) = index
        .edges_from(&r.id)
        .ok()
        .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
      {
        add(r.created, t.dst.id);
      }
    }
    for a in index
      .sources(kinds::ASSIGNMENT, "reports-to", me, None)
      .unwrap_or_default()
    {
      if let Some(t) = a
        .fields
        .get("thread")
        .and_then(Value::as_str)
        .and_then(|t| t.parse::<RecordId>().ok())
      {
        add(a.created, t);
      }
    }
  }
  for (t, at) in s
    .detached
    .outstanding
    .lock()
    .expect("outstanding lock")
    .get(me.as_str())
    .cloned()
    .unwrap_or_default()
  {
    if let Ok(t) = t.parse() {
      add(at, t);
    }
  }
  out.sort_by_key(|o| std::cmp::Reverse(o.0));
  out.into_iter().map(|(_, t)| t).collect()
}

/// A thread's latest approval answered here (or on another node than the
/// one that asked) after that node was last reached: the answer has not
/// reached it yet, so the thread still waits there, whatever its record
/// says. What `threads` says of it then.
fn answer_undelivered(s: &Arc<Shared>, thread: &RecordId) -> Option<String> {
  let (q, a) = {
    let index = s.state().index.lock().expect("index lock");
    let q = index
      .sources(kinds::QUESTION, "thread", thread, None)
      .ok()?
      .into_iter()
      .filter(|q| q.fields.get("approval").and_then(Value::as_bool) == Some(true))
      .max_by_key(|q| q.created)?;
    let a = index
      .sources(kinds::ANSWER, "answers", &q.id, Some(""))
      .ok()?
      .into_iter()
      .last()?;
    // Reported since (by way of another node): it went on.
    let reported = index
      .sources(kinds::REPORT, "thread", thread, None)
      .ok()?
      .iter()
      .any(|r| r.created > q.created);
    if reported {
      return None;
    }
    (q, a)
  };
  if a.node == q.node
    || q.node == s.state().node.as_str()
    || a.fields.get("withdrawn").and_then(Value::as_bool) == Some(true)
  {
    return None;
  }
  // Stepped since the answer (its steps synced here): it had the answer.
  if let Ok((root, _)) = s.thread_place(thread) {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    if chat::last_step_at(&ctx, thread)
      .ok()
      .flatten()
      .is_some_and(|at| at > a.created)
    {
      return None;
    }
  }
  let seen = s.state().seen();
  let peer = seen.peer.get(&q.node)?;
  let answered = a.created.as_second().max(0) as u64;
  // Reached since the answer was written: it may well have it.
  if peer.reached.is_some_and(|r| r >= answered) {
    return None;
  }
  let ago = |secs: u64| {
    let now = Timestamp::now().as_second().max(0) as u64;
    crate::jobs::spoken(Duration::from_secs(now.saturating_sub(secs)))
  };
  let reached = match (peer.reached, peer.failed) {
    (Some(r), Some(f)) => format!(
      "{} not reached since {} ago (tried {} ago)",
      q.node,
      ago(r),
      ago(f)
    ),
    (Some(r), None) => format!("{} last reached {} ago", q.node, ago(r)),
    (None, _) => format!("{} not reached yet", q.node),
  };
  let choice = a
    .fields
    .get("choice")
    .and_then(Value::as_str)
    .unwrap_or("answered");
  Some(format!(
    "blocked: approval {} answered ({choice}, on {}, {} ago), not delivered to {} yet: {reached}; \
     it goes there as soon as {} is reached",
    q.id,
    a.node,
    ago(answered),
    q.node,
    q.node
  ))
}

/// A thread's state as `threads` says it, and the word its filter takes.
pub(crate) fn state_of(s: &Arc<Shared>, thread: &RecordId) -> (String, &'static str) {
  if s
    .detached
    .starting
    .lock()
    .expect("starting lock")
    .contains_key(thread.as_str())
  {
    return (
      "starting once the spawning step is written".into(),
      "working",
    );
  }
  let blocked = open_approvals(s, thread);
  if let Some(q) = blocked.first() {
    let more = match blocked.len() {
      1 => String::new(),
      n => format!(" (and {} more)", n - 1),
    };
    return (
      format!(
        "blocked: approval {}: {}{more}",
        q.id,
        approval_said(s, thread, q)
      ),
      "blocked",
    );
  }
  let runs_here = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .contains_key(thread.as_str());
  if runs_here {
    return ("working".into(), "working");
  }
  // A turn accepted here whose loop has not begun (`Live::queue`).
  if s.live.status(thread.as_str()).queued > 0 {
    return ("working: a turn is starting".into(), "working");
  }
  if let Some(said) = answer_undelivered(s, thread) {
    return (said, "blocked");
  }
  let index = s.state().index.lock().expect("index lock");
  let Ok(Some(record)) = index.record(thread) else {
    drop(index);
    // No start acknowledgement yet: the target may be offline, or its
    // first record may simply not have synced back. Do not infer liveness.
    return match start_outcome(s, thread).as_deref() {
      Some("cancelled") => (
        "ended: cancelled (assignment withdrawn; remote stop unconfirmed)".into(),
        "ended",
      ),
      Some("failed") => {
        let node = assigned_node(s, thread).unwrap_or_else(|| "another node".into());
        let why = start_failure(s, thread).unwrap_or_default();
        (format!("ended: failed to start on {node}: {why}"), "ended")
      }
      Some(_) => {
        let node = assigned_node(s, thread).unwrap_or_else(|| "another node".into());
        let since = spawn_assignment(s, thread)
          .ok()
          .flatten()
          .map(|a| crate::jobs::spoken(Timestamp::now().duration_since(a.created).unsigned_abs()))
          .unwrap_or_default();
        (
          format!(
            "not started: {node} has not confirmed it in {since} (offline, on an older \
             version, or its start failed there); `cancel` withdraws it"
          ),
          "idle",
        )
      }
      None => (
        "starting on another node (start unconfirmed)".into(),
        "working",
      ),
    };
  };
  match chat::detached_state(&index, &record)
    .ok()
    .flatten()
    .as_deref()
  {
    None => ("idle (not a detached thread)".into(), "idle"),
    Some("running") => ("working".into(), "working"),
    Some("waiting") => ("waiting on the user".into(), "blocked"),
    Some("compacted") => ("compacted into a newer thread".into(), "ended"),
    Some("cancelled") => ("ended: cancelled".into(), "ended"),
    Some(outcome) => (
      format!("idle: reported {outcome} (send continues it)"),
      "idle",
    ),
  }
}

/// The first lines of the last thing a thread's model said.
fn last_said(s: &Arc<Shared>, thread: &RecordId) -> Option<String> {
  last_words(s, thread).map(|w| first_lines(&w, 3))
}

/// The last thing a thread's model said.
fn last_words(s: &Arc<Shared>, thread: &RecordId) -> Option<String> {
  let (root, _) = s.thread_place(thread).ok()?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let shown = chat::transcript_shown(&ctx, thread, None).ok()?;
  let not_model = [
    "you", "output", "error", "memory", "tool", "fork", "refusal", "system", "failed",
  ];
  let last = shown
    .iter()
    .rev()
    .find(|e| !not_model.contains(&e.who.as_str()) && !e.text.trim().is_empty())?;
  Some(last.text.trim().to_owned())
}

pub(crate) fn first_lines(text: &str, n: usize) -> String {
  let lines: Vec<&str> = text.trim().lines().collect();
  let more = if lines.len() > n { " …" } else { "" };
  format!("{}{more}", lines[..lines.len().min(n)].join(" / "))
}

/// The threads `threads` lists: those `me` spawned or sent to, newest
/// first, or with `all` every detached thread this node knows of, on any
/// node, the latest started first.
pub(crate) fn listed(s: &Arc<Shared>, me: Option<&RecordId>, all: bool) -> Vec<RecordId> {
  if all {
    let index = s.state().index.lock().expect("index lock");
    let mut detached: Vec<stitch_store::IndexedRecord> = index
      .records(Some(kinds::THREAD))
      .unwrap_or_default()
      .into_iter()
      .filter(|t| t.fields.get("detached").and_then(Value::as_bool) == Some(true))
      .collect();
    let mut rows: Vec<_> = detached.drain(..).map(|t| (t.created, t.id)).collect();
    for a in index.records(Some(kinds::ASSIGNMENT)).unwrap_or_default() {
      if a.fields.get("continue").and_then(Value::as_bool) == Some(true) {
        continue;
      }
      if let Some(id) = a
        .fields
        .get("thread")
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<RecordId>().ok())
        && !rows.iter().any(|(_, t)| *t == id)
      {
        rows.push((a.created, id));
      }
    }
    rows.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
    // A compaction chain is one row, its newest thread (design §2.15).
    rows
      .into_iter()
      .map(|(_, id)| id)
      .filter(|id| index.compacted_into(id).is_none())
      .collect()
  } else {
    me.map(|me| mine(s, me)).unwrap_or_default()
  }
}

/// A thread as `threads` lists it, and apps' live data gives it.
pub(crate) struct Row {
  pub(crate) id: RecordId,
  /// Its state as said, and the word `status` filters on (`working`,
  /// `blocked`, `idle`, `ended`).
  pub(crate) state: String,
  pub(crate) word: &'static str,
  /// What its latest report says of it (`done`, `blocked`, `not-done`,
  /// `failed`, `stopped`, `over-budget`, `cancelled`), or `compacted`.
  pub(crate) outcome: Option<String>,
  /// The approval it waits on: the question and its exact command.
  pub(crate) question: Option<(RecordId, String)>,
  /// Its record is here (else it is starting on another node).
  pub(crate) known: bool,
  pub(crate) title: Option<String>,
  pub(crate) goal: String,
  /// The node that runs it, and the one it started on.
  pub(crate) node: String,
  pub(crate) started_on: String,
  pub(crate) place: String,
  /// Who spawned it (`spawned-by`), else whom it reports to.
  pub(crate) parent: Option<RecordId>,
  pub(crate) detached: bool,
  pub(crate) started: Option<Timestamp>,
  pub(crate) last_step: Option<Timestamp>,
  pub(crate) steps: usize,
  pub(crate) usd: Option<f64>,
  /// The last thing its model said, whole.
  pub(crate) last_words: Option<String>,
}

/// What reading a thread's transcript gives a `Row` (its model calls,
/// their cost, its last words), kept by its last step: a page left open
/// reads each transcript once per step, not once per look.
pub(crate) type Read = (usize, Option<f64>, Option<String>);
pub(crate) type RowCache =
  std::sync::Mutex<std::collections::HashMap<RecordId, (Option<Timestamp>, Read)>>;

/// A thread's row; `cache` keeps what its transcript gave.
pub(crate) fn row(s: &Arc<Shared>, id: &RecordId, cache: Option<&RowCache>) -> Row {
  row_in(s, id, cache, &s.places())
}

/// Reads, for each of `ids` the cache has not got at its last step, what a
/// row takes from the transcript (model calls, cost, last words), on a few
/// threads at once: a list of a thousand threads after a restart read as
/// many transcripts one after another (3 s on gamer's data). What a row
/// then finds is the same as if it had read them itself.
pub(crate) fn warm_rows(s: &Arc<Shared>, ids: &[RecordId], cache: &RowCache, places: &Places) {
  let todo: Vec<(&RecordId, Option<Timestamp>)> = ids
    .iter()
    .filter_map(|id| {
      let at = last_step_in(s, id, places)?;
      let hit = cache
        .lock()
        .expect("rows lock")
        .get(id)
        .is_some_and(|(c, _)| *c == at);
      (!hit).then_some((id, at))
    })
    .collect();
  if todo.len() < 8 {
    return;
  }
  let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
  let next = std::sync::atomic::AtomicUsize::new(0);
  std::thread::scope(|scope| {
    for _ in 0..workers {
      scope.spawn(|| {
        loop {
          let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
          let Some((id, at)) = todo.get(i) else { break };
          let (steps, usd) = crate::detached::steps_of(s, id).unwrap_or((0, None));
          let read = (steps, usd, last_words(s, id));
          cache
            .lock()
            .expect("rows lock")
            .insert((*id).clone(), (*at, read));
        }
      });
    }
  });
}

/// When a thread's latest step was written here; `None` for a thread with
/// no record or place here.
fn last_step_in(s: &Arc<Shared>, id: &RecordId, places: &Places) -> Option<Option<Timestamp>> {
  let (root, _) = s.thread_place_in(id, places).ok()?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  Some(chat::last_step_at(&ctx, id).ok().flatten())
}

/// `row` with the places read once by the caller.
pub(crate) fn row_in(
  s: &Arc<Shared>,
  id: &RecordId,
  cache: Option<&RowCache>,
  places: &Places,
) -> Row {
  let (state, word) = state_of(s, id);
  let (record, title, parent, outcome) = {
    let index = s.state().index.lock().expect("index lock");
    let record = index.record(id).ok().flatten();
    let parent = index.edges_from(id).ok().and_then(|edges| {
      edges
        .iter()
        .find(|e| e.rel == "spawned-by")
        .or_else(|| edges.iter().find(|e| e.rel == "reports-to"))
        // As it goes on now: lists show a compaction chain as its newest
        // (design §2.15, "One conversation"), so a child of an earlier one
        // nests there, not as an orphan.
        .map(|e| index.compaction_newest(&e.dst.id))
    });
    let outcome = record
      .as_ref()
      .and_then(|r| chat::detached_state(&index, r).ok().flatten())
      .filter(|o| !matches!(o.as_str(), "running" | "waiting"));
    (record, index.title(id).ok().flatten(), parent, outcome)
  };
  let question = (word == "blocked")
    .then(|| open_approvals(s, id).into_iter().next())
    .flatten()
    .map(|q| {
      let command = q
        .fields
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(q.title.clone())
        .unwrap_or_default();
      (q.id, command)
    });
  let mut row = Row {
    id: id.clone(),
    state,
    word,
    outcome,
    question,
    known: record.is_some(),
    title,
    goal: String::new(),
    node: String::new(),
    started_on: String::new(),
    place: "?".into(),
    parent,
    detached: false,
    started: None,
    last_step: None,
    steps: 0,
    usd: None,
    last_words: None,
  };
  let Some(record) = record else {
    // Not here yet (starting on another node, or never started there): what
    // its assignment says, so a list shows what it is, where and since when.
    if let Ok(Some(a)) = spawn_assignment(s, id) {
      let goal = a.fields.get("goal").and_then(Value::as_str).unwrap_or("");
      row.goal = first_lines(goal, 1);
      row.title = row.title.or_else(|| a.title.clone());
      row.node = a
        .fields
        .get("node")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
      row.started_on = a.node.clone();
      row.started = Some(a.created);
      row.detached = true;
      if let Some(p) = a.fields.get("place").and_then(Value::as_str) {
        row.place = p.to_owned();
      }
      if row.parent.is_none() {
        let index = s.state().index.lock().expect("index lock");
        row.parent = index.edges_from(&a.id).ok().and_then(|edges| {
          edges
            .into_iter()
            .find(|e| e.rel == "reports-to")
            .map(|e| e.dst.id)
        });
      }
      row.outcome = start_outcome(s, id);
      row.last_words = start_failure(s, id);
    }
    return row;
  };
  let here = s.state().node.to_string();
  row.goal = record
    .fields
    .get("goal")
    .and_then(Value::as_str)
    .map(|g| first_lines(g, 1))
    .or(row.title.clone())
    .unwrap_or_default();
  row.detached = record.fields.get("detached").and_then(Value::as_bool) == Some(true);
  row.node = runner(s, id, &record.node);
  row.started_on = record.node.to_string();
  row.started = Some(record.created);
  if let Ok((root, place)) = s.thread_place_in(id, places) {
    row.place = place.label(&here, places);
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    row.last_step = chat::last_step_at(&ctx, id).ok().flatten();
  }
  let cached = cache.and_then(|c| {
    c.lock()
      .expect("rows lock")
      .get(id)
      .filter(|(at, _)| *at == row.last_step)
      .map(|(_, r)| r.clone())
  });
  let read = cached.unwrap_or_else(|| {
    let (steps, usd) = crate::detached::steps_of(s, id).unwrap_or((0, None));
    let read = (steps, usd, last_words(s, id));
    if let Some(c) = cache {
      c.lock()
        .expect("rows lock")
        .insert(id.clone(), (row.last_step, read.clone()));
    }
    read
  });
  (row.steps, row.usd, row.last_words) = read;
  row
}

/// `threads`: the threads `me` spawned or sent to (with `all`, every
/// detached thread this node knows of, on any node), with `status` one of
/// `working`, `blocked`, `idle`, `ended` (none: all).
pub(crate) fn threads_text(
  s: &Arc<Shared>,
  me: &RecordId,
  all: bool,
  status: Option<&str>,
) -> String {
  let ids = listed(s, Some(me), all);
  let mut rows = Vec::new();
  let mut skipped = 0;
  for id in &ids {
    let (state, word) = state_of(s, id);
    if status.is_some_and(|w| w != word) {
      continue;
    }
    if rows.len() == THREADS_LISTED {
      skipped += 1;
      continue;
    }
    let r = row(s, id, None);
    if !r.known {
      let you = if id == me { " (you)" } else { "" };
      rows.push(format!("{id}{you}: {state}"));
      continue;
    }
    let local = |t: Timestamp| {
      t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
    };
    let you = if id == me { " (you)" } else { "" };
    let mut line = format!(
      "{id}{you}: {}, in {}, {}: {}\n  goal: {}\n  started {}{}; {} model calls{}",
      runs_on(&r.node, &r.started_on),
      r.place,
      crate::node::mode_word(s.read_only(id)),
      r.state,
      r.goal,
      r.started.map(local).unwrap_or_default(),
      r.last_step
        .map(|t| format!(", last step {}", local(t)))
        .unwrap_or_default(),
      r.steps,
      r.usd.map(|d| format!(", ${d:.2}")).unwrap_or_default(),
    );
    if let Some(said) = &r.last_words {
      line.push_str(&format!("\n  last said: {}", first_lines(said, 3)));
    }
    rows.push(line);
  }
  if rows.is_empty() {
    return match (all, status) {
      (false, None) => "No threads: this thread spawned or sent to none.".into(),
      (true, None) => "No detached threads on this node or its peers' synced records.".into(),
      (_, Some(w)) => format!("No threads {w}."),
    };
  }
  if skipped > 0 {
    rows.push(format!(
      "[… {skipped} more, older; `status` narrows the list]"
    ));
  }
  rows.join("\n")
}

/// Where a thread runs, said apart from its ID: the node in an ID is the
/// one it started on, and a thread runs where it was continued last
/// (design §2.7, "Thread IDs and nodes").
pub(crate) fn runs_on(runner: &str, started_on: &str) -> String {
  if runner == started_on || started_on.is_empty() {
    format!("runs on {runner}")
  } else {
    format!("runs on {runner} (started on {started_on})")
  }
}

/// Threads `threads` lists at most.
const THREADS_LISTED: usize = 30;

/// What a thread did so far, from its records: its last words and the
/// calls it made (the last 12).
fn did_so_far(s: &Arc<Shared>, thread: &RecordId) -> String {
  let Ok((root, _)) = s.thread_place(thread) else {
    return String::new();
  };
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let Ok(shown) = chat::transcript_shown(&ctx, thread, None) else {
    return String::new();
  };
  let calls: Vec<&str> = shown
    .iter()
    .filter(|e| e.who == "tool")
    .map(|e| e.text.as_str())
    .collect();
  let mut out = String::new();
  if let Some(said) = last_said(s, thread) {
    out.push_str(&format!("Its last words: {said}\n"));
  }
  match calls.len() {
    0 => out.push_str("It made no tool calls."),
    n => {
      out.push_str(&format!(
        "Its calls ({}):\n",
        if n > 12 {
          format!("the last 12 of {n}")
        } else {
          n.to_string()
        }
      ));
      for c in &calls[n.saturating_sub(12)..] {
        out.push_str(&format!("- {}\n", first_lines(c, 1)));
      }
    }
  }
  out.trim_end().to_owned()
}

/// `threads` and `cancel`.
pub(crate) async fn thread_tool(
  s: &Arc<Shared>,
  me: &RecordId,
  name: &str,
  input: Value,
  ask: &crate::tools::Asker,
) -> ToolOutcome {
  match name {
    "threads" => {
      let i: ThreadsInput = match serde_json::from_value(input) {
        Ok(i) => i,
        Err(e) => return ToolOutcome::err(e.to_string()),
      };
      let status = i.status.as_deref().map(str::trim).filter(|w| !w.is_empty());
      if let Some(w) = status
        && !["working", "blocked", "idle", "ended"].contains(&w)
      {
        return ToolOutcome::err(format!("status `{w}`: working, blocked, idle or ended"));
      }
      // What `wait` would say of a blocked thread is said here now.
      let out = threads_text(s, me, i.all, status);
      for t in mine(s, me) {
        for q in open_approvals(s, &t) {
          s.detached
            .told_blocked
            .lock()
            .expect("told lock")
            .insert(format!("{me} {}", q.id));
        }
      }
      ToolOutcome::ok(out)
    }
    "cancel" => outcome(cancel_tool(s, me, input, ask).await),
    other => ToolOutcome::err(format!("no tool {other}")),
  }
}

/// `cancel`: stops a thread's running turn. One this thread spawned or
/// sent to stops at once; another asks the user first (`ask`).
pub(crate) async fn cancel_tool(
  s: &Arc<Shared>,
  me: &RecordId,
  input: Value,
  ask: &crate::tools::Asker,
) -> Result<String> {
  let i: CancelInput = serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
  let to: RecordId = i.thread.trim().parse()?;
  if &to == me {
    return Err(Error::Config(
      "a thread does not cancel itself: end the turn with your last message".into(),
    ));
  }
  let title = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .title(&to)
    .ok()
    .flatten()
    .unwrap_or_default();
  let reason = i.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
  if !mine(s, me).contains(&to) {
    let what = format!(
      "cancel thread {to} \"{title}\"{}",
      reason.map(|r| format!(": {r}")).unwrap_or_default()
    );
    ask.ask(me, "threads", &what).await.map_err(Error::Config)?;
  }
  let mut did = did_so_far(s, &to);
  // A call waiting on the user is in no written step yet.
  for q in open_approvals(s, &to) {
    let command = q
      .fields
      .get("command")
      .and_then(Value::as_str)
      .unwrap_or("");
    did.push_str(&format!(
      "\nIt was waiting for the user's approval to run `{}` (question {}, withdrawn now).",
      first_lines(command, 1),
      q.id
    ));
  }
  let text = format!(
    "Cancelled by thread {me}{}.\n\n{did}",
    reason.map(|r| format!(": {r}")).unwrap_or_default()
  );
  let at = crate::detached::cancel_with(s, &to, &text)?;
  // Its report goes to whoever started it: when that is this thread, it
  // has it here already.
  let report = {
    let index = s.state().index.lock().expect("index lock");
    index
      .sources(kinds::REPORT, "thread", &to, None)
      .unwrap_or_default()
      .into_iter()
      .filter(|r| r.fields.get("outcome").and_then(Value::as_str) == Some("cancelled"))
      .max_by_key(|r| r.created)
      .filter(|r| {
        index
          .edges_from(&r.id)
          .is_ok_and(|e| e.iter().any(|e| e.rel == "reports-to" && &e.dst.id == me))
      })
  };
  if let Some(r) = report {
    s.detached
      .delivered
      .lock()
      .expect("delivered lock")
      .insert(r.id.to_string());
    if let Some(list) = s
      .detached
      .outstanding
      .lock()
      .expect("outstanding lock")
      .get_mut(me.as_str())
    {
      list.retain(|(t, _)| *t != to.as_str());
    }
  }
  let how = match at {
    crate::detached::CancelledAt::Here => {
      "its running turn is stopped, its tool calls cut, its jobs ended and its sessions closed \
       with what ran in them"
        .to_owned()
    }
    crate::detached::CancelledAt::There(node) => format!(
      "it runs on {node}, which stops it (turn, jobs and sessions) when the cancellation \
       reaches it"
    ),
    crate::detached::CancelledAt::Pending(node) => format!(
      "assignment withdrawn; its start on {node} was not confirmed here. It will not start \
       once the withdrawal arrives; if already running there, it stops then. Remote stop \
       is not yet confirmed"
    ),
  };
  Ok(format!(
    "Cancelled thread {to} \"{title}\": {how}. Its report says cancelled{}. What it did so far:\n{did}",
    reason.map(|r| format!(" ({r})")).unwrap_or_default()
  ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelInput {
  thread: String,
  #[serde(default)]
  reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadsInput {
  #[serde(default)]
  all: bool,
  #[serde(default)]
  status: Option<String>,
}

/// What `wait` returns: the reports, and what their threads spent.
pub(crate) struct Waited {
  pub text: String,
  pub spent: Option<(Usage, Option<f64>)>,
}

/// The threads a `wait` is for, each with when it was asked for: the ones
/// given, else all still out.
fn wait_targets(
  asked: &[(String, Timestamp)],
  threads: Vec<RecordId>,
) -> Vec<(RecordId, Timestamp)> {
  // A thread sent to or spawned several times is asked several times, but
  // reports once each time it is waited for: listed once, from its latest
  // ask (earlier asks may be answered already, by a report that woke the
  // parent without pruning them).
  let mut seen = std::collections::HashSet::new();
  let threads: Vec<RecordId> = if threads.is_empty() {
    asked.iter().filter_map(|(c, _)| c.parse().ok()).collect()
  } else {
    threads
  };
  threads
    .into_iter()
    .filter(|t| seen.insert(t.to_string()))
    .map(|t| {
      let at = asked
        .iter()
        .rev()
        .find(|(c, _)| *c == t.as_str())
        .map_or(Timestamp::UNIX_EPOCH, |(_, at)| *at);
      (t, at)
    })
    .collect()
}

/// Waits for reports to `parent` from `threads` (all the threads it
/// spawned or sent to that have not reported, when empty): until they are
/// in, something else comes for `parent` (woven into its loop next: the
/// user's message, a job's note), or `max` passes when given. Never a
/// guessed timeout: a wait ends on an event (design §2.7).
pub(crate) async fn wait(
  s: &Arc<Shared>,
  parent: &RecordId,
  threads: Vec<RecordId>,
  max: Option<Duration>,
) -> Waited {
  let asked: Vec<(String, Timestamp)> = s
    .detached
    .outstanding
    .lock()
    .expect("outstanding lock")
    .get(parent.as_str())
    .cloned()
    .unwrap_or_default();
  let targets = wait_targets(&asked, threads);
  if targets.is_empty() {
    return Waited {
      text: "Nothing to wait for: no thread this one spawned or sent to is still out.".into(),
      spent: None,
    };
  }
  /// Listed while it waits, however the wait ends: the call interrupted
  /// or its loop aborted included. Then what came meanwhile and was not
  /// taken here wakes it.
  struct Waiting<'a>(&'a Arc<Shared>, String);
  impl Drop for Waiting<'_> {
    fn drop(&mut self) {
      self
        .0
        .detached
        .waiting
        .lock()
        .expect("waiting lock")
        .remove(&self.1);
      deliver(self.0);
    }
  }
  s.detached
    .waiting
    .lock()
    .expect("waiting lock")
    .insert(parent.to_string());
  let waiting = Waiting(s, parent.to_string());
  let report_of = |t: &RecordId, since: Timestamp| {
    // A thread compacted into another reports from there.
    let lineage = crate::fork::lineage(s, t);
    let index = s.state().index.lock().expect("index lock");
    let to_me: Vec<RecordId> = index
      .sources(kinds::REPORT, "reports-to", parent, None)
      .unwrap_or_default()
      .into_iter()
      .map(|r| r.id)
      .collect();
    lineage
      .iter()
      .flat_map(|t| {
        index
          .sources(kinds::REPORT, "thread", t, None)
          .unwrap_or_default()
      })
      .filter(|r| to_me.contains(&r.id) && r.created >= since)
      .max_by_key(|r| r.created)
  };
  let starting = |t: &RecordId| {
    s.detached
      .starting
      .lock()
      .expect("starting lock")
      .contains_key(t.as_str())
  };
  let deadline = max.map(|m| tokio::time::Instant::now() + m);
  // What comes for this thread meanwhile cuts the wait short.
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(parent.as_str())
    .cloned();
  let mut cut = false;
  // Threads waited for that wait on the user in turn: (thread, what).
  let mut blocked: Vec<(RecordId, String)> = Vec::new();
  loop {
    let notified = s.detached.reported.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let asked = s.detached.asked.notified();
    tokio::pin!(asked);
    asked.as_mut().enable();
    let arrival = steer.as_ref().map(|st| st.arrival());
    let mut arrival = std::pin::pin!(async move {
      match arrival {
        Some(a) => a.await,
        None => std::future::pending().await,
      }
    });
    // Polled once, so a message queued from here on wakes it.
    let _ = futures_util::poll!(arrival.as_mut());
    let pending = targets
      .iter()
      .filter(|(t, at)| report_of(t, *at).is_none() && !starting(t))
      .count();
    if pending == 0 {
      break;
    }
    if steer.as_ref().is_some_and(chat::Steer::has_queued) {
      cut = true;
      break;
    }
    // A thread waited for that is stuck on an approval: the waiting
    // thread is told, once, so it can tell the user rather than wait on
    // something only they can move.
    blocked = targets
      .iter()
      .filter(|(t, at)| report_of(t, *at).is_none())
      .flat_map(|(t, _)| {
        blocked_on_user(s, t)
          .into_iter()
          .map(move |(q, what)| (t.clone(), q, what))
      })
      .filter(|(_, q, _)| {
        s.detached
          .told_blocked
          .lock()
          .expect("told lock")
          .insert(format!("{parent} {q}"))
      })
      .map(|(t, _, what)| (t, what))
      .collect();
    if !blocked.is_empty() {
      break;
    }
    let until = async {
      match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
      }
    };
    tokio::select! {
      _ = notified => {}
      _ = asked => {}
      _ = arrival => {}
      _ = until => break,
    }
  }
  let mut text = String::new();
  let mut usage = Usage::default();
  let mut usd: Option<f64> = None;
  let mut any = false;
  let mut done: Vec<String> = Vec::new();
  for (t, at) in &targets {
    if !text.is_empty() {
      text.push_str("\n\n");
    }
    match report_of(t, *at) {
      Some(r) => {
        text.push_str(&report_message(s, &r, Some(parent)));
        s.detached
          .delivered
          .lock()
          .expect("delivered lock")
          .insert(r.id.to_string());
        done.push(t.to_string());
        if let Some((u, d)) = crate::detached::spent_by_chain(s, t, Some(*at)) {
          any = true;
          usage.input_tokens += u.input_tokens;
          usage.output_tokens += u.output_tokens;
          usage.cache_read_tokens += u.cache_read_tokens;
          usage.cache_write_tokens += u.cache_write_tokens;
          if let Some(d) = d {
            *usd.get_or_insert(0.0) += d;
          }
        }
      }
      None if starting(t) => text.push_str(&format!(
        "{t}: starts once this step is written (it reads this conversation); wait for it in \
         your next step"
      )),
      // Blocked whether or not that is what ended the wait: told once
      // (woven in before the wait, say, cutting it short), a thread
      // still waiting on the user is never said to be working.
      None => match blocked
        .iter()
        .find(|(b, _)| b == t)
        .map(|(_, what)| what.clone())
        .or_else(|| blocked_on_user(s, t).into_iter().next().map(|(_, w)| w))
      {
        Some(what) => text.push_str(&format!(
          "{t}: blocked: {what}. It is on the user's attention list (Ctrl+T in the TUI, \
           `stitch attention`); only they can answer it. Tell them, then wait again (its report \
           still wakes you), or cancel it."
        )),
        None => {
          let (state, _) = state_of(s, t);
          let state = if state == "working" {
            "still working"
          } else {
            &state
          };
          text.push_str(&format!("{t}: {state}; its report wakes you when it is in"));
        }
      },
    }
  }
  {
    let mut out = s.detached.outstanding.lock().expect("outstanding lock");
    if let Some(list) = out.get_mut(parent.as_str()) {
      list.retain(|(c, _)| !done.contains(c));
    }
  }
  drop(waiting);
  if cut {
    text = format!("Cut short: something came for this thread (it follows this result).\n\n{text}");
  }
  Waited {
    text,
    spent: any.then_some((usage, usd)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_turn_waits_on_lan_peers_a_second_and_not_on_failed_ones() {
    let peer = |addr: &str| stitch_net::Peer {
      name: "phone".into(),
      id: String::new(),
      addrs: vec![addr.parse().unwrap()],
    };
    let now = 1_000_000;
    let seen = |path, addr: &str, failed: Option<u64>| stitch_net::Seen {
      path: Some(path),
      addr: Some(addr.into()),
      failed,
      ..Default::default()
    };
    use stitch_net::PathKind::{Direct, Relay};
    let lan = peer("192.168.0.20:1234");
    let far = peer("100.70.1.2:1234");
    // Never reached: by the addresses it lists.
    assert_eq!(catch_up_budget(&lan, None, now), CATCH_UP_LAN);
    assert_eq!(catch_up_budget(&far, None, now), CATCH_UP_REMOTE);
    // Reached: by the path it was last reached on.
    let direct = seen(Direct, "192.168.0.20:1234", None);
    assert_eq!(catch_up_budget(&far, Some(&direct), now), CATCH_UP_LAN);
    let relayed = seen(Relay, "192.168.0.20:1234", None);
    assert_eq!(catch_up_budget(&lan, Some(&relayed), now), CATCH_UP_REMOTE);
    // Failed a moment ago: not waited for; a while ago: as before.
    let failed = seen(Direct, "192.168.0.20:1234", Some(now - 10));
    assert_eq!(catch_up_budget(&lan, Some(&failed), now), Duration::ZERO);
    let failed = seen(Direct, "192.168.0.20:1234", Some(now - 600));
    assert_eq!(catch_up_budget(&lan, Some(&failed), now), CATCH_UP_LAN);
    assert!(CATCH_UP_REMOTE <= Duration::from_secs(3));
  }

  #[tokio::test]
  async fn a_peer_not_answering_in_time_is_caught_up_in_the_background() {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let running: Arc<CatchUps> = Default::default();
    let key = |p: &str| (p.to_owned(), "home".to_owned());
    // A peer on the local network that never answers, and a slow one whose
    // segments come after the turn started.
    let asleep = run_once(
      &running,
      key("phone"),
      std::future::pending::<()>(),
      |_, _| async {},
    );
    let taken: Arc<Mutex<Vec<(&str, bool)>>> = Default::default();
    let starts = Arc::new(AtomicUsize::new(0));
    let slow = |taken: Arc<Mutex<Vec<(&'static str, bool)>>>, starts: Arc<AtomicUsize>| {
      run_once(
        &running,
        key("mac"),
        async move {
          starts.fetch_add(1, Ordering::SeqCst);
          tokio::time::sleep(Duration::from_millis(1500)).await;
          "segment"
        },
        move |got, late| async move { taken.lock().unwrap().push((got, late)) },
      )
    };
    let started = std::time::Instant::now();
    let (a, b) = (
      slow(taken.clone(), starts.clone()),
      slow(taken.clone(), starts.clone()),
    );
    futures_util::future::join3(
      asleep.wait(CATCH_UP_LAN),
      a.wait(CATCH_UP_LAN),
      b.wait(CATCH_UP_LAN),
    )
    .await;
    let waited = started.elapsed();
    assert!(waited >= CATCH_UP_LAN, "{waited:?}");
    assert!(waited < Duration::from_millis(1200), "{waited:?}");
    assert!(taken.lock().unwrap().is_empty());
    // The slow one goes on, once (the second turn waited on the first's),
    // and is taken in as late.
    for _ in 0..50 {
      if !taken.lock().unwrap().is_empty() {
        break;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(*taken.lock().unwrap(), [("segment", true)]);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(!running.lock().unwrap().contains_key(&key("mac")));
    // The one asleep is still running: a turn starting now waits on it.
    assert!(running.lock().unwrap().contains_key(&key("phone")));
  }

  #[test]
  fn a_thread_sent_to_many_times_is_waited_for_once() {
    let child = "2026-11-02T085001Z-gamer-efaw";
    let at = |s: &str| s.parse::<Timestamp>().unwrap();
    let asked: Vec<(String, Timestamp)> = (0..7)
      .map(|i| (child.to_string(), at(&format!("2026-11-02T09:0{i}:00Z"))))
      .collect();
    let targets = wait_targets(&asked, Vec::new());
    assert_eq!(targets.len(), 1, "{targets:?}");
    // From the latest ask: an earlier report is not the answer to it.
    assert_eq!(targets[0].1, at("2026-11-02T09:06:00Z"));
    let twice = vec![child.parse().unwrap(), child.parse().unwrap()];
    assert_eq!(wait_targets(&asked, twice), targets);
  }

  #[test]
  fn waits_are_bounded_whatever_the_minutes() {
    // No minutes: no guessed time, it waits for an event.
    assert_eq!(wait_max(None), None);
    assert_eq!(wait_max(Some(2.0)), Some(Duration::from_secs(120)));
    assert_eq!(wait_max(Some(1e300)), Some(WAIT_MAX));
    assert_eq!(wait_max(Some(-1.0)), None);
    assert_eq!(wait_max(Some(f64::INFINITY)), None);
  }

  #[test]
  fn a_goal_reworded_is_the_same_goal_and_a_part_of_it_is_not() {
    let goal = "Redesign the gate adapter in ~/home-notes/projects/gate-adapter/ with the \
                corrected measurements from the user's photos. Only change files there. Read \
                the current .scad and README first.";
    // Handed on with a line added, as the 2026-10-19 chain did at each step.
    let again = format!("{goal} OpenSCAD is in /Applications.");
    let again = again.as_str();
    assert!(same_goal(goal, again));
    assert!(same_goal("Fix the build", "fix the build."));
    // A part of it, or another goal, is not.
    assert!(!same_goal(
      goal,
      "Check the screw pilots in gate_adapter.scad"
    ));
    assert!(!same_goal(goal, ""));
  }

  #[test]
  fn where_a_thread_runs_is_said_apart_from_its_id() {
    assert_eq!(runs_on("mbp", "mbp"), "runs on mbp");
    assert_eq!(runs_on("iphone", "mbp"), "runs on iphone (started on mbp)");
  }

  #[test]
  fn a_budget_cut_to_the_spawners_remainder_is_said() {
    let soon = Timestamp::now() + jiff::SignedDuration::from_mins(6);
    let later = Timestamp::now() + jiff::SignedDuration::from_mins(120);
    let asked = Limits {
      until: Some(later),
      ..Default::default()
    };
    let left = Limits {
      until: Some(soon),
      ..Default::default()
    };
    let said = capped_said(asked, left);
    assert!(
      said.contains("this thread's remainder: it must end by "),
      "{said}"
    );
    assert!(said.contains("minutes from now"), "{said}");
    // Nothing cut: nothing said.
    assert_eq!(capped_said(left, asked), "");
    assert_eq!(capped_said(asked, Limits::default()), "");
  }

  #[test]
  fn a_start_failing_is_given_up_after_tries_and_time() {
    let first: Timestamp = "2026-10-27T03:19:00Z".parse().unwrap();
    let after = |secs| first + jiff::SignedDuration::from_secs(secs);
    // Long enough, too few tries (a node that passed once and slept).
    assert!(!give_up_start(first, 1, after(3600)));
    // Tries enough, too soon (a token not refreshed yet).
    assert!(!give_up_start(first, 9, after(60)));
    assert!(give_up_start(first, 3, after(600)));
    // A clock gone back never gives up.
    assert!(!give_up_start(first, 9, after(-600)));
  }

  #[test]
  fn a_report_held_long_says_when_it_was_written() {
    let written: Timestamp = "2026-10-12T16:05:00Z".parse().unwrap();
    // Delivered at once: nothing said.
    let soon = written + jiff::SignedDuration::from_secs(60);
    assert_eq!(late_note(written, soon), "");
    // A clock behind the writer's: nothing said either.
    let before = written - jiff::SignedDuration::from_secs(3600);
    assert_eq!(late_note(written, before), "");
    // Hours later: when, how long ago, and that it was held.
    let later = written + jiff::SignedDuration::from_secs(12 * 3600 + 48 * 60);
    let note = late_note(written, later);
    assert!(note.starts_with("\n[Late: written 2026-10-1"), "{note}");
    assert!(note.contains("12h48m ago, and held until now"), "{note}");
  }
}
