//! The TUI's state, without the terminal or the node. The conversation goes
//! into the terminal's own scrollback as plain text (`out`, printed by the
//! event loop), so the terminal wraps, scrolls, searches and copies it; the
//! app draws only the live part at the bottom: what is streaming, overlays,
//! the input and the status line.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use stitch_node::api::{Job, Reply};
use stitch_node::tools::Decision;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadItem {
  pub id: String,
  pub title: String,
  pub when: String,
  /// A detached thread's state (`running`, `waiting`, `done`, ...).
  pub state: Option<String>,
  /// Its last activity: the open thread is re-read when it moves.
  pub at: jiff::Timestamp,
  /// Where it belongs, when that is not where the TUI runs.
  pub place: Option<String>,
  /// Its place's key (see `stitch_node::place::Place::key`).
  pub key: String,
  /// Pinned: first in lists.
  pub pinned: bool,
  /// The thread it hangs under in the list (`meta::parent`).
  pub parent: Option<String>,
  /// A spawned thread's goal, its first line.
  pub goal: Option<String>,
}

/// A thread's place in the tree, read from the store whatever place it
/// belongs to: its title, the thread above it, (id, title): the one that
/// spawned it, else the one it reports to, followed to its newest
/// compaction (`/parent`, Ctrl+P), and how many are below it
/// (`/children`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Family {
  pub id: String,
  pub title: String,
  /// Where it belongs, when that is not where the TUI runs.
  pub place: Option<String>,
  pub parent: Option<(String, String)>,
  pub children: usize,
}

/// Something on the attention list; a question can be answered here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Needs {
  pub question: Option<String>,
  pub text: String,
  /// What answering "always" allows from then on (empty: y or n only).
  pub grant: Vec<String>,
  /// What to read before answering (a fixer's landing: the node's facts
  /// and diff), printed into the scrollback with `v`.
  pub detail: Option<String>,
}

/// What the TUI does with a turn it lets go of (another thread opened,
/// `/new`, while it ran): the turn goes on at the node.
const WAITING: &str = "A turn already runs on this thread (a job's report woke it, or another client sent to it): yours goes into it after its tool calls, with what you type meanwhile, or is sent when it ends. What it does shows here meanwhile.";
const LET_GO: &str = "The turn goes on in the background: its approvals wait in Ctrl+T, its reply is in its thread (Ctrl+O).";

/// One message of a stored transcript: who and what.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
  pub who: String,
  pub text: String,
  /// The text without the lines naming its attachments (for editing it
  /// again); empty: `text`.
  pub said: String,
  /// A message's ID (`m12`, what a fork's `upto` takes); tool calls,
  /// results and notes have none.
  pub id: Option<String>,
  /// When a message was written.
  pub at: Option<jiff::Timestamp>,
  /// How many files it had attached.
  pub attachments: usize,
  /// A `suggest_replies` call's preset answers (design §2.30): `/1`… sends
  /// one while they are live (`App::live_replies`).
  pub replies: Vec<stitch_node::replies::Suggested>,
  /// An `offer_actions` call's buttons (design §2.30, step 2): `/1`… runs
  /// one while they are the latest set (`App::live_actions`).
  pub actions: Vec<stitch_node::actions::Action>,
  /// A tool call's ID (an action set's, which a press names).
  pub call: Option<String>,
}

impl Entry {
  /// What was said, without the attachment lines.
  fn said(&self) -> &str {
    if self.said.is_empty() {
      &self.text
    } else {
      &self.said
    }
  }

  /// A message the picker offers: the user's, or a model's reply (not a
  /// tool's output, memory's note, a fork's heading).
  fn pickable(&self) -> bool {
    self.id.is_some()
      && !matches!(
        self.who.as_str(),
        "tool"
          | "output"
          | "error"
          | "refusal"
          | "memory"
          | "fork"
          | "compacted"
          | "forked"
          | "context"
          | "system"
          | "press"
          | "note"
      )
  }
}

/// Something to print into the scrollback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Printed {
  You(String),
  Reply {
    model: String,
    text: String,
  },
  /// A streaming reply: its heading, its completed lines, its end.
  ReplyStart(String),
  ReplyLines(String),
  ReplyEnd,
  /// A tool call, as one line.
  Tool(String),
  Output(String),
  Error(String),
  Refusal(String),
  /// Information from Stitch itself (sign-in URLs, hints).
  Notice(String),
  /// A thread opens here.
  Separator(String),
  /// The thread was compacted here (design §2.15): its divider
  /// (`Compaction::said`), and the summary it continues from, dimmed.
  Compacted {
    line: String,
    summary: String,
  },
}

impl Printed {
  /// A stored transcript entry, as it prints.
  pub fn from_entry(e: &Entry) -> Printed {
    match e.who.as_str() {
      "you" => Printed::You(e.text.clone()),
      "tool" => Printed::Tool(e.text.clone()),
      // An action button pressed: its line, its output's first lines.
      "press" => Printed::Tool(e.text.clone()),
      "output" => Printed::Output(e.text.clone()),
      "error" | "failed" => Printed::Error(e.text.clone()),
      "refusal" => Printed::Refusal(e.text.clone()),
      // A room's note (a participant's question settled, design §2.27).
      "note" => Printed::Notice(e.text.clone()),
      // Memory's own message (a follow-up's note).
      "memory" => Printed::Notice("memory: an afterthought".into()),
      // What a fork continues from (`/fork`, `/summarize`, `/compact`).
      "fork" => Printed::Notice(e.text.clone()),
      // Where the model's reading of what a fork continues starts.
      "context" => Printed::Notice(e.text.clone()),
      // Where its compaction continued it, or where a fork leaves what it
      // continues (`chat::lineage_shown`): a dimmed divider, its summary.
      "compacted" | "forked" => Printed::Compacted {
        line: e.said().to_owned(),
        summary: e
          .text
          .strip_prefix(e.said())
          .unwrap_or_default()
          .trim()
          .to_owned(),
      },
      who => Printed::Reply {
        model: who.to_owned(),
        text: e.text.clone(),
      },
    }
  }
}

/// Which threads the picker lists; Tab goes to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadScope {
  /// This place's and global ones.
  #[default]
  HereAndGlobal,
  /// This place's only.
  Here,
  /// Every place's.
  All,
}

impl ThreadScope {
  pub fn next(self) -> ThreadScope {
    match self {
      ThreadScope::HereAndGlobal => ThreadScope::Here,
      ThreadScope::Here => ThreadScope::All,
      ThreadScope::All => ThreadScope::HereAndGlobal,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
  None,
  Threads,
  Attention,
  /// What runs in the background (`App::jobs`).
  Jobs,
  Auth,
  /// A list to pick from (`App::pick`).
  Pick,
  /// The open thread's messages, to fork from, edit or copy (Esc Esc;
  /// `App::msgs`).
  Messages,
  /// The fork's options as a form (`/fork`, `/compact` alone, the message
  /// picker's `o`; `App::fork`).
  Fork,
}

/// A row of the fork form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkRow {
  /// All of it, or its last N messages.
  Range,
  /// Up to the message picked (only when opened from the picker).
  Upto,
  Attachments,
  Tools,
  Summary,
}

/// The fork form: what `/fork [N] [-a] [-t] [-s] [@m12]` types, picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkForm {
  pub thread: String,
  /// Its last `last` messages, else all of it.
  pub last_on: bool,
  /// N as typed (digits).
  pub last: String,
  /// N was typed into since the form opened: the first digit replaces the
  /// preset one.
  pub last_typed: bool,
  /// The message picked in the message picker, and whether the fork stops
  /// there.
  pub upto: Option<String>,
  pub upto_on: bool,
  pub no_attachments: bool,
  pub no_tools: bool,
  pub summarize: bool,
  /// The row selected, among `rows()`.
  pub row: usize,
  /// The command it stands for (`fork`, `compact`).
  pub name: &'static str,
}

impl ForkForm {
  fn new(thread: String, name: &'static str) -> ForkForm {
    ForkForm {
      thread,
      last_on: false,
      last: "10".into(),
      last_typed: false,
      upto: None,
      upto_on: false,
      no_attachments: false,
      no_tools: false,
      summarize: false,
      row: 0,
      name,
    }
  }

  pub fn rows(&self) -> Vec<ForkRow> {
    let mut rows = vec![ForkRow::Range];
    if self.upto.is_some() {
      rows.push(ForkRow::Upto);
    }
    rows.extend([ForkRow::Attachments, ForkRow::Tools, ForkRow::Summary]);
    rows
  }

  fn toggle(&mut self, row: ForkRow) {
    match row {
      ForkRow::Range => self.last_on = !self.last_on,
      ForkRow::Upto => self.upto_on = !self.upto_on,
      ForkRow::Attachments => self.no_attachments = !self.no_attachments,
      ForkRow::Tools => self.no_tools = !self.no_tools,
      ForkRow::Summary => self.summarize = !self.summarize,
    }
  }

  /// N, when the last messages are asked for and N is a count.
  fn count(&self) -> Option<usize> {
    self
      .last_on
      .then(|| self.last.parse::<usize>().ok())
      .flatten()
      .filter(|n| *n > 0)
  }

  /// The same fork as typed: the fast path, shown under the form.
  pub fn typed(&self) -> String {
    let mut out = "/fork".to_owned();
    if self.upto_on
      && let Some(m) = &self.upto
    {
      out.push_str(&format!(" @{m}"));
    }
    if self.last_on {
      out.push_str(&format!(" {}", self.last));
    }
    for (on, flag) in [
      (self.no_attachments, "-a"),
      (self.no_tools, "-t"),
      (self.summarize, "-s"),
    ] {
      if on {
        out.push(' ');
        out.push_str(flag);
      }
    }
    out
  }
}

/// What a fork would keep of the transcript the TUI holds, as the node
/// shapes it (`chat::shaped`: up to a message with the tool calls that
/// followed it, tool calls dropped, its last N messages counted back to
/// one of the user's): an estimate, without asking the node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForkKeeps {
  /// Messages kept, and of how many.
  pub kept: usize,
  pub of: usize,
  /// Messages the summary stands for.
  pub summarized: usize,
  /// Characters kept (what was said, tool calls and their output).
  pub chars: usize,
  pub attachments: usize,
}

pub fn fork_keeps(entries: &[Entry], f: &ForkForm) -> ForkKeeps {
  // The fork's own notice is not a message; what came before the last
  // compaction is not this thread's (it inherits a summary of it).
  let own = entries
    .iter()
    .rposition(|e| e.who == "compacted" || e.who == "forked")
    .map_or(0, |i| i + 1);
  let mut es: Vec<&Entry> = entries[own..]
    .iter()
    .filter(|e| e.who != "fork" && e.who != "context")
    .collect();
  if f.upto_on
    && let Some(m) = &f.upto
    && let Some(i) = es.iter().position(|e| e.id.as_deref() == Some(m))
  {
    // With the tool calls (and results) that followed it.
    let mut end = i + 1;
    while end < es.len() && (es[end].id.is_none() || es[end].who == "output") {
      end += 1;
    }
    es.truncate(end);
  }
  if f.no_tools {
    es.retain(|e| !matches!(e.who.as_str(), "tool" | "output" | "error"));
  }
  let keep = match (f.last_on, f.summarize) {
    (true, _) => Some(f.count().unwrap_or(0)),
    (false, true) => Some(0),
    (false, false) => None,
  };
  let msgs: Vec<usize> = (0..es.len()).filter(|&i| es[i].id.is_some()).collect();
  let mut start = match keep {
    None => 0,
    Some(0) => es.len(),
    Some(n) if msgs.len() > n => msgs[msgs.len() - n],
    Some(_) => 0,
  };
  if keep.is_some() {
    while start < es.len() && !(es[start].who == "you" && es[start].id.is_some()) {
      start += 1;
    }
  }
  let count = |s: &[&Entry]| s.iter().filter(|e| e.id.is_some()).count();
  let kept = &es[start..];
  ForkKeeps {
    kept: count(kept),
    of: msgs.len(),
    summarized: if f.summarize { count(&es[..start]) } else { 0 },
    chars: kept
      .iter()
      .map(|e| {
        if f.no_attachments {
          e.said().chars().count()
        } else {
          e.text.chars().count()
        }
      })
      .sum(),
    attachments: if f.no_attachments {
      0
    } else {
      kept.iter().map(|e| e.attachments).sum()
    },
  }
}

/// A rough token count for `chars` characters (four to a token).
fn tokens_said(chars: usize) -> String {
  let t = chars.div_ceil(4);
  if t >= 1000 {
    format!("~{:.1}k tokens", t as f64 / 1000.0)
  } else {
    format!("~{t} tokens")
  }
}

/// The message picker: which message, and its actions when open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MsgPick {
  /// Among the open thread's pickable messages (`App::pickable`).
  pub picked: usize,
  /// The actions list, open, and the action selected in it.
  pub menu: Option<usize>,
}

/// What can be done with a picked message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgAction {
  Fork,
  /// The fork form, up to it.
  ForkOptions,
  /// A message of the user's: the thread rewound to the message before
  /// it, its text back in the input.
  Edit,
  /// The thread rewound to it; a message of the user's is sent again.
  Rewind,
  Copy,
}

impl MsgAction {
  fn label(self) -> &'static str {
    match self {
      MsgAction::Fork => "Fork from here",
      MsgAction::ForkOptions => "Fork from here with options… (last N, -a, -t, -s)",
      MsgAction::Edit => {
        "Edit and resend (the thread rewound to just before it, its text in the input)"
      }
      MsgAction::Rewind => {
        "Rewind to here (what came after is set aside; your message is sent again)"
      }
      MsgAction::Copy => "Copy its text",
    }
  }

  fn key(self) -> char {
    match self {
      MsgAction::Fork => 'f',
      MsgAction::ForkOptions => 'o',
      MsgAction::Edit => 'e',
      MsgAction::Rewind => 'r',
      MsgAction::Copy => 'c',
    }
  }
}

/// What the status line says of a message's attachments not carried.
fn lost_attachments(n: usize) -> String {
  match n {
    0 => String::new(),
    1 => " · its attachment is not carried: /attach it again".into(),
    n => format!(" · its {n} attachments are not carried: /attach them again"),
  }
}

/// The `@` file picker, open over the input: the `@` typed at `start` (in
/// characters), what follows it up to the cursor filtering the files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AtPick {
  pub start: usize,
  pub picked: usize,
}

/// A file named with `@` and attached: removing `@path` from the input
/// drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
  /// As written after the `@` (relative).
  pub path: String,
  /// As attached.
  pub attached: String,
}

/// One entry of a picker.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
  pub value: String,
  pub label: String,
  pub detail: String,
}

impl Choice {
  pub fn new(value: impl Into<String>, detail: impl Into<String>) -> Choice {
    let value = value.into();
    Choice {
      label: value.clone(),
      value,
      detail: detail.into(),
    }
  }
}

/// What a picked choice does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
  /// Run (or start typing) a command.
  Command,
  Model,
  /// The node to spawn on; its model is picked next.
  SpawnNode,
  /// The model on that node; the goal is typed next.
  SpawnModel,
  /// The thread to send to; the message is typed next.
  SendTo,
  /// The node whose folder to link; its path is typed next.
  Link,
  Login,
  Logout,
  Memory,
  /// A tool of the thread's set: Enter turns it on or off, the list
  /// staying open.
  Tools,
  Skills,
  /// A search's hit: its thread opens.
  OpenThread,
}

/// A list to pick from: typing filters it, ↑↓ moves, Enter picks.
#[derive(Clone, Debug)]
pub struct Picker {
  pub title: String,
  pub then: Then,
  pub choices: Vec<Choice>,
  pub filter: String,
  pub picked: usize,
  /// Its choices are on their way (from the node).
  pub waiting: bool,
  /// A line under the list (why something is missing).
  pub note: String,
}

impl Picker {
  fn new(title: &str, then: Then, choices: Vec<Choice>) -> Picker {
    Picker {
      title: title.into(),
      then,
      choices,
      filter: String::new(),
      picked: 0,
      waiting: false,
      note: String::new(),
    }
  }

  /// The choices the filter lets through: every word of it in the label
  /// or the detail.
  pub fn shown(&self) -> Vec<&Choice> {
    let words: Vec<String> = self
      .filter
      .split_whitespace()
      .map(str::to_lowercase)
      .collect();
    self
      .choices
      .iter()
      .filter(|c| {
        let hay = format!("{} {}", c.label, c.detail).to_lowercase();
        words.iter().all(|w| hay.contains(w))
      })
      .collect()
  }
}

/// What the TUI asks the node for, to fill a picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
  /// This node's models, or another's.
  Models(Option<String>),
  Peers,
  /// The open thread's tool or skill set (a new thread's, with the
  /// changes kept for it); `tools` and `skills` are changes to make first.
  Toolset {
    skills: bool,
    thread: Option<String>,
    /// The new thread belongs to no folder.
    global: bool,
    tools: Vec<String>,
    skill_changes: Vec<String>,
  },
  /// Messages of every thread (`here`: this place's) matching a query.
  Search {
    query: String,
    here: bool,
  },
}

/// The commands, for the palette: (command, what it does, whether words
/// are typed after it).
const COMMANDS: &[(&str, &str, bool)] = &[
  ("/model", "pick the model for the next messages", false),
  (
    "/spawn",
    "start a thread toward a goal, here or on another machine",
    false,
  ),
  ("/send", "continue any thread with a message", false),
  (
    "/fork",
    "a new thread continuing this one (alone: a form; or N, -a, -t, -s, @m12)",
    false,
  ),
  (
    "/summarize",
    "a new thread continuing this one from a summary of it",
    false,
  ),
  (
    "/compact",
    "a new thread: a summary, then the last [N] messages (alone: the form, 10)",
    false,
  ),
  ("/threads", "pick a thread", false),
  (
    "/parent",
    "open the thread that spawned this one (Ctrl+P)",
    false,
  ),
  (
    "/children",
    "the threads this one spawned: pick one to open",
    false,
  ),
  ("/rename", "name this thread", true),
  ("/pin", "pin this thread first in lists, or unpin it", false),
  (
    "/quote",
    "a new thread given this one as quoted material",
    false,
  ),
  (
    "/search",
    "every thread's messages, by words and meaning (--here: this place's)",
    true,
  ),
  ("/jobs", "what runs in the background", false),
  ("/new", "a new thread here", false),
  ("/new global", "a new thread in no folder", false),
  ("/attach", "attach a file to the next message", true),
  ("/drop", "drop the attached files", false),
  ("/memory", "this thread in memory, or out of it", false),
  (
    "/run-on",
    "run this thread on another node, its turns streaming here (`here`: back)",
    true,
  ),
  (
    "/read-only",
    "this thread read-only (on) or writable (off)",
    true,
  ),
  ("/tools", "this thread's tools: turn them on and off", false),
  (
    "/skills",
    "this thread's skills: turn them on and off",
    false,
  ),
  (
    "/link",
    "make this folder the same as another node's",
    false,
  ),
  ("/unlink", "keep this folder to this node", false),
  ("/copy", "copy the last reply", false),
  (
    "/app",
    "open the latest app this thread showed in the browser (or a pinned app, by name)",
    true,
  ),
  (
    "/ask",
    "approvals: what reaches beyond the sandbox (or the allowed set) asks",
    false,
  ),
  (
    "/auto",
    "approvals: everything runs, unless the hazard guard judges it severe",
    false,
  ),
  (
    "/yolo",
    "approvals: everything runs, unjudged (/yolo off: ask)",
    false,
  ),
  ("/login", "sign in to a subscription, or GitHub", false),
  ("/logout", "sign out of a subscription", false),
  ("/auth", "subscription sign-in status", false),
  ("/help", "the commands, in the scrollback", false),
  ("/quit", "leave", false),
];

/// A message on its way: what streams until it is printed.
#[derive(Clone, Debug, Default)]
pub struct Pending {
  /// The reply so far, whole (for copying).
  pub reply: String,
  /// Its line still being written; completed lines are printed.
  pub partial: String,
  /// Its heading (the model) has been printed.
  pub started: bool,
  /// The Markdown of the lines printed so far, as the printer has it (an
  /// open fence, a table held): the live area renders `partial` after it.
  pub md: super::markdown::Md,
  pub thinking: String,
  /// Another turn runs on the thread: this one waits for it to end (the
  /// thread is followed meanwhile).
  pub waiting: bool,
  /// The tool call running (its line), till its result.
  pub tool: Option<String>,
  /// When what it does now began (sent, waiting, a tool call), for the
  /// time shown next to the spinner.
  pub since: Option<std::time::Instant>,
  /// It failed, but its message was written to the thread (`Reply::Kept`):
  /// it is not given back to be sent again.
  pub kept: bool,
  /// Sent as a new thread's first message.
  pub new: bool,
  /// The message sent, as typed (printed when sent).
  pub sent: String,
}

impl Pending {
  fn now_doing(&mut self) {
    self.since = Some(std::time::Instant::now());
  }

  /// What it does, as the spinner's line says it.
  fn doing(&self) -> String {
    if self.waiting {
      "waiting for the turn already running on this thread".into()
    } else if let Some(tool) = &self.tool {
      format!("running {}", clip(tool, 60))
    } else if !self.partial.is_empty() {
      "writing".into()
    } else if !self.thinking.is_empty() {
      "thinking".into()
    } else if self.started {
      "working".into()
    } else {
      "sending".into()
    }
  }
}

/// The lines of a running call's input the live area shows.
const IN_FLIGHT_ROWS: usize = 8;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What the event loop must do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
  Auth(AuthAction),
  None,
  Quit,
  /// The user's answer to the approval on screen.
  Approve(Decision),
  /// Put this text on the clipboard.
  Copy(String),
  /// Attach the clipboard's image or files (Ctrl+V).
  PasteClipboard,
  /// Continue a thread (print its transcript), or start a new one.
  Open(Option<String>),
  /// List the threads again (here only, or all).
  ListThreads,
  /// Make this folder the same as another (`node:path`), or (`None`) keep
  /// it to this node.
  Link(Option<String>),

  /// Ask the node for a picker's choices.
  Fetch(Fetch),
  /// List the threads below this one, to open one (`show_children`).
  Children(String),
  /// Shell commands run without asking, or ask again.
  /// The approval mode: `ask`, `auto` or `yolo`.
  Mode(String),
  /// Start a thread toward a goal, reporting to the open thread and
  /// starting from its conversation: on `node` (this one when `None`), on
  /// `model` (that node's default when `None`).
  Spawn {
    node: Option<String>,
    model: Option<String>,
    goal: String,
  },
  /// Name a thread.
  Rename {
    thread: String,
    title: String,
  },
  /// Pin a thread, or unpin it.
  Pin {
    thread: String,
    on: bool,
  },
  /// Press an action button (design §2.30, step 2): the node runs its
  /// call; its line comes with the transcript.
  Press {
    thread: String,
    set: String,
    item: u32,
  },
  /// Ask the node what deleting a thread takes (a dry run: its
  /// compaction chain), to ask the user.
  DeletePreview(String),
  /// Delete a thread for good (with its compaction chain), on every node.
  Delete(String),
  /// Rewind a thread to a message (its start: `None`); `resend`: then
  /// send this text again.
  Rewind {
    thread: String,
    upto: Option<String>,
    resend: Option<String>,
  },
  /// A new thread with this one attached as quoted material.
  Quote(String),
  /// Fork the open thread (`/fork`, `/summarize`, `/compact`), then open
  /// the fork.
  Fork {
    thread: String,
    /// Up to this message (`m12`); the latest when `None`.
    upto: Option<String>,
    last: Option<usize>,
    no_attachments: bool,
    no_tools: bool,
    summarize: bool,
  },
  /// Continue a thread with a message; its reply is reported to the open
  /// thread.
  SendTo {
    thread: String,
    text: String,
  },
  /// Take the open thread out of memory, or bring it back (a new one's
  /// choice goes with its first message, `Send::memory`).
  Remember {
    thread: Option<String>,
    on: bool,
  },
  /// Open an app in the browser: a pinned one, a thread's latest, a
  /// version (`Request::AppOpen`).
  OpenApp(String),
  /// Have a thread run on a node (`None`: back to where it is typed).
  RunOn {
    thread: String,
    node: Option<String>,
  },
  /// Switch the open thread to read-only, or writable.
  ReadOnly {
    thread: String,
    on: bool,
  },
  /// Answer a question from the attention list.
  Answer {
    question: String,
    decision: Decision,
  },
  Send {
    thread: Option<String>,
    text: String,
    attachments: Vec<String>,
    model: Option<String>,
    /// A new thread that belongs to no folder.
    global: bool,
    /// Changes to a new thread's tool and skill sets.
    tools: Vec<String>,
    skills: Vec<String>,
    /// A new thread's memory, when `/memory` chose it.
    memory: Option<bool>,
  },
  /// A message for the reply that runs, woven in between its steps.
  Interject {
    id: u64,
    text: String,
    attachments: Vec<String>,
  },
  /// Take back a message not woven yet.
  Retract(u64),
  /// Stop the reply that runs.
  Interrupt,
  /// Stop the turn running on this thread, whoever started it (`Cancel`);
  /// a message of this TUI's waiting behind it is interrupted unsent.
  Cancel(String),
  /// Ask the node what runs in the background.
  Jobs,
  /// Stop a background job (`Job::id`).
  StopJob(String),
}

/// A message typed while a reply ran: given to it between its steps, or
/// sent when it ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
  pub id: u64,
  pub text: String,
  pub attachments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthAction {
  Login { provider: String, file: bool },
  Logout { provider: String },
  Status { provider: Option<String> },
}

/// Tool output lines printed into the scrollback, at most.
const OUTPUT_LINES: usize = 12;
/// Rows the input box grows to before it scrolls.
const INPUT_ROWS: u16 = 10;

/// The slash commands, one per line.
const HELP: &str = "\
/                         the commands, to pick from
/model [provider/model]   pick from the providers' models, or name one
/spawn [@node[:model]] <goal>
                          start a thread toward a goal, from this conversation, here or
                          on another machine (alone: pick the machine, then its model)
/send [thread] [message]  continue any thread; its reply comes back here
/threads                  pick a thread (Tab: here and global, here only, all;
                          Ctrl+T: what waits on you)
/parent                   open the thread that spawned this one, or that it reports to
                          (Ctrl+P; the status line shows ↑ and its title on a child)
/children                 the threads this one spawned, or that report to it: Enter opens one
                          (the status line shows ↓ and how many)
/rename <title>           name this thread
/pin                      pin this thread first in lists (again: unpin)
/quote                    a new thread with this one attached, as quoted material
/search [--here] <words>  every thread's messages, by their words and meaning; Enter opens one
/jobs                     what runs in the background: shell jobs, detached threads (Ctrl+B)
/new [global]             a new thread here, or in no folder
/fork [N] [-a] [-t] [-s] [@m12]
                          a new thread continuing this one: all of it or its last N messages,
                          -a without attachments, -t without tool calls, -s from a summary of
                          what is not kept, @m12 up to that message (Esc Esc picks it); alone,
                          a form: ↑↓ Tab rows, Space changes one, digits N, a t s (u) switch,
                          what it keeps said as it changes, Enter forks, Esc cancels
/summarize                a new thread continuing this one from a summary of it
/compact [N]              a new thread: a summary, then the last N messages (10) as they were
                          (alone: the fork form, so preset)
