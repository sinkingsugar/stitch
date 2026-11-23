//! Harness issues (docs/self-improve.md §2.2, §4, 3): a bug in Stitch
//! itself that a thread ran into, filed with `report_issue` (or `stitch
//! issue report`) and left for the fixer (`self_improve`, the
//! harness-issues schedule) rather than worked around and forgotten.
//!
//! An `issue` record in the home workspace (it syncs to every node):
//! `title`, `area`, the `node` and `build` it was seen on, the place it
//! was seen in, a `thread` reference to the thread that filed it (whose
//! transcript the fixer reads), and the report as its body (what
//! happened, what was expected, the evidence). Its state is `set`s on it:
//! `state` (`open`, `fixing`, `fixed`, `dropped`; none is `open`), `fixer`
//! (the thread fixing it), `fix` (the commit), `note`. Records only: no
//! GitHub issues (§4, 3).

use std::collections::BTreeSet;

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use stitch_records::{Author, Draft, RecordId, Ref};

use crate::node::Shared;
use crate::{Error, Result};

/// The record kind.
pub(crate) const KIND: &str = "issue";

/// At most this many issues in an hour, counted from the records (so a
/// restart forgets nothing): from one thread, from one node, from all
/// nodes. A loop gone wrong files a few, not hundreds.
const PER_THREAD: usize = 5;
const PER_NODE: usize = 10;
const PER_HOUR: usize = 30;
const MAX_TITLE: usize = 120;
const MAX_TEXT: usize = 8000;

/// The states an issue goes through.
pub(crate) const STATES: [&str; 4] = ["open", "fixing", "fixed", "dropped"];

/// What `report_issue` takes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
  pub title: String,
  pub what_happened: String,
  pub expected: String,
  #[serde(default)]
  pub evidence: Option<String>,
  #[serde(default)]
  pub area: Option<String>,
}

/// An issue as it stands.
#[derive(Clone, Debug)]
pub(crate) struct Issue {
  pub id: RecordId,
  pub created: Timestamp,
  pub title: String,
  pub area: Option<String>,
  pub node: String,
  pub build: Option<String>,
  pub thread: Option<String>,
  pub body: String,
  pub state: String,
  pub fixer: Option<String>,
  pub fix: Option<String>,
  pub note: Option<String>,
  /// When it was last set `fixed`.
  pub fixed_at: Option<Timestamp>,
  /// The repository it is fixed in (`[self_improve] places`): set when
  /// filed, by the triage or a fixer (`stitch issue set --repo`), or when
  /// its fixer starts. None: the core (`stitch`), as for every issue
  /// before there were two.
  pub repo: Option<String>,
  /// What the node landed for it, one `<repo> <from> <to>` a line, oldest
  /// first (`selfimprove::land`); none since its last revert.
  pub landed: Option<String>,
  /// When `landed` was last written.
  pub landed_at: Option<Timestamp>,
  /// The landing before there were two repositories: `<from> <to>` on the
  /// core.
  pub landing: Option<String>,
}

/// One landing the node made for an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Landing {
  pub repo: String,
  pub from: String,
  pub to: String,
}

impl Issue {
  /// Open or being fixed.
  pub(crate) fn is_live(&self) -> bool {
    matches!(self.state.as_str(), "open" | "fixing")
  }

  /// The repository it is fixed in: its own, else `default` (the core).
  pub(crate) fn repo_or<'a>(&'a self, default: &'a str) -> &'a str {
    self.repo.as_deref().unwrap_or(default)
  }

  /// What the node landed for it, oldest first (`default`: the core's name,
  /// for a landing from before there were two repositories).
  pub(crate) fn landings(&self, default: &str) -> Vec<Landing> {
    if let Some(l) = &self.landed {
      return l
        .lines()
        .filter_map(|l| {
          let mut w = l.split_whitespace();
          let (repo, from, to) = (w.next()?, w.next()?, w.next()?);
          Some(Landing {
            repo: repo.into(),
            from: from.into(),
            to: to.into(),
          })
        })
        .collect();
    }
    self
      .landing
      .as_deref()
      .and_then(|l| l.split_once(' '))
      .map(|(from, to)| Landing {
        repo: default.into(),
        from: from.trim().into(),
        to: to.trim().into(),
      })
      .into_iter()
      .collect()
  }

