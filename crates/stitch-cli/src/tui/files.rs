//! The files `@` offers in the TUI's input: what a git checkout holds
//! (tracked, and untracked files not ignored), else a bounded walk of the
//! directory; ranked by a fuzzy match of what is typed after the `@`.

use std::collections::BTreeSet;
use std::path::Path;

/// Directories a walk does not enter (a checkout's would be ignored).
const SKIP: &[&str] = &["node_modules", "target", "build", "dist", "__pycache__"];
/// How deep a walk goes.
const DEPTH: usize = 8;

/// The files under `root`, relative to it, with their directories
/// (ending in `/`), sorted; at most `max` files.
pub fn list(root: &Path, max: usize) -> Vec<String> {
  let files = git_files(root, max).unwrap_or_else(|| walk(root, max));
  let mut out: BTreeSet<String> = BTreeSet::new();
  for f in files {
    let mut dir = f.as_str();
    while let Some((parent, _)) = dir.rsplit_once('/') {
      out.insert(format!("{parent}/"));
      dir = parent;
    }
    out.insert(f);
  }
  out.into_iter().collect()
}

/// `git ls-files` there: tracked files that still exist, and untracked
/// ones not ignored, relative to `root`; `None` outside a checkout.
fn git_files(root: &Path, max: usize) -> Option<Vec<String>> {
  let out = std::process::Command::new("git")
    .args([
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      "-z",
    ])
    .current_dir(root)
    .stderr(std::process::Stdio::null())
    .output()
    .ok()?;
  if !out.status.success() {
    return None;
  }
  let mut seen = BTreeSet::new();
  Some(
    out
      .stdout
      .split(|b| *b == 0)
      .filter_map(|p| std::str::from_utf8(p).ok())
      .filter(|p| !p.is_empty() && seen.insert(p.to_string()))
      .filter(|p| root.join(p).is_file())
      .take(max)
      .map(str::to_owned)
      .collect(),
  )
}

/// Files under `root`, breadth first, skipping hidden directories and
/// build outputs, `DEPTH` levels and `max` files at most.
fn walk(root: &Path, max: usize) -> Vec<String> {
  let mut out = Vec::new();
  let mut dirs = std::collections::VecDeque::from([(String::new(), 0usize)]);
  while let Some((rel, depth)) = dirs.pop_front() {
    let Ok(read) = std::fs::read_dir(root.join(&rel)) else {
      continue;
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
      let Some(name) = e.file_name().to_str().map(str::to_owned) else {
        continue;
      };
      if name.starts_with('.') {
        continue;
      }
      let path = if rel.is_empty() {
        name.clone()
      } else {
        format!("{rel}/{name}")
      };
      let Ok(kind) = e.file_type() else { continue };
      if kind.is_dir() {
        if depth + 1 < DEPTH && !SKIP.contains(&name.as_str()) {
          dirs.push_back((path, depth + 1));
        }
      } else {
        out.push(path);
        if out.len() >= max {
          return out;
        }
      }
    }
  }
  out
}

/// How well `query` matches `path`, as a subsequence (case aside):
/// characters in a row, at a word's start and in the file's own name
/// count for more, a long path for less. `None`: no match.
pub fn score(query: &str, path: &str) -> Option<i64> {
  let p: Vec<char> = path.to_lowercase().chars().collect();
  let q: Vec<char> = query.to_lowercase().chars().collect();
  let base = path.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1);
  let length = -(p.len() as i64) / 4;
  if q.is_empty() {
    // Nothing typed: the top of the tree first.
    let depth = path.trim_end_matches('/').matches('/').count() as i64;
    return Some(-depth * 10 + length);
  }
  let rate = |at: &[usize]| -> i64 {
    let mut s = 0;
    for (k, &i) in at.iter().enumerate() {
      s += 1;
      if i == 0 || matches!(p[i - 1], '/' | '_' | '-' | '.' | ' ') {
        s += 8;
      }
      if k > 0 && at[k - 1] + 1 == i {
        s += 6;
      }
      if i >= base {
        s += 3;
      }
    }
    s
  };
  // From the left, and from the right (which finds it in the file's name
  // when it is there): the better of the two.
  let mut left = Vec::with_capacity(q.len());
  let mut i = 0;
  for c in &q {
    while i < p.len() && p[i] != *c {
      i += 1;
    }
    if i == p.len() {
      return None;
    }
    left.push(i);
    i += 1;
  }
  let mut right = Vec::with_capacity(q.len());
  let mut j = p.len();
  for c in q.iter().rev() {
    while j > 0 && p[j - 1] != *c {
      j -= 1;
    }
    if j == 0 {
      break;
    }
    j -= 1;
    right.push(j);
  }
  right.reverse();
  let best = if right.len() == q.len() {
    rate(&left).max(rate(&right))
  } else {
    rate(&left)
  };
  Some(best * 4 + length)
}

