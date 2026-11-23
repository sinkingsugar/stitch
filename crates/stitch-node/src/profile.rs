//! Capability profiles (docs/events.md, "Capability profiles"): what a
//! turn started by a schedule or an event subscription may use, enforced
//! by the node at every tool call and every Otter compile, never by the
//! prompt.
//!
//! - **A profile** is an allow-list of tools (`tools`: a built-in group, a
//!   tool's name, or a pattern over names: a skill's Otter tools, MCP
//!   tools), a rule for Otter's shards (`shards`: `+X`/`-X` entries over
//!   shard names, the last matching deciding; `effects`: the effect classes
//!   an untrusted profile lets a shard have by default), and whether its
//!   events are trusted. `event_reply` is always in it: answering an ask is
//!   the action boundary (its schema checks the answer).
//! - **A ceiling:** a turn's tools are what its thread's tool set offers
//!   *and* every profile on it allows. Profiles only add up (intersect);
//!   nothing a wake or the model does takes one off.
//! - **Untrusted** (the default): the profile also taints. The node writes
//!   it on the thread (a user `set` of `profile`) before the payload
//!   reaches the model, so a turn running on it (a woven ask) is held to it
//!   from its next call, and so is every later turn of that thread, its
//!   forks and compactions, until the user lifts it (`stitch event
//!   untaint`). Its shards are allowed by effect class (pure ones, plus
//!   `effects` and those named), so a shard added to the catalog later is
//!   not silently allowed; tools that would carry its reach past the turn
//!   (the shell, the browser, spawning, sending, schedules, wake-ups,
//!   subscriptions) are refused in it.
//! - **Trusted:** a ceiling for the turns it starts only; its shard entries
//!   are a denylist on top of everything (`-FS.*`).

use std::sync::{Arc, Weak};

use otter_core::ShardType;
use otter_core::signature::Effects;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, RecordId};

use crate::mcp::glob;
use crate::node::Shared;
use crate::toolset::{GROUPS, group_named};
use crate::{Error, Result};

/// The field a thread's taint is set in (a user `set`, as its mode is).
pub const FIELD: &str = "profile";

/// Always allowed: the ask's answer, checked by its schema.
pub const ALWAYS: &[&str] = &["event_reply"];

/// Never in an untrusted profile: what reaches past the turn (the user's
/// shell and CLI, their browser, other threads, later wakes, new
/// subscriptions or schedules, Stitch's own fixers).
pub const NEVER_UNTRUSTED: &[&str] = &[
  "spawn",
  "send",
  "cancel",
  "schedule",
  "wake_at",
  "subscribe",
  "self_improve",
  "commit_push",
];

/// Shards whose calls are the thread's own tool calls, each checked
/// against the ceiling as it is made: allowed unless an entry says no.
pub const GATEWAYS: &[&str] = &["Tool.Call", "Decide"];

/// The effect classes, by Otter's names.
pub const EFFECTS: &[&str] = &["io", "time", "suspends", "random", "unknown"];

/// A profile, as a schedule's `profile` field and a thread's taint keep it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
  /// The subscription or schedule it is from (set by the node).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub from: Option<String>,
  /// Allowed tools: a group (`viewer`), a name (`notify`), a pattern
  /// (`support_*`, `github__get_*`).
  #[serde(default)]
  pub tools: Vec<String>,
  /// Shard entries: `+Http.Get`, `-FS.*`; the last matching decides.
  #[serde(default)]
  pub shards: Vec<String>,
  /// Untrusted: the effect classes a shard may have and still be allowed
  /// without being named (none: only pure shards).
  #[serde(default)]
  pub effects: Vec<String>,
  /// Its events come from the user's own programs: no taint, and its shard
  /// entries only deny.
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub trusted: bool,
}

fn name_ok(p: &str) -> bool {
  !p.is_empty()
    && p.len() <= 128
    && p
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '*'))
}

