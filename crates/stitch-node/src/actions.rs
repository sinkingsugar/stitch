//! Action buttons (design §2.30, "Inline controls", step 2): the
//! `offer_actions` tool. The agent offers buttons that each carry one tool
//! call; pressing one runs that call on the node that offered it, through
//! the gates the agent's own call would pass (the thread's tools, its
//! capability profiles, sandbox, approvals, `otter_run`'s effect consent),
//! with no model call. With `wake`, the thread then gets the press as the
//! user's message, and a turn.
//!
//! As with preset answers (`crate::replies`) nothing is stored apart: the
//! tool call is the offer, and a press is a `press` event in the thread's
//! transcript (`{t: "press", set, item, label, tool, ok, output, wake}`),
//! which clients show as a line and older builds keep unread. A set is
//! live while it is the thread's latest accepted `offer_actions` call
//! (`live`): unlike preset answers, the user writing does not end it.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::{Event, RecordId, Segment};

use crate::chat::{self, ChatContext};
use crate::node::Shared;
use crate::tools::{NodeTools, ToolOutcome, ToolRunner};
use crate::{Error, Result};

/// The tool's name.
pub const TOOL: &str = "offer_actions";
/// At most this many buttons in a set (a table's rows).
pub const MAX: usize = 24;
/// A label's length at most, in characters.
pub const LABEL_MAX: usize = crate::replies::LABEL_MAX;
/// The event a press writes.
pub const PRESS: &str = "press";
/// What a press keeps of its call's output in the transcript, in
/// characters.
pub const KEPT: usize = 20_000;
/// What the model is told of a press, in characters.
const TOLD: usize = 4_000;
/// A press's output in the note of presses the model reads with the
/// user's next message.
const NOTED: usize = 600;

/// Tools no button runs: offering buttons, and what only makes sense
/// inside a turn (waiting for threads or jobs, waking the agent later,
/// answering an event's ask).
pub const NOT_ACTIONS: &[&str] = &[
  TOOL,
  crate::replies::TOOL,
  "wait",
  "wake_at",
  "shell_wait",
  "event_reply",
];

/// Tools a read-only thread's buttons may run besides `shell_run` with a
/// reading command and pure `otter_run` code: those that change nothing.
const READ_ONLY: &[&str] = &[
  "view_file",
  "zoom_image",
  "browse_web",
  "search_web",
  "recall",
  "decide",
  "read_thread",
  "threads",
  "models",
  "jobs",
  "job_output",
  "shell_view",
  "otter_check",
  "otter_search",
  "otter_describe",
];

/// One button: its `label`, the call it runs, and whether its press then
/// wakes the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
  pub label: String,
  pub tool: String,
  pub input: Value,
  #[serde(default)]
  pub wake: bool,
}