  /// One line, for lists.
  pub(crate) fn line(&self) -> String {
    let mut l = format!("{} [{}] {}", self.id, self.state, self.title);
    if let Some(a) = &self.area {
      l.push_str(&format!(" ({a})"));
    }
    if let Some(r) = &self.repo {
      l.push_str(&format!(" [repo {r}]"));
    }
    if let Some(f) = &self.fixer {
      l.push_str(&format!("; fixer {f}"));
    }
    if let Some(f) = &self.fix {
      l.push_str(&format!("; fixed in {f}"));
    }
    l
  }

  /// All of it, for a fixer's goal or `stitch issue show`.
  pub(crate) fn text(&self) -> String {
    let mut out = format!(
      "Issue {} [{}]: {}\nSeen on {} ({}), {}",
      self.id,
      self.state,
      self.title,
      self.node,
      self.build.as_deref().unwrap_or("build unknown"),
      self.created
    );
    if let Some(a) = &self.area {
      out.push_str(&format!(", area {a}"));
    }
    if let Some(t) = &self.thread {
      out.push_str(&format!(
        "\nFiled by thread {t} (its transcript: `stitch thread {t}`)"
      ));
    }
    for (k, v) in [
      ("repo", &self.repo),
      ("fixer", &self.fixer),
      ("fix", &self.fix),
      ("note", &self.note),
    ] {
      if let Some(v) = v {
        out.push_str(&format!("\n{k}: {v}"));
      }
    }
    if let Some(l) = &self.landed {
      for l in l.lines() {
        out.push_str(&format!("\nlanded: {l}"));
      }
    }
    out.push_str("\n\n");
    out.push_str(self.body.trim());
    out
  }
}

fn cut(s: &str, max: usize) -> String {
  let s = s.trim();
  if s.chars().count() <= max {
    return s.to_owned();
  }
  let mut out: String = s.chars().take(max - 1).collect();
  out.push('…');
  out
}

/// Words that say nothing of which bug it is.
const COMMON: &[&str] = &[
  "a", "an", "the", "is", "are", "was", "be", "it", "its", "of", "to", "in", "on", "for", "and",
  "or", "not", "no", "with", "when", "from", "by", "at", "as", "that", "this", "does", "doesn",
  "do", "t", "s",
];

/// The words of a title that tell a bug, for telling a duplicate.
fn words(s: &str) -> BTreeSet<String> {
  s.to_lowercase()
    .split(|c: char| !c.is_alphanumeric())
    .filter(|w| !w.is_empty() && !COMMON.contains(w))
    .map(str::to_owned)
    .collect()
}

/// Whether two titles say the same bug: most of their words shared
/// (Jaccard 0.6 or more), or one's words all in the other's (3 or more).
pub(crate) fn same_bug(a: &str, b: &str) -> bool {
  let (a, b) = (words(a), words(b));
  if a.is_empty() || b.is_empty() {
    return false;
  }
  let shared = a.intersection(&b).count();
  let union = a.union(&b).count();
  let smaller = a.len().min(b.len());
  shared * 10 >= union * 6 || (smaller >= 3 && shared == smaller)
}

/// Whether one more issue may be filed now (by `thread` on `node`), from
/// the records of the last hour.
fn admit(issues: &[Issue], node: &str, thread: Option<&RecordId>, now: Timestamp) -> Result<()> {
  let hour = now - jiff::SignedDuration::from_secs(3600);
  let recent: Vec<&Issue> = issues.iter().filter(|i| i.created > hour).collect();
  let by_thread = thread.map_or(0, |t| {
    recent
      .iter()
      .filter(|i| i.thread.as_deref() == Some(t.as_str()))
      .count()
  });
  let by_node = recent.iter().filter(|i| i.node == node).count();
  let why = if by_thread >= PER_THREAD {
    format!("this thread filed {PER_THREAD} issues in the last hour")
  } else if by_node >= PER_NODE {
    format!("this node filed {PER_NODE} issues in the last hour")
  } else if recent.len() >= PER_HOUR {
    format!("{PER_HOUR} issues were filed in the last hour")
  } else {
    return Ok(());
  };
  Err(Error::Config(format!(
    "not filed: {why}, the most there may be. Put what is left into your report, and go on \
     with the task."
  )))
}

fn str_field(r: &stitch_store::IndexedRecord, k: &str) -> Option<String> {
  r.fields
    .get(k)
    .and_then(Value::as_str)
    .map(str::to_owned)
    .filter(|s| !s.is_empty())
}