/// Whether tool entry `entry` takes tool `tool`: its group's name, its
/// own, or a pattern over names.
fn entry_takes(entry: &str, tool: &str) -> bool {
  if let Some(g) = group_named(entry) {
    // `spawn` names the weave group in tool sets; here it is the tool.
    if entry != "spawn" {
      return GROUPS
        .iter()
        .any(|(name, tools)| *name == g && tools.contains(&tool));
    }
  }
  glob(entry, tool)
}

/// The tools of the groups that reach past the turn, never allowed to an
/// untrusted profile: `NEVER_UNTRUSTED`, the shell's and the browser's.
fn never_untrusted() -> Vec<&'static str> {
  let mut out: Vec<&'static str> = NEVER_UNTRUSTED.to_vec();
  for (g, tools) in GROUPS {
    if matches!(*g, "shell" | "browser") {
      out.extend(tools.iter().copied());
    }
  }
  out
}

impl Profile {
  /// Read from what a request gave (`{tools, shards, effects, trusted}`),
  /// checked.
  pub fn from_input(v: &Value) -> Result<Profile> {
    let p: Profile =
      serde_json::from_value(v.clone()).map_err(|e| Error::Config(format!("profile: {e}")))?;
    p.checked()
  }

  /// As kept in records (JSON, as indexed); `None` when unreadable.
  pub fn from_record(v: &Value) -> Option<Profile> {
    serde_json::from_value(v.clone()).ok()
  }

  /// Refused when it names what it cannot: a bad entry, an unknown effect
  /// class, or (untrusted) a tool that reaches past the turn.
  pub fn checked(mut self) -> Result<Profile> {
    self.tools.retain(|t| !t.trim().is_empty());
    for t in &mut self.tools {
      *t = t.trim().to_owned();
      if !name_ok(t) {
        return Err(Error::Config(format!(
          "profile tools: `{t}` is not a tool group, a tool's name or a pattern"
        )));
      }
    }
    self.shards.retain(|t| !t.trim().is_empty());
    for e in &mut self.shards {
      let entry = crate::toolset::Entry::parse(e)
        .map_err(|_| Error::Config(format!("profile shards: `{e}` is not `+Name` or `-Name`")))?;
      *e = entry.to_string();
    }
    for e in &self.effects {
      if !EFFECTS.contains(&e.as_str()) {
        return Err(Error::Config(format!(
          "profile effects: `{e}` is not one of {}",
          EFFECTS.join(", ")
        )));
      }
    }
    if self.trusted && !self.effects.is_empty() {
      return Err(Error::Config(
        "profile effects bound an untrusted profile's shards; a trusted one allows every \
         shard but those its `shards` entries deny"
          .into(),
      ));
    }
    if !self.trusted {
      for t in &self.tools {
        if let Some(n) = never_untrusted().into_iter().find(|n| entry_takes(t, n)) {
          return Err(Error::Config(format!(
            "an untrusted profile cannot allow `{t}`: it takes {n}, which reaches past the turn \
             (the shell, the browser, other threads, later wakes, schedules and \
             subscriptions are never given to a turn that reads untrusted events)"
          )));
        }
      }
    }
    Ok(self)
  }

  /// As a record's field.
  pub fn to_toml(&self) -> toml::Value {
    let v = serde_json::to_value(self).expect("a profile serializes");
    json_to_toml(&v)
  }

  /// Whether it lets tool `name` run.
  pub fn allows_tool(&self, name: &str) -> bool {
    if ALWAYS.contains(&name) {
      return true;
    }
    if !self.trusted && never_untrusted().contains(&name) {
      return false;
    }
    self.tools.iter().any(|e| entry_takes(e, name))
  }

