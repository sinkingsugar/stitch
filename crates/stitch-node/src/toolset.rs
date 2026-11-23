//! Tool and skill sets (design §2.14): what a thread has, as ordered
//! entries that turn things on (`+x`, or `x`) or off (`-x`), the last entry
//! matching an item deciding.
//!
//! - **Tools:** a built-in group (`shell`, `web`, `viewer`, `files`, `weave`, `apps`,
//!   `memory`, `browser`, `calendar`, `maps`), an MCP server (`github`: all it offers), or a pattern over
//!   namespaced MCP tool names (`github__get_*`). A bare name is a group
//!   when it names one, else a server; `<name>__*` always means the server.
//! - **Skills:** a skill's name, or a pattern.
//!
//! Layers, in order: the built-in default (groups on, MCP tools off,
//! skills on), the node's `[mcp]` enable lists, the place's settings, then
//! the thread's own changes: `toolset` records in its repository, folded in
//! the order written, so changes made on two nodes offline both count.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, Draft, NodeName, RecordId, Ref, kinds};
use stitch_store::{Index, RecordRoot};

use crate::mcp::glob;
use crate::{Error, Result};

/// The built-in tool groups and their tools.
pub const GROUPS: &[(&str, &[&str])] = &[
  (
    "shell",
    &[
      "shell_run",
      "shell_send",
      "shell_watch",
      "shell_wait",
      "watch_file",
      "unwatch",
      "jobs",
      "job_stop",
      "job_output",
      "shell_view",
      "shell_close",
    ],
  ),
  ("web", &["browse_web", "web_eval", "search_web"]),
  ("viewer", &["view_file", "zoom_image"]),
  ("files", &["write_file", "edit_file", "commit_push"]),
  (
    "weave",
    &[
      "spawn",
      "send",
      "wait",
      "threads",
      "cancel",
      "models",
      "read_thread",
      "schedule",
      "wake_at",
      "subscribe",
      "event_reply",
      "notify",
      "suggest_replies",
      "offer_actions",
      "report_issue",
    ],
  ),
  // Off unless turned on (`OFF_BY_DEFAULT`): it starts fixers that change
  // Stitch's own code and spend money.
  ("self_improve", &["self_improve"]),
  ("memory", &["recall"]),
  ("images", &["generate_image"]),
  // Closed questions over a text, answered by the decision model (design
  // §2.29): priced to the thread.
  ("decide", &["decide"]),
  ("apps", &["show_app"]),
  ("browser", crate::browser::engine::TOOLS),
  ("calendar", crate::personal::CALENDAR_TOOLS),
  ("maps", crate::personal::MAPS_TOOLS),
];

/// Groups no thread has unless something turns them on: the user (`stitch
/// toolset`, a place's settings) or the node for its own threads (the
/// harness-issues schedule's). An agent cannot hand them on (`spawn`).
pub const OFF_BY_DEFAULT: &[&str] = &["self_improve"];

/// Whether `entries` turn on a group that is off by default.
pub fn enables_guarded(entries: &[Entry]) -> Option<&'static str> {
  OFF_BY_DEFAULT
    .iter()
    .copied()
    .find(|g| entries.iter().any(|e| e.on && e.pattern == *g))
}

/// The group a built-in tool belongs to.
pub fn group_of(tool: &str) -> Option<&'static str> {
  GROUPS
    .iter()
    .find(|(_, tools)| tools.contains(&tool))
    .map(|(g, _)| *g)
}

/// The group an entry's pattern names exactly (`spawn` names `weave`).
pub fn group_named(pattern: &str) -> Option<&'static str> {
  let pattern = if pattern == "spawn" { "weave" } else { pattern };
  GROUPS.iter().find(|(g, _)| *g == pattern).map(|(g, _)| *g)
}

/// One change: something turned on or off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
  pub on: bool,
  pub pattern: String,
}

impl Entry {
  /// `+x` or `x` (on), `-x` (off).
  pub fn parse(s: &str) -> Result<Entry> {
    let s = s.trim();
    let (on, pattern) = match s.strip_prefix('-') {
      Some(p) => (false, p),
      None => (true, s.strip_prefix('+').unwrap_or(s)),
    };
    let ok = !pattern.is_empty()
      && pattern.len() <= 128
      && !pattern.starts_with(['+', '-'])
      && pattern
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '*'));
    if !ok {
      return Err(Error::Config(format!(
        "`{s}`: name a tool group, an MCP server, a pattern such as github__get_* or a \
         skill, with + to turn it on or - to turn it off"
      )));
    }
    Ok(Entry {
      on,
      pattern: pattern.to_owned(),
    })
  }

  pub fn on(pattern: &str) -> Entry {
    Entry {
      on: true,
      pattern: pattern.to_owned(),
    }
  }

  pub fn off(pattern: &str) -> Entry {
    Entry {
      on: false,
      pattern: pattern.to_owned(),
    }
  }
}