/// Every issue here, oldest first.
pub(crate) fn all(s: &Shared) -> Result<Vec<Issue>> {
  let state = s.state();
  let index = state.index.lock().expect("index lock");
  let mut out = Vec::new();
  for r in index.records(Some(KIND))? {
    let current = |field: &str| {
      index
        .current_value(&r.id, field)
        .ok()
        .flatten()
        .and_then(|v| v.value.as_str().map(str::to_owned))
        .filter(|v| !v.is_empty())
    };
    let thread = index
      .edges_from(&r.id)?
      .into_iter()
      .find(|e| e.rel == "thread")
      .map(|e| e.dst.id.to_string());
    let body = state
      .root(&r.repo)
      .and_then(|root| root.read_record(&r.id).ok())
      .map(|rec| rec.body)
      .unwrap_or_default();
    out.push(Issue {
      title: str_field(&r, "title").unwrap_or_else(|| r.title.clone().unwrap_or_default()),
      area: str_field(&r, "area"),
      node: r.node.clone(),
      build: str_field(&r, "build"),
      state: current("state").unwrap_or_else(|| "open".into()),
      fixer: current("fixer"),
      fix: current("fix"),
      note: current("note"),
      fixed_at: index
        .current_value(&r.id, "state")
        .ok()
        .flatten()
        .filter(|v| v.value.as_str() == Some("fixed"))
        .and_then(|v| index.record(&v.record).ok().flatten())
        .map(|set| set.created),
      repo: current("repo").or_else(|| str_field(&r, "repo")),
      landed: current("landed"),
      landed_at: index
        .current_value(&r.id, "landed")
        .ok()
        .flatten()
        .and_then(|v| index.record(&v.record).ok().flatten())
        .map(|set| set.created),
      landing: current("landing"),
      id: r.id.clone(),
      created: r.created,
      thread,
      body,
    });
  }
  Ok(out)
}

/// The issue an ID (or the end of one, as lists show them) names.
pub(crate) fn find(s: &Shared, id: &str) -> Result<Issue> {
  let id = id.trim();
  let found: Vec<Issue> = all(s)?
    .into_iter()
    .filter(|i| i.id.as_str() == id || (id.len() >= 4 && i.id.as_str().ends_with(id)))
    .collect();
  match found.len() {
    1 => Ok(found.into_iter().next().expect("one")),
    0 => Err(Error::Config(format!("no issue {id} here"))),
    _ => Err(Error::Config(format!(
      "{id} names {} issues: give the whole ID",
      found.len()
    ))),
  }
}

/// Files an issue (from `thread`, when a thread files it), unless a live
/// one has the same title: then that one's ID. Its ID, and whether it is
/// new.
pub(crate) fn report(
  s: &Shared,
  thread: Option<&RecordId>,
  r: Report,
  author: Author,
) -> Result<(RecordId, bool)> {
  let title = cut(&r.title.replace('\n', " "), MAX_TITLE);
  if title.is_empty() || r.what_happened.trim().is_empty() {
    return Err(Error::Config(
      "an issue needs a title and what happened".into(),
    ));
  }
  let issues = all(s)?;
  if let Some(same) = issues
    .iter()
    .find(|i| i.is_live() && same_bug(&i.title, &title))
  {
    return Ok((same.id.clone(), false));
  }
  let state = s.state();
  admit(&issues, state.node.as_str(), thread, Timestamp::now())?;
  let home = state
    .home()
    .ok_or_else(|| Error::Config("no home workspace here".into()))?;
  let mut body = format!(
    "# {title}\n\n## What happened\n\n{}\n\n## Expected\n\n{}\n",
    cut(&r.what_happened, MAX_TEXT),
    cut(&r.expected, MAX_TEXT)
  );
  if let Some(e) = r.evidence.as_deref().filter(|e| !e.trim().is_empty()) {
    body.push_str(&format!("\n## Evidence\n\n{}\n", cut(e, MAX_TEXT)));
  }
  let mut draft = Draft::new(KIND, author)?
    .body(body)
    .field("title", title.as_str())?
    .field("build", crate::build())?
    .field("version", crate::VERSION)?;
  if let Some(a) = r.area.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
    draft = draft.field("area", a)?;
  }
  if let Some(t) = thread {
    draft = draft.reference("thread", Ref::to(t.clone()))?;
    if let Ok((_, place)) = s.thread_place(t) {
      draft = draft.field("place", place.key())?;
    }
  }
  let rec = home.publish(&state.node, draft)?;
  state.index.lock().expect("index lock").refresh(&home)?;
  Ok((rec.id, true))
}