  /// Whether it lets shard `ty` be compiled in; `Err`: why not.
  pub fn allows_shard(&self, ty: &ShardType) -> std::result::Result<(), String> {
    let names: Vec<&str> = std::iter::once(ty.name())
      .chain(ty.desc.aliases.iter().copied())
      .collect();
    let named = self.shards.iter().rev().find_map(|e| {
      let (on, pattern) = match e.strip_prefix('-') {
        Some(p) => (false, p),
        None => (true, e.strip_prefix('+').unwrap_or(e)),
      };
      names
        .iter()
        .any(|n| glob(pattern, n))
        .then_some((on, e.as_str()))
    });
    match named {
      Some((true, _)) => return Ok(()),
      Some((false, e)) => return Err(format!("its entry `{e}` denies it")),
      None => {}
    }
    if self.trusted || GATEWAYS.contains(&ty.name()) {
      return Ok(());
    }
    let has = effect_names(ty.desc.effects);
    let over: Vec<&str> = has
      .iter()
      .copied()
      .filter(|e| !self.effects.iter().any(|a| a == e))
      .collect();
    if over.is_empty() {
      Ok(())
    } else {
      Err(format!(
        "its effects ({}) are not among those the untrusted profile allows ({}), and no `+` \
         entry names it",
        over.join(", "),
        if self.effects.is_empty() {
          "none: pure shards only".to_owned()
        } else {
          self.effects.join(", ")
        }
      ))
    }
  }

  /// In a line, for messages and listings.
  pub fn said(&self) -> String {
    let mut tools: Vec<String> = ALWAYS.iter().map(|s| (*s).to_owned()).collect();
    tools.extend(self.tools.iter().cloned());
    let mut out = format!(
      "{}: tools {}",
      if self.trusted { "trusted" } else { "untrusted" },
      tools.join(", ")
    );
    if !self.shards.is_empty() {
      out.push_str(&format!("; shards {}", self.shards.join(" ")));
    }
    if !self.trusted {
      out.push_str(&format!(
        "; shard effects {}",
        if self.effects.is_empty() {
          "none (pure)".to_owned()
        } else {
          self.effects.join(", ")
        }
      ));
    }
    out
  }
}

/// Otter's effect names of `e`.
fn effect_names(e: Effects) -> Vec<&'static str> {
  [
    (e.io, "io"),
    (e.time, "time"),
    (e.suspends, "suspends"),
    (e.random, "random"),
    (e.unknown, "unknown"),
  ]
  .into_iter()
  .filter_map(|(on, n)| on.then_some(n))
  .collect()
}

fn json_to_toml(v: &Value) -> toml::Value {
  match v {
    Value::Bool(b) => toml::Value::Boolean(*b),
    Value::Number(n) => n
      .as_i64()
      .map(toml::Value::Integer)
      .unwrap_or_else(|| toml::Value::Float(n.as_f64().unwrap_or(0.0))),
    Value::String(s) => toml::Value::String(s.clone()),
    Value::Array(a) => toml::Value::Array(a.iter().map(json_to_toml).collect()),
    Value::Object(m) => toml::Value::Table(
      m.iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), json_to_toml(v)))
        .collect(),
    ),
    Value::Null => toml::Value::String(String::new()),
  }
}

/// The profiles a thread is tainted with (the user's `set` of `profile`).
pub fn taint_of(index: &stitch_store::Index, thread: &RecordId) -> Vec<Profile> {
  let Some(v) = index.current_user_value(thread, FIELD).ok().flatten() else {
    return Vec::new();
  };
  match &v.value {
    Value::Array(a) => a.iter().filter_map(Profile::from_record).collect(),
    _ => Vec::new(),
  }
}

/// Taints `thread` with `p` (an untrusted profile): written before the
/// payload reaches its model; the thread goes out of memory. Nothing when
/// it already has it.
pub(crate) fn taint(s: &Shared, thread: &RecordId, p: &Profile) -> Result<()> {
  if p.trusted {
    return Ok(());
  }
  let mut now = taint_of(&s.state().index.lock().expect("index lock"), thread);
  if now.contains(p) {
    return Ok(());
  }
  now.push(p.clone());
  write(s, thread, &now)?;
  // What it holds is not recalled into other threads.
  if let Ok((root, _)) = s.thread_place(thread)
    && let Err(e) = s.memory.remember(s.state(), &root, thread, false)
  {
    eprintln!("thread {thread}: not taken out of memory: {e}");
  }
  Ok(())
}