/// `+x` or `x` (on), `-x` (off), unchecked: for entries written in code.
impl From<&str> for Entry {
  fn from(s: &str) -> Entry {
    match s.strip_prefix('-') {
      Some(p) => Entry::off(p),
      None => Entry::on(s.strip_prefix('+').unwrap_or(s)),
    }
  }
}

impl PartialEq<&str> for Entry {
  fn eq(&self, other: &&str) -> bool {
    let (on, pattern) = match other.strip_prefix('-') {
      Some(p) => (false, p),
      None => (true, other.strip_prefix('+').unwrap_or(other)),
    };
    self.on == on && self.pattern == pattern
  }
}

impl std::fmt::Display for Entry {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}{}", if self.on { '+' } else { '-' }, self.pattern)
  }
}

/// Entries as written (`+x`, `x`, `-x`).
pub fn parse_all(list: &[String]) -> Result<Vec<Entry>> {
  list
    .iter()
    .filter(|s| !s.trim().is_empty())
    .map(|s| Entry::parse(s))
    .collect()
}

/// Whether `pattern` matches built-in group `group`.
pub fn matches_group(pattern: &str, group: &str) -> bool {
  if pattern.contains("__") {
    return false;
  }
  match group_named(pattern) {
    Some(g) => g == group,
    None => pattern.contains('*') && glob(pattern, group),
  }
}

/// Whether `pattern` matches MCP tool `name` (namespaced) of `server`.
pub fn matches_tool(pattern: &str, server: &str, name: &str) -> bool {
  if pattern.contains("__") {
    glob(pattern, name)
  } else if group_named(pattern).is_some() {
    false
  } else {
    glob(pattern, server)
  }
}

/// Whether `pattern` could match some tool of `server`.
pub fn may_match_server(pattern: &str, server: &str) -> bool {
  match pattern.split_once("__") {
    Some((s, _)) => glob(s, server),
    None => group_named(pattern).is_none() && glob(pattern, server),
  }
}

/// The last entry matching, or `default`.
pub fn decide(entries: &[Entry], default: bool, matches: impl Fn(&str) -> bool) -> bool {
  entries
    .iter()
    .rev()
    .find(|e| matches(&e.pattern))
    .map_or(default, |e| e.on)
}

/// A thread's tool and skill entries, every layer in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
  pub tools: Vec<Entry>,
  pub skills: Vec<Entry>,
  /// The skills it has whose files are another node's: `read_skill`
  /// returns their text.
  pub remote_skills: Vec<RemoteSkill>,
  /// The thread's mode as the user last switched it (`true`: read-only),
  /// told to the model with its instructions; `None`: never switched.
  pub read_only_switch: Option<bool>,
  /// An app's thread (design §2.22): only these skills, the ones its
  /// grant binds that are still what was granted.
  pub bound: Option<Bound>,
  /// The capability profiles of the schedule that started the turn
  /// (docs/events.md): a ceiling on its tools and Otter shards, on top of
  /// its thread's taint (read at each call, `crate::profile::Ceiling`).
  pub profiles: Vec<crate::profile::Profile>,
}

/// The skills an app's grant binds, as they stand: their SKILL.md files
/// here, and Stitch's own by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bound {
  pub files: Vec<std::path::PathBuf>,
  pub built_in: Vec<String>,
}

/// A skill that is not a file here, read with `read_skill`: a place's as
/// its record carries it (design §2.14), or one of Stitch's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSkill {
  pub name: String,
  /// The node whose files it was read from (`stitch`: built in).
  pub node: String,
  pub text: String,
  /// The files next to its SKILL.md (a built-in's: references, templates),
  /// by path.
  pub files: Vec<(String, String)>,
}

impl Selection {
  /// Whether group `group` is on (on unless an entry turns it off).
  pub fn group_on(&self, group: &str) -> bool {
    // A group off by default is turned on by its name only, never by a
    // pattern (`+*` turns the others on again, not it).
    if OFF_BY_DEFAULT.contains(&group) {
      return decide(&self.tools, false, |p| p == group);
    }
    decide(&self.tools, true, |p| matches_group(p, group))
  }

  /// Whether MCP tool `name` of `server` is on (off until listed).
  pub fn tool_on(&self, server: &str, name: &str) -> bool {
    decide(&self.tools, false, |p| matches_tool(p, server, name))
  }

  /// Whether skill `name` is on (on unless turned off).
  pub fn skill_on(&self, name: &str) -> bool {
    decide(&self.skills, true, |p| glob(p, name))
  }

  /// The tool entries that turn MCP tools on or off, for `McpTools`.
  pub fn mcp(&self) -> Vec<Entry> {
    self
      .tools
      .iter()
      .filter(|e| e.pattern.contains("__") || group_named(&e.pattern).is_none())
      .cloned()
      .collect()
  }
}