impl Action {
  /// The call as one line, as transcripts show calls.
  pub fn call(&self) -> String {
    chat::call_summary(&self.tool, &self.input)
      .lines()
      .next()
      .unwrap_or_default()
      .to_owned()
  }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
  actions: Vec<InputAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputAction {
  label: String,
  tool: String,
  #[serde(default)]
  input: Option<Value>,
  #[serde(default)]
  wake: bool,
}

/// The buttons a call offers, as far as their shape goes, or why they are
/// refused: 1 to `MAX`, labels as preset answers' (short, single-line,
/// distinct), a tool that may be a button, its input an object. Whether
/// the thread has the tool and the input fits it is `check`'s, when it is
/// offered.
pub fn validate(input: &Value) -> std::result::Result<Vec<Action>, String> {
  let i: Input = serde_json::from_value(input.clone())
    .map_err(|e| format!("{TOOL} takes {{actions: [{{label, tool, input, wake?}}]}}: {e}"))?;
  if i.actions.is_empty() || i.actions.len() > MAX {
    return Err(format!("give 1 to {MAX} actions (got {})", i.actions.len()));
  }
  let mut out: Vec<Action> = Vec::with_capacity(i.actions.len());
  for a in i.actions {
    let label = a.label.trim().to_owned();
    if label.is_empty() {
      return Err("an action's label is empty".into());
    }
    if label.contains('\n') {
      return Err(format!(
        "label {label:?} spans lines: keep it to a few words"
      ));
    }
    let n = label.chars().count();
    if n > LABEL_MAX {
      return Err(format!(
        "label {label:?} is {n} characters: at most {LABEL_MAX}"
      ));
    }
    if out.iter().any(|o| o.label == label) {
      return Err(format!("label {label:?} is given twice"));
    }
    let tool = a.tool.trim().to_owned();
    if tool.is_empty() {
      return Err(format!("{label:?} names no tool"));
    }
    if NOT_ACTIONS.contains(&tool.as_str()) {
      return Err(format!("{label:?}: {tool} cannot be a button"));
    }
    let input = match a.input {
      None | Some(Value::Null) => json!({}),
      Some(v @ Value::Object(_)) => v,
      Some(v) => return Err(format!("{label:?}: input is an object, not {v}")),
    };
    out.push(Action {
      label,
      tool,
      input,
      wake: a.wake,
    });
  }
  Ok(out)
}

/// A transcript's tool call: its buttons, when it is an `offer_actions`
/// whose shape is valid (whether it was accepted is its result's `ok`).
pub fn of_call(name: &str, input: &Value) -> Option<Vec<Action>> {
  (name == TOOL).then(|| validate(input).ok()).flatten()
}

/// The tool call as one line in a transcript: its labels.
pub(crate) fn summary(input: &Value) -> String {
  match validate(input) {
    Ok(a) => format!(
      "{TOOL}: {}",
      a.iter()
        .map(|a| format!("▶ {}", a.label))
        .collect::<Vec<_>>()
        .join(" · ")
    ),
    Err(_) => format!("{TOOL} {input}"),
  }
}

/// Why this turn may not offer buttons: a capability profile that taints
/// it (untrusted content reached it, design §2.28 and docs/events.md).
fn tainted(tools: &NodeTools) -> Option<String> {
  let p = tools.ceiling.profiles().into_iter().find(|p| !p.trusted)?;
  Some(format!(
    "{TOOL} is refused in this turn: it runs under the untrusted capability profile of {} \
     (content from outside reached it), and a button would let that content run a call by the \
     user's press. Offer preset answers (suggest_replies) instead.",
    p.from
      .as_deref()
      .map_or_else(|| "a schedule".to_owned(), |f| format!("subscription {f}"))
  ))
}

/// Whether a read-only thread may offer `a` (what its mode would run).
fn read_only_allows(tools: &NodeTools, a: &Action) -> std::result::Result<(), String> {
  let refused = |why: &str| {
    Err(format!(
      "{:?}: this thread is read-only, so its buttons run only what it may run: {why}",
      a.label
    ))
  };
  match a.tool.as_str() {
    "shell_run" => {
      if a.input.get("host").is_some() {
        return refused("no commands on SSH hosts");
      }
      let cmd = a.input.get("command").and_then(Value::as_str).unwrap_or("");
      if tools.shell.rules.allows(cmd) {
        Ok(())
      } else {
        refused(&format!("`{cmd}` is not a reading command it runs"))
      }
    }
    "otter_run" => Ok(()),
    t if READ_ONLY.contains(&t) => Ok(()),
    t => refused(&format!("{t} may change something")),
  }
}

/// `otter_run` code: checked now, and, in a read-only thread, pure.
async fn otter_checked(
  input: &Value,
  cwd: &Path,
  read_only: bool,
) -> std::result::Result<(), String> {
  use crate::otter::language;
  let req = language::Request::parse(input, cwd)?;
  let cwd = cwd.to_owned();
  let effects = tokio::task::spawn_blocking(move || language::effects(&req, &cwd))
    .await
    .map_err(|e| format!("the check's thread failed: {e}"))??;
  if read_only && effects != otter_core::signature::Effects::NONE {
    return Err(format!(
      "this thread is read-only: its otter_run buttons run pure code only (this has {})",
      language::effects_text(effects)
    ));
  }
  Ok(())
}

/// Checks a set against the thread's tools as they are offered to this
/// turn: no taint, each tool there and allowed, its input fitting its
/// schema, `otter_run` code that checks, and in a read-only thread only
/// what it may run.
pub(crate) async fn check(
  tools: &NodeTools,
  actions: &[Action],
) -> std::result::Result<(), String> {
  if let Some(why) = tainted(tools) {
    return Err(why);
  }
  let defs = tools.definitions();
  let read_only = tools.shell.rules.is_read_only();
  for a in actions {
    let Some(def) = defs.iter().find(|d| d.name == a.tool) else {
      return Err(format!(
        "{:?}: this thread has no tool {} (its buttons run only its own tools)",
        a.label, a.tool
      ));
    };
    if let Some(why) = tools.ceiling.refuses(&a.tool) {
      return Err(format!("{:?}: {why}", a.label));
    }
    if let Some(e) = crate::events::schema_error(&def.input_schema, &a.input, "input") {
      return Err(format!("{:?}: {} {e}", a.label, a.tool));
    }
    if read_only {
      read_only_allows(tools, a)?;
    }
    if a.tool == "otter_run" {
      otter_checked(&a.input, &tools.otter.cwd, read_only)
        .await
        .map_err(|e| format!("{:?}: its code does not check: {e}", a.label))?;
    }
  }
  Ok(())
}

/// The tool: checked against the thread's tools, then shown.
pub(crate) async fn call(tools: &NodeTools, input: &Value) -> ToolOutcome {
  let actions = match validate(input) {
    Ok(a) => a,
    Err(e) => return ToolOutcome::err(e),
  };
  if let Err(e) = check(tools, &actions).await {
    return ToolOutcome::err(e);
  }
  ToolOutcome::ok(format!(
    "Shown: {} under your reply. They stay live until you offer a newer set; a press runs its \
     call without you{}.",
    actions
      .iter()
      .map(|a| format!("[▶ {}]", a.label))
      .collect::<Vec<_>>()
      .join(" "),
    if actions.iter().any(|a| a.wake) {
      " (those with wake then give you the result as the user's message)"
    } else {
      ""
    }
  ))
}

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: TOOL.into(),
    description: format!(
      "Offer the user action buttons under your reply: each runs one tool call of yours when \
       pressed, without a model call, through the same approvals as if you made it. For \
       repetitive things the user would otherwise copy and paste or ask for one by one: the \
       rows of a table (enter this waypoint, open this run), a command to run again. They stay \
       live until you offer a newer set, so the user can press each in turn. `wake: true` also \
       gives you the result as the user's message and a turn; by default you are not woken \
       (you see the presses with their next message). 1 to {MAX} actions; a label is a few \
       words. For answers to a question, use suggest_replies."
    ),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "actions": {
          "type": "array", "minItems": 1, "maxItems": MAX,
          "items": {
            "type": "object", "additionalProperties": false,
            "properties": {
              "label": {"type": "string", "description": "The button: a few words."},
              "tool": {"type": "string", "description": "One of your tools (`otter_run`, `shell_run`, …)."},
              "input": {"type": "object", "description": "Its input, as you would call it."},
              "wake": {"type": "boolean", "description": "Wake you with the result after the press (default false)."}
            },
            "required": ["label", "tool", "input"]
          }
        }
      },
      "required": ["actions"]
    }),
  }
}