/// Writes a thread's taint as it is to stand (empty: lifted).
pub(crate) fn write(s: &Shared, thread: &RecordId, profiles: &[Profile]) -> Result<()> {
  let value = toml::Value::Array(profiles.iter().map(Profile::to_toml).collect());
  crate::meta::set(
    s,
    thread,
    FIELD,
    value,
    Author::user(s.state().node.clone()),
  )
}

/// Lifts a thread's taint (`stitch event untaint`, the user's).
pub(crate) fn untaint(s: &Shared, thread: &RecordId) -> Result<String> {
  let had = taint_of(&s.state().index.lock().expect("index lock"), thread);
  if had.is_empty() {
    return Ok(format!("Thread {thread} is not tainted."));
  }
  write(s, thread, &[])?;
  Ok(format!(
    "Lifted {} profile(s) from thread {thread}: its next turn has its own tools again. What the \
     untrusted events said is still in its history: its model reads it as before.",
    had.len()
  ))
}

/// What holds a loop's calls: the profiles of the turn (`fixed`, from the
/// schedule that started it), and its thread's taint, read at each call so
/// an untrusted ask woven into the turn holds it from its next call.
#[derive(Clone, Default)]
pub struct Ceiling {
  fixed: Vec<Profile>,
  thread: Option<(Weak<Shared>, RecordId)>,
}

impl std::fmt::Debug for Ceiling {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Ceiling({:?}, {:?})",
      self.fixed,
      self.thread.as_ref().map(|t| &t.1)
    )
  }
}

impl Ceiling {
  pub(crate) fn new(fixed: Vec<Profile>, s: &Arc<Shared>, thread: Option<&RecordId>) -> Ceiling {
    Ceiling {
      fixed,
      thread: thread.map(|t| (Arc::downgrade(s), t.clone())),
    }
  }

  /// Every profile holding it now.
  pub fn profiles(&self) -> Vec<Profile> {
    let mut out = self.fixed.clone();
    if let Some((s, t)) = &self.thread
      && let Some(s) = s.upgrade()
    {
      for p in taint_of(&s.state().index.lock().expect("index lock"), t) {
        if !out.contains(&p) {
          out.push(p);
        }
      }
    }
    out
  }

  /// Why tool `name` may not run now, if it may not.
  pub fn refuses(&self, name: &str) -> Option<String> {
    let profiles = self.profiles();
    let p = profiles.iter().find(|p| !p.allows_tool(name))?;
    Some(refusal(name, p))
  }

  /// The shard rule, when a profile holds the turn.
  pub fn shards(&self) -> Option<ShardRule> {
    let p = self.profiles();
    (!p.is_empty()).then_some(ShardRule(p))
  }
}

/// What a refused call says.
pub fn refusal(name: &str, p: &Profile) -> String {
  format!(
    "{name} is not allowed in this turn: it runs under the capability profile of {} ({}). The \
     node enforces it; nothing said in the conversation changes it.{}",
    p.from
      .as_deref()
      .map_or_else(|| "a schedule".to_owned(), |f| format!("subscription {f}")),
    p.said(),
    if p.trusted {
      ""
    } else {
      " Only the user lifts it (`stitch event untaint <thread>`)."
    }
  )
}

/// Which shards Otter code may use in a turn: what every profile allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardRule(pub Vec<Profile>);

impl ShardRule {
  /// Why shard `ty` is denied, if it is.
  pub fn denies(&self, ty: &ShardType) -> Option<String> {
    self
      .0
      .iter()
      .find_map(|p| p.allows_shard(ty).err().map(|why| (p, why)))
      .map(|(p, why)| {
        format!(
          "`{}` is denied in this turn by the capability profile of {}: {why}",
          ty.name(),
          p.from
            .as_deref()
            .map_or_else(|| "a schedule".to_owned(), |f| format!("subscription {f}"))
        )
      })
  }

