//! The artefact index's extraction (design §2.25, step 2): what a tool call
//! touched (a path, a URL, a commit, a thread), read from the call and its
//! result by rule, never by a model. Pure: `(tool name, input, result)` in,
//! rows out, so every node derives the same ones from the same segment.
//!
//! Nothing node-local enters a row: a path is kept as the call gave it
//! (not made absolute, not `~`-folded: the records do not say, for a
//! relative path, which directory it was relative to, the same on every
//! node, and `$HOME` is the node's).

use std::collections::{BTreeMap, HashMap};

use jiff::Timestamp;
use serde_json::Value;
use stitch_records::{Event, RecordId, Segment};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
  Path,
  Url,
  Commit,
  Thread,
}

impl Kind {
  pub fn as_str(self) -> &'static str {
    match self {
      Kind::Path => "path",
      Kind::Url => "url",
      Kind::Commit => "commit",
      Kind::Thread => "thread",
    }
  }

  pub fn parse(s: &str) -> Option<Kind> {
    Some(match s {
      "path" => Kind::Path,
      "url" => Kind::Url,
      "commit" => Kind::Commit,
      "thread" => Kind::Thread,
      _ => return None,
    })
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verb {
  Read,
  Wrote,
  Spawned,
  Told,
}

impl Verb {
  pub fn as_str(self) -> &'static str {
    match self {
      Verb::Read => "read",
      Verb::Wrote => "wrote",
      Verb::Spawned => "spawned",
      Verb::Told => "told",
    }
  }
}

/// One thing a call touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artefact {
  pub kind: Kind,
  pub value: String,
  pub verb: Verb,
  /// A commit's subject line.
  pub subject: Option<String>,
}

fn art(kind: Kind, value: impl Into<String>, verb: Verb) -> Artefact {
  Artefact {
    kind,
    value: value.into(),
    verb,
    subject: None,
  }
}

/// The text of a tool result's output (`{"text": …}` or a bare string);
/// empty for a blob reference or anything else.
fn text_of(output: &Value) -> &str {
  output
    .as_str()
    .or_else(|| output.get("text").and_then(Value::as_str))
    .unwrap_or("")
}

fn path_of(input: &Value) -> Option<String> {
  let p = input.get("path")?.as_str()?.trim();
  (!p.is_empty()).then(|| p.to_owned())
}

/// An `http(s)` URL without its fragment; anything else (`back`, `reload`,
/// `about:blank`) is not one.
pub fn clean_url(u: &str) -> Option<String> {
  let u = u.trim();
  let u = u.split('#').next().unwrap_or(u);
  let lower = u.get(..8).unwrap_or(u).to_ascii_lowercase();
  let schemed = lower.starts_with("http://") || lower.starts_with("https://");
  (schemed && u.len() > lower.find("//").unwrap_or(0) + 2).then(|| u.to_owned())
}

fn thread_id(s: &str) -> Option<String> {
  let s = s
    .trim()
    .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
  s.parse::<RecordId>().ok().map(|id| id.to_string())
}

/// Git's commit line, `[branch abc1234] subject`, also
/// `[branch (root-commit) abc1234] subject` and `[detached HEAD abc1234]
/// subject`: the commit's abbreviated hash and subject.
fn commit_line(line: &str) -> Option<(String, String)> {
  let line = line.trim_start().strip_prefix('[')?;
  let close = line.find(']')?;
  let (inside, rest) = (&line[..close], &line[close + 1..]);
  let subject = rest.strip_prefix(' ')?.trim();
  if subject.is_empty() {
    return None;
  }
  let toks: Vec<&str> = inside.split(' ').collect();
  let (hash, head) = toks.split_last()?;
  if !(7..=40).contains(&hash.len()) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  let head = match head {
    [h @ .., last] if last.starts_with('(') && last.ends_with(')') => h,
    h => h,
  };
  let branch_ok = match head {
    [b] => !b.is_empty(),
    ["detached", "HEAD"] => true,
    _ => false,
  };
  branch_ok.then(|| {
    (
      hash.to_ascii_lowercase(),
      subject.chars().take(200).collect(),
    )
  })
}