/// Which entry's set is live, by index: `entries` in order, each
/// `(has_actions, refused)`; the last accepted set. Unlike preset answers
/// (`replies::live`), neither the user writing nor a turn running ends it:
/// only a newer set (a press made while a turn runs waits for it).
pub fn live(entries: impl IntoIterator<Item = (bool, bool)>) -> Option<usize> {
  let mut at = None;
  for (n, (actions, refused)) in entries.into_iter().enumerate() {
    if actions && !refused {
      at = Some(n);
    }
  }
  at
}

/// A press as the transcript keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Press {
  /// The `offer_actions` call's ID.
  pub set: String,
  pub item: usize,
  pub label: String,
  pub tool: String,
  pub ok: bool,
  pub output: String,
  #[serde(default)]
  pub wake: bool,
}

impl Press {
  /// A `press` event's press.
  pub fn of(e: &Event) -> Option<Press> {
    match e {
      Event::Other(v) if v.get("t").and_then(Value::as_str) == Some(PRESS) => {
        serde_json::from_value(v.clone()).ok()
      }
      _ => None,
    }
  }

  fn event(&self) -> Event {
    let mut v = serde_json::to_value(self).expect("a press serializes");
    v["t"] = PRESS.into();
    v["at"] = jiff::Timestamp::now().to_string().into();
    Event::Other(v)
  }