  /// `base` without the shards it denies.
  pub fn catalog(
    &self,
    base: &otter_core::Catalog,
  ) -> std::result::Result<otter_core::Catalog, String> {
    let kept: Vec<&'static ShardType> = base
      .shards()
      .iter()
      .copied()
      .filter(|t| self.denies(t).is_none())
      .collect();
    let catalog = otter_core::Catalog::new(&[&kept])?;
    // The object types scripts name in signatures stay.
    let objects: Vec<&'static otter_core::object::ObjectType> = base
      .objects()
      .iter()
      .copied()
      .filter(|o| catalog.object_type(o.name).is_none())
      .collect();
    if objects.is_empty() {
      Ok(catalog)
    } else {
      catalog.with_objects(&[&objects])
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn p(v: Value) -> Result<Profile> {
    Profile::from_input(&v)
  }

  #[test]
  fn untrusted_profiles_never_reach_past_the_turn() {
    for bad in [
      "shell",
      "shell_run",
      "spawn",
      "send",
      "weave",
      "*",
      "sub*",
      "browser",
    ] {
      assert!(
        p(serde_json::json!({"tools": [bad]})).is_err(),
        "{bad} was let into an untrusted profile"
      );
    }
    let ok = p(serde_json::json!({"tools": ["notify", "viewer", "support_*"]})).unwrap();
    assert!(
      ok.allows_tool("event_reply"),
      "the ask's answer is always there"
    );
    assert!(ok.allows_tool("notify"));
    assert!(ok.allows_tool("view_file"));
    assert!(ok.allows_tool("support_lookup"));
    assert!(!ok.allows_tool("shell_run"));
    assert!(!ok.allows_tool("browse_web"));
    assert!(!ok.allows_tool("recall"));
    assert!(!ok.allows_tool("write_file"));
    // A trusted one may name anything.
    let t = p(serde_json::json!({"tools": ["shell", "spawn"], "trusted": true})).unwrap();
    assert!(t.allows_tool("shell_run") && t.allows_tool("spawn"));
    assert!(
      !t.allows_tool("send"),
      "spawn names the tool, not the weave group"
    );
    assert!(p(serde_json::json!({"effects": ["io"], "trusted": true})).is_err());
    assert!(p(serde_json::json!({"effects": ["network"]})).is_err());
    assert!(p(serde_json::json!({"shards": ["+a b"]})).is_err());
    assert!(
      p(serde_json::json!({"tool": ["x"]})).is_err(),
      "unknown keys refused"
    );
  }

  #[test]
  fn shards_by_effect_for_untrusted_and_by_entry_for_trusted() {
    let catalog = crate::otter::host_catalog().unwrap();
    let get = |n: &str| catalog.get(n).unwrap_or_else(|| panic!("no shard {n}"));
    let untrusted = p(serde_json::json!({"shards": ["+Http.Get"]})).unwrap();
    assert!(untrusted.allows_shard(get("Math.Add")).is_ok(), "pure");
    assert!(untrusted.allows_shard(get("Http.Get")).is_ok(), "named");
    assert!(untrusted.allows_shard(get("FS.Write")).is_err(), "io");
    assert!(
      untrusted.allows_shard(get("Tool.Call")).is_ok(),
      "a gateway"
    );
    let timed = p(serde_json::json!({"effects": ["time", "suspends"]})).unwrap();
    assert!(timed.allows_shard(get("Pause")).is_ok());
    let trusted = p(serde_json::json!({"shards": ["-FS.*"], "trusted": true})).unwrap();
    assert!(trusted.allows_shard(get("FS.Write")).is_err());
    assert!(trusted.allows_shard(get("Http.Get")).is_ok());
    let rule = ShardRule(vec![untrusted, trusted]);
    let restricted = rule.catalog(catalog).unwrap();
    assert!(restricted.get("FS.Write").is_none());
    assert!(restricted.get("FS.Read").is_none());
    assert!(restricted.get("Http.Get").is_some());
    assert!(restricted.get("Math.Add").is_some());
  }
}