/// A thread's own changes, in the order written: tools, then skills.
pub fn thread_changes(index: &Index, thread: &RecordId) -> (Vec<Entry>, Vec<Entry>) {
  let mut records = index
    .sources(kinds::TOOLSET, "thread", thread, None)
    .unwrap_or_default();
  records.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
  let mut tools = Vec::new();
  let mut skills = Vec::new();
  for r in records {
    let list = |key: &str| -> Vec<Entry> {
      r.fields
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
          a.iter()
            .filter_map(Value::as_str)
            .filter_map(|s| Entry::parse(s).ok())
            .collect()
        })
        .unwrap_or_default()
    };
    tools.extend(list("tools"));
    skills.extend(list("skills"));
  }
  (tools, skills)
}

/// Writes changes to a thread's sets as a `toolset` record in `root`, its
/// repository (the thread may not be written yet: a new one's changes
/// come before its first step).
pub fn write_changes(
  root: &RecordRoot,
  index: &std::sync::Mutex<Index>,
  node: &NodeName,
  thread: &RecordId,
  tools: &[Entry],
  skills: &[Entry],
) -> Result<()> {
  if tools.is_empty() && skills.is_empty() {
    return Ok(());
  }
  let list = |l: &[Entry]| toml::Value::Array(l.iter().map(|e| e.to_string().into()).collect());
  let mut said: Vec<String> = tools.iter().map(Entry::to_string).collect();
  said.extend(skills.iter().map(|e| format!("skill {e}")));
  let mut draft = Draft::new(kinds::TOOLSET, Author::user(node.clone()))?
    .reference("thread", Ref::to(thread.clone()))?
    .body(format!("# {}\n", said.join(" ")));
  if !tools.is_empty() {
    draft = draft.field("tools", list(tools))?;
  }
  if !skills.is_empty() {
    draft = draft.field("skills", list(skills))?;
  }
  root.publish(node, draft)?;
  index.lock().expect("index lock").refresh(root)?;
  Ok(())
}

/// What turning `name` on or off says when it is not.
pub fn how_to_change(thread: &RecordId, name: &str, on: bool) -> String {
  let sign = if on { '+' } else { '-' };
  format!("/tools in the TUI, or `stitch thread tools {thread} {sign}{name}`")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entries(list: &[&str]) -> Vec<Entry> {
    list.iter().map(|s| Entry::parse(s).unwrap()).collect()
  }

  #[test]
  fn the_last_matching_entry_decides() {
    let s = Selection {
      tools: entries(&[
        "github",
        "-web",
        "github__create_*",
        "-github__create_issue",
        "+*",
      ]),
      skills: entries(&["-*", "+mail"]),
      ..Selection::default()
    };
    // `+*` turns every group and server on again, but those off by
    // default.
    assert!(s.group_on("web"));
    assert!(!s.group_on("self_improve"));
    assert!(!Selection::default().group_on("self_improve"));
    let on = Selection {
      tools: entries(&["+self_improve"]),
      ..Selection::default()
    };
    assert!(on.group_on("self_improve"));
    assert_eq!(
      enables_guarded(&entries(&["+self_improve"])),
      Some("self_improve")
    );
    assert_eq!(enables_guarded(&entries(&["-self_improve", "+web"])), None);
    assert!(s.tool_on("github", "github__create_issue"));
    let s = Selection {
      tools: entries(&["github", "-web", "-github__create_*", "spawn"]),
      skills: entries(&["-*", "+mail"]),
      ..Selection::default()
    };
    assert!(!s.group_on("web"));
    assert!(s.group_on("shell"), "groups are on by default");
    assert!(s.group_on("weave"), "spawn names weave");
    assert!(s.tool_on("github", "github__get_issue"));
    assert!(!s.tool_on("github", "github__create_issue"));
    assert!(
      !s.tool_on("fetch", "fetch__fetch"),
      "MCP tools are off until listed"
    );
    assert!(s.skill_on("mail"));
    assert!(!s.skill_on("release"));
  }

  #[test]
  fn a_bare_group_name_is_the_group_and_name_star_the_server() {
    let s = Selection {
      tools: entries(&["-memory", "memory__*"]),
      ..Selection::default()
    };
    assert!(!s.group_on("memory"));
    assert!(s.tool_on("memory", "memory__create_entities"));
    let s = Selection {
      tools: entries(&["memory"]),
      ..Selection::default()
    };
    assert!(!s.tool_on("memory", "memory__create_entities"));
    assert!(may_match_server("memory__*", "memory"));
    assert!(!may_match_server("memory", "memory"));
    assert!(may_match_server("git*", "github"));
    assert_eq!(s.mcp(), Vec::<Entry>::new());
  }

  #[test]
  fn entries_parse_with_their_sign() {
    assert_eq!(Entry::parse("+github").unwrap(), Entry::on("github"));
    assert_eq!(
      Entry::parse("github__get_*").unwrap(),
      Entry::on("github__get_*")
    );
    assert_eq!(Entry::parse(" -web ").unwrap(), Entry::off("web"));
    assert_eq!(Entry::off("web").to_string(), "-web");
    for bad in ["", "+", "-", "a b", "x/y", "+-x"] {
      assert!(Entry::parse(bad).is_err(), "{bad}");
    }
  }
}