/attach <file>            attach a file to the next message (/drop clears)
/memory [on|off]          this thread in memory or out of it (the status line shows which)
/read-only [on|off]       this thread read-only (its shells write only temp) or writable
/run-on <node|here>       run this thread's turns on another node, streaming here (here: back)
/tools, /skills           this thread's tools and skills: Enter turns one on or off
/link [--force] node:path make this folder the same as another node's
/unlink                   keep this folder to this node
/ask, /auto, /yolo        the approval mode (the status line shows auto and yolo):
                          ask: what reaches beyond asks; auto: everything runs unless
                          the hazard guard judges it severe (no verdict: not run);
                          yolo: everything runs, unjudged (/yolo off, /auto off: ask)
/copy                     copy the last reply
/1 … /6                   send a preset reply the agent offered (listed ↳ after its reply),
                          while nothing was written since
/app [name]               open the latest app this thread showed (or a pinned one) in the browser
/login [provider]         sign in to a subscription, or github (--file: no keychain);
                          antigravity: agy signs in from a shell (`stitch auth login antigravity`)
/logout [provider]        sign out
/auth [provider]          subscription sign-in status
/help                     this list
(keys)                    a new line: Shift+Enter, Alt+Enter, Ctrl+J, or \\ then Enter
(Esc Esc)                 the open thread's messages (on an empty input): ↑↓ picks one, Enter
                          its actions: f fork from here, o the fork form, e edit and resend
                          (yours: a fork just before it, its text in the input), c copy