/// Sets one of an issue's values (`state`, `fixer`, `fix`, `note`).
pub(crate) fn set(
  s: &Shared,
  id: &RecordId,
  field: &str,
  value: &str,
  author: Author,
) -> Result<()> {
  if field == "state" && !STATES.contains(&value) {
    return Err(Error::Config(format!(
      "an issue's state is one of {}",
      STATES.join(", ")
    )));
  }
  crate::meta::set(s, id, field, value.into(), author)
}

/// The `report_issue` tool.
pub(crate) async fn tool(s: &Shared, thread: &RecordId, input: Value) -> Result<String> {
  let r: Report = serde_json::from_value(input).map_err(|e| Error::Config(e.to_string()))?;
  let node = s.state().node.clone();
  let (id, new) = report(s, Some(thread), r, Author::node(node))?;
  Ok(if new {
    format!(
      "Filed issue {id}. Go on with your task (work around it if you must); the fixer \
       takes it from here."
    )
  } else {
    format!("Already filed as issue {id} (open): nothing added. Go on with your task.")
  })
}

pub(crate) fn definition() -> ToolDef {
  ToolDef {
    name: "report_issue".into(),
    description: "File a bug in Stitch itself (the harness running you: its tools, shells, \
      threads, sync, the app), not in the user's project: a tool that failed or answered \
      wrongly, an error that should not happen, a limit that got in the way. It is kept as an \
      issue record for Stitch's fixer, who reads this thread's transcript. File it and carry on \
      with the user's task (work around it if you must): do not try to fix Stitch in the middle \
      of it. Cheap, never asks; a few per run at most."
      .into(),
    input_schema: json!({
      "type": "object", "additionalProperties": false,
      "properties": {
        "title": {"type": "string", "description": "What is wrong, in a line."},
        "what_happened": {"type": "string", "description": "What you did and what Stitch did: the call, its input, what came back."},
        "expected": {"type": "string", "description": "What should have happened."},
        "evidence": {"type": "string", "description": "Error text, log lines, the command: exact, short."},
        "area": {"type": "string", "description": "The part of Stitch, if you know it (shell, web, weave, schedules, sync, app…)."}
      },
      "required": ["title", "what_happened", "expected"]
    }),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn issue(node: &str, thread: Option<&str>, ago_secs: i64) -> Issue {
    Issue {
      id: "2026-10-28T120000Z-mbp-abcd".parse().unwrap(),
      created: Timestamp::now() - jiff::SignedDuration::from_secs(ago_secs),
      title: "t".into(),
      area: None,
      node: node.into(),
      build: None,
      thread: thread.map(str::to_owned),
      body: String::new(),
      state: "open".into(),
      fixer: None,
      fix: None,
      note: None,
      fixed_at: None,
      repo: None,
      landed: None,
      landed_at: None,
      landing: None,
    }
  }

  #[test]
  fn filing_is_limited_by_thread_node_and_all_from_the_records() {
    let t: RecordId = "2026-10-28T110000Z-mbp-wxyz".parse().unwrap();
    let now = Timestamp::now();
    let mut v: Vec<Issue> = (0..PER_THREAD)
      .map(|_| issue("mbp", Some(t.as_str()), 60))
      .collect();
    assert!(admit(&v, "mbp", Some(&t), now).is_err());
    // An hour on, they no longer count.
    v.iter_mut()
      .for_each(|i| i.created = now - jiff::SignedDuration::from_secs(3700));
    admit(&v, "mbp", Some(&t), now).unwrap();
    // The node's, without a thread (`stitch issue report`).
    let v: Vec<Issue> = (0..PER_NODE).map(|_| issue("mbp", None, 60)).collect();
    assert!(admit(&v, "mbp", None, now).is_err());
    admit(&v, "geek", None, now).unwrap();
    let v: Vec<Issue> = (0..PER_HOUR).map(|_| issue("x", None, 60)).collect();
    assert!(admit(&v, "geek", None, now).is_err());
  }

  #[test]
  fn titles_compare_by_their_words() {
    assert!(same_bug(
      "shell_run: exit code lost!",
      "Shell_run — exit code lost"
    ));
    assert!(same_bug(
      "shell_run loses the exit code",
      "the exit code is lost by shell_run sometimes (loses exit code)"
    ));
    assert!(same_bug(
      "web_eval drops console lines",
      "web_eval drops the last console lines"
    ));
    assert!(!same_bug(
      "web_eval drops console lines",
      "shell_run loses the exit code"
    ));
    assert!(!same_bug("a b", "a c"));
  }
}