  /// One line: `▶ label: ok` or `▶ label: error …`.
  pub fn said(&self) -> String {
    if self.ok {
      format!("▶ {}: ok", self.label)
    } else {
      let first = self.output.lines().next().unwrap_or_default();
      format!("▶ {}: error {}", self.label, cut(first, 200))
    }
  }

  /// The note the model reads of a press that did not wake it, with the
  /// user's next message.
  pub(crate) fn noted(&self) -> String {
    format!(
      "{} ({}){}",
      self.said(),
      self.tool,
      if self.ok && !self.output.trim().is_empty() {
        format!(": {}", cut(self.output.trim(), NOTED))
      } else {
        String::new()
      }
    )
  }
}

/// The note on the presses a model was not woken for, put before the
/// user's next message.
pub(crate) fn presses_note(presses: &[String]) -> String {
  format!(
    "[Since your last turn the user pressed action buttons you offered, each running its call \
     without you:\n{}]",
    presses.join("\n")
  )
}

fn cut(s: &str, n: usize) -> String {
  if s.chars().count() <= n {
    s.to_owned()
  } else {
    format!("{}…", s.chars().take(n).collect::<String>())
  }
}

/// What a press returns: whether its call succeeded, and its output (cut
/// for a client).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pressed {
  pub ok: bool,
  pub output: String,
}

/// An offer found in a thread's line: who wrote it, its buttons, and
/// whether it is still the live set.
struct Offer {
  node: String,
  actions: Vec<Action>,
  live: bool,
}

/// Set `set` in `segments` (a thread's line), if offered and accepted.
fn find(segments: &[Segment], set: &str) -> Option<Offer> {
  let mut calls: Vec<(String, String, Vec<Action>)> = Vec::new();
  let mut refused: Vec<String> = Vec::new();
  let mut done: Vec<String> = Vec::new();
  for s in segments {
    for e in &s.events {
      match e {
        Event::ToolCall(c) => {
          if let Some(a) = of_call(&c.name, &c.input) {
            calls.push((c.id.clone(), s.header.node.to_string(), a));
          }
        }
        Event::ToolResult(r) => {
          done.push(r.id.clone());
          if !r.ok {
            refused.push(r.id.clone());
          }
        }
        _ => {}
      }
    }
  }
  // Accepted: a result that is not an error.
  calls.retain(|(id, _, _)| done.contains(id) && !refused.contains(id));
  let last = calls.last().map(|(id, _, _)| id.clone());
  let (id, node, actions) = calls.into_iter().find(|(id, _, _)| id == set)?;
  Some(Offer {
    node,
    actions,
    live: last.as_deref() == Some(id.as_str()),
  })
}

fn offer_of(s: &Shared, thread: &RecordId, set: &str) -> Result<Offer> {
  let (root, _) = s.thread_place(thread)?;
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let line = chat::line_to(&ctx, thread, None)?;
  find(&line.segments, set)
    .ok_or_else(|| Error::Config(format!("thread {thread} offered no action set {set}")))
}