(@)                       a file of this folder (git's files, or a walk): typing filters, ↑↓,
                          Tab or Enter picks, Esc closes; a text file up to 256 KB is attached
                          while @path stays in the text, others are named for the agent to read
/quit                     leave";

/// How soon a second Esc must follow the first to open the message picker.
const ESC_AGAIN: std::time::Duration = std::time::Duration::from_millis(1000);

/// Files `@` offers, at most (a directory walk stops there).
const FILES_MAX: usize = 20_000;
/// The largest file `@` attaches; a bigger one is named only.
const ATTACH_MAX: u64 = 256 * 1024;
/// Rows the `@` picker shows.
const AT_ROWS: usize = 8;

/// How long after the user's last keystroke (or a prompt appearing) the
/// prompt's keys are still taken as typing.
const TYPING_GRACE: std::time::Duration = std::time::Duration::from_millis(1200);

pub struct App {
  pub node: String,
  pub default_model: String,
  /// `None`: the configured default.
  pub model: Option<String>,
  /// The thread being continued; `None` starts a new one.
  pub thread: Option<String>,
  pub threads: Vec<ThreadItem>,
  /// The open thread's title, parent and children, read from the store
  /// as it opens and at each poll (the status line's ↑ and ↓); one of
  /// another thread is ignored.
  pub family: Option<Family>,
  /// Where the TUI runs: a project, a folder or `global`.
  pub here: String,
  /// The next new thread belongs to no folder (`/new global`).
  pub global: bool,
  /// Which threads the thread list shows.
  pub scope: ThreadScope,
  /// In the thread picker: 0 is "new thread", then the threads.
  pub picked: usize,
  /// In the thread picker: the thread Delete asks to delete, till y or n.
  pub deleting: Option<String>,
  /// In the thread picker: the words typed, which filter it.
  pub thread_filter: String,
  /// In the thread picker: the threads whose spawns the user opened, or
  /// closed (→ ←); the rest are open while something below them works or
  /// waits on the user. For the session.
  pub opened: std::collections::HashSet<String>,
  pub closed: std::collections::HashSet<String>,
  pub input: String,
  /// Where in `input` typing goes, in characters; `None`: at its end.
  pub cursor: Option<usize>,
  pub attachments: Vec<String>,
  pub overlay: Overlay,
  pub attention: Vec<Needs>,
  /// What runs in the background, as the node last said.
  pub jobs: Vec<Job>,
  pub picked_job: usize,
  /// The selected entry of the attention list.
  pub picked_needs: usize,
  /// Declining a question from the attention list (with `reason`).
  pub answering: Option<String>,
  pub pending: Option<Pending>,
  /// The open thread's step whose calls run now, as the node keeps it
  /// (`chat::step_in_flight`), for a turn this TUI did not start: the
  /// steps are written whole once their calls return, so a long command
  /// showed nothing but "a turn is running" (2026-11-01). Its thread.
  pub in_flight: Option<(String, stitch_node::chat::StepInFlight)>,
  pub status: String,
  pub auth_pending: bool,
  pub auth_text: String,
  /// A command waiting for approval: (session, command).
  pub approval: Option<(String, String)>,
  /// Its question record, on the attention list as well.
  pub approval_question: Option<String>,
  /// What answering it "always" allows from then on.
  pub approval_grant: Vec<String>,
  /// When it appeared, and when the user last typed: keys typed for a
  /// message are not taken as its answer (`TYPING_GRACE`).
  pub approval_at: Option<std::time::Instant>,
  pub typed_at: Option<std::time::Instant>,
  /// Questions already told of, or there when the TUI started (the count
  /// in the status line has those).
  pub told: std::collections::HashSet<String>,
  /// Declining it: the reason being typed.
  pub reason: Option<String>,
  /// The last full reply, for copying.
  pub last_reply: Option<String>,
  /// How much of the open thread's transcript is in the scrollback, and
  /// the activity it was read at: what the thread gains while nobody here
  /// chats (a detached thread's report waking it) is printed as it lands.
  pub shown: (usize, Option<jiff::Timestamp>),
  /// A message printed when sent that waits for another turn on the
  /// thread and may be woven into it: as that turn is followed, its
  /// message from the user that starts with it is printed without it.
  pub echo: Option<String>,
  /// To print into the scrollback, in order; the event loop drains it.
  pub out: Vec<Printed>,
  /// The open picker.
  pub pick: Option<Picker>,
  /// Shell commands run without asking (as the node last said).
  /// The approval mode, as the node said it: `ask`, `auto` or `yolo`.
  pub mode: String,
  /// Messages typed while a reply ran: woven into it between its steps;
  /// what it did not take is sent, as one, when it ends.
  pub queued: Vec<Queued>,
  /// The last queued message's ID.
  queued_id: u64,
  /// The queued message taken back to be edited.
  editing: Option<Queued>,
  /// The node whose model is being picked, to delegate to.
  pub delegate_to: Option<String>,
  /// Whether the open thread (or a new one) is in memory; `None` when
  /// memory does not run on the node.
  pub memory: Option<bool>,
  /// Whether the open thread is read-only (the status line says so).
  pub read_only: bool,
  /// The node the open thread runs on, when the user had it run on one
  /// (`/run-on`; the status line says so).
  pub runs_on: Option<String>,
  /// Changes to a new thread's tool and skill sets, sent with its first
  /// message.
  pub new_tools: Vec<String>,
  pub new_skills: Vec<String>,
  /// A new thread's memory as `/memory` chose it, sent with its first
  /// message (there is no thread yet to switch).
  pub new_memory: Option<bool>,
  /// The open thread's transcript as last read (the message picker's).
  pub entries: Vec<Entry>,
  /// The entry of `entries` whose preset answers were last said in the
  /// scrollback (design §2.30): each live set is said once.
  pub replies_said: Option<usize>,
  /// The entry whose action buttons were last said (design §2.30, step 2).
  pub actions_said: Option<usize>,
  /// The message picker, while `Overlay::Messages` shows.
  pub msgs: MsgPick,
  /// The fork form, while `Overlay::Fork` shows.
  pub fork: Option<ForkForm>,
  /// When Esc was pressed on an empty input: a second one soon after
  /// opens the message picker.
  esc_at: Option<std::time::Instant>,
  /// The `@` file picker, while open.
  pub at_pick: Option<AtPick>,
  /// The files `@` offers, relative to `files_root`.
  pub files: Vec<String>,
  /// Where the TUI runs: the files are listed again from it each time `@`
  /// opens (`None`: `files` as they are, for tests).
  pub files_root: Option<std::path::PathBuf>,
  /// Files named with `@` and attached.
  pub mentions: Vec<Mention>,
}

impl App {
  pub fn new(node: impl Into<String>, default_model: impl Into<String>) -> Self {
    App {
      node: node.into(),
      default_model: default_model.into(),
      model: None,
      thread: None,
      threads: Vec::new(),
      family: None,
      here: "global".into(),
      global: false,
      scope: ThreadScope::default(),
      picked: 0,
      deleting: None,
      thread_filter: String::new(),
      opened: Default::default(),
      closed: Default::default(),
      input: String::new(),
      cursor: None,
      attachments: Vec::new(),
      overlay: Overlay::None,
      attention: Vec::new(),
      picked_needs: 0,
      jobs: Vec::new(),
      picked_job: 0,
      answering: None,
      pending: None,
      in_flight: None,
      status: String::new(),
      auth_pending: false,
      auth_text: String::new(),
      approval: None,
      approval_question: None,
      approval_grant: Vec::new(),
      approval_at: None,
      typed_at: None,
      told: std::collections::HashSet::new(),
      reason: None,
      last_reply: None,
      shown: (0, None),
      echo: None,
      out: Vec::new(),
      pick: None,
      mode: "ask".into(),
      queued: Vec::new(),
      queued_id: 0,
      editing: None,
      delegate_to: None,
      memory: None,
      read_only: false,
      runs_on: None,
      new_tools: Vec::new(),
      new_skills: Vec::new(),
      new_memory: None,
      entries: Vec::new(),
      replies_said: None,
      actions_said: None,
      msgs: MsgPick::default(),
      fork: None,
      esc_at: None,
      at_pick: None,
      files: Vec::new(),
      files_root: None,
      mentions: Vec::new(),
    }
  }

  pub fn model_name(&self) -> &str {
    match self.model.as_deref() {
      Some(m) if !stitch_node::modelset::ALIASES.contains(&m) => m,
      _ => &self.default_model,
    }
  }

  /// The model as replies name it: without the provider.
  fn model_label(&self) -> String {
    let m = self.model_name();
    m.split_once('/').map_or(m, |(_, model)| model).to_owned()
  }

  fn thread_title(&self, id: &str) -> String {
    // The list shows this place's threads; the open one may be another
    // place's (a child opened from `/children`, a report's), read on its
    // own. A thread that started a moment ago is in neither yet.
    self
      .threads
      .iter()
      .find(|t| t.id == id)
      .map(|t| t.title.clone())
      .or_else(|| self.family_of(id).map(|f| f.title.clone()))
      .unwrap_or_else(|| "new thread".to_owned())
  }

  /// What was read of `id` as it was opened, when it is the open thread.
  fn family_of(&self, id: &str) -> Option<&Family> {
    self.family.as_ref().filter(|f| f.id == id)
  }

  /// The open thread's parent, (id, title).
  fn parent(&self) -> Option<&(String, String)> {
    self.family_of(self.thread.as_deref()?)?.parent.as_ref()
  }

  /// How many threads are below the open one.
  fn children(&self) -> usize {
    self
      .thread
      .as_deref()
      .and_then(|id| self.family_of(id))
      .map_or(0, |f| f.children)
  }

  /// `/parent`, Ctrl+P: opens the thread above the open one.
  fn open_parent(&mut self) -> Action {
    match self.parent() {
      Some((id, _)) => Action::Open(Some(id.clone())),
      None => {
        self.status = if self.thread.is_some() {
          "no parent: this thread was not spawned by another".into()
        } else {
          "no thread yet".into()
        };
        Action::None
      }
    }
  }

  /// `/children`: the threads below the open one, in a picker.
  fn open_children(&mut self) -> Action {
    match self.thread.clone() {
      Some(id) => Action::Children(id),
      None => {
        self.status = "no thread yet".into();
        Action::None
      }
    }
  }

  /// The threads below the open one were read: a picker opens them, or
  /// the status line says there are none.
  pub fn show_children(&mut self, choices: Vec<Choice>) {
    if choices.is_empty() {
      self.status = "no children: this thread spawned none".into();
      return;
    }
    let title = format!("{} below “{}”", choices.len(), {
      let id = self.thread.clone().unwrap_or_default();
      clip(&self.thread_title(&id), 30)
    });
    self.open(Picker::new(&title, Then::OpenThread, choices), None);
  }

  /// The open thread was compacted into `to` (a turn the node ran on it):
  /// it is open as `to` from now, the same conversation; its transcript,
  /// followed next, prints the divider and what came since.
  pub fn compacted_into(&mut self, to: String) {
    if self.thread.as_deref() == Some(to.as_str()) {
      return;
    }
    self.thread = Some(to);
    self.status = "compacted: this conversation goes on in a new thread".into();
  }

  /// A thread was opened: its transcript goes into the scrollback.
  pub fn opened(&mut self, thread: Option<String>, transcript: &[Entry]) {
    let title = match &thread {
      Some(id) => self.thread_title(id),
      None => "new thread".into(),
    };
    self.out.push(Printed::Separator(title));
    self.last_reply = None;
    if thread.is_some() {
      self.global = false;
    }
    self.new_tools.clear();
    self.new_skills.clear();
    self.new_memory = None;
    self.thread = thread;
    self.shown = (0, self.activity());
    self.echo = None;
    self.replies_said = None;
    self.actions_said = None;
    self.followed(transcript);
    self.overlay = Overlay::None;
  }

  /// The open thread's last activity, as the thread list last saw it.
  pub fn activity(&self) -> Option<jiff::Timestamp> {
    let id = self.thread.as_deref()?;
    self.threads.iter().find(|t| t.id == id).map(|t| t.at)
  }

  /// The open thread was re-read: what it gained is printed.
  pub fn followed(&mut self, transcript: &[Entry]) {
    let new = transcript.get(self.shown.0..).unwrap_or_default();
    for e in new {
      // The message printed when sent, woven into the turn followed (the
      // first of the user's after it waited, with what was queued
      // after it run together): what it adds is printed, not it again.
      if e.who == "you"
        && let Some(sent) = self.echo.take_if(|s| e.text.starts_with(s.as_str()))
      {
        let rest = e.text[sent.len()..].trim_start_matches('\n');
        // Its attachments were printed with it, as paths.
        let named = |l: &str| l.starts_with('[') && l.ends_with(']');
        if !rest.lines().all(named) {
          self.out.push(Printed::You(rest.to_owned()));
        }
        continue;
      }
      self.out.push(Printed::from_entry(e));
    }
    if let Some(reply) = new.iter().rev().find(|e| {
      !matches!(
        e.who.as_str(),
        "you"
          | "tool"
          | "output"
          | "error"
          | "refusal"
          | "memory"
          | "failed"
          | "compacted"
          | "forked"
          | "context"
          | "press"
          | "note"
      )
    }) {
      self.last_reply = Some(reply.text.clone());
    }
    self.shown.0 = transcript.len();
    self.entries = transcript.to_vec();
    self.say_replies();
  }

  /// The preset answers live now (design §2.30): the last
  /// `suggest_replies` of the open thread, while nothing the user wrote
  /// came after it and no turn of this TUI's runs. `/1`… sends one.
  pub fn live_replies(&self) -> Option<(usize, &[stitch_node::replies::Suggested])> {
    let at = stitch_node::replies::live(
      self
        .entries
        .iter()
        .map(|e| (e.who == "you", !e.replies.is_empty())),
      self.pending.is_some(),
    )?;
    Some((at, &self.entries[at].replies))
  }

  /// The action buttons live now (design §2.30, step 2): the open
  /// thread's latest `offer_actions` set, whatever came after it; `/1`…
  /// runs one (when no preset answers are live). Its entry, its set's ID
  /// and its buttons.
  pub fn live_actions(&self) -> Option<(usize, &str, &[stitch_node::actions::Action])> {
    let at =
      stitch_node::actions::live(self.entries.iter().map(|e| (!e.actions.is_empty(), false)))?;
    let e = &self.entries[at];
    Some((at, e.call.as_deref()?, &e.actions))
  }

  /// Says live preset answers and action buttons in the scrollback, once
  /// each set.
  fn say_replies(&mut self) {
    if let Some((at, _, actions)) = self.live_actions()
      && self.actions_said != Some(at)
    {
      let line = actions
        .iter()
        .enumerate()
        .map(|(i, a)| format!("/{} ▶ {}", i + 1, a.label))
        .collect::<Vec<_>>()
        .join("   ");
      self.out.push(Printed::Notice(format!("↳ {line}")));
      self.actions_said = Some(at);
    }
    let Some((at, replies)) = self.live_replies() else {
      return;
    };
    if self.replies_said == Some(at) {
      return;
    }
    let line = replies
      .iter()
      .enumerate()
      .map(|(i, r)| format!("/{} {}", i + 1, r.label))
      .collect::<Vec<_>>()
      .join("   ");
    self.out.push(Printed::Notice(format!("↳ {line}")));
    self.replies_said = Some(at);
  }

  pub fn on_key(&mut self, key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
      return Action::None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if let Some(reason) = &mut self.reason {
      // Declining: Enter sends the reason (none if empty), Esc sends none.
      return match key.code {
        KeyCode::Char('c') if ctrl => Action::Quit,
        KeyCode::Enter | KeyCode::Esc => {
          let reason = (key.code == KeyCode::Enter).then(|| reason.trim().to_owned());
          self.reason = None;
          self.status = "declined".into();
          match self.answering.take() {
            Some(question) => Action::Answer {
              question,
              decision: Decision::decline(reason),
            },
            None => {
              self.approval = None;
              Action::Approve(Decision::decline(reason))
            }
          }
        }
        KeyCode::Backspace => {
          reason.pop();
          Action::None
        }
        KeyCode::Char(c) => {
          reason.push(c);
          Action::None
        }
        _ => Action::None,
      };
    }
    // A prompt that appears while the user types a message does not take
    // what they type: its keys answer it once they paused (a `y` in "yet"
    // allowed a browser handoff nobody meant to end).
    let recent = |t: Option<std::time::Instant>| t.is_some_and(|t| t.elapsed() < TYPING_GRACE);
    let text_key = !ctrl && !alt && matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace);
    if self.approval.is_some() && text_key && (recent(self.approval_at) || recent(self.typed_at)) {
      self.typed_at = Some(std::time::Instant::now());
      self.status = "approval waiting: pause typing, then y/n".into();
    } else if self.approval.is_some() {
      return match key.code {
        KeyCode::Char('c') if ctrl => Action::Quit,
        // The approval stays on the attention list; the turn goes on.
        KeyCode::Char('n') if ctrl => self.new_thread(false),
        KeyCode::Char('y') | KeyCode::Char('Y') => {
          self.approval = None;
          self.status = "approved".into();
          Action::Approve(Decision::allow())
        }
        KeyCode::Char('a') | KeyCode::Char('A') if !self.approval_grant.is_empty() => {
          self.approval = None;
          self.status = format!(
            "approved; from now on here: {}",
            self.approval_grant.join(", ")
          );
          Action::Approve(Decision::always())
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
          self.reason = Some(String::new());
          Action::None
        }
        KeyCode::Esc => {
          self.approval = None;
          self.status = "declined".into();
          Action::Approve(Decision::decline(None))
        }
        _ => Action::None,
      };
    }
    if text_key {
      self.typed_at = Some(std::time::Instant::now());
    }
    match key.code {
      KeyCode::Char('c') | KeyCode::Char('q') if ctrl => return Action::Quit,
      KeyCode::Char('y') if ctrl => return self.copy(),
      KeyCode::Char('v') if ctrl && self.reason.is_none() && self.overlay == Overlay::None => {
        return Action::PasteClipboard;
      }
      KeyCode::Char('n') if ctrl => return self.new_thread(false),
      KeyCode::Char('p') if ctrl => return self.open_parent(),
      KeyCode::Char('o') if ctrl => {
        self.open_picker();
        return Action::None;
      }
      KeyCode::Char('t') if ctrl => {
        self.overlay = match self.overlay {
          Overlay::Attention => Overlay::None,
          _ => Overlay::Attention,
        };
        return Action::None;
      }
      KeyCode::Char('b') if ctrl => {
        if self.overlay == Overlay::Jobs {
          self.overlay = Overlay::None;
          return Action::None;
        }
        return self.open_jobs();
      }
      _ => {}
    }
    // The picker asked whether to delete a thread: y deletes, anything
    // else does not.
    if self.overlay == Overlay::Threads
      && let Some(thread) = self.deleting.take()
    {
      if matches!(key.code, KeyCode::Char('y' | 'Y')) {
        self.status = "deleting…".into();
        return Action::Delete(thread);
      }
      self.status = "not deleted".into();
      return Action::None;
    }
    match self.overlay {
      Overlay::Threads => self.on_threads_key(key, ctrl || alt),
      Overlay::Attention => match key.code {
        KeyCode::Esc => {
          self.overlay = Overlay::None;
          Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
          self.picked_needs = self.picked_needs.saturating_sub(1);
          Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
          self.picked_needs = (self.picked_needs + 1).min(self.attention.len().saturating_sub(1));
          Action::None
        }
        KeyCode::Char('v') => {
          match self
            .attention
            .get(self.picked_needs)
            .and_then(|n| n.detail.clone())
          {
            Some(d) => {
              self.out.push(Printed::Notice(d));
              self.status = "printed above: scroll up to read it, then y or n".into();
            }
            None => self.status = "nothing more to read for this one".into(),
          }
          Action::None
        }
        KeyCode::Char('y') | KeyCode::Char('n') | KeyCode::Char('a') => {
          let Some((question, grant)) = self
            .attention
            .get(self.picked_needs)
            .and_then(|n| Some((n.question.clone()?, n.grant.clone())))
          else {
            return Action::None;
          };
          if key.code == KeyCode::Char('a') {
            if grant.is_empty() {
              self.status = "nothing to allow always for this one: y or n".into();
              return Action::None;
            }
            self.status = format!("approved; from now on there: {}", grant.join(", "));
            Action::Answer {
              question,
              decision: Decision::always(),
            }
          } else if key.code == KeyCode::Char('y') {
            self.status = "approved".into();
            Action::Answer {
              question,
              decision: Decision::allow(),
            }
          } else {
            self.answering = Some(question);
            self.reason = Some(String::new());
            Action::None
          }
        }
        _ => Action::None,
      },
      Overlay::Jobs => match key.code {
        KeyCode::Esc => {
          self.overlay = Overlay::None;
          Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
          self.picked_job = self.picked_job.saturating_sub(1);
          Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
          self.picked_job = (self.picked_job + 1).min(self.jobs.len().saturating_sub(1));
          Action::None
        }
        KeyCode::Enter => match self.jobs.get(self.picked_job) {
          Some(j) => Action::Open(Some(j.thread.clone())),
          None => Action::None,
        },
        KeyCode::Char('x') => match self.jobs.get(self.picked_job) {
          Some(j) => {
            self.status = format!("stopping {}", j.id);
            Action::StopJob(j.id.clone())
          }
          None => Action::None,
        },
        _ => Action::None,
      },
      Overlay::Auth if key.code == KeyCode::Esc => {
        self.overlay = Overlay::None;
        Action::None
      }
      Overlay::Pick => self.pick_key(key),
      Overlay::Messages => self.msg_key(key),
      Overlay::Fork => self.fork_key(key),
      _ => {
        let action = self.input_key(key, ctrl, alt);
        self.at_follow(key.code == KeyCode::Char('@') && !ctrl && !alt);
        action
      }
    }
  }

  fn input_key(&mut self, key: KeyEvent, ctrl: bool, alt: bool) -> Action {
    if key.code != KeyCode::Esc {
      self.esc_at = None;
    }
    // The `@` file picker takes the keys that move and pick in it; the
    // rest edit the input, which filters it.
    if self.at_pick.is_some() {
      match key.code {
        KeyCode::Esc => {
          self.at_pick = None;
          return Action::None;
        }
        KeyCode::Up => {
          if let Some(p) = &mut self.at_pick {
            p.picked = p.picked.saturating_sub(1);
          }
          return Action::None;
        }
        KeyCode::Down => {
          let n = self.at_choices().len();
          if let Some(p) = &mut self.at_pick {
            p.picked = (p.picked + 1).min(n.saturating_sub(1));
          }
          return Action::None;
        }
        KeyCode::Tab | KeyCode::Enter
          if !alt && !ctrl && !key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
          let choices = self.at_choices();
          let picked = self.at_pick.as_ref().map_or(0, |p| p.picked);
          match choices.get(picked.min(choices.len().saturating_sub(1))) {
            Some(path) => {
              let path = path.clone();
              self.pick_file(&path);
              return Action::None;
            }
            // Nothing matches: Tab does nothing, Enter is Enter.
            None if key.code == KeyCode::Tab => return Action::None,
            None => self.at_pick = None,
          }
        }
        _ => {}
      }
    }
    match key.code {
      // The last queued message comes back to be edited.
      KeyCode::Up if self.input.is_empty() && !self.queued.is_empty() => {
        let q = self.queued.pop().expect("checked");
        self.input = q.text.clone();
        self.cursor = None;
        self.attachments = q.attachments.clone();
        let id = q.id;
        self.editing = Some(q);
        self.status = format!(
          "editing a queued message (Enter queues it again, Esc drops it) · {} queued",
          self.queued.len()
        );
        Action::Retract(id)
      }
      KeyCode::Esc if self.editing.is_some() => {
        self.editing = None;
        self.input.clear();
        self.cursor = None;
        self.attachments.clear();
        self.status = "queued message dropped".into();
        Action::None
      }
      // Waiting behind another turn on the thread (a report's, a job's
      // note's): that turn is the one working, so Esc stops it too, not
      // only this message, which comes back unsent.
      KeyCode::Esc if self.pending.as_ref().is_some_and(|p| p.waiting) && self.thread.is_some() => {
        self.status = "stopping the turn running on this thread".into();
        Action::Cancel(self.thread.clone().expect("a thread"))
      }
      KeyCode::Esc if self.pending.is_some() => {
        self.status = "interrupting".into();
        Action::Interrupt
      }
      // A turn this TUI did not start runs on the open thread (woken by a
      // job, a report; typed elsewhere): Esc stops it, as the web app's
      // Cancel does.
      KeyCode::Esc if self.followed_turn_runs() => {
        self.status = "stopping the turn running on this thread".into();
        Action::Cancel(self.thread.clone().expect("a thread"))
      }
      // Esc Esc on an empty input: the message picker.
      KeyCode::Esc if self.input.is_empty() => {
        // Two Escs read at once come as one with Alt.
        let again = alt || self.esc_at.take().is_some_and(|t| t.elapsed() < ESC_AGAIN);
        if again {
          self.open_messages();
        } else {
          self.esc_at = Some(std::time::Instant::now());
          if self.pickable().is_empty() {
            self.status.clear();
          } else {
            self.status = "Esc again: pick a message to fork from, edit or copy".into();
          }
        }
        Action::None
      }
      // Shift+Enter where the terminal tells it from Enter (the TUI asks
      // for that), Alt+Enter where Option is Meta, Ctrl+J anywhere.
      KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => {
        self.insert('\n');
        Action::None
      }
      KeyCode::Char('j') if ctrl => {
        self.insert('\n');
        Action::None
      }
      // A terminal that sends Shift+Enter as Enter: `\` and Enter.
      KeyCode::Enter if self.at() > 0 && self.input.chars().nth(self.at() - 1) == Some('\\') => {
        let at = self.at();
        self.remove(at - 1..at);
        self.insert('\n');
        Action::None
      }
      KeyCode::Enter => self.submit(),
      KeyCode::Backspace if alt => {
        self.delete_word_back();
        Action::None
      }
      KeyCode::Backspace => {
        let at = self.at();
        if at > 0 {
          self.remove(at - 1..at);
        }
        Action::None
      }
      KeyCode::Delete => {
        let at = self.at();
        if at < self.chars() {
          self.remove(at..at + 1);
        }
        Action::None
      }
      // Option+←/→ as macOS terminals send them by default.
      KeyCode::Char('b') if alt => {
        self.cursor = Some(self.word_back());
        Action::None
      }
      KeyCode::Char('f') if alt => {
        self.cursor = Some(self.word_forward());
        Action::None
      }
      KeyCode::Left if alt || ctrl => {
        self.cursor = Some(self.word_back());
        Action::None
      }
      KeyCode::Right if alt || ctrl => {
        self.cursor = Some(self.word_forward());
        Action::None
      }
      KeyCode::Left => {
        self.cursor = Some(self.at().saturating_sub(1));
        Action::None
      }
      KeyCode::Right => {
        self.cursor = Some((self.at() + 1).min(self.chars()));
        Action::None
      }
      KeyCode::Home => {
        self.cursor = Some(self.line_start());
        Action::None
      }
      KeyCode::End => {
        self.cursor = Some(self.line_end());
        Action::None
      }
      KeyCode::Char('a') if ctrl => {
        self.cursor = Some(self.line_start());
        Action::None
      }
      KeyCode::Char('e') if ctrl => {
        self.cursor = Some(self.line_end());
        Action::None
      }
      KeyCode::Char('u') if ctrl => {
        let (start, at) = (self.line_start(), self.at());
        self.remove(start..at);
        Action::None
      }
      KeyCode::Char('k') if ctrl => {
        let (at, end) = (self.at(), self.line_end());
        self.remove(at..end);
        Action::None
      }
      KeyCode::Char('w') if ctrl => {
        self.delete_word_back();
        Action::None
      }
      KeyCode::Up => {
        self.move_line(-1);
        Action::None
      }
      KeyCode::Down => {
        self.move_line(1);
        Action::None
      }
      KeyCode::Char(c) if !ctrl => {
        self.insert(c);
        Action::None
      }
      _ => Action::None,
    }
  }

  /// Characters in the input.
  fn chars(&self) -> usize {
    self.input.chars().count()
  }

  /// The cursor, in characters.
  fn at(&self) -> usize {
    self.cursor.unwrap_or(usize::MAX).min(self.chars())
  }

  /// The byte offset of character `i`.
  fn byte(&self, i: usize) -> usize {
    self
      .input
      .char_indices()
      .nth(i)
      .map_or(self.input.len(), |(b, _)| b)
  }

  fn insert(&mut self, c: char) {
    let at = self.at();
    let b = self.byte(at);
    self.input.insert(b, c);
    self.cursor = Some(at + 1);
  }

  /// Pasted text, as one: into the input at the cursor, or typed into
  /// the field that has the keys (a picker's filter, a reason), on one line.
  pub fn paste(&mut self, text: &str) {
    let text = clean(text);
    if self.reason.is_none() && self.approval.is_none() && self.overlay == Overlay::None {
      // Files dropped on the terminal paste their paths: attached.
      if let Some(files) = super::clipboard::dropped(&text) {
        self.attach_files(&files);
        return;
      }
      let at = self.at();
      let b = self.byte(at);
      self.input.insert_str(b, &text);
      self.cursor = Some(at + text.chars().count());
      self.at_follow(false);
      return;
    }
    // The message picker's and the fork form's keys are actions, not text.
    if matches!(self.overlay, Overlay::Messages | Overlay::Fork) {
      return;
    }
    for c in text.chars().filter(|c| *c != '\n') {
      self.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
  }

  /// Removes characters `range`; the cursor goes to where they were.
  fn remove(&mut self, range: std::ops::Range<usize>) {
    if range.start >= range.end {
      return;
    }
    let (a, b) = (self.byte(range.start), self.byte(range.end));
    self.input.replace_range(a..b, "");
    self.cursor = Some(range.start);
  }

  fn line_start(&self) -> usize {
    let at = self.at();
    let before: Vec<char> = self.input.chars().take(at).collect();
    before.iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1)
  }

  fn line_end(&self) -> usize {
    let at = self.at();
    self
      .input
      .chars()
      .skip(at)
      .position(|c| c == '\n')
      .map_or(self.chars(), |i| at + i)
  }

  /// The start of the word before the cursor.
  fn word_back(&self) -> usize {
    let chars: Vec<char> = self.input.chars().collect();
    let mut i = self.at();
    while i > 0 && chars[i - 1].is_whitespace() {
      i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
      i -= 1;
    }
    i
  }

  /// The end of the word after the cursor.
  fn word_forward(&self) -> usize {
    let chars: Vec<char> = self.input.chars().collect();
    let mut i = self.at();
    while i < chars.len() && chars[i].is_whitespace() {
      i += 1;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
      i += 1;
    }
    i
  }

  fn delete_word_back(&mut self) {
    let (from, at) = (self.word_back(), self.at());
    self.remove(from..at);
  }

  /// Moves the cursor a line up (`-1`) or down (`1`), keeping its column
  /// as far as the line allows.
  fn move_line(&mut self, by: i32) {
    let lines: Vec<usize> = self.input.split('\n').map(|l| l.chars().count()).collect();
    let (start, at) = (self.line_start(), self.at());
    let column = at - start;
    let row = self.input.chars().take(at).filter(|c| *c == '\n').count();
    let target = row as i32 + by;
    if target < 0 || target as usize >= lines.len() {
      return;
    }
    let target = target as usize;
    let begin: usize = lines[..target].iter().map(|n| n + 1).sum();
    self.cursor = Some(begin + column.min(lines[target]));
  }

  /// The input as shown: the cursor marked where it is (a tab as a space,
  /// so characters keep their places).
  fn input_shown(&self) -> String {
    let mut text = self.input.replace('\t', " ");
    text.insert(self.byte(self.at()), '▏');
    text
  }

  /// Rows the input takes at `width` (wrapped), and the row of the cursor.
  fn input_rows(&self, width: u16) -> (u16, u16) {
    let text = self.input_shown();
    let all = Paragraph::new(text.as_str())
      .wrap(Wrap { trim: false })
      .line_count(width)
      .max(1) as u16;
    let upto: String = text.chars().take(self.at() + 1).collect();
    let row = Paragraph::new(upto.as_str())
      .wrap(Wrap { trim: false })
      .line_count(width)
      .max(1) as u16
      - 1;
    (all, row)
  }

  /// The thread picker opens on the open thread, the threads above it
  /// opened so it shows, with no filter.
  fn open_picker(&mut self) {
    self.overlay = Overlay::Threads;
    self.thread_filter.clear();
    self.picked = 0;
    let Some(id) = self.thread.clone() else {
      return;
    };
    for a in super::tree::ancestors(&self.threads, &id) {
      self.closed.remove(&a);
      self.opened.insert(a);
    }
    let rows = self.thread_rows();
    if let Some(i) = rows.iter().position(|r| self.threads[r.at].id == id) {
      self.picked = i + 1;
    }
  }

  /// The threads as read again: the thread picker stays on the thread it
  /// was on (its row moves when a thread above it moves, or opens).
  pub fn set_threads(&mut self, threads: Vec<ThreadItem>) {
    let rows = self.thread_rows();
    let was = self
      .picked
      .checked_sub(1)
      .and_then(|i| rows.get(i))
      .map(|r| self.threads[r.at].id.clone());
    self.threads = threads;
    let rows = self.thread_rows();
    self.picked = match was {
      Some(id) => rows
        .iter()
        .position(|r| self.threads[r.at].id == id)
        .map_or(self.picked.min(rows.len()), |i| i + 1),
      None => self.picked.min(rows.len()),
    };
  }

  /// The thread picker's rows (after "+ new thread").
  pub fn thread_rows(&self) -> Vec<super::tree::Row> {
    super::tree::rows(
      &self.threads,
      &self.thread_filter,
      &self.opened,
      &self.closed,
    )
  }

  /// What deleting `thread` takes, as the node listed it: the picker asks
  /// (y deletes), unless it was closed meanwhile.
  pub fn on_delete_preview(&mut self, thread: String, doomed: &[crate::doomed::Doomed]) {
    if self.overlay != Overlay::Threads || doomed.is_empty() {
      return;
    }
    self.status = format!("{} y/n", crate::doomed::question(doomed));
    self.deleting = Some(thread);
  }

  /// The thread picker's keys: typing filters (every word in the title
  /// or place), ↑↓ move, → opens a thread's spawns (or goes to the
  /// first), ← closes them (or goes to its parent), Enter opens the
  /// thread, Delete deletes it, Tab changes which places, Esc clears the
  /// filter, then closes.
  fn on_threads_key(&mut self, key: KeyEvent, modified: bool) -> Action {
    let rows = self.thread_rows();
    let n = rows.len();
    let row = self.picked.checked_sub(1).and_then(|i| rows.get(i));
    let id = row.map(|r| self.threads[r.at].id.clone());
    match key.code {
      KeyCode::Delete => {
        if let Some(t) = row.map(|r| &self.threads[r.at]) {
          // The node says what it takes first; the question waits for it.
          self.status = format!("deleting \"{}\": checking what goes…", t.title);
          self.deleting = None;
          return Action::DeletePreview(t.id.clone());
        }
      }
      KeyCode::Esc if !self.thread_filter.is_empty() => {
        self.thread_filter.clear();
        // Back on the thread that was picked, in the tree.
        self.picked = 0;
        if let Some(id) = id {
          for a in super::tree::ancestors(&self.threads, &id) {
            self.closed.remove(&a);
            self.opened.insert(a);
          }
          if let Some(i) = self
            .thread_rows()
            .iter()
            .position(|r| self.threads[r.at].id == id)
          {
            self.picked = i + 1;
          }
        }
      }
      KeyCode::Esc => self.overlay = Overlay::None,
      KeyCode::Up => self.picked = self.picked.saturating_sub(1),
      KeyCode::Down => self.picked = (self.picked + 1).min(n),
      KeyCode::PageUp => self.picked = self.picked.saturating_sub(10),
      KeyCode::PageDown => self.picked = (self.picked + 10).min(n),
      KeyCode::Home => self.picked = 0,
      KeyCode::End => self.picked = n,
      KeyCode::Right => {
        if let (Some(r), Some(id)) = (row, id) {
          match r.open {
            Some(false) => {
              self.closed.remove(&id);
              self.opened.insert(id);
            }
            Some(true) => self.picked += 1,
            None => {}
          }
        }
      }
      KeyCode::Left => {
        if let (Some(r), Some(id)) = (row, id) {
          if r.open == Some(true) {
            self.opened.remove(&id);
            self.closed.insert(id);
          } else if let Some(p) = self.threads[r.at].parent.as_ref()
            && let Some(i) = rows[..self.picked - 1]
              .iter()
              .rposition(|r| &self.threads[r.at].id == p)
          {
            self.picked = i + 1;
          }
        }
      }
      KeyCode::Tab => {
        self.scope = self.scope.next();
        self.picked = 0;
        return Action::ListThreads;
      }
      KeyCode::Backspace => {
        self.thread_filter.pop();
        self.picked = usize::from(!self.thread_filter.is_empty());
      }
      KeyCode::Char(c) if !modified => {
        if c == ' ' && self.thread_filter.is_empty() {
          return Action::None;
        }
        self.thread_filter.push(c);
        // The first match, rather than "+ new thread".
        self.picked = 1;
      }
      KeyCode::Enter => {
        return match id {
          Some(id) => Action::Open(Some(id)),
          // Nothing matches the words: look for them in the messages.
          None if n == 0 && !self.thread_filter.trim().is_empty() => {
            let query = self.thread_filter.trim().to_owned();
            self.thread_filter.clear();
            self.open(
              Picker::new(&format!("search: {query}"), Then::OpenThread, Vec::new()),
              Some(Fetch::Search { query, here: false }),
            )
          }
          None => self.new_thread(false),
        };
      }
      _ => {}
    }
    Action::None
  }

  /// Opens the fork form.
  fn open_fork(&mut self, form: ForkForm) {
    self.fork = Some(form);
    self.overlay = Overlay::Fork;
    self.status.clear();
  }

  /// The fork form's keys: ↑↓ (Tab, Shift+Tab) move between its rows,
  /// Space (←→ too) changes the one selected, digits type N (its last N
  /// messages), `a` `t` `s` `u` switch their row from anywhere, Enter
  /// forks, Esc cancels.
  fn fork_key(&mut self, key: KeyEvent) -> Action {
    let Some(f) = &mut self.fork else {
      self.overlay = Overlay::None;
      return Action::None;
    };
    let rows = f.rows();
    let row = rows[f.row.min(rows.len() - 1)];
    match key.code {
      KeyCode::Esc => {
        self.fork = None;
        self.overlay = Overlay::None;
        self.status = "no fork".into();
      }
      KeyCode::Up | KeyCode::Char('k') => f.row = f.row.saturating_sub(1),
      KeyCode::Down | KeyCode::Char('j') => f.row = (f.row + 1).min(rows.len() - 1),
      KeyCode::Tab => f.row = (f.row + 1) % rows.len(),
      KeyCode::BackTab => f.row = (f.row + rows.len() - 1) % rows.len(),
      KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => f.toggle(row),
      KeyCode::Char(c) if c.is_ascii_digit() => {
        if !f.last_typed {
          f.last.clear();
          f.last_typed = true;
        }
        if f.last.len() < 6 {
          f.last.push(c);
        }
        f.last_on = true;
        f.row = 0;
      }
      KeyCode::Backspace => {
        f.last_typed = true;
        f.last.pop();
        f.row = 0;
      }
      KeyCode::Char(c @ ('a' | 't' | 's' | 'u')) => {
        let r = match c {
          'a' => ForkRow::Attachments,
          't' => ForkRow::Tools,
          's' => ForkRow::Summary,
          _ => ForkRow::Upto,
        };
        if let Some(i) = rows.iter().position(|x| *x == r) {
          f.toggle(r);
          f.row = i;
        }
      }
      KeyCode::Enter => return self.fork_go(),
      _ => {}
    }
    Action::None
  }

  /// The form's fork, when it can be made; else the form stays, the
  /// status line saying why.
  fn fork_go(&mut self) -> Action {
    let Some(f) = self.fork.clone() else {
      return Action::None;
    };
    if self.pending.is_some() {
      self.status = "a turn runs: fork once it ends (Esc stops it)".into();
      return Action::None;
    }
    if f.last_on && f.count().is_none() {
      self.status = "the last how many messages? type a number".into();
      return Action::None;
    }
    self.fork = None;
    self.overlay = Overlay::None;
    self.status = if f.summarize {
      "summarizing (a model call)…".into()
    } else if let (true, Some(m)) = (f.upto_on, &f.upto) {
      format!("forking up to {m}…")
    } else {
      "forking…".into()
    };
    Action::Fork {
      thread: f.thread.clone(),
      upto: f.upto.clone().filter(|_| f.upto_on),
      last: f.count(),
      no_attachments: f.no_attachments,
      no_tools: f.no_tools,
      summarize: f.summarize,
    }
  }

  /// What the fork form's fork would keep, said in a line.
  fn fork_preview(&self, f: &ForkForm) -> String {
    if f.last_on && f.count().is_none() {
      return "type how many of the last messages to keep".into();
    }
    let k = fork_keeps(&self.entries, f);
    // A fork's own transcript holds only its messages, not those it
    // continues: what it reads from its parent is not counted.
    let parent = if self.entries.first().is_some_and(|e| e.who == "fork")
      || self
        .entries
        .iter()
        .any(|e| e.who == "compacted" || e.who == "forked")
    {
      " (+ what this thread continues)"
    } else {
      ""
    };
    if f.summarize && k.summarized == 0 && parent.is_empty() {
      return format!(
        "keeps all {} messages: nothing is left for a summary (the node refuses it)",
        k.kept
      );
    }
    if k.kept == 0 && !f.summarize {
      return "keeps no message (the node refuses an empty fork)".into();
    }
    let mut out = format!("keeps {} of {} messages{parent}", k.kept, k.of);
    out.push_str(&format!(" · {}", tokens_said(k.chars)));
    match k.attachments {
      0 => {}
      1 => out.push_str(" · 1 attachment"),
      n => out.push_str(&format!(" · {n} attachments")),
    }
    if f.summarize {
      out.push_str(&format!(
        " · + a summary of {} message{} (a model call)",
        k.summarized,
        if k.summarized == 1 { "" } else { "s" }
      ));
    }
    out
  }

  fn fork_lines(&self, width: u16) -> Vec<Line<'static>> {
    let Some(f) = &self.fork else {
      return Vec::new();
    };
    let dim = Style::new().fg(Color::DarkGray);
    let rows = f.rows();
    let sel = f.row.min(rows.len() - 1);
    let check = |on: bool| if on { "[x]" } else { "[ ]" };
    let mut lines = Vec::new();
    for (i, r) in rows.iter().enumerate() {
      let (text, flag) = match r {
        ForkRow::Range => {
          let n = if f.last.is_empty() { "_" } else { &f.last };
          let radio = |on: bool| if on { "(•)" } else { "( )" };
          (
            format!(
              "{} all of it   {} its last {n} messages",
              radio(!f.last_on),
              radio(f.last_on)
            ),
            "N",
          )
        }
        ForkRow::Upto => {
          let m = f.upto.as_deref().unwrap_or("");
          let said = self
            .entries
            .iter()
            .find(|e| e.id.as_deref() == Some(m))
            .map(|e| clip(e.said().lines().next().unwrap_or(""), 40))
            .unwrap_or_default();
          (format!("{} up to {m} \"{said}\"", check(f.upto_on)), "@")
        }
        ForkRow::Attachments => (
          format!("{} without attachments", check(f.no_attachments)),
          "-a",
        ),
        ForkRow::Tools => (format!("{} without tool calls", check(f.no_tools)), "-t"),
        ForkRow::Summary => (
          format!("{} a summary of what is not kept", check(f.summarize)),
          "-s",
        ),
      };
      let text = format!("{} {text}", if i == sel { "›" } else { " " });
      let style = if i == sel {
        Style::new()
          .bg(Color::Indexed(236))
          .add_modifier(Modifier::BOLD)
      } else {
        Style::new()
      };
      lines.push(Line::from(vec![
        Span::styled(text, style),
        Span::styled(format!("  {flag}"), dim),
      ]));
    }
    let width = (width as usize).max(10);
    lines.push(Line::from(Span::styled(
      clip(&format!("  {}", self.fork_preview(f)), width),
      Style::new().fg(Color::Cyan),
    )));
    lines.push(Line::from(Span::styled(
      clip(&format!("  as typed: {}", f.typed()), width),
      dim,
    )));
    lines
  }

  fn render_fork(&self, f: &mut Frame, area: Rect) {
    let name = self.fork.as_ref().map_or("fork", |f| f.name);
    let keys = if self.fork.as_ref().is_some_and(|f| f.upto.is_some()) {
      "u a t s"
    } else {
      "a t s"
    };
    let title = format!("{name} · ↑↓ Tab · Space changes · digits: N · {keys} · Enter · Esc");
    let lines = self.fork_lines(area.width.saturating_sub(2));
    f.render_widget(Paragraph::new(Text::from(lines)).block(frame(&title)), area);
  }

  /// The open thread's messages the picker offers, by their index in
  /// `entries`.
  pub fn pickable(&self) -> Vec<usize> {
    if self.thread.is_none() {
      return Vec::new();
    }
    (0..self.entries.len())
      .filter(|&i| self.entries[i].pickable())
      .collect()
  }

  /// The message picker, at the latest message.
  fn open_messages(&mut self) {
    let n = self.pickable().len();
    if n == 0 {
      self.status = "no messages to pick yet".into();
      return;
    }
    self.msgs = MsgPick {
      picked: n - 1,
      menu: None,
    };
    self.overlay = Overlay::Messages;
    self.status.clear();
  }

  /// The message picked, by its index in `entries`.
  fn picked_entry(&self) -> Option<usize> {
    let all = self.pickable();
    all
      .get(self.msgs.picked.min(all.len().saturating_sub(1)))
      .copied()
  }

  /// What can be done with the message at `i` of `entries`.
  fn msg_actions(&self, i: usize) -> Vec<MsgAction> {
    let mut out = vec![MsgAction::Fork, MsgAction::ForkOptions];
    if self.entries.get(i).is_some_and(|e| e.who == "you") {
      out.push(MsgAction::Edit);
    }
    out.push(MsgAction::Rewind);
    out.push(MsgAction::Copy);
    out
  }

  fn msg_key(&mut self, key: KeyEvent) -> Action {
    let Some(i) = self.picked_entry() else {
      self.overlay = Overlay::None;
      return Action::None;
    };
    let n = self.pickable().len();
    let actions = self.msg_actions(i);
    if let Some(sel) = self.msgs.menu {
      return match key.code {
        KeyCode::Esc | KeyCode::Left => {
          self.msgs.menu = None;
          Action::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
          self.msgs.menu = Some(sel.saturating_sub(1));
          Action::None
        }
        KeyCode::Down | KeyCode::Char('j') => {
          self.msgs.menu = Some((sel + 1).min(actions.len() - 1));
          Action::None
        }
        KeyCode::Enter => self.msg_do(i, actions[sel.min(actions.len() - 1)]),
        KeyCode::Char(c) => match actions.iter().find(|a| a.key() == c) {
          Some(a) => self.msg_do(i, *a),
          None => Action::None,
        },
        _ => Action::None,
      };
    }
    match key.code {
      KeyCode::Esc => {
        self.overlay = Overlay::None;
        Action::None
      }
      KeyCode::Up | KeyCode::Char('k') => {
        self.msgs.picked = self.msgs.picked.saturating_sub(1);
        Action::None
      }
      KeyCode::Down | KeyCode::Char('j') => {
        self.msgs.picked = (self.msgs.picked + 1).min(n - 1);
        Action::None
      }
      KeyCode::Home | KeyCode::Char('g') => {
        self.msgs.picked = 0;
        Action::None
      }
      KeyCode::End | KeyCode::Char('G') => {
        self.msgs.picked = n - 1;
        Action::None
      }
      KeyCode::Enter | KeyCode::Right => {
        self.msgs.menu = Some(0);
        Action::None
      }
      KeyCode::Char(c) => match actions.iter().find(|a| a.key() == c) {
        Some(a) => self.msg_do(i, *a),
        None => Action::None,
      },
      _ => Action::None,
    }
  }

  /// Does `what` with the message at `i` of `entries`.
  fn msg_do(&mut self, i: usize, what: MsgAction) -> Action {
    let Some(e) = self.entries.get(i).cloned() else {
      return Action::None;
    };
    let Some(thread) = self.thread.clone() else {
      return Action::None;
    };
    let id = e.id.clone().unwrap_or_default();
    if matches!(what, MsgAction::Fork | MsgAction::Edit | MsgAction::Rewind)
      && self.pending.is_some()
    {
      self.status = "a turn runs: wait for it to end (Esc stops it)".into();
      return Action::None;
    }
    // The message before it (any with an ID: a reply, a tool's result).
    let before = self.entries[..i].iter().rev().find_map(|e| e.id.clone());
    self.overlay = Overlay::None;
    self.msgs.menu = None;
    match what {
      MsgAction::Fork => {
        self.status = format!("forking up to {id}…");
        Action::Fork {
          thread,
          upto: Some(id),
          last: None,
          no_attachments: false,
          no_tools: false,
          summarize: false,
        }
      }
      MsgAction::ForkOptions => {
        let mut form = ForkForm::new(thread, "fork");
        form.upto = Some(id);
        form.upto_on = true;
        self.open_fork(form);
        Action::None
      }
      MsgAction::Copy => {
        self.status = "message copied".into();
        Action::Copy(e.said().to_owned())
      }
      MsgAction::Edit => {
        // The thread rewound to the message before it, as the phone's
        // edit: it goes on from there, here, what came after set aside.
        self.input = e.said().to_owned();
        self.cursor = None;
        let lost = lost_attachments(e.attachments);
        self.status = format!("rewinding to before it; its text is in the input{lost}");
        Action::Rewind {
          thread,
          upto: before,
          resend: None,
        }
      }
      MsgAction::Rewind if e.who == "you" => {
        // Sent again: rewound to before it, then the same text.
        let lost = lost_attachments(e.attachments);
        self.status = format!("rewinding to before it, to send it again{lost}");
        Action::Rewind {
          thread,
          upto: before,
          resend: Some(e.said().to_owned()),
        }
      }
      MsgAction::Rewind => {
        self.status = format!("rewinding to {id}…");
        Action::Rewind {
          thread,
          upto: Some(id),
          resend: None,
        }
      }
    }
  }

  /// The thread was rewound (to `upto`, its start when `None`): nothing is
  /// printed again, the conversation above being the terminal's; a line
  /// says where it goes on from, and that what is printed after that
  /// message, above, is set aside. `transcript` is the thread's line now.
  pub fn rewound(&mut self, upto: Option<&str>, transcript: &[Entry]) {
    let what = match upto.and_then(|m| transcript.iter().find(|e| e.id.as_deref() == Some(m))) {
      Some(e) => {
        let line = e
          .said()
          .lines()
          .find(|l| !l.trim().is_empty())
          .unwrap_or("");
        let short: String = line.chars().take(50).collect();
        let more = if line.chars().count() > 50 { "…" } else { "" };
        format!(
          "goes on from {} \"{short}{more}\"",
          e.id.as_deref().unwrap_or("")
        )
      }
      None => "starts over".into(),
    };
    self.out.push(Printed::Separator(format!(
      "rewound: the thread {what}; what came after it above is set aside"
    )));
    self.last_reply = transcript
      .iter()
      .rev()
      .find(|e| {
        !matches!(
          e.who.as_str(),
          "you"
            | "tool"
            | "output"
            | "error"
            | "refusal"
            | "memory"
            | "failed"
            | "fork"
            | "compacted"
            | "forked"
            | "context"
            | "note"
        )
      })
      .map(|e| e.text.clone());
    self.shown = (transcript.len(), self.activity());
    self.entries = transcript.to_vec();
  }

  /// After a key in the input: `@` typed where a word starts opens the
  /// file picker; the picker closes once the cursor leaves its word.
  fn at_follow(&mut self, typed_at: bool) {
    let at = self.at();
    let chars: Vec<char> = self.input.chars().collect();
    if typed_at && self.at_pick.is_none() && at > 0 && chars.get(at - 1) == Some(&'@') {
      let starts_word = at == 1 || chars[at - 2].is_whitespace();
      // Commands take `@node` (`/spawn @mbp`).
      if starts_word && !self.input.starts_with('/') {
        if let Some(root) = &self.files_root {
          self.files = super::files::list(root, FILES_MAX);
        }
        self.at_pick = Some(AtPick {
          start: at - 1,
          picked: 0,
        });
        return;
      }
    }
    let Some(p) = &mut self.at_pick else { return };
    let inside = at > p.start
      && chars.get(p.start) == Some(&'@')
      && !chars[p.start + 1..at].iter().any(|c| c.is_whitespace());
    if !inside {
      self.at_pick = None;
      return;
    }
    let n = self.at_choices().len();
    if let Some(p) = &mut self.at_pick {
      p.picked = p.picked.min(n.saturating_sub(1));
    }
  }

  /// The files matching what follows the `@`, best first.
  pub fn at_choices(&self) -> Vec<String> {
    let Some(p) = &self.at_pick else {
      return Vec::new();
    };
    let query: String = self
      .input
      .chars()
      .skip(p.start + 1)
      .take(self.at().saturating_sub(p.start + 1))
      .collect();
    super::files::rank(&self.files, &query, 50)
  }

  /// `@path` in place of what was typed after the `@`; a text file small
  /// enough is attached too.
  fn pick_file(&mut self, path: &str) {
    let Some(p) = self.at_pick.take() else { return };
    let at = self.at();
    self.remove(p.start..at);
    let start = self.at();
    let b = self.byte(start);
    let written = format!("@{path} ");
    self.input.insert_str(b, &written);
    self.cursor = Some(start + written.chars().count());
    let name = path.trim_end_matches('/');
    let full = match &self.files_root {
      Some(root) => root.join(name),
      None => std::path::PathBuf::from(name),
    };
    match super::files::attachable(&full, ATTACH_MAX) {
      Ok(()) => {
        let attached = full.display().to_string();
        if !self.attachments.contains(&attached) {
          self.attachments.push(attached.clone());
          self.mentions.push(Mention {
            path: name.to_owned(),
            attached,
          });
        }
        self.status = format!("@{name} attached");
      }
      Err(why) => self.status = format!("@{name}: {why}, named only"),
    }
  }

  /// Attaches files (pasted, dropped) as `@name` at the cursor: deleting
  /// the name takes the file back, as for files picked with `@`.
  pub fn attach_files(&mut self, files: &[std::path::PathBuf]) {
    let mut names = Vec::new();
    for file in files {
      let attached = file.display().to_string();
      let Some(name) = file.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        continue;
      };
      if self.attachments.contains(&attached) {
        continue;
      }
      let at = self.at();
      let before = self.input.chars().nth(at.wrapping_sub(1));
      let written = match before {
        Some(c) if !c.is_whitespace() => format!(" @{name} "),
        _ => format!("@{name} "),
      };
      let b = self.byte(at);
      self.input.insert_str(b, &written);
      self.cursor = Some(at + written.chars().count());
      self.attachments.push(attached.clone());
      self.mentions.push(Mention {
        path: name.clone(),
        attached,
      });
      names.push(format!("@{name}"));
    }
    self.at_pick = None;
    self.status = match names.len() {
      0 => "already attached".into(),
      _ => format!("{} attached", names.join(" ")),
    };
  }

  /// Files named with `@` whose name left the input are not attached.
  fn prune_mentions(&mut self) {
    let input = self.input.clone();
    let (kept, gone): (Vec<Mention>, Vec<Mention>) = std::mem::take(&mut self.mentions)
      .into_iter()
      .partition(|m| super::files::mentioned(&input, &m.path));
    for m in gone {
      if let Some(i) = self.attachments.iter().position(|a| *a == m.attached) {
        self.attachments.remove(i);
      }
    }
    self.mentions = kept;
  }

  /// A new thread here, or (`global`) one that belongs to no folder.
  fn new_thread(&mut self, global: bool) -> Action {
    self.overlay = Overlay::None;
    self.global = global;
    // The status line shows where it goes.
    self.status.clear();
    if self.thread.is_none() && self.pending.is_none() {
      return Action::None;
    }
    Action::Open(None)
  }

  fn copy(&mut self) -> Action {
    let streaming = self
      .pending
      .as_ref()
      .map(|p| p.reply.clone())
      .filter(|r| !r.is_empty());
    match streaming.or_else(|| self.last_reply.clone()) {
      Some(text) => {
        self.status = "reply copied".into();
        Action::Copy(text)
      }
      None => {
        self.status = "no reply to copy".into();
        Action::None
      }
    }
  }

  pub fn submit(&mut self) -> Action {
    let mut text = self.input.trim().to_owned();
    if text.is_empty() {
      return Action::None;
    }
    // `/1`…`/6`: a preset answer (design §2.30), sent as if typed (a
    // message, even one starting with `/`). With none live, `/1`…`/24`
    // runs an action button of the latest set (step 2).
    let mut picked = false;
    if let Some(n) = text
      .strip_prefix('/')
      .filter(|d| (1..=2).contains(&d.len()))
      .and_then(|d| d.parse::<usize>().ok())
    {
      match (
        self.live_replies().map(|(_, r)| r),
        self.live_actions(),
        &self.thread,
      ) {
        (Some(r), _, _) => match r.get(n.wrapping_sub(1)) {
          Some(r) => (text, picked) = (r.text.clone(), true),
          None => {
            self.status = format!("/{n}: there are {} preset replies", r.len());
            return Action::None;
          }
        },
        (None, Some((_, set, actions)), Some(thread)) => {
          let Some(a) = actions.get(n.wrapping_sub(1)) else {
            self.status = format!("/{n}: there are {} action buttons", actions.len());
            return Action::None;
          };
          let press = Action::Press {
            thread: thread.clone(),
            set: set.to_owned(),
            item: (n - 1) as u32,
          };
          self.status = format!("▶ {}: running…", a.label);
          self.input.clear();
          self.cursor = None;
          return press;
        }
        _ => {
          self.status = "no preset replies or action buttons to pick now".into();
          return Action::None;
        }
      }
    }
    if text == "/" && !picked {
      self.input.clear();
      self.cursor = None;
      let choices = COMMANDS
        .iter()
        .map(|(c, what, _)| Choice::new(*c, *what))
        .collect();
      return self.open(Picker::new("commands", Then::Command, choices), None);
    }
    if let Some(cmd) = text.strip_prefix('/').filter(|_| !picked) {
      self.input.clear();
      self.cursor = None;
      let (name, arg) = cmd
        .split_once(' ')
        .map_or((cmd, ""), |(n, a)| (n, a.trim()));
      match name {
        "attach" | "a" if !arg.is_empty() => {
          let path = expand_home(arg);
          if std::path::Path::new(&path).is_file() {
            self.attachments.push(path);
            self.status = format!("{} attached", self.attachments.len());
          } else {
            self.status = format!("no file {path}");
          }
        }
        "link" if !arg.is_empty() => return Action::Link(Some(arg.to_owned())),
        "link" => {
          return self.open(
            Picker::new("link this folder with one on", Then::Link, Vec::new()),
            Some(Fetch::Peers),
          );
        }
        "spawn" if arg.is_empty() => {
          return self.open(
            Picker::new("spawn on", Then::SpawnNode, Vec::new()),
            Some(Fetch::Peers),
          );
        }
        "spawn" if arg.starts_with('@') && !arg.contains(' ') => {
          let to = &arg[1..];
          if to.contains(':') {
            self.input = format!("/spawn {arg} ");
            self.cursor = None;
            self.status = format!("the goal for {to}, then Enter");
          } else {
            return self.pick_model_on(to);
          }
        }
        "parent" => return self.open_parent(),
        "children" => return self.open_children(),
        "rename" | "pin" | "quote" => {
          let Some(thread) = self.thread.clone() else {
            self.status = "no thread yet".into();
            return Action::None;
          };
          return match name {
            "rename" if arg.trim().is_empty() => {
              self.status = "/rename <title>".into();
              Action::None
            }
            "rename" => Action::Rename {
              thread,
              title: arg.trim().to_owned(),
            },
            "pin" => {
              let on = !self.threads.iter().any(|t| t.id == thread && t.pinned);
              Action::Pin { thread, on }
            }
            _ => Action::Quote(thread),
          };
        }
        "search" => {
          let (here, query) = match arg.strip_prefix("--here") {
            Some(rest) => (true, rest.trim()),
            None => (false, arg.trim()),
          };
          if query.is_empty() {
            self.status = "/search <words>".into();
            return Action::None;
          }
          let title = format!("search: {query}{}", if here { " (here)" } else { "" });
          return self.open(
            Picker::new(&title, Then::OpenThread, Vec::new()),
            Some(Fetch::Search {
              query: query.to_owned(),
              here,
            }),
          );
        }
        "fork" | "summarize" | "compact" => {
          let Some(thread) = self.thread.clone() else {
            self.status = "no thread to fork yet".into();
            return Action::None;
          };
          if self.pending.is_some() {
            self.status = "a turn runs: fork once it ends (Esc stops it)".into();
            return Action::None;
          }
          // Alone, `/fork` and `/compact` open the form (`/compact`'s
          // preset: a summary, then the last 10); typed flags stay the
          // fast path.
          if arg.trim().is_empty() && name != "summarize" {
            let mut form = ForkForm::new(thread, if name == "fork" { "fork" } else { "compact" });
            if name == "compact" {
              form.last_on = true;
              form.summarize = true;
            }
            self.open_fork(form);
            return Action::None;
          }
          let mut last = None;
          let (mut no_attachments, mut no_tools, mut summary) = (false, false, false);
          let mut upto = None;
          for w in arg.split_whitespace() {
            match w {
              "-a" | "--no-attachments" => no_attachments = true,
              "-t" | "--no-tools" => no_tools = true,
              "-s" | "--summarize" => summary = true,
              // Up to a message (the message picker's options).
              m if m.len() > 1 && m.starts_with('@') => upto = Some(m[1..].to_owned()),
              n => match n.parse::<usize>() {
                Ok(n) if n > 0 => last = Some(n),
                _ => {
                  self.status = format!("/{name}: `{n}` is not a count, @message, -a, -t or -s");
                  return Action::None;
                }
              },
            }
          }
          let summarize = name != "fork" || summary;
          if name == "compact" {
            last = last.or(Some(10));
          }
          if name == "summarize" {
            last = None;
          }
          self.status = if summarize {
            "summarizing (a model call)…".into()
          } else {
            "forking…".into()
          };
          return Action::Fork {
            thread,
            upto,
            last,
            no_attachments,
            no_tools,
            summarize,
          };
        }
        "spawn" => {
          // `@node[:model]` first names where; else here, on the model
          // picked here (none picked: the node's default).
          let (node, model, goal) = match arg.strip_prefix('@') {
            Some(rest) => {
              let (to, goal) = rest.split_once(' ').unwrap_or((rest, ""));
              match to.split_once(':') {
                Some((n, m)) => (Some(n.to_owned()), Some(m.to_owned()), goal),
                None => (Some(to.to_owned()), self.model.clone(), goal),
              }
            }
            None => (None, self.model.clone(), arg),
          };
          if goal.trim().is_empty() {
            self.status = "a goal after the machine, then Enter".into();
            return Action::None;
          }
          self.status = format!(
            "spawning{}",
            node
              .as_ref()
              .map(|n| format!(" on {n}"))
              .unwrap_or_default()
          );
          return Action::Spawn {
            node,
            model,
            goal: goal.trim().to_owned(),
          };
        }
        "send" if arg.is_empty() => {
          let choices = self
            .threads
            .iter()
            .filter(|t| Some(&t.id) != self.thread.as_ref())
            .map(|t| Choice {
              value: t.id.clone(),
              label: t.title.clone(),
              detail: t.when.clone(),
            })
            .collect();
          return self.open(Picker::new("send to", Then::SendTo, choices), None);
        }
        "send" => {
          let Some((thread, text)) = arg.split_once(' ') else {
            self.input = format!("/send {arg} ");
            self.cursor = None;
            self.status = "the message, then Enter".into();
            return Action::None;
          };
          self.status = "sending to the thread".into();
          return Action::SendTo {
            thread: thread.to_owned(),
            text: text.trim().to_owned(),
          };
        }
        "unlink" => return Action::Link(None),
        "yolo" | "auto" => {
          let on = match arg {
            "on" => true,
            "off" => false,
            _ => self.mode != name,
          };
          return Action::Mode(if on { name.to_owned() } else { "ask".into() });
        }
        "ask" => return Action::Mode("ask".into()),
        "run-on" => {
          let Some(thread) = self.thread.clone() else {
            self.status = "no thread yet".into();
            return Action::None;
          };
          if arg.is_empty() {
            self.status = match &self.runs_on {
              Some(n) => format!("this thread runs on {n}: /run-on here brings it back"),
              None => "this thread runs where it is typed: /run-on <node>".into(),
            };
            return Action::None;
          }
          return Action::RunOn {
            thread,
            node: (arg != "here").then(|| arg.to_owned()),
          };
        }
        "read-only" => {
          let Some(thread) = self.thread.clone() else {
            self.status = "no thread yet".into();
            return Action::None;
          };
          return match arg {
            "on" | "off" => Action::ReadOnly {
              thread,
              on: arg == "on",
            },
            _ => {
              self.status = format!(
                "this thread is {}: /read-only on|off",
                if self.read_only {
                  "read-only"
                } else {
                  "writable"
                }
              );
              Action::None
            }
          };
        }
        "memory" if matches!(arg, "on" | "off") => {
          return self.switch_memory(arg == "on");
        }
        "tools" | "skills" => {
          let skills = name == "skills";
          let title = if self.thread.is_some() {
            format!("this thread's {name}")
          } else {
            format!("the new thread's {name}")
          };
          let then = if skills { Then::Skills } else { Then::Tools };
          let fetch = self.toolset_fetch(skills, None);
          return self.open(Picker::new(&title, then, Vec::new()), Some(fetch));
        }
        "memory" => {
          let Some(on) = self.memory else {
            self.status = "memory does not run on this node (`stitch memory install`)".into();
            return Action::None;
          };
          let now = |state: bool| if state == on { " (now)" } else { "" };
          let choices = vec![
            Choice::new("on", format!("remembered and recalled{}", now(true))),
            Choice::new("off", format!("kept out of memory{}", now(false))),
          ];
          let mut p = Picker::new("this thread's memory", Then::Memory, choices);
          p.picked = usize::from(!on);
          return self.open(p, None);
        }
        "drop" => {
          self.attachments.clear();
          self.mentions.clear();
          self.status = "attachments dropped".into();
        }
        "model" | "m" if arg.is_empty() => {
          return self.open(
            Picker::new("model", Then::Model, Vec::new()),
            Some(Fetch::Models(None)),
          );
        }
        "model" | "m" => {
          self.model = Some(arg.to_owned());
          self.status = format!("model {}", self.model_name());
        }
        "login" | "logout" if arg.is_empty() => {
          let then = if name == "login" {
            Then::Login
          } else {
            Then::Logout
          };
          let choices = vec![
            Choice::new("anthropic", "Claude subscription"),
            Choice::new("openai", "ChatGPT subscription (Codex)"),
            Choice::new("grok", "SuperGrok subscription"),
            Choice::new("github", "GitHub account"),
          ];
          return self.open(Picker::new(&format!("{name} to"), then, choices), None);
        }
        "login" | "logout" | "auth" => {
          if self.auth_pending {
            self.status = "sign-in is already in progress; complete it in the browser".into();
            self.overlay = Overlay::Auth;
            return Action::None;
          }
          let args: Vec<_> = arg.split_whitespace().collect();
          let action = match (name, args.as_slice()) {
            ("login", [provider]) => AuthAction::Login {
              provider: (*provider).into(),
              file: false,
            },
            ("login", [provider, "--file"]) => AuthAction::Login {
              provider: (*provider).into(),
              file: true,
            },
            ("logout", [provider]) => AuthAction::Logout {
              provider: (*provider).into(),
            },
            ("auth", []) => AuthAction::Status { provider: None },
            ("auth", [provider]) => AuthAction::Status {
              provider: Some((*provider).into()),
            },
            _ => {
              self.status =
                "use /login <provider> [--file], /logout <provider>, or /auth [provider]".into();
              return Action::None;
            }
          };
          self.auth_pending = true;
          self.auth_text = "Checking subscription sign-in…".into();
          self.overlay = Overlay::Auth;
          return Action::Auth(action);
        }
        "copy" => return self.copy(),
        "app" => {
          let what = arg.trim();
          if !what.is_empty() {
            return Action::OpenApp(what.to_owned());
          }
          let Some(thread) = self.thread.clone() else {
            self.status = "no thread yet: /app <pinned app>".into();
            return Action::None;
          };
          return Action::OpenApp(thread);
        }
        "threads" | "t" => self.open_picker(),
        "jobs" => return self.open_jobs(),
        "new" => return self.new_thread(arg == "global"),
        "quit" | "q" => return Action::Quit,
        "help" | "h" | "?" => self.out.push(Printed::Notice(HELP.into())),
        _ => {
          self.status = format!("no command /{name}: /help lists them");
        }
      }
      return Action::None;
    }
    // Files named with `@` go only while their names stay.
    self.prune_mentions();
    self.mentions.clear();
    self.at_pick = None;
    if self.pending.is_some() {
      // Typed while it works: given to it after its tool calls, or sent
      // when it ends; editable till then.
      self.input.clear();
      self.cursor = None;
      self.editing = None;
      self.queued_id += 1;
      let q = Queued {
        id: self.queued_id,
        text,
        attachments: std::mem::take(&mut self.attachments),
      };
      self.queued.push(q.clone());
      self.status = format!(
        "{} queued: woven in after its tool calls, or sent when it ends · ↑ edits · Esc interrupts",
        self.queued.len()
      );
      return Action::Interject {
        id: q.id,
        text: q.text,
        attachments: q.attachments,
      };
    }
    self.input.clear();
    self.cursor = None;
    let attachments = std::mem::take(&mut self.attachments);
    let mut shown = text.clone();
    for a in &attachments {
      shown.push_str(&format!("\n[{a}]"));
    }
    self.out.push(Printed::You(shown));
    self.pending = Some(Pending {
      since: Some(std::time::Instant::now()),
      new: self.thread.is_none(),
      sent: text.clone(),
      ..Pending::default()
    });
    self.overlay = Overlay::None;
    self.status = "sending".into();
    let new = self.thread.is_none();
    Action::Send {
      thread: self.thread.clone(),
      text,
      attachments,
      model: self.model.clone(),
      global: self.global && new,
      tools: if new {
        std::mem::take(&mut self.new_tools)
      } else {
        Vec::new()
      },
      skills: if new {
        std::mem::take(&mut self.new_skills)
      } else {
        Vec::new()
      },
      // Kept until another thread opens: the status line goes on showing
      // it, and a first message that failed is sent with it again.
      memory: if new { self.new_memory } else { None },
    }
  }

  /// `/memory on|off`: the open thread is switched by the node; a new
  /// one, not written yet, keeps the choice for its first message.
  fn switch_memory(&mut self, on: bool) -> Action {
    if self.thread.is_some() {
      return Action::Remember {
        thread: self.thread.clone(),
        on,
      };
    }
    if self.memory.is_none() {
      self.status = "memory does not run on this node (`stitch memory install`)".into();
      return Action::None;
    }
    self.new_memory = Some(on);
    self.memory = Some(on);
    self.overlay = Overlay::None;
    self.status = if on {
      "the new thread will be remembered".into()
    } else {
      "the new thread will be kept out of memory".into()
    };
    Action::None
  }

  /// Memory's state as the node said it: a new thread's choice, not
  /// sent yet, stands.
  pub fn on_memory(&mut self, state: Option<bool>) {
    self.memory = match (state, &self.thread, self.new_memory) {
      (Some(_), None, Some(chosen)) => Some(chosen),
      (state, ..) => state,
    };
  }

  /// The queued messages, as one, once nothing runs.
  pub fn send_queued(&mut self) -> Option<Action> {
    if self.pending.is_some() || self.queued.is_empty() {
      return None;
    }
    let (texts, atts): (Vec<String>, Vec<Vec<String>>) = std::mem::take(&mut self.queued)
      .into_iter()
      .map(|q| (q.text, q.attachments))
      .unzip();
    let typed = std::mem::replace(&mut self.input, texts.join("\n\n"));
    let attached = std::mem::replace(&mut self.attachments, atts.concat());
    let action = self.submit();
    // What was being typed meanwhile stays.
    self.input = typed;
    self.cursor = None;
    self.attachments.extend(attached);
    Some(action)
  }

  /// Where the open thread (or the next new one) belongs.
  pub fn place(&self) -> String {
    match &self.thread {
      Some(id) => self
        .threads
        .iter()
        .find(|t| &t.id == id)
        .map(|t| t.place.clone())
        .or_else(|| self.family_of(id).map(|f| f.place.clone()))
        .flatten()
        .unwrap_or_else(|| self.here.clone()),
      None if self.global => "global".into(),
      None => self.here.clone(),
    }
  }

  /// A command waits for the user's approval.
  pub fn ask(&mut self, session: String, command: String, question: String, grant: Vec<String>) {
    self.status = if grant.is_empty() {
      "approve? y/n".into()
    } else {
      "approve? y/n/a".into()
    };
    self.approval = Some((session, command));
    self.approval_at = Some(std::time::Instant::now());
    self.approval_grant = grant;
    self.told.insert(question.clone());
    self.approval_question = Some(question);
  }

  /// The approval on screen was answered elsewhere, or its loop stopped.
  pub fn unask(&mut self) {
    if self.approval.take().is_some() {
      if self.answering.is_none() {
        self.reason = None;
      }
      self.approval_question = None;
      self.status = "the approval was answered elsewhere".into();
    }
  }

  /// The attention list as read again: questions that came since are
  /// told once, in the conversation (where a spawned thread's approval
  /// would otherwise go unseen).
  pub fn attention_read(&mut self, attention: Vec<Needs>, first: bool) {
    for n in &attention {
      let Some(q) = &n.question else { continue };
      if self.told.insert(q.clone()) && !first {
        self.out.push(Printed::Notice(format!(
          "⚑ needs you: {} · Ctrl+T to answer",
          n.text
        )));
      }
    }
    self.attention = attention;
    self.picked_needs = self
      .picked_needs
      .min(self.attention.len().saturating_sub(1));
  }

  /// Lets go of the turn running (another thread is opened): it goes on
  /// at the node; what was typed for it and not yet taken is shown, not
  /// lost.
  pub fn let_go(&mut self) {
    if self.pending.is_none() {
      return;
    }
    self.flush_reply(&self.model_label());
    self.pending = None;
    self.approval = None;
    self.approval_question = None;
    if self.answering.is_none() {
      self.reason = None;
    }
    self.editing = None;
    let mut note = LET_GO.to_owned();
    for q in std::mem::take(&mut self.queued) {
      note.push_str(&format!(
        "\nNot sent (typed for it, not yet taken): {}",
        q.text
      ));
    }
    self.out.push(Printed::Notice(note));
  }

  /// Prints the reply streamed so far, before what comes after it.
  /// Ends the reply streamed so far: its last line is printed, then the
  /// end of the reply, before what comes after it.
  fn flush_reply(&mut self, model: &str) {
    let Some(p) = &mut self.pending else { return };
    if !p.started {
      if p.partial.trim().is_empty() {
        p.partial.clear();
        return;
      }
      p.started = true;
      self.out.push(Printed::ReplyStart(model.to_owned()));
    }
    let rest = std::mem::take(&mut p.partial);
    if !rest.is_empty() {
      self.out.push(Printed::ReplyLines(rest));
    }
    self.out.push(Printed::ReplyEnd);
    p.started = false;
    p.md = Default::default();
    let text = std::mem::take(&mut p.reply);
    if !text.trim().is_empty() {
      self.last_reply = Some(text);
    }
  }

  /// Prints the completed lines of the streaming reply.
  fn stream(&mut self, model: &str, t: &str) {
    let Some(p) = &mut self.pending else { return };
    p.thinking.clear();
    p.reply.push_str(t);
    p.partial.push_str(t);
    if let Some(end) = p.partial.rfind('\n') {
      let done: String = p.partial.drain(..=end).collect();
      let done = done.strip_suffix('\n').unwrap_or(&done).to_owned();
      if !p.started {
        if done.trim().is_empty() {
          return;
        }
        p.started = true;
        self.out.push(Printed::ReplyStart(model.to_owned()));
      }
      p.md.push(&done);
      self.out.push(Printed::ReplyLines(done));
    }
  }

  pub fn on_reply(&mut self, r: &Reply) {
    if self.pending.is_none() {
      return;
    }
    let model = self.model_label();
    match r {
      Reply::Thread(id) => self.thread = Some(id.clone()),
      Reply::Kept => {
        if let Some(p) = &mut self.pending {
          p.kept = true;
        }
      }
      Reply::Text(t) => self.stream(&model, t),
      Reply::Thinking(t) => {
        if let Some(p) = &mut self.pending {
          p.thinking.push_str(t);
        }
      }
      Reply::ToolCall(line) => {
        self.flush_reply(&model);
        if let Some(p) = &mut self.pending {
          p.thinking.clear();
          p.tool = Some(line.clone());
          p.now_doing();
        }
        self.out.push(Printed::Tool(line.clone()));
      }
      Reply::ToolResult { ok, output } => {
        if let Some(p) = &mut self.pending {
          p.tool = None;
          p.now_doing();
        }
        let lines: Vec<&str> = output.lines().collect();
        let mut text = lines
          .iter()
          .take(OUTPUT_LINES)
          .copied()
          .collect::<Vec<_>>()
          .join("\n");
        if lines.len() > OUTPUT_LINES {
          text.push_str(&format!("\n[… {} more lines]", lines.len() - OUTPUT_LINES));
        }
        self.out.push(if *ok {
          Printed::Output(text)
        } else {
          Printed::Error(text)
        });
      }
      Reply::App { name, url } => {
        self.flush_reply(&model);
        self.out.push(Printed::Notice(match url {
          Some(url) => format!("App \"{name}\": {url}\n(/app opens it in the browser)"),
          None => format!("App \"{name}\" shown (this node serves no apps to a browser)"),
        }));
      }
      Reply::Woven(ids) if self.pending.as_ref().is_some_and(|p| p.waiting) => {
        // Gone into the turn that runs on the thread (0: the message
        // itself): it shows where it went as that turn is followed.
        for id in ids {
          if *id == 0 {
            self.status =
              "your message went into the turn running on this thread · Enter queue · Esc interrupts"
                .into();
          } else if let Some(i) = self.queued.iter().position(|q| q.id == *id) {
            self.queued.remove(i);
          } else if self.editing.take_if(|q| q.id == *id).is_some() {
            self.status = "the message being edited had already gone to the model".into();
          }
        }
      }
      Reply::Woven(ids) => {
        // Given to the model now: in the transcript where it went.
        for id in ids {
          let q = match self.queued.iter().position(|q| q.id == *id) {
            Some(i) => self.queued.remove(i),
            None => match self.editing.take_if(|q| q.id == *id) {
              Some(q) => {
                self.status = "the message being edited had already gone to the model".into();
                q
              }
              None => continue,
            },
          };
          self.flush_reply(&model);
          let mut shown = q.text;
          for a in &q.attachments {
            shown.push_str(&format!("\n[{a}]"));
          }
          self.out.push(Printed::You(shown));
        }
      }
      Reply::Note(text) => {
        // A job ended, a report came: given to the model now.
        self.flush_reply(&model);
        self.out.push(Printed::Notice(text.clone()));
      }
      Reply::Compacted(c) => {
        // The turn goes on in the new thread: one conversation, a divider.
        self.flush_reply(&model);
        self.out.push(Printed::Compacted {
          line: c.said(),
          summary: c.summary.clone(),
        });
        self.thread = Some(c.to.clone());
      }
      Reply::Retry(why) => {
        // The model call is sent again: what it wrote so far is dropped,
        // and the reply starts over below a notice.
        if let Some(p) = &mut self.pending {
          p.reply.clear();
          p.partial.clear();
          p.thinking.clear();
          p.started = false;
        }
        self.out.push(Printed::Notice(format!(
          "[cut off by the network, sending it again: {why}]"
        )));
      }
      Reply::Fetched(n) => self.status = format!("fetched {n} attachment(s) from peers"),
      Reply::Waiting(waiting) => {
        if let Some(p) = &mut self.pending {
          p.waiting = *waiting;
          p.now_doing();
          if *waiting && !p.sent.is_empty() {
            self.echo = Some(p.sent.clone());
          }
        }
        self.status = if *waiting {
          self.out.push(Printed::Notice(WAITING.into()));
          "waiting for the turn running on this thread · Enter queue · Esc interrupts".into()
        } else {
          "sending".into()
        };
      }
      Reply::Afterthought(t) => {
        // Memory's unprompted follow-up: a reply of its own.
        self.flush_reply(&model);
        self
          .out
          .push(Printed::Notice("memory: an afterthought".into()));
        self.stream(&model, t);
        self.stream(&model, "\n");
        self.flush_reply(&model);
      }
      _ => {}
    }
  }

  /// The node finished a chat; returns the thread it went to.
  pub fn on_done(&mut self, result: Result<Reply, String>) -> Option<String> {
    match result {
      Ok(Reply::ChatDone {
        thread,
        model,
        stop,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        usd,
        thread_usd,
        ..
      }) => {
        self.flush_reply(&self.model_label());
        self.pending = None;
        self.approval = None;
        self.approval_question = None;
        if stop == "interrupted" && !self.queued.is_empty() {
          // Stopped: what waited comes back to the input, not sent.
          let mut texts: Vec<String> = Vec::new();
          for q in std::mem::take(&mut self.queued) {
            texts.push(q.text);
            self.attachments.extend(q.attachments);
          }
          if !self.input.is_empty() {
            texts.push(std::mem::take(&mut self.input));
          }
          self.input = texts.join("\n\n");
          self.cursor = None;
        }
        if let Some(category) = stop.strip_prefix("refusal") {
          self
            .out
            .push(Printed::Refusal(format!("declined{category}")));
        }
        let cost = match (usd, thread_usd) {
          (Some(u), Some(t)) if t > u => format!(" · ${u:.4} (thread ${t:.2})"),
          (Some(u), _) => format!(" · ${u:.4}"),
          _ => String::new(),
        };
        let stop = if stop == "end_turn" {
          String::new()
        } else {
          format!(" · {stop}")
        };
        let tokens = stitch_node::api::tokens_text(
          input_tokens,
          cache_read_tokens,
          cache_write_tokens,
          output_tokens,
        );
        self.status = if stop == " · woven" {
          // It went into the turn that ran on the thread, which ended.
          "your message went into the turn that ran on this thread".into()
        } else {
          format!("{model} · {tokens}{cost}{stop}")
        };
        self.thread = Some(thread.clone());
        self.global = false;
        Some(thread)
      }
      // Interrupted while another turn ran on the thread: its message was
      // never sent. It leaves the conversation and comes back to the
      // input, with its attachments and what was queued after it.
      Ok(Reply::Unsent) => {
        self.pending = None;
        self.approval = None;
        self.approval_question = None;
        let mut texts: Vec<String> = Vec::new();
        if let Some(at) = self.out.iter().rposition(|p| matches!(p, Printed::You(_)))
          && let Printed::You(shown) = self.out.remove(at)
        {
          let mut lines: Vec<&str> = shown.lines().collect();
          let mut attachments = Vec::new();
          while let Some(a) = lines
            .last()
            .and_then(|l| l.strip_prefix('[')?.strip_suffix(']'))
          {
            attachments.push(a.to_owned());
            lines.pop();
          }
          attachments.reverse();
          texts.push(lines.join("\n"));
          self.attachments.splice(0..0, attachments);
        }
        for q in std::mem::take(&mut self.queued) {
          texts.push(q.text);
          self.attachments.extend(q.attachments);
        }
        if !self.input.is_empty() {
          texts.push(std::mem::take(&mut self.input));
        }
        self.input = texts.join("\n\n");
        self.cursor = None;
        self.status = "interrupted before it was sent: the message is back in the input".into();
        None
      }
      Ok(other) => {
        self.pending = None;
        self.status = format!("unexpected reply: {other:?}");
        None
      }
      Err(e) => {
        self.flush_reply(&self.model_label());
        let (kept, new) = self
          .pending
          .take()
          .map_or((false, false), |p| (p.kept, p.new));
        self.out.push(Printed::Error(e.clone()));
        if kept {
          // In the thread already: the next message goes on from it.
          self.status = "error; your message is kept in the thread".into();
          return None;
        }
        // A new thread that was never written: the message starts it again.
        if new {
          self.thread = None;
        }
        self.status = "error; the message is back in the input".into();
        if let Some(Printed::You(text)) =
          self.out.iter().rev().find(|p| matches!(p, Printed::You(_)))
        {
          self.input = text
            .lines()
            .filter(|l| !l.starts_with('['))
            .collect::<Vec<_>>()
            .join("\n");
          self.cursor = None;
        }
        None
      }
    }
  }

  /// Shows what runs in the background, asking the node for it.
  fn open_jobs(&mut self) -> Action {
    self.overlay = Overlay::Jobs;
    self.picked_job = 0;
    Action::Jobs
  }

  /// What the node said runs in the background.
  pub fn jobs_listed(&mut self, jobs: Vec<Job>) {
    self.jobs = jobs;
    self.picked_job = self.picked_job.min(self.jobs.len().saturating_sub(1));
  }

  /// Opens a picker; one that waits for the node asks with `fetch`.
  fn open(&mut self, picker: Picker, fetch: Option<Fetch>) -> Action {
    self.pick = Some(Picker {
      waiting: fetch.is_some(),
      note: if fetch.is_some() {
        "asking the node…".into()
      } else {
        String::new()
      },
      ..picker
    });
    self.overlay = Overlay::Pick;
    match fetch {
      Some(f) => Action::Fetch(f),
      None => Action::None,
    }
  }

  /// Picks the model on `node` to spawn on it with: its own list, the
  /// model picked here first when it has it, else its default.
  fn pick_model_on(&mut self, node: &str) -> Action {
    self.delegate_to = Some(node.to_owned());
    self.open(
      Picker::new(&format!("model on {node}"), Then::SpawnModel, Vec::new()),
      Some(Fetch::Models(Some(node.to_owned()))),
    )
  }

  /// What the node answered for the open picker.
  pub fn filled(&mut self, choices: Vec<Choice>, note: String) {
    let current = self.model_name().to_owned();
    let chosen = self.model.clone();
    if let Some(p) = &mut self.pick {
      p.waiting = false;
      p.note = if choices.is_empty() && note.is_empty() {
        "nothing to pick".into()
      } else {
        note
      };
      p.choices = choices;
      // The current model, when it is there.
      if p.then == Then::Model {
        p.picked = p
          .choices
          .iter()
          .position(|c| c.value == current)
          .unwrap_or(0);
      }
      if p.then == Then::SpawnNode {
        p.choices
          .insert(0, Choice::new(self.node.clone(), "this machine"));
        if p.note.starts_with("no other machines") {
          p.note.clear();
        }
      }
      if p.then == Then::SpawnModel {
        p.picked = chosen
          .and_then(|m| p.choices.iter().position(|c| c.value == m))
          .or_else(|| p.choices.iter().position(|c| c.detail == "default"))
          .unwrap_or(0);
      }
    }
  }

  fn pick_key(&mut self, key: KeyEvent) -> Action {
    let Some(p) = &mut self.pick else {
      self.overlay = Overlay::None;
      return Action::None;
    };
    let n = p.shown().len();
    match key.code {
      KeyCode::Esc => {
        self.pick = None;
        self.overlay = Overlay::None;
        Action::None
      }
      KeyCode::Up => {
        p.picked = p.picked.saturating_sub(1);
        Action::None
      }
      KeyCode::Down => {
        p.picked = (p.picked + 1).min(n.saturating_sub(1));
        Action::None
      }
      KeyCode::Backspace => {
        p.filter.pop();
        p.picked = 0;
        Action::None
      }
      KeyCode::Char(c) => {
        p.filter.push(c);
        p.picked = 0;
        Action::None
      }
      KeyCode::Enter => {
        let then = p.then;
        let Some(choice) = p
          .shown()
          .get(p.picked.min(n.saturating_sub(1)))
          .map(|c| (*c).clone())
        else {
          return Action::None;
        };
        if matches!(then, Then::Tools | Then::Skills) {
          // Toggled: the list stays, filled again with the new state.
          if p.waiting {
            return Action::None;
          }
          p.waiting = true;
          return Action::Fetch(self.toolset_fetch(then == Then::Skills, Some(choice.value)));
        }
        self.pick = None;
        self.overlay = Overlay::None;
        self.picked_choice(then, choice)
      }
      _ => Action::None,
    }
  }

  fn picked_choice(&mut self, then: Then, c: Choice) -> Action {
    match then {
      Then::Command => {
        let takes_words = COMMANDS
          .iter()
          .any(|(name, _, words)| *name == c.value && *words);
        if takes_words {
          self.input = format!("{} ", c.value);
          self.cursor = None;
          Action::None
        } else {
          self.input = c.value;
          self.cursor = None;
          self.submit()
        }
      }
      Then::Model => {
        self.model = Some(c.value);
        self.status = format!("model {}", self.model_name());
        Action::None
      }
      Then::SpawnNode => self.pick_model_on(&c.value),
      Then::SpawnModel => {
        let node = self.delegate_to.take().unwrap_or_default();
        self.input = format!("/spawn @{node}:{} ", c.value);
        self.cursor = None;
        self.status = format!("the goal for {node}, then Enter");
        Action::None
      }
      Then::SendTo => {
        self.input = format!("/send {} ", c.value);
        self.cursor = None;
        self.status = format!("the message for \"{}\", then Enter", c.label);
        Action::None
      }
      Then::Link => {
        self.input = format!("/link {}:", c.value);
        self.cursor = None;
        self.status = format!("the folder's path on {}, then Enter", c.value);
        Action::None
      }
      Then::Login | Then::Logout => {
        let verb = if then == Then::Login {
          "login"
        } else {
          "logout"
        };
        self.input = format!("/{verb} {}", c.value);
        self.cursor = None;
        self.submit()
      }
      Then::Memory => self.switch_memory(c.value == "on"),
      Then::Tools | Then::Skills => Action::None,
      Then::OpenThread => Action::Open(Some(c.value)),
    }
  }

  /// What to ask the node for a tool or skill set, `change` (`+x`, `-x`)
  /// made first: written for the open thread, kept here for a new one.
  fn toolset_fetch(&mut self, skills: bool, change: Option<String>) -> Fetch {
    match &self.thread {
      Some(t) => {
        let changes: Vec<String> = change.into_iter().collect();
        Fetch::Toolset {
          skills,
          thread: Some(t.clone()),
          global: false,
          tools: if skills { Vec::new() } else { changes.clone() },
          skill_changes: if skills { changes } else { Vec::new() },
        }
      }
      None => {
        if let Some(c) = change {
          if skills {
            self.new_skills.push(c);
          } else {
            self.new_tools.push(c);
          }
        }
        Fetch::Toolset {
          skills,
          thread: None,
          global: self.global,
          tools: self.new_tools.clone(),
          skill_changes: self.new_skills.clone(),
        }
      }
    }
  }

  /// Rows the live area needs, for a terminal `width` wide.
  pub fn height(&self, width: u16) -> u16 {
    let input = self.input_rows(width).0;
    let live = match self.overlay {
      Overlay::Threads => {
        let goal = self.picked_goal().is_some();
        (self.thread_rows().len() as u16 + 1).min(12) + 2 + u16::from(goal)
      }
      Overlay::Pick => {
        let p = self.pick.as_ref();
        let n = p.map_or(0, |p| p.shown().len()) as u16;
        n.clamp(1, 12) + 2 + u16::from(p.is_some_and(|p| !p.note.is_empty()))
      }
      Overlay::Attention => {
        (self.attention.len().max(1) as u16).min(8) + 2 + u16::from(self.reason.is_some())
      }
      Overlay::Jobs => (self.jobs.len().max(1) as u16 * 2).min(16) + 2,
      Overlay::Messages => self.msg_lines(width.saturating_sub(2)).len() as u16 + 2,
      Overlay::Fork => self.fork_lines(width.saturating_sub(2)).len() as u16 + 2,
      Overlay::Auth => {
        (Paragraph::new(self.auth_text.as_str())
          .wrap(Wrap { trim: false })
          .line_count(width.saturating_sub(2)) as u16)
          .min(10)
          + 2
      }
      Overlay::None => {
        let streaming = match &self.pending {
          Some(p) => {
            let text = self.live_text(p, width);
            (Paragraph::new(text)
              .wrap(Wrap { trim: false })
              .line_count(width) as u16)
              .clamp(1, 12)
          }
          None => {
            let lines = self.in_flight_lines();
            if lines.is_empty() {
              0
            } else {
              (Paragraph::new(Text::from(lines))
                .wrap(Wrap { trim: false })
                .line_count(width) as u16)
                .clamp(1, 12)
            }
          }
        };
        streaming
          + if self.approval.is_some() { 2 } else { 0 }
          + (self.queued.len() as u16).min(4)
          + self.at_lines().len() as u16
      }
    };
    live + input.min(INPUT_ROWS) + 2 + 1
  }

  fn live_text(&self, p: &Pending, width: u16) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if !p.thinking.is_empty() && p.partial.is_empty() {
      for l in p.thinking.lines() {
        lines.push(Line::from(Span::styled(
          l.to_owned(),
          Style::new().fg(Color::DarkGray).italic(),
        )));
      }
    }
    // A held table, then the line being written, as they render now.
    let mut md = p.md.clone();
    md.width = width;
    lines.extend(
      md.preview(&p.partial)
        .into_iter()
        .map(super::markdown::to_line),
    );
    // Waiting for another turn: what that turn runs now.
    if p.waiting {
      lines.extend(self.in_flight_lines());
    }
    // The spinner: it works, what at, and for how long.
    let elapsed = p.since.map(|t| t.elapsed()).unwrap_or_default();
    let spin = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
    let color = if p.waiting {
      Color::Yellow
    } else {
      Color::Cyan
    };
    lines.push(Line::from(vec![
      Span::styled(format!("{spin} "), Style::new().fg(color)),
      Span::styled(p.doing(), Style::new().fg(color)),
      Span::styled(
        format!(" · {} · Esc interrupts", spoken(elapsed)),
        Style::new().fg(Color::DarkGray),
      ),
    ]));
    Text::from(lines)
  }

  /// A turn this TUI did not start runs on the open thread: a call of it
  /// is in flight, or the list says the thread works.
  pub fn followed_turn_runs(&self) -> bool {
    let Some(open) = &self.thread else {
      return false;
    };
    self.pending.is_none()
      && (self.in_flight_call().is_some()
        || self
          .threads
          .iter()
          .any(|t| &t.id == open && super::tree::working(t)))
  }

  /// The open thread's running call, for a turn this TUI did not start
  /// (`in_flight`): its input whole (a command on its lines), then a
  /// spinner with what runs and for how long.
  pub fn in_flight_call(&self) -> Option<&stitch_node::chat::CallInFlight> {
    let (thread, step) = self.in_flight.as_ref()?;
    if self.thread.as_ref() != Some(thread) || self.pending.as_ref().is_some_and(|p| !p.waiting) {
      return None;
    }
    step.running()
  }

  fn in_flight_lines(&self) -> Vec<Line<'static>> {
    let Some(call) = self.in_flight_call() else {
      return Vec::new();
    };
    let step = &self.in_flight.as_ref().expect("in flight").1;
    let elapsed = call
      .since
      .and_then(|t| std::time::Duration::try_from(jiff::Timestamp::now().duration_since(t)).ok())
      .unwrap_or_default();
    let spin = SPINNER[(jiff::Timestamp::now().as_millisecond() / 100) as usize % SPINNER.len()];
    let of = if step.calls.len() > 1 {
      format!(
        " · call {} of {}",
        step.running.unwrap_or(0) + 1,
        step.calls.len()
      )
    } else {
      String::new()
    };
    let mut lines = Vec::new();
    // Its input whole, at most `IN_FLIGHT_ROWS` lines of it.
    let text = clean(&call.summary);
    let all: Vec<&str> = text.lines().collect();
    for (i, l) in all.iter().enumerate() {
      if i + 1 == IN_FLIGHT_ROWS && all.len() > IN_FLIGHT_ROWS {
        lines.push(Line::from(Span::styled(
          format!("  … {} more lines", all.len() - i),
          Style::new().fg(Color::DarkGray),
        )));
        break;
      }
      lines.push(Line::from(Span::styled(
        format!("  {l}"),
        Style::new().fg(Color::Gray),
      )));
    }
    // The spinner last, by the input, as a turn's own is.
    lines.push(Line::from(vec![
      Span::styled(format!("{spin} "), Style::new().fg(Color::Cyan)),
      Span::styled(
        format!("running {}", call.headline()),
        Style::new().fg(Color::Cyan),
      ),
      Span::styled(
        format!(" · {}{of}", spoken(elapsed)),
        Style::new().fg(Color::DarkGray),
      ),
    ]));
    lines
  }

  pub fn render(&self, f: &mut Frame) {
    let area = f.area();
    let input_lines = self.input_rows(area.width).0;
    let [live, input, status] = Layout::vertical([
      Constraint::Min(0),
      Constraint::Length(input_lines.min(INPUT_ROWS) + 2),
      Constraint::Length(1),
    ])
    .areas(area);
    match self.overlay {
      Overlay::Threads => self.render_threads(f, live),
      Overlay::Pick => self.render_pick(f, live),
      Overlay::Attention => self.render_attention(f, live),
      Overlay::Jobs => self.render_jobs(f, live),
      Overlay::Messages => self.render_messages(f, live),
      Overlay::Fork => self.render_fork(f, live),
      Overlay::Auth => f.render_widget(
        Paragraph::new(self.auth_text.as_str())
          .wrap(Wrap { trim: false })
          .block(frame("subscription sign-in · Esc")),
        live,
      ),
      Overlay::None => self.render_live(f, live),
    }
    self.render_input(f, input);
    let attached = match self.attachments.len() {
      0 => String::new(),
      n if self.mentions.is_empty() => format!(" · {n} attached"),
      n => format!(
        " · {n} attached ({})",
        clip(
          &self
            .mentions
            .iter()
            .map(|m| format!("@{}", m.path))
            .collect::<Vec<_>>()
            .join(" "),
          40
        )
      ),
    };
    let thread = match &self.thread {
      Some(id) => {
        // Where it hangs in the tree: above it (the key that goes up),
        // how many below it (the command that goes down).
        let mut s = match self.parent() {
          Some((_, parent)) => format!(
            "{} ↑ {} (^P)",
            clip(&self.thread_title(id), 30),
            clip(parent, 24)
          ),
          None => clip(&self.thread_title(id), 40),
        };
        if self.children() > 0 {
          s.push_str(&format!(" ↓ {} (/children)", self.children()));
        }
        s
      }
      None => "new thread".into(),
    };
    let needs = match self.attention.len() {
      0 => Span::raw(""),
      n => Span::styled(
        format!(" ⚑ {n} · Ctrl+T "),
        Style::new().fg(Color::Black).bg(Color::Yellow),
      ),
    };
    let line = Line::from(vec![
      Span::styled(format!(" {} ", self.node), Style::new().reversed()),
      needs,
      Span::raw(format!(
        " {} · {} · {}{attached}{}{}{}{}",
        self.model_name(),
        thread,
        clip_start(&self.place(), 30),
        match (&self.runs_on, self.read_only && self.thread.is_some()) {
          (Some(n), true) => format!(" · runs on {n} · read-only"),
          (Some(n), false) => format!(" · runs on {n}"),
          (None, true) => " · read-only".into(),
          (None, false) => String::new(),
        },
        match self.memory {
          Some(true) => " · memory on",
          Some(false) => " · memory off",
          None => "",
        },
        match self.mode.as_str() {
          "yolo" => " · yolo",
          "auto" => " · auto",
          _ => "",
        },
        if self.status.is_empty() { "" } else { " · " }
      )),
      Span::styled(self.status.clone(), Style::new().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(line), status);
  }

  fn render_live(&self, f: &mut Frame, area: Rect) {
    let mut lines = Vec::new();
    match &self.pending {
      Some(p) => lines.extend(self.live_text(p, area.width).lines),
      None => lines.extend(self.in_flight_lines()),
    }
    if let Some((session, command)) = &self.approval {
      let asked = if session == stitch_node::tools::STEPS_SESSION {
        command.clone()
      } else if session == "main" {
        format!("run: {command}")
      } else {
        format!("run [{session}]: {command}")
      };
      lines.push(Line::from(Span::styled(
        asked,
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
      )));
      let prompt = match &self.reason {
        Some(r) => format!("why not? (Enter sends, Esc skips): {r}▏"),
        None if self.approval_grant.is_empty() => "allow? y / n (n lets you say why)".into(),
        None => format!(
          "allow? y / n (n lets you say why) / a: always allow {} here",
          self
            .approval_grant
            .iter()
            .map(|g| format!("`{g}`"))
            .collect::<Vec<_>>()
            .join(", ")
        ),
      };
      lines.push(Line::from(Span::styled(
        prompt,
        Style::new().fg(Color::Yellow),
      )));
    }
    // What waits to be sent, one line each (the last ones).
    for q in self.queued.iter().rev().take(4).rev() {
      let first = q.text.lines().next().unwrap_or("");
      let more = if q.text.lines().count() > 1 || !q.attachments.is_empty() {
        " …"
      } else {
        ""
      };
      lines.push(Line::from(Span::styled(
        format!("queued: {first}{more}"),
        Style::new().fg(Color::DarkGray).italic(),
      )));
    }
    // The `@` picker, under it all, by the input.
    lines.extend(self.at_lines());
    let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    // Show the end of what streams.
    let total = para.line_count(area.width);
    let top = total.saturating_sub(area.height as usize);
    f.render_widget(para.scroll((top.min(u16::MAX as usize) as u16, 0)), area);
  }

  /// The `@` picker's rows: its best files, then a hint.
  fn at_lines(&self) -> Vec<Line<'static>> {
    let Some(p) = &self.at_pick else {
      return Vec::new();
    };
    let choices = self.at_choices();
    let picked = p.picked.min(choices.len().saturating_sub(1));
    // A window of rows that keeps the picked one in it.
    let top = (picked + 1).saturating_sub(AT_ROWS);
    let mut lines: Vec<Line<'static>> = choices
      .iter()
      .enumerate()
      .skip(top)
      .take(AT_ROWS)
      .map(|(i, f)| {
        if i == picked {
          Line::from(Span::styled(
            format!("› @{f}"),
            Style::new()
              .bg(Color::Indexed(236))
              .add_modifier(Modifier::BOLD),
          ))
        } else {
          Line::from(Span::raw(format!("  @{f}")))
        }
      })
      .collect();
    let hint = if choices.is_empty() {
      "@ files: no match · Esc".to_owned()
    } else {
      format!(
        "@ files: {} match{} · ↑↓ · Tab/Enter pick · Esc",
        choices.len(),
        if choices.len() == 1 { "" } else { "es" }
      )
    };
    lines.push(Line::from(Span::styled(
      hint,
      Style::new().fg(Color::DarkGray),
    )));
    lines
  }

  /// The message picker's rows, inside its frame `width` wide: the
  /// message (who, when, its start), and its actions when open.
  fn msg_lines(&self, width: u16) -> Vec<Line<'static>> {
    let Some(i) = self.picked_entry() else {
      return Vec::new();
    };
    let e = &self.entries[i];
    let when = e
      .at
      .map(|t| {
        t.to_zoned(jiff::tz::TimeZone::system())
          .strftime("%Y-%m-%d %H:%M")
          .to_string()
      })
      .unwrap_or_default();
    let who = if e.who == "you" {
      "you".to_owned()
    } else {
      e.who.clone()
    };
    let mut head = format!("{who} · {when} · {}", e.id.as_deref().unwrap_or(""));
    if e.attachments > 0 {
      head.push_str(&format!(" · {} attached", e.attachments));
    }
    let color = if e.who == "you" {
      Color::Green
    } else {
      Color::Magenta
    };
    let mut lines = vec![Line::from(Span::styled(
      head,
      Style::new().fg(color).add_modifier(Modifier::BOLD),
    ))];
    // Its start, wrapped, some rows of it.
    let rows = if self.msgs.menu.is_some() { 3 } else { 6 };
    let width = width.max(10) as usize;
    let mut shown = 0;
    'out: for l in e.said().lines() {
      let chars: Vec<char> = l.chars().collect();
      let mut at = 0;
      loop {
        if shown == rows {
          lines.push(Line::from(Span::styled(
            "…",
            Style::new().fg(Color::DarkGray),
          )));
          break 'out;
        }
        let piece: String = chars[at..(at + width).min(chars.len())].iter().collect();
        lines.push(Line::from(piece));
        shown += 1;
        at += width;
        if at >= chars.len() {
          break;
        }
      }
    }
    if let Some(sel) = self.msgs.menu {
      lines.push(Line::from(""));
      for (k, a) in self.msg_actions(i).into_iter().enumerate() {
        let text = format!(
          "{} {}  ({})",
          if k == sel { "›" } else { " " },
          a.label(),
          a.key()
        );
        lines.push(if k == sel {
          Line::from(Span::styled(
            text,
            Style::new()
              .bg(Color::Indexed(236))
              .add_modifier(Modifier::BOLD),
          ))
        } else {
          Line::from(text)
        });
      }
    }
    lines
  }

  fn render_messages(&self, f: &mut Frame, area: Rect) {
    let n = self.pickable().len();
    let at = self.msgs.picked.min(n.saturating_sub(1)) + 1;
    let title = if self.msgs.menu.is_some() {
      format!("message {at} of {n} · ↑↓ Enter · Esc back")
    } else {
      format!(
        "message {at} of {n} · ↑↓ · Enter actions · f fork · o options · e edit · c copy · Esc"
      )
    };
    let lines = self.msg_lines(area.width.saturating_sub(2));
    f.render_widget(Paragraph::new(Text::from(lines)).block(frame(&title)), area);
  }

  fn render_pick(&self, f: &mut Frame, area: Rect) {
    let Some(p) = &self.pick else { return };
    let [list_area, note] = Layout::vertical([
      Constraint::Min(1),
      Constraint::Length(u16::from(!p.note.is_empty())),
    ])
    .areas(area);
    let shown = p.shown();
    let items: Vec<ListItem> = shown
      .iter()
      .map(|c| {
        ListItem::new(Line::from(vec![
          Span::raw(c.label.clone()),
          Span::styled(
            if c.detail.is_empty() {
              String::new()
            } else {
              format!("  {}", c.detail)
            },
            Style::new().fg(Color::DarkGray),
          ),
        ]))
      })
      .collect();
    let mut state = ListState::default();
    if !shown.is_empty() {
      state.select(Some(p.picked.min(shown.len() - 1)));
    }
    let title = if p.filter.is_empty() {
      format!("{} · type to filter · ↑↓ Enter · Esc", p.title)
    } else {
      format!("{} · {}▏ · ↑↓ Enter · Esc", p.title, p.filter)
    };
    let list = List::new(items).block(frame(&title)).highlight_style(
      Style::new()
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(list, list_area, &mut state);
    if !p.note.is_empty() {
      f.render_widget(
        Paragraph::new(Span::styled(
          clip(&p.note, (area.width as usize).max(2)),
          Style::new().fg(Color::DarkGray),
        )),
        note,
      );
    }
  }

  /// The goal of the thread picked in the thread picker, when it was
  /// spawned with one (a line under the list).
  fn picked_goal(&self) -> Option<String> {
    let rows = self.thread_rows();
    let r = self.picked.checked_sub(1).and_then(|i| rows.get(i))?;
    let t = &self.threads[r.at];
    // A spawn's title is often its goal, cut: then it says nothing more.
    let title = t.title.trim_end_matches('…').trim().to_lowercase();
    t.goal
      .clone()
      .filter(|g| !g.trim().is_empty() && !g.to_lowercase().starts_with(&title))
  }

  fn render_threads(&self, f: &mut Frame, area: Rect) {
    let goal = self.picked_goal();
    let [list_area, note] = Layout::vertical([
      Constraint::Min(1),
      Constraint::Length(u16::from(goal.is_some())),
    ])
    .areas(area);
    let rows = self.thread_rows();
    let dim = Style::new().fg(Color::DarkGray);
    let mut items = vec![ListItem::new(Line::from("+ new thread").italic())];
    items.extend(rows.iter().map(|r| {
      let t = &self.threads[r.at];
      let mut spans = vec![Span::styled(r.lines.clone(), dim)];
      spans.push(Span::raw(match r.open {
        Some(true) => "▾ ",
        Some(false) => "▸ ",
        None if r.depth == 0 && self.thread_filter.is_empty() => "  ",
        None => "",
      }));
      if t.pinned {
        spans.push(Span::raw("📌 "));
      }
      if let Some(state) = &t.state {
        let color = if super::tree::working(t) {
          Color::Cyan
        } else if super::tree::waiting(t) {
          Color::Yellow
        } else if super::tree::failed(t) {
          Color::Red
        } else {
          Color::DarkGray
        };
        let word = if super::tree::waiting(t) {
          "needs you"
        } else {
          state
        };
        spans.push(Span::styled(format!("[{word}] "), Style::new().fg(color)));
      }
      spans.push(Span::raw(t.title.clone()));
      if r.open == Some(false) {
        let b = &r.below;
        spans.push(Span::styled(
          format!("  {}", b.summary()),
          Style::new().fg(if b.waiting > 0 {
            Color::Yellow
          } else {
            Color::DarkGray
          }),
        ));
      }
      if let Some(under) = &r.under {
        spans.push(Span::styled(format!("  ↖ {under}"), dim));
      }
      spans.push(Span::styled(
        match &t.place {
          Some(p) => format!("  {} · {p}", t.when),
          None => format!("  {}", t.when),
        },
        dim,
      ));
      ListItem::new(Line::from(spans))
    }));
    let mut state = ListState::default();
    state.select(Some(self.picked.min(rows.len())));
    // What is shown, then what Tab switches to: a title naming only the
    // next one (`Tab: here only`) read as the current one, and a row of
    // all three with the current in brackets read as noise (2026-10-23).
    let name = |s: ThreadScope| match s {
      ThreadScope::HereAndGlobal => "here and global",
      ThreadScope::Here => "here only",
      ThreadScope::All => "all places",
    };
    let title = if self.thread_filter.is_empty() {
      format!(
        "showing: {} · Tab → {} · type to filter · ↑↓ ←→ Enter · Del delete · Esc",
        name(self.scope),
        name(self.scope.next())
      )
    } else if rows.is_empty() {
      format!(
        "{}▏ · no thread's title matches: Enter searches the messages · Esc clears",
        self.thread_filter
      )
    } else {
      format!(
        "{}▏ · {} found, {} · Tab → {} · ↑↓ Enter · Esc clears",
        self.thread_filter,
        rows.len(),
        name(self.scope),
        name(self.scope.next())
      )
    };
    let list = List::new(items).block(frame(&title)).highlight_style(
      Style::new()
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(list, list_area, &mut state);
    if let Some(g) = goal {
      f.render_widget(
        Paragraph::new(Span::styled(
          clip(&format!("goal: {g}"), (area.width as usize).max(2)),
          dim,
        )),
        note,
      );
    }
  }

  /// The attention list's title: the keys the picked item takes. `a`
  /// shows only when answering it "always" grants something, and names
  /// what, as the inline prompt does.
  fn attention_title(&self) -> String {
    let mut keys = vec!["needs you", "↑↓"];
    let picked = self.attention.get(
      self
        .picked_needs
        .min(self.attention.len().saturating_sub(1)),
    );
    if picked.is_some_and(|n| n.detail.is_some()) {
      keys.push("v read it");
    }
    let always;
    if let Some(n) = picked.filter(|n| n.question.is_some()) {
      keys.push("y allow");
      if !n.grant.is_empty() {
        always = format!(
          "a always allow {}",
          n.grant
            .iter()
            .map(|g| format!("`{g}`"))
            .collect::<Vec<_>>()
            .join(", ")
        );
        keys.push(&always);
      }
      keys.push("n decline");
    }
    keys.push("Esc");
    keys.join(" · ")
  }

  fn render_attention(&self, f: &mut Frame, area: Rect) {
    let title = self.attention_title();
    let title = title.as_str();
    if self.attention.is_empty() {
      f.render_widget(
        Paragraph::new(Text::from("Nothing needs you.".dark_gray())).block(frame(title)),
        area,
      );
      return;
    }
    let [list_area, prompt] = Layout::vertical([
      Constraint::Min(1),
      Constraint::Length(u16::from(self.answering.is_some())),
    ])
    .areas(area);
    let items: Vec<ListItem> = self
      .attention
      .iter()
      .map(|n| {
        ListItem::new(Line::from(match n.detail {
          Some(_) => format!("{} · v reads it", n.text),
          None => n.text.clone(),
        }))
      })
      .collect();
    let mut state = ListState::default();
    state.select(Some(self.picked_needs.min(self.attention.len() - 1)));
    let list = List::new(items).block(frame(title)).highlight_style(
      Style::new()
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(list, list_area, &mut state);
    if self.answering.is_some() {
      let r = self.reason.as_deref().unwrap_or("");
      f.render_widget(
        Paragraph::new(Span::styled(
          format!("why not? (Enter sends, Esc skips): {r}▏"),
          Style::new().fg(Color::Yellow),
        )),
        prompt,
      );
    }
  }

  fn render_jobs(&self, f: &mut Frame, area: Rect) {
    let title = "in the background · ↑↓ · Enter its thread · x stop · Esc";
    if self.jobs.is_empty() {
      f.render_widget(
        Paragraph::new(Text::from("Nothing runs in the background.".dark_gray()))
          .block(frame(title)),
        area,
      );
      return;
    }
    let now = jiff::Timestamp::now();
    let items: Vec<ListItem> = self
      .jobs
      .iter()
      .map(|j| {
        let ago = j
          .since
          .parse::<jiff::Timestamp>()
          .ok()
          .map(|t| format!(" · {}", spoken(now.duration_since(t).unsigned_abs())))
          .unwrap_or_default();
        let whose = if j.title.is_empty() {
          clip(&j.thread, 24)
        } else {
          clip(&j.title, 40)
        };
        let place = if j.node == self.node {
          String::new()
        } else {
          format!(" · on {}", j.node)
        };
        let first = j.what.lines().next().unwrap_or("");
        ListItem::new(Text::from(vec![
          Line::from(vec![
            Span::styled(format!("{:<6} ", j.kind), Style::new().fg(Color::Cyan)),
            Span::raw(clip(first, 100)),
          ]),
          Line::from(Span::styled(
            format!("       {}{ago}{place} · {whose}", j.state),
            Style::new().fg(Color::DarkGray),
          )),
        ]))
      })
      .collect();
    let mut state = ListState::default();
    state.select(Some(self.picked_job.min(self.jobs.len() - 1)));
    let list = List::new(items).block(frame(title)).highlight_style(
      Style::new()
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD),
    );
    f.render_stateful_widget(list, area, &mut state);
  }

  fn render_input(&self, f: &mut Frame, area: Rect) {
    let title = if self.pending.is_some() {
      "Enter queue · Esc interrupt · Shift+Enter newline · Ctrl+O threads · Ctrl+T attention · /help"
    } else {
      "Enter send · Shift+Enter newline · @ files · Esc Esc messages · Ctrl+O threads · Ctrl+T attention · /help"
    };
    let text = self.input_shown();
    // Taller than the box: scrolled to keep the cursor in sight.
    let (_, row) = self.input_rows(area.width);
    let shown = area.height.saturating_sub(2).max(1);
    let top = (row + 1).saturating_sub(shown);
    f.render_widget(
      Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((top, 0))
        .block(
          Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::new().fg(Color::DarkGray))
            .title(title),
        ),
      area,
    );
  }
}