/// Tools whose result can hold the output of a `git commit` run in a shell.
const SHELL_OUTPUT: [&str; 4] = ["shell_run", "shell_send", "shell_wait", "job_output"];

/// What one tool call touched. `result` is `(ok, output)` when the call has
/// its result; a failed call touched nothing, and what needs the result
/// (a commit, a spawned thread's ID) needs it.
pub fn extract(name: &str, input: &Value, result: Option<(bool, &Value)>) -> Vec<Artefact> {
  if result.is_some_and(|(ok, _)| !ok) {
    return Vec::new();
  }
  let mut out = Vec::new();
  match name {
    "edit_file" | "write_file" => {
      // A write counts once it is known to have happened.
      if result.is_some()
        && let Some(p) = path_of(input)
      {
        out.push(art(Kind::Path, p, Verb::Wrote));
      }
    }
    "view_file" => out.extend(path_of(input).map(|p| art(Kind::Path, p, Verb::Read))),
    "browse_web" | "web_eval" | "browser_navigate" => out.extend(
      input
        .get("url")
        .and_then(Value::as_str)
        .and_then(clean_url)
        .map(|u| art(Kind::Url, u, Verb::Read)),
    ),
    "browser_tabs" => {
      if input.get("action").and_then(Value::as_str) == Some("new") {
        out.extend(
          input
            .get("url")
            .and_then(Value::as_str)
            .and_then(clean_url)
            .map(|u| art(Kind::Url, u, Verb::Read)),
        );
      }
    }
    "spawn" => {
      if let Some((_, output)) = result
        && let Some(rest) = text_of(output).split("Spawned thread ").nth(1)
        && let Some(id) = rest.split_whitespace().next().and_then(thread_id)
      {
        out.push(art(Kind::Thread, id, Verb::Spawned));
      }
    }
    "send" => out.extend(
      input
        .get("thread")
        .and_then(Value::as_str)
        .and_then(thread_id)
        .map(|t| art(Kind::Thread, t, Verb::Told)),
    ),
    n if SHELL_OUTPUT.contains(&n) => {
      if let Some((_, output)) = result {
        for line in text_of(output).lines() {
          if let Some((hash, subject)) = commit_line(line) {
            out.push(Artefact {
              kind: Kind::Commit,
              value: hash,
              verb: Verb::Wrote,
              subject: Some(subject),
            });
          }
        }
      }
    }
    _ => {}
  }
  out
}

/// One row of a segment's artefacts: what was touched in it, once per
/// `(kind, value, verb)`, with when it was first and last, how many calls
/// touched it, and the messages (local IDs, `m12`: the latest `msg` event
/// before the call) of the first and the last. A commit keeps the subject
/// of its last occurrence.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
  pub kind: Kind,
  pub value: String,
  pub verb: Verb,
  pub first_at: Timestamp,
  pub last_at: Timestamp,
  pub n: u32,
  pub first_msg: Option<String>,
  pub last_msg: Option<String>,
  pub subject: Option<String>,
}

