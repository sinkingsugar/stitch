//! What a mapped Stitch thread said in turns the bridge did not start (a
//! child's report waking it, a schedule, a `send`, `stitch chat`), read
//! from the node's files: the sealed segments
//! (`<home>/threads/<thread>/<segment>.jsonl`) and the open one of a turn
//! running (`<home>/journal/<thread>/`). Reading files keeps the bridge a
//! plain client of the node (it shares the data volume), and costs a
//! directory listing per mapped thread a poll while nothing changes.

use std::path::{Path, PathBuf};

/// A message of a thread: its number (`m12` → 12), its role, its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Said {
  pub n: u64,
  pub assistant: bool,
  pub text: String,
}

/// Where a thread's files are.
pub struct Files {
  pub sealed: PathBuf,
  pub open: PathBuf,
}

pub fn files(home: &Path, thread: &str) -> Files {
  Files {
    sealed: home.join("threads").join(thread),
    open: home.join("journal").join(thread),
  }
}

fn jsonl(dir: &Path) -> Vec<PathBuf> {
  let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
    .map(|rd| {
      rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
    })
    .unwrap_or_default();
  v.sort();
  v
}

/// Whether a turn is running on the thread (its open segment is there).
pub fn running(f: &Files) -> bool {
  !jsonl(&f.open).is_empty()
}

/// What changes when a turn ends: the sealed segments' names.
pub fn stamp(f: &Files) -> String {
  jsonl(&f.sealed)
    .iter()
    .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
    .collect::<Vec<_>>()
    .join(",")
}

/// Every message of the thread, sealed and open, in order of number.
pub fn messages(f: &Files) -> Vec<Said> {
  let mut out = Vec::new();
  for p in jsonl(&f.sealed).into_iter().chain(jsonl(&f.open)) {
    let Ok(text) = std::fs::read_to_string(&p) else {
      continue;
    };
    for line in text.lines() {
      let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        continue;
      };
      if v["t"] != "msg" {
        continue;
      }
      let Some(n) = v["id"]
        .as_str()
        .and_then(|id| id.strip_prefix('m'))
        .and_then(|n| n.parse().ok())
      else {
        continue;
      };
      let text = v["parts"]
        .as_array()
        .map(|ps| {
          ps.iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
        })
        .unwrap_or_default();
      out.push(Said {
        n,
        assistant: v["role"] == "assistant",
        text,
      });
    }
  }
  out.sort_by_key(|s| s.n);
  out.dedup_by_key(|s| s.n);
  out
}

/// The last message's number (0: none).
pub fn latest(f: &Files) -> u64 {
  messages(f).last().map_or(0, |s| s.n)
}

/// What the assistant said after message `after`, its texts joined; and
/// the last message's number.
pub fn said_after(f: &Files, after: u64) -> (String, u64) {
  let all = messages(f);
  let last = all.last().map_or(after, |s| s.n);
  let text = all
    .iter()
    .filter(|s| s.n > after && s.assistant && !s.text.trim().is_empty())
    .filter(|s| !crate::text::is_pass(&s.text))
    .map(|s| s.text.trim())
    .collect::<Vec<_>>()
    .join("\n\n");
  (text, last)
}