/// A duration as people say it: `45s`, `4m12s`, `2h05m`.
fn spoken(d: std::time::Duration) -> String {
  stitch_node::tools::spoken(d)
}

/// Text as the terminal may show it: line ends as newlines (CR LF and a
/// lone CR, as terminals paste them), and no other control characters (a
/// CR or an escape sequence would move the cursor and hide what it wrote).
pub fn clean(text: &str) -> String {
  text
    .replace("\r\n", "\n")
    .replace('\r', "\n")
    .chars()
    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
    .collect()
}

fn frame(title: &str) -> Block<'static> {
  Block::new()
    .borders(Borders::ALL)
    .border_style(Style::new().fg(Color::Cyan))
    .title(title.to_owned())
}

pub(crate) fn clip(s: &str, n: usize) -> String {
  if s.chars().count() <= n {
    s.to_owned()
  } else {
    format!("{}…", s.chars().take(n - 1).collect::<String>())
  }
}

/// Like `clip`, keeping the end (of a path).
fn clip_start(s: &str, n: usize) -> String {
  let count = s.chars().count();
  if count <= n {
    s.to_owned()
  } else {
    format!("…{}", s.chars().skip(count - (n - 1)).collect::<String>())
  }
}

fn expand_home(p: &str) -> String {
  match (p.strip_prefix("~/"), std::env::var("HOME")) {
    (Some(rest), Ok(home)) => format!("{home}/{rest}"),
    _ => p.to_owned(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::Terminal;
  use ratatui::backend::TestBackend;

  fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
  }

  fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
  }

  fn typed(app: &mut App, s: &str) {
    for c in s.chars() {
      app.on_key(key(KeyCode::Char(c)));
    }
  }

  fn screen(app: &App) -> String {
    let h = app.height(80);
    let mut t = Terminal::new(TestBackend::new(80, h)).unwrap();
    t.draw(|f| app.render(f)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..buf.area.height)
      .map(|y| {
        (0..buf.area.width)
          .map(|x| buf[(x, y)].symbol())
          .collect::<String>()
      })
      .collect::<Vec<_>>()
      .join("\n")
  }

  fn done(thread: &str) -> Reply {
    Reply::ChatDone {
      thread: thread.into(),
      segment: "s".into(),
      model: "claude-opus-5-5".into(),
      stop: "end_turn".into(),
      input_tokens: 10,
      output_tokens: 5,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      usd: Some(0.0001),
      forked: false,
      thread_usd: None,
    }
  }

  fn threads() -> Vec<ThreadItem> {
    vec![
      ThreadItem {
        id: "t2".into(),
        title: "Second".into(),
        when: "today".into(),
        state: Some("running".into()),
        at: jiff::Timestamp::UNIX_EPOCH,
        place: None,
        key: "folder:mbp:/src/app".into(),
        pinned: false,
        parent: None,
        goal: None,
      },
      ThreadItem {
        id: "t1".into(),
        title: "First".into(),
        when: "yesterday".into(),
        state: None,
        at: jiff::Timestamp::UNIX_EPOCH,
        place: Some("global".into()),
        key: "global".into(),
        pinned: false,
        parent: None,
        goal: None,
      },
    ]
  }

  #[test]
  fn a_message_streams_live_and_lands_in_the_scrollback() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    typed(&mut app, "hello there");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send {
        thread: None,
        text: "hello there".into(),
        attachments: vec![],
        model: None,
        global: false,
        tools: vec![],
        skills: vec![],
        memory: None,
      }
    );
    assert_eq!(app.out, vec![Printed::You("hello there".into())]);
    app.on_reply(&Reply::Thinking("pondering".into()));
    assert!(screen(&app).contains("pondering"));
    app.on_reply(&Reply::Text("General ".into()));
    app.on_reply(&Reply::Text("Kenobi".into()));
    let s = screen(&app);
    assert!(
      s.contains("General Kenobi") && !s.contains("pondering"),
      "{s}"
    );
    // A second message is offered to the first's reply.
    typed(&mut app, "again");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Interject { .. }
    ));
    assert_eq!(app.on_done(Ok(done("t9"))).as_deref(), Some("t9"));
    assert_eq!(app.thread.as_deref(), Some("t9"));
    assert_eq!(
      app.out[1..],
      [
        Printed::ReplyStart("claude-opus-5-5".into()),
        Printed::ReplyLines("General Kenobi".into()),
        Printed::ReplyEnd
      ]
    );
    assert_eq!(app.last_reply.as_deref(), Some("General Kenobi"));
    assert!(app.pending.is_none());
    assert!(screen(&app).contains("10 in"));
    assert!(
      !screen(&app).contains("General Kenobi"),
      "printed, not live"
    );
  }

  #[test]
  fn pickers_for_models_nodes_memory_and_commands() {
    let mut app = App::new("phone", "anthropic/claude-opus-5-5");
    // /model asks the node, then picks from what it has.
    typed(&mut app, "/model");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fetch(Fetch::Models(None))
    );
    assert_eq!(app.overlay, Overlay::Pick);
    app.filled(
      vec![
        Choice::new("anthropic/claude-opus-5-5", "default"),
        Choice::new("anthropic/claude-sonnet-5-5", ""),
        Choice::new("openai/gpt-6-astra", ""),
      ],
      "grok: no key".into(),
    );
    assert!(screen(&app).contains("grok: no key"));
    typed(&mut app, "sonnet");
    assert_eq!(app.pick.as_ref().unwrap().shown().len(), 1);
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.model.as_deref(), Some("anthropic/claude-sonnet-5-5"));
    assert_eq!(app.overlay, Overlay::None);
    // /spawn picks the machine (this one first), then a model from that
    // machine's list, the one picked here first; then the goal is typed.
    app.thread = Some("t1".into());
    typed(&mut app, "/spawn");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Fetch(Fetch::Peers));
    app.filled(vec![Choice::new("mac", "")], String::new());
    assert_eq!(app.pick.as_ref().unwrap().choices[0].value, "phone");
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fetch(Fetch::Models(Some("mac".into())))
    );
    app.filled(
      vec![
        Choice::new("openai/gpt-6-astra", "default"),
        Choice::new("anthropic/claude-sonnet-5-5", ""),
      ],
      String::new(),
    );
    assert_eq!(app.pick.as_ref().unwrap().picked, 1);
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.input, "/spawn @mac:anthropic/claude-sonnet-5-5 ");
    typed(&mut app, "write the notes");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Spawn {
        node: Some("mac".into()),
        model: Some("anthropic/claude-sonnet-5-5".into()),
        goal: "write the notes".into()
      }
    );
    // Its own default, when it lacks the one picked here.
    app.model = Some("grok/grok-5".into());
    typed(&mut app, "/spawn @mac");
    app.on_key(key(KeyCode::Enter));
    app.filled(
      vec![
        Choice::new("anthropic/claude-sonnet-5-5", ""),
        Choice::new("openai/gpt-6-astra", "default"),
      ],
      String::new(),
    );
    assert_eq!(app.pick.as_ref().unwrap().picked, 1);
    app.on_key(key(KeyCode::Esc));
    // Typed without a model: the one picked here.
    typed(&mut app, "/spawn @mac tidy up");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Spawn {
        node: Some("mac".into()),
        model: Some("grok/grok-5".into()),
        goal: "tidy up".into()
      }
    );
    app.model = Some("anthropic/claude-sonnet-5-5".into());
    // Memory's state shows; /memory offers the other one.
    app.memory = Some(true);
    assert!(screen(&app).contains("memory on"));
    typed(&mut app, "/memory");
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Remember {
        thread: Some("t1".into()),
        on: false
      }
    );
    // `/` alone: the commands; one that takes words waits for them.
    typed(&mut app, "/");
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "attach");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.input, "/attach ");
  }

  #[test]
  fn typed_while_it_works_is_queued_editable_and_sent_after() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "first");
    app.on_key(key(KeyCode::Enter));
    // While it works: queued, and offered to the reply that runs.
    typed(&mut app, "second");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Interject {
        id: 1,
        text: "second".into(),
        attachments: vec![]
      }
    );
    typed(&mut app, "thrd");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.queued.len(), 2);
    assert!(screen(&app).contains("queued: second"));
    // ↑ takes the last back to edit; Enter queues it again.
    assert_eq!(app.on_key(key(KeyCode::Up)), Action::Retract(2));
    assert_eq!(app.input, "thrd");
    app.input = "third".into();
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Interject { id: 3, .. }
    ));
    // One being typed when the reply ends stays in the input.
    typed(&mut app, "draft");
    app.on_done(Ok(done("t1")));
    let Some(Action::Send { text, thread, .. }) = app.send_queued() else {
      panic!("the queue goes")
    };
    assert_eq!(text, "second\n\nthird");
    assert_eq!(thread.as_deref(), Some("t1"));
    assert!(app.queued.is_empty());
    assert_eq!(app.input, "draft");
    assert!(app.send_queued().is_none(), "nothing more while it runs");
  }

  /// Waiting for another turn, the message and what is queued go into it:
  /// said, left out of the queue (the turn followed shows them), and the
  /// chat ends with that turn, what it did not take sent after.
  #[test]
  fn a_message_waiting_goes_into_the_turn_that_runs() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "hello?");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Waiting(true));
    for s in ["one", "two"] {
      typed(&mut app, s);
      app.on_key(key(KeyCode::Enter));
    }
    app.out.clear();
    app.on_reply(&Reply::Woven(vec![0, 1]));
    assert!(
      app
        .status
        .starts_with("your message went into the turn running")
    );
    assert_eq!(app.queued.len(), 1, "one taken");
    assert!(app.out.is_empty(), "shown as the turn is followed");
    app.on_reply(&Reply::Waiting(false));
    let mut woven = done("t1");
    if let Reply::ChatDone { stop, .. } = &mut woven {
      *stop = "woven".into();
    }
    assert!(app.on_done(Ok(woven)).is_some());
    assert_eq!(
      app.status,
      "your message went into the turn that ran on this thread"
    );
    let Some(Action::Send { text, .. }) = app.send_queued() else {
      panic!("the one not taken goes after")
    };
    assert_eq!(text, "two");
  }

  #[test]
  fn a_send_waiting_for_another_turn_says_so() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "hello?");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.status, "sending");
    app.on_reply(&Reply::Waiting(true));
    assert!(app.pending.as_ref().is_some_and(|p| p.waiting));
    assert!(app.status.starts_with("waiting for the turn running"));
    assert_eq!(app.out.last(), Some(&Printed::Notice(WAITING.into())));
    assert!(screen(&app).contains("waiting for the turn already running on this thread"));
    // Typed meanwhile: queued, as while its own turn runs.
    typed(&mut app, "more");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Interject { .. }
    ));
    app.on_reply(&Reply::Waiting(false));
    assert!(app.pending.as_ref().is_some_and(|p| !p.waiting));
    assert_eq!(app.status, "sending");
    // The spinner says what runs, till its result.
    app.on_reply(&Reply::ToolCall("$ sleep 180".into()));
    assert!(screen(&app).contains("running $ sleep 180 · 0s"));
    app.on_reply(&Reply::ToolResult {
      ok: true,
      output: String::new(),
    });
    assert!(!screen(&app).contains("running $ sleep"));
  }

  /// A turn this TUI did not start (a report's, a detached thread's) runs
  /// a long call: its step is written only once the call returns, so the
  /// live area shows the call in flight, its command whole, a spinner and
  /// its time (2026-11-01: a 13-minute deploy looked stuck).
  #[test]
  fn a_followed_turns_running_call_shows_in_flight() {
    use stitch_node::chat::{CallInFlight, StepInFlight};
    let mut app = App::new("mbp", "m");
    app.thread = Some("t1".into());
    let call = |id: &str, input: serde_json::Value, since, ok| CallInFlight {
      id: id.into(),
      name: "shell_run".into(),
      summary: stitch_node::chat::call_summary("shell_run", &input),
      input,
      since,
      ok,
    };
    let since = jiff::Timestamp::now() - jiff::SignedDuration::from_secs(192);
    let step = StepInFlight {
      calls: vec![
        call(
          "c1",
          serde_json::json!({"command": "git status"}),
          None,
          Some(true),
        ),
        call(
          "c2",
          serde_json::json!({
            "command": "git push && fly deploy\ncargo build --release",
            "session": "deploy",
            "host": "devbox-1",
          }),
          Some(since),
          None,
        ),
      ],
      running: Some(1),
    };
    app.in_flight = Some(("t1".into(), step.clone()));
    let s = screen(&app);
    assert!(
      s.contains("running shell_run in `deploy` on devbox-1 · 3m12s · call 2 of 2"),
      "{s}"
    );
    assert!(s.contains("[deploy] $ git push && fly deploy"), "{s}");
    assert!(s.contains("cargo build --release"), "{s}");
    // Another thread opened: not its call.
    app.thread = Some("t2".into());
    assert!(!screen(&app).contains("running shell_run"));
    // A message sent meanwhile waits for that turn, and says what it runs.
    app.thread = Some("t1".into());
    typed(&mut app, "is it stuck?");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Waiting(true));
    let s = screen(&app);
    assert!(s.contains("running shell_run in `deploy`"), "{s}");
    assert!(s.contains("waiting for the turn already running"), "{s}");
    // This TUI's own turn shows its own calls, not the kept step.
    app.on_reply(&Reply::Waiting(false));
    assert!(!screen(&app).contains("running shell_run in"));
  }

  #[test]
  fn a_shown_app_prints_its_link_and_app_opens_it() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "/app");
    assert!(matches!(app.on_key(key(KeyCode::Enter)), Action::None));
    assert!(app.status.starts_with("no thread yet"));
    typed(&mut app, "make an app");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Thread("t1".into()));
    app.on_reply(&Reply::App {
      name: "Todo".into(),
      url: Some("http://abc.localhost:7177/?k=tok".into()),
    });
    assert!(
      matches!(app.out.last(), Some(Printed::Notice(n))
        if n.starts_with("App \"Todo\": http://abc.localhost:7177/?k=tok")),
      "{:?}",
      app.out.last()
    );
    app.on_reply(&done("t1"));
    typed(&mut app, "/app");
    assert!(matches!(app.on_key(key(KeyCode::Enter)), Action::OpenApp(t) if t == "t1"));
    typed(&mut app, "/app Todo");
    assert!(matches!(app.on_key(key(KeyCode::Enter)), Action::OpenApp(t) if t == "Todo"));
  }

  #[test]
  fn input_edits_at_the_cursor() {
    let mut app = App::new("mbp", "m");
    let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
    typed(&mut app, "helo world");
    for _ in 0..7 {
      app.on_key(key(KeyCode::Left));
    }
    typed(&mut app, "l");
    assert_eq!(app.input, "hello world");
    app.on_key(key(KeyCode::End));
    typed(&mut app, "!");
    assert_eq!(app.input, "hello world!");
    app.on_key(alt(KeyCode::Left));
    app.on_key(key(KeyCode::Delete));
    assert_eq!(app.input, "hello orld!");
    app.on_key(ctrl('a'));
    typed(&mut app, "> ");
    app.on_key(ctrl('k'));
    assert_eq!(app.input, "> ");
    typed(&mut app, "one two");
    app.on_key(ctrl('w'));
    assert_eq!(app.input, "> one ");
    app.on_key(key(KeyCode::Backspace));
    app.on_key(ctrl('u'));
    assert_eq!(app.input, "");
    // Lines: ↑ and ↓ keep the column, as far as the line goes.
    typed(&mut app, "abcdef");
    app.on_key(alt(KeyCode::Enter));
    typed(&mut app, "xy");
    app.on_key(key(KeyCode::Up));
    typed(&mut app, "|");
    assert_eq!(app.input, "ab|cdef\nxy");
    app.on_key(key(KeyCode::End));
    app.on_key(key(KeyCode::Down));
    typed(&mut app, "z");
    assert_eq!(app.input, "ab|cdef\nxyz");
    // Not ASCII: characters, not bytes.
    app.on_key(ctrl('u'));
    typed(&mut app, "è€x");
    app.on_key(key(KeyCode::Left));
    app.on_key(key(KeyCode::Backspace));
    assert_eq!(app.input, "ab|cdef\nèx");
  }

  #[test]
  fn long_input_wraps_and_grows() {
    let mut app = App::new("mbp", "m");
    let one = app.height(80);
    typed(&mut app, &"word ".repeat(40));
    assert_eq!(app.height(80), one + 2, "200 characters: three rows at 80");
    let s = screen(&app);
    assert!(s.contains("word word"));
    assert!(s.contains('▏'));
    // Past the cap it scrolls, the cursor in sight.
    typed(&mut app, &"more ".repeat(400));
    assert_eq!(app.height(80), one + INPUT_ROWS - 1);
    assert!(screen(&app).contains('▏'));
    app.on_key(ctrl('a'));
    let s = screen(&app);
    assert!(s.contains("▏word"), "{s}");
  }

  #[test]
  fn queued_messages_woven_in_show_where_they_went() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "first");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Thread("t1".into()));
    app.on_reply(&Reply::Text("Looking.\n".into()));
    for s in ["one", "two"] {
      typed(&mut app, s);
      app.on_key(key(KeyCode::Enter));
    }
    app.out.clear();
    app.on_reply(&Reply::Woven(vec![1]));
    assert_eq!(app.queued.len(), 1);
    assert!(
      matches!(app.out.last(), Some(Printed::You(t)) if t == "one"),
      "{:?}",
      app.out
    );
    // One being edited when it was woven: said so, shown where it went.
    assert_eq!(app.on_key(key(KeyCode::Up)), Action::Retract(2));
    app.on_reply(&Reply::Woven(vec![2]));
    assert!(matches!(app.out.last(), Some(Printed::You(t)) if t == "two"));
    assert!(app.status.contains("already gone"));
    // Nothing is left to send after.
    app.input.clear();
    app.on_done(Ok(done("t1")));
    assert!(app.send_queued().is_none());
  }

  #[test]
  fn esc_while_waiting_for_another_turn_takes_the_message_back() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "how is it going?");
    app.attachments.push("/tmp/a.png".into());
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Waiting(true));
    typed(&mut app, "and this");
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "draft");
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Interrupt);
    assert_eq!(app.status, "interrupting");
    assert!(app.on_done(Ok(Reply::Unsent)).is_none());
    assert!(app.pending.is_none());
    assert!(app.send_queued().is_none(), "not sent: interrupted");
    assert!(
      !app.out.iter().any(|p| matches!(p, Printed::You(_))),
      "never sent"
    );
    assert_eq!(app.input, "how is it going?\n\nand this\n\ndraft");
    assert_eq!(app.attachments, vec!["/tmp/a.png".to_owned()]);
    assert!(app.status.starts_with("interrupted before it was sent"));
  }

  #[test]
  fn esc_interrupts_and_the_queue_comes_back_to_edit() {
    let mut app = App::new("mbp", "m");
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None, "nothing runs");
    typed(&mut app, "first");
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "then this");
    app.on_key(key(KeyCode::Enter));
    typed(&mut app, "draft");
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Interrupt);
    let mut stopped = done("t1");
    if let Reply::ChatDone { stop, .. } = &mut stopped {
      *stop = "interrupted".into();
    }
    app.on_done(Ok(stopped));
    assert!(app.send_queued().is_none(), "not sent: stopped");
    assert_eq!(app.input, "then this\n\ndraft");
    assert!(app.status.contains("interrupted"));
  }

  #[test]
  fn a_paste_goes_in_as_one_and_shows() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "see: ");
    // As terminals paste: CR line ends, and stray control characters.
    app.paste("line one\r\nline two\rthree\u{1b}[2J\tend");
    assert_eq!(app.input, "see: line one\nline two\nthree[2J\tend");
    let s = screen(&app);
    assert!(
      s.contains("see: line one") && s.contains("three[2J end"),
      "{s}"
    );
    // Nothing went: a newline in a paste is no Enter.
    assert!(app.pending.is_none());
    // In a picker, it filters.
    app.input.clear();
    app.cursor = None;
    typed(&mut app, "/model");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.overlay, Overlay::Pick);
    app.paste("son\n");
    assert_eq!(app.pick.as_ref().unwrap().filter, "son");
  }

  #[test]
  fn jobs_list_opens_and_stops() {
    let mut app = App::new("mbp", "m");
    assert_eq!(app.on_key(ctrl('b')), Action::Jobs);
    assert_eq!(app.overlay, Overlay::Jobs);
    assert!(screen(&app).contains("Nothing runs in the background."));
    let job = |kind: &str, id: &str, what: &str, state: &str, node: &str| Job {
      kind: kind.into(),
      id: id.into(),
      thread: "t1".into(),
      title: "Build things".into(),
      node: node.into(),
      what: what.into(),
      state: state.into(),
      since: jiff::Timestamp::now().to_string(),
    };
    app.jobs_listed(vec![
      job(
        "shell",
        "t1/bg1",
        "cargo build",
        "job 1: tells when it ends",
        "mbp",
      ),
      job("thread", "t2", "tidy the docs", "running", "linux"),
    ]);
    let s = screen(&app);
    assert!(s.contains("shell  cargo build"), "{s}");
    assert!(
      s.contains("job 1: tells when it ends · 0s · Build things"),
      "{s}"
    );
    assert!(s.contains("running · 0s · on linux"), "{s}");
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.on_key(key(KeyCode::Char('x'))),
      Action::StopJob("t2".into())
    );
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Open(Some("t1".into()))
    );
    // Fewer after: the pick stays on the list.
    app.jobs_listed(vec![]);
    assert_eq!(app.picked_job, 0);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::None);
    typed(&mut app, "/jobs");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Jobs);
  }

  #[test]
  fn a_note_from_the_node_is_shown_where_it_went() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "build it");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Text("Started.\n".into()));
    app.on_reply(&Reply::Note("[Job 1 ended with exit 0]".into()));
    assert!(
      matches!(app.out.last(), Some(Printed::Notice(t)) if t == "[Job 1 ended with exit 0]"),
      "{:?}",
      app.out
    );
  }

  /// The chat's thread compacted (design §2.15): the divider prints where
  /// it happened, and the TUI is on the new thread from then on.
  #[test]
  fn a_compaction_in_its_chat_prints_the_divider_and_switches() {
    let mut app = App::new("mbp", "m");
    app.thread = Some("t1".into());
    typed(&mut app, "go on");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Compacted(stitch_node::api::Compaction {
      from: "t1".into(),
      to: "t2".into(),
      model: "anthropic/m".into(),
      window: 200_000,
      tokens: Some(170_000),
      kept: 10,
      summary: "What came before.".into(),
    }));
    assert_eq!(
      app.out.last(),
      Some(&Printed::Compacted {
        line: "compacted here (170k of 200k window): summary + last 10 messages".into(),
        summary: "What came before.".into(),
      })
    );
    assert_eq!(app.thread.as_deref(), Some("t2"));
    // An entry of a lineage's transcript prints the same.
    let e = Entry {
      who: "compacted".into(),
      text: "compacted here (x): a summary\n\nS".into(),
      said: "compacted here (x): a summary".into(),
      id: None,
      at: None,
      attachments: 0,
      replies: Vec::new(),
      actions: Vec::new(),
      call: None,
    };
    assert_eq!(
      Printed::from_entry(&e),
      Printed::Compacted {
        line: "compacted here (x): a summary".into(),
        summary: "S".into(),
      }
    );
    assert!(!e.pickable());
  }

  #[test]
  fn the_approval_mode_switches_and_shows() {
    let mode = |m: &str| Action::Mode(m.into());
    let mut app = App::new("mbp", "m");
    typed(&mut app, "/yolo");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("yolo"));
    app.mode = "yolo".into();
    assert!(screen(&app).contains("yolo"));
    typed(&mut app, "/yolo");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("ask"));
    typed(&mut app, "/yolo on");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("yolo"));
    typed(&mut app, "/auto");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("auto"));
    app.mode = "auto".into();
    assert!(screen(&app).contains("· auto"));
    typed(&mut app, "/auto off");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("ask"));
    typed(&mut app, "/ask");
    assert_eq!(app.on_key(key(KeyCode::Enter)), mode("ask"));
  }

  #[test]
  fn help_lists_one_command_per_line() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "/help");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    let Some(Printed::Notice(help)) = app.out.last() else {
      panic!("{:?}", app.out)
    };
    assert!(help.lines().count() > 10);
    assert!(
      help
        .lines()
        .any(|l| l.starts_with("/spawn [@node[:model]] <goal>"))
    );
    typed(&mut app, "/nope");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.status, "no command /nope: /help lists them");
  }

  #[test]
  fn read_only_switches_the_open_thread_and_shows() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    typed(&mut app, "/read-only on");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.status, "no thread yet");
    app.thread = Some("t1".into());
    typed(&mut app, "/read-only on");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::ReadOnly {
        thread: "t1".into(),
        on: true
      }
    );
    typed(&mut app, "/read-only off");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::ReadOnly {
        thread: "t1".into(),
        on: false
      }
    );
    assert!(!screen(&app).contains("read-only"));
    app.read_only = true;
    assert!(screen(&app).contains("· read-only"));
  }

  #[test]
  fn run_on_moves_the_open_thread_and_shows_where_it_runs() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    app.thread = Some("t1".into());
    typed(&mut app, "/run-on gamer");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::RunOn {
        thread: "t1".into(),
        node: Some("gamer".into())
      }
    );
    typed(&mut app, "/run-on here");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::RunOn {
        thread: "t1".into(),
        node: None
      }
    );
    assert!(!screen(&app).contains("runs on"));
    app.runs_on = Some("gamer".into());
    assert!(screen(&app).contains("· runs on gamer"));
  }

  #[test]
  fn memory_commands_and_afterthoughts() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    // A new thread keeps the choice for its first message: no other
    // thread (the latest here) is switched.
    app.memory = Some(true);
    typed(&mut app, "/memory off");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert!(screen(&app).contains("memory off"));
    // The node's answer for a new thread here does not undo it.
    app.on_memory(Some(true));
    assert_eq!(app.memory, Some(false));
    typed(&mut app, "first");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send {
        thread: None,
        memory: Some(false),
        ..
      }
    ));
    app.on_done(Ok(done("t1")));
    app.thread = Some("t1".into());
    typed(&mut app, "/memory on");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Remember {
        thread: Some("t1".into()),
        on: true
      }
    );
    // An afterthought prints as a reply of its own, after the answer.
    typed(&mut app, "hello");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Text("The answer.".into()));
    app.on_reply(&Reply::Afterthought("One more thing.".into()));
    app.on_done(Ok(done("t1")));
    let notice = app
      .out
      .iter()
      .position(|p| *p == Printed::Notice("memory: an afterthought".into()))
      .expect("announced");
    assert!(app.out[..notice].contains(&Printed::ReplyLines("The answer.".into())));
    assert!(app.out[notice..].contains(&Printed::ReplyLines("One more thing.".into())));
  }

  #[test]
  fn tool_steps_print_in_order_and_approvals_take_y_or_n() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "do it");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Text("Checking.".into()));
    app.on_reply(&Reply::ToolCall("$ git status".into()));
    app.on_reply(&Reply::ToolResult {
      ok: true,
      output: "exit 0\n\nclean".into(),
    });
    app.on_reply(&Reply::Text("All clean.".into()));
    app.on_done(Ok(done("t1")));
    assert_eq!(
      app.out[1..],
      [
        Printed::ReplyStart("m".into()),
        Printed::ReplyLines("Checking.".into()),
        Printed::ReplyEnd,
        Printed::Tool("$ git status".into()),
        Printed::Output("exit 0\n\nclean".into()),
        Printed::ReplyStart("m".into()),
        Printed::ReplyLines("All clean.".into()),
        Printed::ReplyEnd,
      ]
    );
    // The user paused typing: the prompt's keys answer it.
    let paused = |app: &mut App| {
      app.approval_at = None;
      app.typed_at = None;
    };
    // Appearing while the user types, it does not take their keys: a `y`
    // in "yet" goes into the message, not the answer.
    app.ask("main".into(), "rm -rf target".into(), "q0".into(), vec![]);
    typed(&mut app, "yet");
    assert!(app.approval.is_some(), "still waiting");
    assert_eq!(app.input, "yet");
    app.input.clear();
    app.approval = None;
    app.ask("main".into(), "rm -rf target".into(), "q1".into(), vec![]);
    paused(&mut app);
    assert!(screen(&app).contains("run: rm -rf target"));
    assert_eq!(app.on_key(key(KeyCode::Char('x'))), Action::None);
    assert!(app.input.is_empty(), "typing does not leak into the input");
    assert_eq!(
      app.on_key(key(KeyCode::Char('y'))),
      Action::Approve(Decision::allow())
    );
    app.ask("main".into(), "git push".into(), "q2".into(), vec![]);
    paused(&mut app);
    assert_eq!(
      app.on_key(key(KeyCode::Esc)),
      Action::Approve(Decision::decline(None))
    );
    // `n` asks why; the reason goes to the model.
    app.ask("main".into(), "cargo build".into(), "q3".into(), vec![]);
    paused(&mut app);
    assert_eq!(app.on_key(key(KeyCode::Char('n'))), Action::None);
    typed(&mut app, "takes 10 min");
    assert!(screen(&app).contains("why not?"));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Approve(Decision::decline(Some("takes 10 min".into())))
    );
    assert!(app.approval.is_none() && app.reason.is_none());
    assert!(
      app.input.is_empty(),
      "the reason did not go into the message"
    );
  }

  #[test]
  fn a_child_names_its_parent_and_ctrl_p_opens_it() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    // t2 was spawned by t1: the status line says so, the scrollback has
    // only the separator.
    app.family = Some(Family {
      id: "t2".into(),
      title: "Second".into(),
      parent: Some(("t1".into(), "First".into())),
      ..Default::default()
    });
    app.opened(Some("t2".into()), &[]);
    assert_eq!(app.out, vec![Printed::Separator("Second".into())]);
    assert!(screen(&app).contains("Second ↑ First (^P)"));
    assert_eq!(app.on_key(ctrl('p')), Action::Open(Some("t1".into())));
    typed(&mut app, "/parent");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Open(Some("t1".into()))
    );
    // t1 has none: nothing opens, the status line says why.
    app.opened(Some("t1".into()), &[]);
    assert_eq!(app.on_key(ctrl('p')), Action::None);
    assert!(app.status.contains("no parent"));
    // Ctrl+U still clears the input.
    typed(&mut app, "draft");
    app.on_key(ctrl('u'));
    assert!(app.input.is_empty());
    // Going down: the status line counts them, /children asks for them,
    // a picker opens one.
    app.family = Some(Family {
      id: "t1".into(),
      title: "First".into(),
      children: 2,
      ..Default::default()
    });
    assert!(screen(&app).contains("First ↓ 2 (/children)"));
    typed(&mut app, "/children");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Children("t1".into())
    );
    app.show_children(vec![Choice::new("t2", "running")]);
    assert_eq!(app.overlay, Overlay::Pick);
    assert!(screen(&app).contains("1 below “First”"));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Open(Some("t2".into()))
    );
    app.opened(Some("t2".into()), &[]);
    assert_eq!(
      app.children(),
      0,
      "another thread's count is not this one's"
    );
    app.show_children(Vec::new());
    assert!(app.status.contains("no children"));
    // A thread of another place is not in the list: what was read of it
    // names it, its place and its parent all the same.
    app.family = Some(Family {
      id: "t9".into(),
      title: "Elsewhere".into(),
      place: Some("~/devel/otter".into()),
      parent: Some(("t2".into(), "Second".into())),
      children: 0,
    });
    app.opened(Some("t9".into()), &[]);
    assert_eq!(
      app.out.last(),
      Some(&Printed::Separator("Elsewhere".into()))
    );
    let shown = screen(&app);
    assert!(shown.contains("Elsewhere ↑ Second (^P)"), "{shown}");
    assert!(shown.contains("~/devel/otter"), "{shown}");
    assert_eq!(app.on_key(ctrl('p')), Action::Open(Some("t2".into())));
  }

  /// The thread list as a tree: spawns under their parent, → and ←
  /// open and close them, typing filters by title, Enter on no match
  /// searches the messages, letters no longer move or delete.
  #[test]
  fn the_thread_list_is_a_tree_filtered_by_typing() {
    let mut app = App::new("mbp", "m");
    let at = |s| jiff::Timestamp::from_second(s).unwrap();
    let item = |id: &str, title: &str, parent: Option<&str>, s, state: Option<&str>| ThreadItem {
      id: id.into(),
      title: title.into(),
      when: "today".into(),
      state: state.map(Into::into),
      at: at(s),
      place: None,
      key: "global".into(),
      pinned: false,
      parent: parent.map(Into::into),
      goal: None,
    };
    app.threads = vec![
      item("c1", "Run just check", Some("p"), 30, Some("done")),
      item("p", "Thread list tree", None, 10, None),
      item("q", "Weekend basket", None, 20, None),
    ];
    app.threads[0].goal = Some("In a worktree, run just check-apple and report".into());
    app.thread = Some("q".into());
    app.on_key(ctrl('o'));
    let s = screen(&app);
    assert!(s.contains("▸ Thread list tree  1 below"), "{s}");
    assert!(!s.contains("Run just check"), "closed: {s}");
    // On the open thread (q, below p: p's spawn is newer).
    assert_eq!(app.picked, 2);
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Right));
    let s = screen(&app);
    assert!(s.contains("▾ Thread list tree"), "{s}");
    assert!(s.contains("└ [done] Run just check"), "{s}");
    app.on_key(key(KeyCode::Down));
    assert!(screen(&app).contains("goal: In a worktree, run just"));
    // ← on a child goes to its parent, ← again closes it.
    app.on_key(key(KeyCode::Left));
    assert_eq!(app.picked, 1);
    app.on_key(key(KeyCode::Left));
    assert!(!screen(&app).contains("Run just check"));
    // Typing filters, flat, saying what a match hangs under; `d` and `j`
    // are letters now.
    typed(&mut app, "just");
    let s = screen(&app);
    assert!(
      s.contains("[done] Run just check  ↖ Thread list tree"),
      "{s}"
    );
    assert!(!s.contains("Weekend basket  today"), "{s}");
    assert!(app.deleting.is_none());
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Open(Some("c1".into()))
    );
    // Esc clears the filter, back on that thread in the tree.
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::Threads);
    assert!(app.thread_filter.is_empty());
    assert_eq!(app.picked, 2);
    assert!(screen(&app).contains("└ [done] Run just check"));
    // Read again with a newer thread on top: still on that thread.
    let mut again = app.threads.clone();
    again.push(item("n", "Newer", None, 99, None));
    app.set_threads(again);
    assert_eq!(app.picked, 3);
    // A goal that is the title, cut, is not said again.
    app.threads[0].goal = Some("Run just check, then report".into());
    assert!(!screen(&app).contains("goal:"));
    // Delete asks the node what goes, then the user.
    assert_eq!(
      app.on_key(key(KeyCode::Delete)),
      Action::DeletePreview("c1".into())
    );
    assert!(app.deleting.is_none());
    let doomed: Vec<_> = ["b1\tRun just check\tearlier", "c1\tRun just check\tthis"]
      .iter()
      .map(|l| crate::doomed::parse(l))
      .collect();
    app.on_delete_preview("c1".into(), &doomed);
    assert_eq!(app.deleting.as_deref(), Some("c1"));
    assert!(
      app
        .status
        .contains("summarized and continued in a new thread once"),
      "{}",
      app.status
    );
    app.on_key(key(KeyCode::Char('n')));
    assert!(app.deleting.is_none());
    // Nothing matches: Enter looks in the messages.
    typed(&mut app, "zebra stripes");
    assert!(screen(&app).contains("Enter searches the messages"));
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fetch(Fetch::Search { ref query, here: false }) if query == "zebra stripes"
    ));
  }

  #[test]
  fn threads_are_picked_and_their_transcript_printed() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.on_key(ctrl('o'));
    assert_eq!(app.overlay, Overlay::Threads);
    assert!(screen(&app).contains("Second"));
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Open(Some("t1".into()))
    );
    let transcript = vec![
      Entry {
        who: "you".into(),
        text: "q".into(),
        ..Default::default()
      },
      Entry {
        who: "tool".into(),
        text: "$ ls".into(),
        ..Default::default()
      },
      Entry {
        who: "claude-opus-5-5".into(),
        text: "the answer".into(),
        ..Default::default()
      },
    ];
    app.opened(Some("t1".into()), &transcript);
    assert_eq!(app.out[0], Printed::Separator("First".into()));
    assert_eq!(app.out[2], Printed::Tool("$ ls".into()));
    assert_eq!(app.overlay, Overlay::None);
    assert_eq!(app.on_key(ctrl('y')), Action::Copy("the answer".into()));
    // The picker opens on the current thread; a new thread is above it.
    app.on_key(ctrl('o'));
    assert_eq!(app.picked, 2);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.on_key(ctrl('n')), Action::Open(None));
  }

  #[test]
  fn completed_lines_reach_the_scrollback_while_the_reply_streams() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    typed(&mut app, "tell me");
    app.on_key(key(KeyCode::Enter));
    app.out.clear();
    app.on_reply(&Reply::Text("First line\nsecond li".into()));
    // The finished line is printed now; the rest stays live.
    assert_eq!(
      app.out,
      [
        Printed::ReplyStart("claude-opus-5-5".into()),
        Printed::ReplyLines("First line".into())
      ]
    );
    assert!(screen(&app).contains("second li"));
    app.on_reply(&Reply::Text("ne\n\nA new paragraph".into()));
    // A blank line between paragraphs is kept.
    assert_eq!(app.out[2], Printed::ReplyLines("second line\n".into()));
    app.on_done(Ok(done("t1")));
    assert_eq!(
      app.out[3..],
      [
        Printed::ReplyLines("A new paragraph".into()),
        Printed::ReplyEnd
      ]
    );
    assert_eq!(
      app.last_reply.as_deref(),
      Some("First line\nsecond line\n\nA new paragraph")
    );
  }

  #[test]
  fn the_live_area_renders_markdown_after_what_was_printed() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "tell me");
    app.on_key(key(KeyCode::Enter));
    // Inside a fence, the line being written is code, as written.
    app.on_reply(&Reply::Text("```\n**not bold".into()));
    assert!(screen(&app).contains("**not bold"));
    // A table's rows stay live, aligned, till it ends.
    app.on_reply(&Reply::Text(
      "\n```\n| a | bb |\n|---|---|\n| ccc | d".into(),
    ));
    let s = screen(&app);
    assert!(s.contains("a   │ bb"), "{s}");
    assert!(s.contains("ccc │ d"), "{s}");
    app.on_reply(&Reply::Text(" |\nSo **that**".into()));
    assert!(screen(&app).contains("So that"));
    // The copy is the reply as written.
    app.on_done(Ok(done("t1")));
    assert!(app.last_reply.as_deref().unwrap().ends_with("So **that**"));
  }

  #[test]
  fn a_new_thread_while_a_turn_runs_lets_go_of_it() {
    let mut app = App::new("mbp", "m");
    app.thread = Some("t1".into());
    typed(&mut app, "long job");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send { .. }
    ));
    typed(&mut app, "and then");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Interject { .. }
    ));
    app.ask("main".into(), "make".into(), "q1".into(), vec![]);
    // Ctrl+N works while asked: the approval goes with the turn.
    assert_eq!(app.on_key(ctrl('n')), Action::Open(None));
    app.let_go();
    app.opened(None, &[]);
    assert!(app.pending.is_none() && app.approval.is_none() && app.queued.is_empty());
    let notes: Vec<String> = app
      .out
      .iter()
      .filter_map(|p| match p {
        Printed::Notice(n) => Some(n.clone()),
        _ => None,
      })
      .collect();
    assert!(notes.iter().any(
      |n| n.starts_with(LET_GO) && n.contains("Not sent (typed for it, not yet taken): and then")
    ));
    // The next message starts the new thread, not an interjection.
    typed(&mut app, "fresh");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send { thread: None, .. }
    ));
  }

  #[test]
  fn approvals_that_come_are_told_once_and_counted() {
    let mut app = App::new("mbp", "m");
    let needs = |q: &str| Needs {
      detail: None,
      question: Some(q.into()),
      grant: Vec::new(),
      text: format!("Run `{q}`?"),
    };
    // There when it started: counted, not told.
    app.attention_read(vec![needs("old")], true);
    assert!(app.out.is_empty());
    assert!(screen(&app).contains("⚑ 1"));
    app.attention_read(vec![needs("old"), needs("new")], false);
    app.attention_read(vec![needs("old"), needs("new")], false);
    let told = app
      .out
      .iter()
      .filter(|p| matches!(p, Printed::Notice(n) if n.contains("Run `new`?")))
      .count();
    assert_eq!(told, 1);
    // The one asked inline is not told again.
    app.ask("main".into(), "make".into(), "inline".into(), vec![]);
    app.attention_read(vec![needs("inline")], false);
    assert!(
      !app
        .out
        .iter()
        .any(|p| matches!(p, Printed::Notice(n) if n.contains("inline")))
    );
    app.unask();
    assert!(app.approval.is_none());
  }

  #[test]
  fn fork_summarize_and_compact_name_what_they_take() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "/fork");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::None,
      "no thread yet"
    );
    app.thread = Some("t1".into());
    let fork = |app: &mut App, line: &str| {
      typed(app, line);
      app.on_key(key(KeyCode::Enter))
    };
    assert_eq!(
      fork(&mut app, "/fork 5 -t"),
      Action::Fork {
        thread: "t1".into(),
        upto: None,
        last: Some(5),
        no_attachments: false,
        no_tools: true,
        summarize: false,
      }
    );
    assert_eq!(
      fork(&mut app, "/compact 10"),
      Action::Fork {
        thread: "t1".into(),
        upto: None,
        last: Some(10),
        no_attachments: false,
        no_tools: false,
        summarize: true,
      }
    );
    assert!(matches!(
      fork(&mut app, "/summarize"),
      Action::Fork {
        last: None,
        summarize: true,
        ..
      }
    ));
    assert_eq!(fork(&mut app, "/fork x"), Action::None);
  }

  /// `/fork` alone opens a form: all or the last N, -a, -t, -s, with what
  /// it keeps said as it changes; Enter forks, Esc does not.
  #[test]
  fn fork_alone_opens_a_form_that_says_what_it_keeps() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.opened(Some("t1".into()), &conversation());
    let open = |app: &mut App, line: &str| {
      typed(app, line);
      assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
      assert_eq!(app.overlay, Overlay::Fork, "{line}");
    };
    // All of it, as `/fork` forked before.
    open(&mut app, "/fork");
    let s = screen(&app);
    for want in [
      "all of it",
      "its last 10 messages",
      "without attachments",
      "without tool calls",
      "a summary of what is not kept",
      "keeps 5 of 5 messages",
      "1 attachment",
      "as typed: /fork",
    ] {
      assert!(s.contains(want), "{want}: {s}");
    }
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: None,
        last: None,
        no_attachments: false,
        no_tools: false,
        summarize: false,
      }
    );
    assert_eq!(app.overlay, Overlay::None);
    // Its last 2 messages (the first digit replaces the 10), without
    // tool calls (m2, a tool's result, goes with them), from a summary.
    open(&mut app, "/fork");
    app.on_key(key(KeyCode::Right));
    app.on_key(key(KeyCode::Char('2')));
    app.on_key(key(KeyCode::Char('t')));
    // Space changes the row selected: Tab to the summary's.
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Char(' ')));
    let s = screen(&app);
    assert!(s.contains("keeps 2 of 4 messages"), "{s}");
    assert!(s.contains("a summary of 2 messages"), "{s}");
    assert!(s.contains("as typed: /fork 2 -t -s"), "{s}");
    assert!(!s.contains("1 attachment"), "m1 is not kept: {s}");
    // No N: it says so and stays.
    app.on_key(key(KeyCode::Backspace));
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.overlay, Overlay::Fork);
    app.on_key(key(KeyCode::Char('2')));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: None,
        last: Some(2),
        no_attachments: false,
        no_tools: true,
        summarize: true,
      }
    );
    // Esc: no fork.
    open(&mut app, "/fork");
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
    assert_eq!(app.overlay, Overlay::None);
    assert!(app.fork.is_none());
    // `/compact` alone: the form, a summary then the last 10, which here
    // is all of it (the node would refuse: said).
    open(&mut app, "/compact");
    let s = screen(&app);
    assert!(s.contains("nothing is left for a summary"), "{s}");
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Up));
    // The last 3 count back to one of the user's: m4 and m5.
    app.on_key(key(KeyCode::Char('3')));
    let s = screen(&app);
    assert!(s.contains("keeps 2 of 5 messages"), "{s}");
    assert!(s.contains("a summary of 3 messages"), "{s}");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: None,
        last: Some(3),
        no_attachments: false,
        no_tools: false,
        summarize: true,
      }
    );
  }

  /// The message picker's `o`: the form, the fork up to the message
  /// picked (`u` lets it go: all of it).
  #[test]
  fn fork_options_from_a_picked_message_open_the_form_up_to_it() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.opened(Some("t1".into()), &conversation());
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Char('o')));
    assert_eq!(app.overlay, Overlay::Fork);
    let s = screen(&app);
    assert!(s.contains("[x] up to m3 \"first answer\""), "{s}");
    // m1, its tool call and m2 (its result), m3.
    assert!(s.contains("keeps 3 of 3 messages"), "{s}");
    assert!(s.contains("as typed: /fork @m3"), "{s}");
    app.on_key(key(KeyCode::Char('a')));
    assert!(!screen(&app).contains("1 attachment"));
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: Some("m3".into()),
        last: None,
        no_attachments: true,
        no_tools: false,
        summarize: false,
      }
    );
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Char('o')));
    app.on_key(key(KeyCode::Char('u')));
    assert!(screen(&app).contains("keeps 5 of 5 messages"));
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork { upto: None, .. }
    ));
  }

  #[test]
  fn shift_enter_and_backslash_enter_add_a_line() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "one");
    let shift = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
    assert_eq!(app.on_key(shift), Action::None);
    typed(&mut app, "two\\");
    // A terminal that cannot tell Shift+Enter: the `\` becomes the line.
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    typed(&mut app, "three");
    assert_eq!(app.input, "one\ntwo\nthree");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send { text, .. } if text == "one\ntwo\nthree"
    ));
  }

  #[test]
  fn a_failed_send_kept_in_the_thread_is_not_given_back() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "out of usage?");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Kept);
    let said = "Anthropic (Claude subscription): usage limit reached; it resets at 18:00";
    app.on_done(Err(said.into()));
    assert_eq!(app.input, "");
    assert_eq!(app.out.last(), Some(&Printed::Error(said.into())));
    assert!(app.status.contains("kept in the thread"), "{}", app.status);
  }

  #[test]
  fn a_failed_send_gives_the_message_back() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "lost?");
    app.on_key(key(KeyCode::Enter));
    app.on_reply(&Reply::Thread("t1".into()));
    app.on_done(Err("no API key".into()));
    assert_eq!(app.input, "lost?");
    assert_eq!(app.out.last(), Some(&Printed::Error("no API key".into())));
    // The new thread it was to start was never written: the next send
    // starts one again rather than naming it.
    assert_eq!(app.thread, None);
  }

  #[test]
  fn pasted_and_dropped_files_are_attached_as_mentions() {
    let mut app = App::new("mbp", "m");
    assert_eq!(app.on_key(ctrl('v')), Action::PasteClipboard);
    let dir = std::env::temp_dir().join(format!("stitch-tui-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let shot = dir.join("my shot.png");
    std::fs::write(&shot, b"\x89PNG\r\n\x1a\n\0\0").unwrap();
    typed(&mut app, "what is");
    // A file dropped on the terminal pastes its path, escaped.
    app.paste(&shot.display().to_string().replace(' ', "\\ "));
    assert_eq!(app.input, "what is @my shot.png ");
    assert_eq!(app.attachments, [shot.display().to_string()]);
    // Pasting it again attaches nothing more.
    app.attach_files(std::slice::from_ref(&shot));
    assert_eq!(app.attachments.len(), 1);
    assert_eq!(app.status, "already attached");
    // Text stays text, a path to a text file too.
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "hi").unwrap();
    app.paste(&notes.display().to_string());
    assert_eq!(app.attachments.len(), 1);
    // Deleting its name takes it back.
    app.input = "what is this".into();
    app.cursor = None;
    match app.on_key(key(KeyCode::Enter)) {
      Action::Send { attachments, .. } => assert!(attachments.is_empty(), "{attachments:?}"),
      other => panic!("{other:?}"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
  }

  #[test]
  fn deliberate_model_picks_are_not_collapsed_into_todays_default() {
    let mut app = App::new("mbp", "openai/gpt-6.1-sol");
    typed(&mut app, "/model openai/gpt-6.1-sol");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.model.as_deref(), Some("openai/gpt-6.1-sol"));
    app.default_model = "openai/gpt-6.1-terra".into();
    assert_eq!(app.model_name(), "openai/gpt-6.1-sol");
    typed(&mut app, "/model default");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.model.as_deref(), Some("default"));
    assert_eq!(
      app.model_name(),
      "openai/gpt-6.1-terra",
      "show resolved role, not its alias"
    );
    typed(&mut app, "/model quick");
    app.on_key(key(KeyCode::Enter));
    app.default_model = "openai/gpt-6.1-sol".into();
    assert_eq!(app.model_name(), "openai/gpt-6.1-sol");
  }

  #[test]
  fn commands_set_the_model_and_attachments() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    typed(&mut app, "/model local/qwen3");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.model.as_deref(), Some("local/qwen3"));
    typed(&mut app, "/attach /definitely/not/here");
    app.on_key(key(KeyCode::Enter));
    assert!(app.attachments.is_empty());
    let file = std::env::temp_dir().join("stitch-tui-attach-test.txt");
    std::fs::write(&file, "x").unwrap();
    typed(&mut app, &format!("/attach {}", file.display()));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.attachments.len(), 1);
    assert!(screen(&app).contains("1 attached"));
    let _ = std::fs::remove_file(file);
    assert!(app.out.is_empty(), "commands print nothing");
  }

  #[test]
  fn sign_in_commands_never_become_chat_messages() {
    let mut app = App::new("node", "openai/model");
    let enter = key(KeyCode::Enter);
    app.input = "/login openai --file".into();
    assert_eq!(
      app.on_key(enter),
      Action::Auth(AuthAction::Login {
        provider: "openai".into(),
        file: true
      })
    );
    assert!(app.auth_pending);
    assert!(app.pending.is_none());
    assert_eq!(app.overlay, Overlay::Auth);
    assert!(app.out.is_empty());
    app.input = "/login grok".into();
    assert_eq!(app.on_key(enter), Action::None);
    app.auth_pending = false;
    app.input = "/auth".into();
    assert_eq!(
      app.on_key(enter),
      Action::Auth(AuthAction::Status { provider: None })
    );
    app.auth_pending = false;
    app.input = "/logout anthropic".into();
    assert_eq!(
      app.on_key(enter),
      Action::Auth(AuthAction::Logout {
        provider: "anthropic".into()
      })
    );
    app.auth_pending = false;
    app.input = "/login openai unexpected".into();
    assert_eq!(app.on_key(enter), Action::None);
    assert!(!app.auth_pending);
    app.input = "hello".into();
    assert!(matches!(app.on_key(enter), Action::Send { .. }));
    assert_eq!(app.overlay, Overlay::None);
  }

  #[test]
  fn the_live_area_grows_with_what_it_shows() {
    let mut app = App::new("mbp", "m");
    let idle = app.height(80);
    app.threads = threads();
    app.on_key(ctrl('o'));
    assert!(app.height(80) > idle);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.height(80), idle);
  }

  #[test]
  fn spawn_and_send_and_drop_clears_attachments() {
    let mut app = App::new("mbp", "m");
    typed(&mut app, "/spawn clean the build cache");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Spawn {
        node: None,
        model: None,
        goal: "clean the build cache".into()
      }
    );
    // /send picks the thread, then the message is typed.
    app.threads = threads();
    typed(&mut app, "/send");
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.input, "/send t1 ");
    typed(&mut app, "and the tests?");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::SendTo {
        thread: "t1".into(),
        text: "and the tests?".into()
      }
    );
    app.attachments.push("/tmp/x".into());
    typed(&mut app, "/drop");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert!(app.attachments.is_empty());
  }

  #[test]
  fn questions_are_answered_from_the_attention_list() {
    let mut app = App::new("mbp", "m");
    app.attention = vec![
      Needs {
        detail: None,
        grant: Vec::new(),
        question: None,
        text: "still failing  x".into(),
      },
      Needs {
        detail: None,
        grant: Vec::new(),
        question: Some("q1".into()),
        text: "Run `rm -rf build`?".into(),
      },
      Needs {
        detail: None,
        grant: Vec::new(),
        question: Some("q2".into()),
        text: "Run `git push`?".into(),
      },
    ];
    app.on_key(ctrl('t'));
    assert_eq!(app.overlay, Overlay::Attention);
    // Not a question: nothing to answer.
    assert_eq!(app.on_key(key(KeyCode::Char('y'))), Action::None);
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.on_key(key(KeyCode::Char('y'))),
      Action::Answer {
        question: "q1".into(),
        decision: Decision::allow()
      }
    );
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.picked_needs, 2, "stays on the last item");
    assert_eq!(app.on_key(key(KeyCode::Char('n'))), Action::None);
    typed(&mut app, "not today");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Answer {
        question: "q2".into(),
        decision: Decision::decline(Some("not today".into()))
      }
    );
    assert!(app.answering.is_none() && app.reason.is_none());
  }

  #[test]
  fn the_attention_title_offers_always_only_when_it_grants_something() {
    let mut app = App::new("mbp", "m");
    assert_eq!(app.attention_title(), "needs you · ↑↓ · Esc");
    app.attention = vec![
      Needs {
        detail: None,
        grant: Vec::new(),
        question: None,
        text: "still failing  x".into(),
      },
      Needs {
        detail: None,
        grant: Vec::new(),
        question: Some("q1".into()),
        text: "Run `systemctl --user cat x`?".into(),
      },
      Needs {
        detail: Some("all of it".into()),
        grant: vec!["cargo test".into(), "git log".into()],
        question: Some("q2".into()),
        text: "Run `cargo test; git log`?".into(),
      },
    ];
    app.on_key(ctrl('t'));
    assert_eq!(app.overlay, Overlay::Attention);
    // Not a question: nothing to answer.
    assert_eq!(app.attention_title(), "needs you · ↑↓ · Esc");
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.attention_title(),
      "needs you · ↑↓ · y allow · n decline · Esc"
    );
    // Not offered, and not taken.
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), Action::None);
    app.on_key(key(KeyCode::Down));
    assert_eq!(
      app.attention_title(),
      "needs you · ↑↓ · v read it · y allow · a always allow `cargo test`, `git log` · n decline · Esc"
    );
  }

  #[test]
  fn the_open_thread_prints_what_it_gains() {
    let entry = |who: &str, text: &str| Entry {
      who: who.into(),
      text: text.into(),
      ..Default::default()
    };
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    let mut t = vec![entry("you", "hi"), entry("claude", "hello")];
    app.opened(Some("t1".into()), &t);
    app.out.clear();
    app.followed(&t);
    assert!(app.out.is_empty(), "nothing new");
    t.push(entry("you", "[Report from the detached thread]"));
    t.push(entry("claude", "Noted."));
    app.followed(&t);
    assert_eq!(app.out.len(), 2);
    assert_eq!(app.last_reply.as_deref(), Some("Noted."));
  }

  #[test]
  fn new_threads_belong_here_unless_global() {
    let mut app = App::new("mbp", "m");
    app.here = "~/src/app".into();
    app.threads = threads();
    typed(&mut app, "/new global");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.place(), "global");
    typed(&mut app, "hi");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send { global: true, .. }
    ));
    app.on_done(Ok(done("t9")));
    assert!(!app.global, "a thread, once made, has its place");
    assert_eq!(app.on_key(ctrl('n')), Action::Open(None));
    app.opened(None, &[]);
    assert_eq!(app.place(), "~/src/app");
    // A thread from elsewhere shows its place; one from here, where it runs.
    app.opened(Some("t1".into()), &[]);
    assert_eq!(app.place(), "global");
    app.opened(Some("t2".into()), &[]);
    assert_eq!(app.place(), "~/src/app");
    // Tab in the picker lists this place's threads only, then every
    // place's, then this place's and global ones again.
    app.on_key(ctrl('o'));
    assert_eq!(app.scope, ThreadScope::HereAndGlobal);
    assert_eq!(app.on_key(key(KeyCode::Tab)), Action::ListThreads);
    assert_eq!(app.scope, ThreadScope::Here);
    assert_eq!(app.on_key(key(KeyCode::Tab)), Action::ListThreads);
    assert_eq!(app.scope, ThreadScope::All);
    assert_eq!(app.on_key(key(KeyCode::Tab)), Action::ListThreads);
    assert_eq!(app.scope, ThreadScope::HereAndGlobal);
  }

  #[test]
  fn tools_toggle_in_a_list_that_stays_and_go_with_a_new_thread() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    typed(&mut app, "/tools");
    let asked = app.on_key(key(KeyCode::Enter));
    assert_eq!(
      asked,
      Action::Fetch(Fetch::Toolset {
        skills: false,
        thread: None,
        global: false,
        tools: vec![],
        skill_changes: vec![],
      })
    );
    app.filled(
      vec![
        Choice {
          value: "-web".into(),
          label: "[x] web".into(),
          detail: "browse_web".into(),
        },
        Choice {
          value: "+github".into(),
          label: "[ ] github".into(),
          detail: "MCP server".into(),
        },
      ],
      String::new(),
    );
    // Enter on the second: kept for the new thread, the list asked again.
    app.on_key(key(KeyCode::Down));
    let toggled = app.on_key(key(KeyCode::Enter));
    assert_eq!(
      toggled,
      Action::Fetch(Fetch::Toolset {
        skills: false,
        thread: None,
        global: false,
        tools: vec!["+github".into()],
        skill_changes: vec![],
      })
    );
    assert_eq!(app.overlay, Overlay::Pick, "the list stays open");
    // A second Enter while it is asked is ignored.
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    app.on_key(key(KeyCode::Esc));
    typed(&mut app, "check mail");
    let Action::Send { tools, thread, .. } = app.on_key(key(KeyCode::Enter)) else {
      panic!("a message")
    };
    assert_eq!((thread, tools), (None, vec!["+github".to_owned()]));
    assert!(app.new_tools.is_empty(), "sent once");
    // An open thread's change is the node's to write.
    app.thread = Some("2026-10-12T100000Z-mbp-abcd".into());
    typed(&mut app, "/skills");
    app.on_key(key(KeyCode::Enter));
    app.filled(
      vec![Choice {
        value: "-release".into(),
        label: "[x] release".into(),
        detail: String::new(),
      }],
      String::new(),
    );
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fetch(Fetch::Toolset {
        skills: true,
        thread: Some("2026-10-12T100000Z-mbp-abcd".into()),
        global: false,
        tools: vec![],
        skill_changes: vec!["-release".into()],
      })
    );
  }

  /// A thread's transcript: a question with an attachment, a tool call,
  /// a reply, a second question and its reply.
  fn conversation() -> Vec<Entry> {
    let msg = |who: &str, text: &str, id: &str| Entry {
      who: who.into(),
      text: text.into(),
      id: Some(id.into()),
      at: Some(jiff::Timestamp::UNIX_EPOCH),
      ..Default::default()
    };
    vec![
      Entry {
        said: "first question".into(),
        attachments: 1,
        ..msg("you", "first question\n[image/png a.png]", "m1")
      },
      Entry {
        who: "tool".into(),
        text: "$ ls".into(),
        ..Default::default()
      },
      msg("output", "a.png", "m2"),
      msg("claude", "first answer", "m3"),
      msg("you", "second question\nwith two lines", "m4"),
      msg("claude", "second answer", "m5"),
    ]
  }

  fn esc_esc(app: &mut App) {
    app.on_key(key(KeyCode::Esc));
    app.on_key(key(KeyCode::Esc));
  }

  /// After a rewind nothing is printed again: a line says where the
  /// thread goes on from, and what is followed after is printed once.
  #[test]
  fn a_rewind_prints_a_line_and_nothing_again() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.opened(Some("t1".into()), &conversation());
    app.out.clear();
    // Rewound to m3 ("first answer"): the line is its first four entries.
    let line: Vec<Entry> = conversation().into_iter().take(4).collect();
    app.rewound(Some("m3"), &line);
    assert_eq!(app.out.len(), 1, "{:?}", app.out);
    let Printed::Separator(said) = &app.out[0] else {
      panic!("{:?}", app.out)
    };
    assert!(
      said.contains("m3") && said.contains("first answer") && said.contains("set aside"),
      "{said}"
    );
    assert_eq!(app.pickable(), vec![0, 3], "the picker has the line only");
    assert_eq!(app.on_key(ctrl('y')), Action::Copy("first answer".into()));
    // The edited message and its answer, followed: printed once.
    app.out.clear();
    let mut more = line.clone();
    more.push(Entry {
      who: "you".into(),
      text: "another question".into(),
      id: Some("m6".into()),
      ..Default::default()
    });
    app.followed(&more);
    app.followed(&more);
    assert_eq!(app.out, vec![Printed::You("another question".into())]);
  }

  /// Preset answers (design §2.30): said once under the turn, `/2` sends
  /// the second's text as a message (a `/` in it included), and once
  /// anything is written they are gone.
  #[test]
  fn a_preset_reply_is_picked_with_its_number() {
    use stitch_node::replies::Suggested;
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    let mut t = conversation();
    t.push(Entry {
      who: "tool".into(),
      text: "suggest_replies: Yes · Compact".into(),
      replies: vec![
        Suggested {
          label: "Yes".into(),
          text: "Yes".into(),
        },
        Suggested {
          label: "Compact".into(),
          text: "/compact please".into(),
        },
      ],
      ..Default::default()
    });
    t.push(Entry {
      who: "claude".into(),
      text: "Go on?".into(),
      ..Default::default()
    });
    app.opened(Some("t1".into()), &t);
    let said: Vec<_> = app
      .out
      .iter()
      .filter(|p| matches!(p, Printed::Notice(n) if n.starts_with('↳')))
      .collect();
    assert_eq!(said, vec![&Printed::Notice("↳ /1 Yes   /2 Compact".into())]);
    // Followed again: not said twice.
    app.followed(&t);
    assert_eq!(
      app
        .out
        .iter()
        .filter(|p| matches!(p, Printed::Notice(n) if n.starts_with('↳')))
        .count(),
      1
    );
    typed(&mut app, "/3");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert!(app.status.contains("2 preset replies"), "{}", app.status);
    app.input.clear();
    typed(&mut app, "/2");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Send { text, .. } if text == "/compact please"
    ));
    // A turn runs: none to pick.
    app.input.clear();
    typed(&mut app, "/1");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert_eq!(
      app.status,
      "no preset replies or action buttons to pick now"
    );
    // It ended, with the message in the transcript: inert.
    app.pending = None;
    t.push(Entry {
      who: "you".into(),
      text: "/compact please".into(),
      ..Default::default()
    });
    app.followed(&t);
    assert!(app.live_replies().is_none());
  }

  /// Action buttons (design §2.30, step 2): said once, `/2` presses the
  /// second of the latest set (no message sent), and they stay live after
  /// the user writes; preset answers live meanwhile take the number.
  #[test]
  fn an_action_button_is_pressed_with_its_number() {
    use stitch_node::actions::Action as Button;
    let button = |label: &str| Button {
      label: label.into(),
      tool: "shell_run".into(),
      input: serde_json::json!({"command": "echo x"}),
      wake: false,
    };
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    let mut t = conversation();
    t.push(Entry {
      who: "tool".into(),
      text: "offer_actions: ▶ Dock · ▶ Undock".into(),
      actions: vec![button("Dock"), button("Undock")],
      call: Some("o1".into()),
      ..Default::default()
    });
    t.push(Entry {
      who: "claude".into(),
      text: "Pick a waypoint.".into(),
      ..Default::default()
    });
    app.opened(Some("t1".into()), &t);
    assert!(
      app
        .out
        .contains(&Printed::Notice("↳ /1 ▶ Dock   /2 ▶ Undock".into()))
    );
    typed(&mut app, "/2");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Press {
        thread: "t1".into(),
        set: "o1".into(),
        item: 1,
      }
    );
    assert!(app.input.is_empty());
    typed(&mut app, "/3");
    assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
    assert!(app.status.contains("2 action buttons"), "{}", app.status);
    app.input.clear();
    // The user wrote, a press's line came: still live.
    t.push(Entry {
      who: "you".into(),
      text: "next".into(),
      ..Default::default()
    });
    t.push(Entry {
      who: "press".into(),
      text: "▶ Undock: ok".into(),
      ..Default::default()
    });
    app.followed(&t);
    assert!(app.out.contains(&Printed::Tool("▶ Undock: ok".into())));
    assert_eq!(app.live_actions().map(|(_, set, _)| set), Some("o1"));
    typed(&mut app, "/1");
    assert!(matches!(
      app.on_key(key(KeyCode::Enter)),
      Action::Press { item: 0, .. }
    ));
  }

  #[test]
  fn esc_esc_picks_a_message_and_forks_from_it() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.opened(Some("t1".into()), &conversation());
    // One Esc says what a second does.
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::None);
    assert!(app.status.contains("Esc again"), "{}", app.status);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::Messages);
    // The user's and the model's messages, the latest picked.
    assert_eq!(app.pickable(), vec![0, 3, 4, 5]);
    let s = screen(&app);
    assert!(s.contains("message 4 of 4"), "{s}");
    assert!(s.contains("claude · ") && s.contains("m5"), "{s}");
    assert!(s.contains("second answer"), "{s}");
    app.on_key(key(KeyCode::Up));
    let s = screen(&app);
    assert!(
      s.contains("second question") && s.contains("with two lines"),
      "{s}"
    );
    // Enter: its actions; a user's message can be edited.
    app.on_key(key(KeyCode::Enter));
    let s = screen(&app);
    assert!(
      s.contains("Fork from here") && s.contains("Edit and resend"),
      "{s}"
    );
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: Some("m4".into()),
        last: None,
        no_attachments: false,
        no_tools: false,
        summarize: false,
      }
    );
    assert_eq!(app.overlay, Overlay::None);
    // A reply's actions have no edit; Esc goes back, then out.
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Enter));
    assert!(!screen(&app).contains("Edit and resend"));
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.msgs.menu, None);
    assert_eq!(app.overlay, Overlay::Messages);
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::None);
    // Esc Esc with text in the input, or with nothing open, does not.
    typed(&mut app, "x");
    esc_esc(&mut app);
    assert_eq!(app.overlay, Overlay::None);
    let mut fresh = App::new("mbp", "m");
    esc_esc(&mut fresh);
    assert_eq!(fresh.overlay, Overlay::None);
  }

  #[test]
  fn picked_messages_fork_with_options_edit_and_copy() {
    let mut app = App::new("mbp", "m");
    app.threads = threads();
    app.opened(Some("t1".into()), &conversation());
    // Options: the fork form, up to m3; `/fork @m3` typed takes them too.
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Up));
    app.on_key(key(KeyCode::Char('o')));
    assert_eq!(app.overlay, Overlay::Fork);
    assert_eq!(app.input, "");
    assert_eq!(app.fork.as_ref().unwrap().upto.as_deref(), Some("m3"));
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.overlay, Overlay::None);
    typed(&mut app, "/fork @m3 4 -a -t -s");
    assert_eq!(
      app.on_key(key(KeyCode::Enter)),
      Action::Fork {
        thread: "t1".into(),
        upto: Some("m3".into()),
        last: Some(4),
        no_attachments: true,
        no_tools: true,
        summarize: true,
      }
    );
    // Copy: its text, without the attachment lines.
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Home));
    assert_eq!(
      app.on_key(key(KeyCode::Char('c'))),
      Action::Copy("first question".into())
    );
    // Edit: the same thread rewound to the message before it (not a
    // fork), its text in the input.
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Up));
    assert_eq!(
      app.on_key(key(KeyCode::Char('e'))),
      Action::Rewind {
        thread: "t1".into(),
        upto: Some("m3".into()),
        resend: None,
      }
    );
    assert_eq!(app.input, "second question\nwith two lines");
    // The first message: rewound to the thread's start, its attachment not
    // carried.
    app.input.clear();
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Home));
    assert_eq!(
      app.on_key(key(KeyCode::Char('e'))),
      Action::Rewind {
        thread: "t1".into(),
        upto: None,
        resend: None,
      }
    );
    assert_eq!(app.input, "first question");
    assert!(app.status.contains("not carried"), "{}", app.status);
    // Rewind to here: a reply goes on from it; a message of the user's is
    // sent again.
    app.input.clear();
    esc_esc(&mut app);
    assert_eq!(
      app.on_key(key(KeyCode::Char('r'))),
      Action::Rewind {
        thread: "t1".into(),
        upto: Some("m5".into()),
        resend: None,
      }
    );
    esc_esc(&mut app);
    app.on_key(key(KeyCode::Up));
    assert_eq!(
      app.on_key(key(KeyCode::Char('r'))),
      Action::Rewind {
        thread: "t1".into(),
        upto: Some("m3".into()),
        resend: Some("second question\nwith two lines".into()),
      }
    );
    // While a turn runs, Esc interrupts it rather than picking.
    app.input.clear();
    app.pending = Some(Pending::default());
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Interrupt);
  }

  #[test]
  fn esc_stops_a_turn_this_tui_did_not_start() {
    let mut app = App::new("mbp", "anthropic/claude-opus-5-5");
    app.set_threads(threads());
    // t2 works (a job's note woke it): Esc stops it, not the picker.
    app.thread = Some("t2".into());
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Cancel("t2".into()));
    // Waiting behind it with a message: that turn is stopped too.
    app.pending = Some(Pending {
      waiting: true,
      ..Pending::default()
    });
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Cancel("t2".into()));
    // Its own turn: interrupted on its connection, as before.
    app.pending = Some(Pending::default());
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Interrupt);
    // Nothing runs on t1: Esc is the picker's first press.
    app.pending = None;
    app.thread = Some("t1".into());
    assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
  }

  #[test]
  fn at_picks_a_file_and_attaches_it_while_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    std::fs::write(root.join("logo.png"), [0x89u8, b'P', b'N', b'G', 0, 0]).unwrap();
    let mut app = App::new("mbp", "m");
    app.files_root = Some(root.to_owned());
    typed(&mut app, "look at @");
    assert!(app.at_pick.is_some());
    let s = screen(&app);
    assert!(s.contains("@ files") && s.contains("@src/"), "{s}");
    // Typing filters; Tab picks.
    typed(&mut app, "mai");
    assert_eq!(app.at_choices()[0], "src/main.rs");
    app.on_key(key(KeyCode::Tab));
    assert!(app.at_pick.is_none());
    assert_eq!(app.input, "look at @src/main.rs ");
    let main = root.join("src/main.rs").display().to_string();
    assert_eq!(app.attachments, vec![main.clone()]);
    assert!(screen(&app).contains("1 attached (@src/main.rs)"));
    // A binary is named only; Enter picks too.
    typed(&mut app, "and @logo");
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.input, "look at @src/main.rs and @logo.png ");
    assert!(app.status.contains("not text"), "{}", app.status);
    assert_eq!(app.attachments.len(), 1);
    // Esc closes it, leaving what was typed; an e-mail opens nothing.
    typed(&mut app, "@sr");
    app.on_key(key(KeyCode::Esc));
    assert!(app.at_pick.is_none());
    assert!(app.input.ends_with("@sr"));
    typed(&mut app, " me@x");
    assert!(app.at_pick.is_none());
    // Nor does `/spawn @node`.
    let mut cmd = App::new("mbp", "m");
    cmd.files_root = Some(root.to_owned());
    typed(&mut cmd, "/spawn @");
    assert!(cmd.at_pick.is_none());
    // A space closes it; the file named stays attached when sent.
    let mut app2 = App::new("mbp", "m");
    app2.files_root = Some(root.to_owned());
    typed(&mut app2, "@lib");
    app2.on_key(key(KeyCode::Enter));
    typed(&mut app2, "then @x ");
    assert!(app2.at_pick.is_none());
    match app2.on_key(key(KeyCode::Enter)) {
      Action::Send { attachments, .. } => {
        assert_eq!(
          attachments,
          vec![root.join("src/lib.rs").display().to_string()]
        )
      }
      other => panic!("{other:?}"),
    }
    // Its name taken out of the text: its attachment goes too.
    app.input = "look at it".into();
    app.cursor = None;
    match app.on_key(key(KeyCode::Enter)) {
      Action::Send { attachments, .. } => assert!(attachments.is_empty(), "{attachments:?}"),
      other => panic!("{other:?}"),
    }
    assert!(app.mentions.is_empty());
    // A file attached with /attach stays whatever the text says (queued,
    // the turn sent above running).
    app.attachments.push(main.clone());
    typed(&mut app, "hello");
    match app.on_key(key(KeyCode::Enter)) {
      Action::Interject { attachments, .. } => assert_eq!(attachments, vec![main]),
      other => panic!("{other:?}"),
    }
  }
}