/// The user pressed button `item` of set `set` in `thread`: its call is
/// looked up in the thread's own records (never taken from the client)
/// and run on the node that offered it; here, after the turn running on
/// the thread, if one runs (a press waits for it rather than being
/// refused); there, asked of it (`Question::Press`).
pub(crate) async fn press(
  s: &Arc<Shared>,
  thread: &RecordId,
  set: &str,
  item: usize,
) -> Result<Pressed> {
  let offer = offer_of(s, thread, set)?;
  if offer.node != s.state().node.as_str() {
    return crate::elsewhere::press(s, &offer.node, thread, set, item).await;
  }
  press_here(s, thread, set, item).await
}

/// `press`, on the node that offered the set.
pub(crate) async fn press_here(
  s: &Arc<Shared>,
  thread: &RecordId,
  set: &str,
  item: usize,
) -> Result<Pressed> {
  if crate::fork::current(s, thread) != *thread {
    return Err(Error::Config(format!(
      "thread {thread} goes on in another thread since it was compacted: its buttons are no \
       longer live"
    )));
  }
  let lock = s.detached.locks.of(thread);
  let held = lock.lock_owned().await;
  // Read again under the lock: the turn it waited for may have offered a
  // newer set.
  let offer = offer_of(s, thread, set)?;
  if !offer.live {
    return Err(Error::Config(
      "a newer set of buttons replaced this one: it is no longer live".into(),
    ));
  }
  let Some(action) = offer.actions.get(item).cloned() else {
    return Err(Error::Config(format!(
      "set {set} has {} buttons, no button {item}",
      offer.actions.len()
    )));
  };
  let (root, place) = s.thread_place(thread)?;
  // Another node's loop may be writing the thread now (it was set to run
  // there since): a step written here would fork it.
  {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    if let Some(chat::Unfinished {
      elsewhere: Some(node),
      ..
    }) = chat::unfinished(&ctx, thread)?
      && node != state.node.as_str()
    {
      return Err(Error::Config(format!(
        "thread {thread}'s turn is {node}'s now: press again when it ends"
      )));
    }
  }
  let cwd = {
    root.read_record(thread).ok().and_then(|r| {
      r.fields
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
    })
  };
  let start = s.start_dir(&place, cwd.as_deref().map(Path::new), Some(thread));
  let selection = s.selection(&place, start.as_deref(), Some(thread));
  let model = crate::meta::thread_model(&s.state().index.lock().expect("index lock"), thread);
  let model = crate::modelset::named(s, Some(&place), model.as_deref()).await;
  // The tools a detached turn of the thread would have: its tool set and
  // profiles, its mode, the sandbox, approvals asked as records.
  let tools = crate::detached::tools_for(
    s,
    &root,
    &place,
    s.read_only(thread),
    thread,
    start.as_deref(),
    false,
    &selection,
    &model,
    false,
  );
  tools.prepare().await;
  let call = format!("{set}.{item}");
  let out = tools.call(thread, &call, &action.tool, &action.input).await;
  if let Some(spent) = &out.spent {
    tools.metered(thread, &spent.model, &spent.usage, spent.usd);
  }
  let output = cut(&tools.redact(out.output.clone()), KEPT);
  let press = Press {
    set: set.to_owned(),
    item,
    label: action.label.clone(),
    tool: action.tool.clone(),
    ok: out.ok,
    output: output.clone(),
    wake: action.wake,
  };
  {
    let state = s.state();
    let ctx = ChatContext {
      node: &state.node,
      root: &root,
      index: &state.index,
      blobs: &state.blobs,
    };
    chat::append_events(&ctx, thread, vec![press.event()])?;
  }
  drop(held);
  if action.wake {
    let message = format!(
      "{} ({})\n\n{}",
      press.said(),
      action.call(),
      cut(output.trim(), TOLD)
    );
    crate::weave::send(s, None, thread, message.trim_end(), true, false).await?;
  }
  Ok(Pressed {
    ok: out.ok,
    output: cut(&output, TOLD),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn what_is_refused_says_why() {
    let bad = |v: Value| validate(&v).unwrap_err();
    assert!(bad(json!({"actions": []})).contains("1 to 24"));
    let many: Vec<Value> = (0..25)
      .map(|i| json!({"label": i.to_string(), "tool": "shell_run", "input": {"command": "ls"}}))
      .collect();
    assert!(bad(json!({ "actions": many })).contains("got 25"));
    let one = |a: Value| bad(json!({ "actions": [a] }));
    assert!(one(json!({"label": " ", "tool": "x", "input": {}})).contains("empty"));
    assert!(one(json!({"label": "a\nb", "tool": "x", "input": {}})).contains("spans lines"));
    assert!(one(json!({"label": "x".repeat(61), "tool": "x", "input": {}})).contains("at most 60"));
    assert!(one(json!({"label": "A", "tool": "", "input": {}})).contains("names no tool"));
    assert!(one(json!({"label": "A", "tool": "wait", "input": {}})).contains("cannot be a button"));
    assert!(one(json!({"label": "A", "tool": TOOL, "input": {}})).contains("cannot be a button"));
    assert!(
      one(json!({"label": "A", "tool": "suggest_replies", "input": {}}))
        .contains("cannot be a button")
    );
    assert!(one(json!({"label": "A", "tool": "x", "input": "ls"})).contains("an object"));
    assert!(one(json!({"label": "A", "text": "x", "tool": "x"})).contains("takes"));
    assert!(
      bad(json!({"actions": [
        {"label": "A", "tool": "x", "input": {}},
        {"label": "A ", "tool": "y", "input": {}}
      ]}))
      .contains("twice")
    );
  }

  #[test]
  fn a_set_reads_as_buttons() {
    let v = json!({"actions": [
      {"label": " Dock ", "tool": "otter_run", "input": {"code": "1"}},
      {"label": "Status", "tool": "shell_run", "input": {"command": "uptime"}, "wake": true},
      {"label": "Bare", "tool": "jobs"}
    ]});
    let a = validate(&v).unwrap();
    assert_eq!(a[0].label, "Dock");
    assert!(!a[0].wake && a[1].wake);
    assert_eq!(a[2].input, json!({}));
    assert_eq!(a[1].call(), "$ uptime");
    assert_eq!(summary(&v), "offer_actions: ▶ Dock · ▶ Status · ▶ Bare");
    assert!(of_call("suggest_replies", &v).is_none());
    assert_eq!(of_call(TOOL, &v).unwrap().len(), 3);
  }

  #[test]
  fn the_latest_accepted_set_stays_live() {
    // (has actions, refused)
    let e = [
      (false, false),
      (true, false),
      (false, false),
      (false, false),
    ];
    // The user writing after it, or a turn: still live.
    assert_eq!(live(e), Some(1));
    // A newer set replaces it; a refused one does not.
    let e = [(true, false), (false, false), (true, false), (true, true)];
    assert_eq!(live(e), Some(2));
    assert_eq!(live([]), None);
  }

  #[test]
  fn a_press_is_said_in_a_line_and_noted_for_the_model() {
    let p = Press {
      set: "c1".into(),
      item: 0,
      label: "Dock".into(),
      tool: "otter_run".into(),
      ok: true,
      output: "docked at A".into(),
      wake: false,
    };
    assert_eq!(p.said(), "▶ Dock: ok");
    assert_eq!(p.noted(), "▶ Dock: ok (otter_run): docked at A");
    let e = p.event();
    assert_eq!(Press::of(&e), Some(p.clone()));
    let failed = Press {
      ok: false,
      output: "no such waypoint\nmore".into(),
      ..p
    };
    assert_eq!(failed.said(), "▶ Dock: error no such waypoint");
  }
}