/// A segment's artefact rows, in a fixed order (by kind, value, verb): the
/// same for the same segment on every node. Calls are paired with their
/// results by ID (a step is written whole, so both are in one segment); a
/// call with none is extracted without one. The instant is the call's.
pub fn segment_rows(s: &Segment) -> Vec<Row> {
  struct Call<'a> {
    name: &'a str,
    input: &'a Value,
    at: Timestamp,
    msg: Option<&'a str>,
  }
  let mut open: HashMap<&str, Call> = HashMap::new();
  let mut order: Vec<&str> = Vec::new();
  let mut rows: BTreeMap<(Kind, String, Verb), Row> = BTreeMap::new();
  let mut take = |c: &Call, result: Option<(bool, &Value)>| {
    for a in extract(c.name, c.input, result) {
      let key = (a.kind, a.value.clone(), a.verb);
      let msg = c.msg.map(str::to_owned);
      let row = rows.entry(key).or_insert_with(|| Row {
        kind: a.kind,
        value: a.value.clone(),
        verb: a.verb,
        first_at: c.at,
        last_at: c.at,
        n: 0,
        first_msg: msg.clone(),
        last_msg: msg.clone(),
        subject: None,
      });
      row.n += 1;
      if c.at >= row.last_at {
        row.last_at = c.at;
        row.last_msg = msg;
        if a.subject.is_some() {
          row.subject = a.subject;
        }
      }
      if c.at < row.first_at {
        row.first_at = c.at;
        row.first_msg = c.msg.map(str::to_owned);
      }
    }
  };
  let mut msg: Option<&str> = None;
  for e in &s.events {
    match e {
      Event::Msg(m) => msg = Some(m.id.as_str()),
      Event::ToolCall(c) => {
        order.push(c.id.as_str());
        open.insert(
          c.id.as_str(),
          Call {
            name: &c.name,
            input: &c.input,
            at: c.at,
            msg,
          },
        );
      }
      Event::ToolResult(r) => {
        if let Some(c) = open.remove(r.id.as_str()) {
          take(&c, Some((r.ok, &r.output)));
        }
      }
      _ => {}
    }
  }
  for id in order {
    if let Some(c) = open.remove(id) {
      take(&c, None);
    }
  }
  rows.into_values().collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn run(name: &str, input: Value, ok: bool, text: &str) -> Vec<Artefact> {
    extract(name, &input, Some((ok, &json!({ "text": text }))))
  }

  fn one(a: &[Artefact]) -> (Kind, &str, Verb) {
    assert_eq!(a.len(), 1, "{a:?}");
    (a[0].kind, a[0].value.as_str(), a[0].verb)
  }

  #[test]
  fn files_are_read_or_written_as_the_call_gave_them() {
    let a = run(
      "edit_file",
      json!({"path": "src/a.rs", "old_string": "x"}),
      true,
      "ok",
    );
    assert_eq!(one(&a), (Kind::Path, "src/a.rs", Verb::Wrote));
    let a = run("write_file", json!({"path": " /home/u/x.md "}), true, "ok");
    assert_eq!(one(&a), (Kind::Path, "/home/u/x.md", Verb::Wrote));
    let a = run(
      "view_file",
      json!({"path": "~/docs/y.md", "lines": 5}),
      true,
      "…",
    );
    assert_eq!(one(&a), (Kind::Path, "~/docs/y.md", Verb::Read));
    assert!(run("view_file", json!({"path": ""}), true, "").is_empty());
    assert!(run("view_file", json!({}), true, "").is_empty());
  }

  #[test]
  fn a_failed_call_touched_nothing() {
    assert!(run("edit_file", json!({"path": "a"}), false, "no match").is_empty());
    assert!(run("write_file", json!({"path": "a"}), false, "denied").is_empty());
    assert!(run("view_file", json!({"path": "a"}), false, "no such file").is_empty());
    assert!(run("browse_web", json!({"url": "https://a.b/"}), false, "").is_empty());
    assert!(
      run(
        "send",
        json!({"thread": "2026-11-11T090307Z-gamer-x789"}),
        false,
        ""
      )
      .is_empty()
    );
    // A write with no result at all is not known to have happened; a read is.
    assert!(extract("edit_file", &json!({"path": "a"}), None).is_empty());
    assert_eq!(extract("view_file", &json!({"path": "a"}), None).len(), 1);
  }

  #[test]
  fn urls_lose_their_fragment_and_only_http_counts() {
    for tool in ["browse_web", "web_eval", "browser_navigate"] {
      let a = run(
        tool,
        json!({"url": "https://x.org/a?b=1#sec-2", "code": "1"}),
        true,
        "",
      );
      assert_eq!(
        one(&a),
        (Kind::Url, "https://x.org/a?b=1", Verb::Read),
        "{tool}"
      );
    }
    assert!(run("browser_navigate", json!({"url": "back"}), true, "").is_empty());
    assert!(run("browser_navigate", json!({"url": "reload"}), true, "").is_empty());
    assert!(run("browse_web", json!({"url": "file:///etc/passwd"}), true, "").is_empty());
    assert!(run("browse_web", json!({"url": "https://"}), true, "").is_empty());
    let a = run(
      "browser_tabs",
      json!({"action": "new", "url": "HTTP://Localhost:3000/#x"}),
      true,
      "",
    );
    assert_eq!(one(&a), (Kind::Url, "HTTP://Localhost:3000/", Verb::Read));
    assert!(run("browser_tabs", json!({"action": "list"}), true, "").is_empty());
    assert!(
      run(
        "browser_tabs",
        json!({"action": "select", "tab": 3}),
        true,
        ""
      )
      .is_empty()
    );
  }

  #[test]
  fn commits_come_from_gits_line_in_a_results_text() {
    let out = "\
exit 0

Running pre-commit hooks…
[feature/otter-integration 8841b08da5] ridgewood: record Promenade vacancy
 3 files changed, 40 insertions(+), 2 deletions(-)
 create mode 100644 docs/x.md
[main (root-commit) A1B2C3D] Initial commit
[detached HEAD 0123abc] cherry-picked one
 Date: Wed Nov 11 10:00:00 2026
$ ";
    let a = run(
      "shell_run",
      json!({"command": "git commit -am x"}),
      true,
      out,
    );
    let got: Vec<_> = a
      .iter()
      .map(|x| {
        (
          x.kind,
          x.value.as_str(),
          x.verb,
          x.subject.as_deref().unwrap(),
        )
      })
      .collect();
    assert_eq!(
      got,
      vec![
        (
          Kind::Commit,
          "8841b08da5",
          Verb::Wrote,
          "ridgewood: record Promenade vacancy"
        ),
        (Kind::Commit, "a1b2c3d", Verb::Wrote, "Initial commit"),
        (Kind::Commit, "0123abc", Verb::Wrote, "cherry-picked one"),
      ]
    );
    // The same output through the other shell tools; a bare-string output.
    assert_eq!(run("shell_send", json!({}), true, out).len(), 3);
    assert_eq!(
      extract(
        "job_output",
        &json!({}),
        Some((true, &json!("[m abcdef1] s")))
      )
      .len(),
      1
    );
  }

  #[test]
  fn things_that_look_like_a_commit_line_but_are_not() {
    for l in [
      "[ok] done",
      "[main abc12] too short a hash",
      "[main abcdefg] not hex",
      "[main abc1234]",
      "[main abc1234]subject glued",
      "[a b c abc1234] too many words",
      "[INFO 2026-11-11] 10:00 log",
      "see [main abc1234] mid-line",
      "commit abc1234567",
      "[ 1234567] empty branch",
    ] {
      assert!(
        run("shell_run", json!({}), true, l).is_empty(),
        "matched: {l}"
      );
    }
    // A failed shell call, and a blob reference, give nothing.
    assert!(run("shell_run", json!({}), false, "[main abc1234] s").is_empty());
    let blob = json!({"blob": "blake3:00"});
    assert!(extract("shell_run", &json!({}), Some((true, &blob))).is_empty());
    // Other tools' results are not scanned.
    assert!(
      run("view_file", json!({"path": "x"}), true, "[main abc1234] s")
        .iter()
        .all(|a| a.kind != Kind::Commit)
    );
  }

  #[test]
  fn spawn_and_send_name_threads_and_wait_names_none() {
    let a = run(
      "spawn",
      json!({"goal": "do it"}),
      true,
      "Spawned thread 2026-10-12T123254Z-mbp-yhj2 on mbp. Its report comes back here.",
    );
    assert_eq!(
      one(&a),
      (Kind::Thread, "2026-10-12T123254Z-mbp-yhj2", Verb::Spawned)
    );
    // The ID at the end of the sentence, with a full stop.
    let a = run(
      "spawn",
      json!({}),
      true,
      "Spawned thread 2026-10-12T123254Z-mbp-yhj2.",
    );
    assert_eq!(
      one(&a),
      (Kind::Thread, "2026-10-12T123254Z-mbp-yhj2", Verb::Spawned)
    );
    assert!(run("spawn", json!({}), true, "Refused: budget").is_empty());
    assert!(run("spawn", json!({}), true, "Spawned thread not-an-id on mbp").is_empty());
    let a = run(
      "send",
      json!({"thread": "2026-10-15T060040Z-mbp-dc79", "message": "hi"}),
      true,
      "Sent",
    );
    assert_eq!(
      one(&a),
      (Kind::Thread, "2026-10-15T060040Z-mbp-dc79", Verb::Told)
    );
    assert!(run("send", json!({"thread": "garbage"}), true, "").is_empty());
    assert!(
      run(
        "wait",
        json!({"threads": ["2026-10-15T060040Z-mbp-dc79"]}),
        true,
        ""
      )
      .is_empty()
    );
    assert!(run("notify", json!({"body": "x"}), true, "").is_empty());
  }

  #[test]
  fn a_segments_rows_are_deduped_and_tied_to_their_messages() {
    let text = r##"{"v":1,"segment":"2026-11-11T100000Z-gamer-aaaa","thread":"2026-11-11T095900Z-gamer-bbbb","created":"2026-11-11T10:00:00.000Z","node":"gamer","prev":null}
{"t":"msg","id":"m1","at":"2026-11-11T10:00:01.000Z","role":"user","parts":[{"text":"go"}]}
{"t":"msg","id":"m2","at":"2026-11-11T10:00:02.000Z","role":"assistant","parts":[{"text":"ok"}]}
{"t":"tool_call","id":"c1","at":"2026-11-11T10:00:02.000Z","name":"edit_file","input":{"path":"a.rs"}}
{"t":"tool_call","id":"c2","at":"2026-11-11T10:00:02.000Z","name":"shell_run","input":{"command":"git commit"}}
{"t":"tool_result","id":"c1","at":"2026-11-11T10:00:03.000Z","ok":true,"output":{"text":"ok"}}
{"t":"tool_result","id":"c2","at":"2026-11-11T10:00:04.000Z","ok":true,"output":{"text":"[main abcdef1] first"}}
{"t":"msg","id":"m3","at":"2026-11-11T10:00:05.000Z","role":"assistant","parts":[{"text":"again"}]}
{"t":"tool_call","id":"c3","at":"2026-11-11T10:00:05.000Z","name":"edit_file","input":{"path":"a.rs"}}
{"t":"tool_result","id":"c3","at":"2026-11-11T10:00:06.000Z","ok":true,"output":{"text":"ok"}}
{"t":"tool_call","id":"c4","at":"2026-11-11T10:00:07.000Z","name":"view_file","input":{"path":"b.rs"}}
"##;
    let rows = segment_rows(&Segment::parse(text).unwrap());
    let got: Vec<_> = rows
      .iter()
      .map(|r| {
        (
          r.kind.as_str(),
          r.value.as_str(),
          r.verb.as_str(),
          r.n,
          r.first_msg.as_deref(),
          r.last_msg.as_deref(),
        )
      })
      .collect();
    assert_eq!(
      got,
      vec![
        ("path", "a.rs", "wrote", 2, Some("m2"), Some("m3")),
        ("path", "b.rs", "read", 1, Some("m3"), Some("m3")),
        ("commit", "abcdef1", "wrote", 1, Some("m2"), Some("m2")),
      ]
    );
    assert_eq!(rows[2].subject.as_deref(), Some("first"));
    assert!(rows[0].first_at < rows[0].last_at);
  }
}