/// The `n` best matches of `query` among `files`.
pub fn rank(files: &[String], query: &str, n: usize) -> Vec<String> {
  let mut scored: Vec<(i64, &String)> = files
    .iter()
    .filter_map(|f| score(query, f).map(|s| (s, f)))
    .collect();
  scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
  scored.into_iter().take(n).map(|(_, f)| f.clone()).collect()
}

/// Whether a file is attached when picked: a text file (no NUL, UTF-8, in
/// its first 8 KB) of at most `max` bytes. Else why not.
pub fn attachable(path: &Path, max: u64) -> Result<(), String> {
  use std::io::Read;
  let meta = std::fs::metadata(path).map_err(|_| "not found".to_owned())?;
  if meta.is_dir() {
    return Err("a directory".into());
  }
  if meta.len() > max {
    return Err(format!("{} KB, too big to attach", meta.len() / 1024));
  }
  let mut head = Vec::new();
  std::fs::File::open(path)
    .and_then(|f| f.take(8192).read_to_end(&mut head))
    .map_err(|e| e.to_string())?;
  let text = match std::str::from_utf8(&head) {
    Ok(_) => true,
    // Cut in the middle of a character at 8 KB.
    Err(e) => head.len() == 8192 && e.error_len().is_none(),
  };
  if !text || head.contains(&0) {
    return Err("not text".into());
  }
  Ok(())
}

/// Whether `text` still names `@path`: followed by its end, a space or
/// punctuation, not by more of a path.
pub fn mentioned(text: &str, path: &str) -> bool {
  let needle = format!("@{path}");
  let mut from = 0;
  while let Some(i) = text[from..].find(&needle) {
    let start = from + i;
    let end = start + needle.len();
    let before = text[..start].chars().next_back();
    let mut after = text[end..].chars();
    let ends = match after.next() {
      None => true,
      Some(c) if c.is_alphanumeric() || matches!(c, '/' | '_' | '-') => false,
      // A full stop ends it; `.rs` goes on.
      Some('.') => after.next().is_none_or(|c| !c.is_alphanumeric()),
      Some(_) => true,
    };
    if ends && before.is_none_or(char::is_whitespace) {
      return true;
    }
    from = end;
  }
  false
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_file_s_own_name_wins() {
    let files: Vec<String> = [
      "docs/",
      "docs/design.md",
      "src/",
      "src/main.rs",
      "src/mod/app.rs",
      "README.md",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(rank(&files, "main", 3)[0], "src/main.rs");
    assert_eq!(rank(&files, "dsg", 3)[0], "docs/design.md");
    assert_eq!(rank(&files, "app", 1), vec!["src/mod/app.rs"]);
    assert!(rank(&files, "zzz", 3).is_empty());
    // Nothing typed: the top of the tree first.
    let top = rank(&files, "", 3);
    assert!(top.iter().all(|f| !f.trim_end_matches('/').contains('/')));
  }

  #[test]
  fn a_mention_is_the_whole_path() {
    assert!(mentioned("see @src/a.rs please", "src/a.rs"));
    assert!(mentioned("see @src/a.rs.", "src/a.rs"));
    assert!(mentioned("@src/a.rs", "src/a.rs"));
    assert!(!mentioned("see @src/a.rsx", "src/a.rs"));
    assert!(!mentioned("see src/a.rs", "src/a.rs"));
    assert!(!mentioned("x@src/a.rs", "src/a.rs"));
  }

  #[test]
  fn lists_walk_and_attach_only_small_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    std::fs::write(root.join("src/deep/a.txt"), "hello").unwrap();
    std::fs::write(root.join(".hidden/x"), "no").unwrap();
    std::fs::write(root.join("bin"), [0u8, 1, 2]).unwrap();
    std::fs::write(root.join("big.txt"), "x".repeat(2000)).unwrap();
    let files = list(root, 100);
    assert_eq!(
      files,
      vec!["big.txt", "bin", "src/", "src/deep/", "src/deep/a.txt"]
    );
    assert!(attachable(&root.join("src/deep/a.txt"), 1000).is_ok());
    assert_eq!(attachable(&root.join("bin"), 1000).unwrap_err(), "not text");
    assert!(attachable(&root.join("big.txt"), 1000).is_err());
    assert_eq!(
      attachable(&root.join("src"), 1000).unwrap_err(),
      "a directory"
    );
  }
}
