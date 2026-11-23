//! The thread list as a tree (design §2.16, "Thread list"; the phone's
//! `ThreadTree`): a spawned thread hangs under the thread that spawned it
//! (else the one it reports to), so a conversation and the work it
//! started read as one entry. A thread whose parent is not listed is a
//! root. Typing filters: the matches show flat, each saying what it hangs
//! under.

use std::collections::{HashMap, HashSet};

use super::app::ThreadItem;

/// It works now: a turn runs in it.
pub fn working(t: &ThreadItem) -> bool {
  matches!(t.state.as_deref(), Some("running" | "working"))
}

/// It waits on the user: an approval, a question.
pub fn waiting(t: &ThreadItem) -> bool {
  matches!(t.state.as_deref(), Some("waiting" | "blocked"))
}

pub fn failed(t: &ThreadItem) -> bool {
  t.state.as_deref() == Some("failed")
}

/// A thread with the threads below it, newest activity first.
#[derive(Clone, Debug)]
struct Node {
  /// Its index in the list it was built from.
  at: usize,
  children: Vec<Node>,
  latest: jiff::Timestamp,
}

/// What is below a thread, at every depth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Below {
  pub all: usize,
  pub working: usize,
  pub waiting: usize,
  pub failed: usize,
}

impl Below {
  /// The closed summary: `3 below · 1 working · 1 needs you`.
  pub fn summary(&self) -> String {
    let mut s = format!("{} below", self.all);
    for (n, word) in [
      (self.working, "working"),
      (self.waiting, "needs you"),
      (self.failed, "failed"),
    ] {
      if n > 0 {
        s.push_str(&format!(" · {n} {word}"));
      }
    }
    s
  }
}

/// A row as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
  /// The thread's index in `App::threads`.
  pub at: usize,
  pub depth: usize,
  /// The tree's lines before it (`│ ├ `), empty for a root.
  pub lines: String,
  /// Whether its spawns show; `None` without any.
  pub open: Option<bool>,
  pub below: Below,
  /// Filtering: the title of the thread it hangs under.
  pub under: Option<String>,
}

fn below(n: &Node, threads: &[ThreadItem]) -> Below {
  let mut b = Below::default();
  for c in &n.children {
    let t = &threads[c.at];
    let cb = below(c, threads);
    b.all += 1 + cb.all;
    b.working += usize::from(working(t)) + cb.working;
    b.waiting += usize::from(waiting(t)) + cb.waiting;
    b.failed += usize::from(failed(t)) + cb.failed;
  }
  b
}

fn roots(threads: &[ThreadItem], parents: &HashMap<&str, usize>) -> Vec<Node> {
  let mut under: HashMap<usize, Vec<usize>> = HashMap::new();
  let mut tops = Vec::new();
  for (i, t) in threads.iter().enumerate() {
    // A pinned thread is a root of its own, whatever spawned it, so it is
    // listed with the pinned rather than under a parent that may be closed.
    match t
      .parent
      .as_deref()
      .filter(|p| *p != t.id && !t.pinned)
      .and_then(|p| parents.get(p))
    {
      Some(&p) => under.entry(p).or_default().push(i),
      None => tops.push(i),
    }
  }
  let mut seen = HashSet::new();
  fn build(
    i: usize,
    depth: usize,
    threads: &[ThreadItem],
    under: &HashMap<usize, Vec<usize>>,
    seen: &mut HashSet<usize>,
  ) -> Node {
    seen.insert(i);
    let mut children = Vec::new();
    if depth <= 32 {
      for &c in under.get(&i).map(Vec::as_slice).unwrap_or_default() {
        if !seen.contains(&c) {
          children.push(build(c, depth + 1, threads, under, seen));
        }
      }
    }
    children.sort_by_key(|c| std::cmp::Reverse(c.latest));
    let latest = children
      .iter()
      .map(|c| c.latest)
      .fold(threads[i].at, std::cmp::max);
    Node {
      at: i,
      children,
      latest,
    }
  }
  let mut out: Vec<Node> = tops
    .into_iter()
    .map(|i| build(i, 0, threads, &under, &mut seen))
    .collect();
  // A cycle (records that disagree) would leave threads out: whatever no
  // root reaches becomes one.
  for i in 0..threads.len() {
    if !seen.contains(&i) {
      out.push(build(i, 0, threads, &under, &mut seen));
    }
  }
  // Pinned first, then by the latest activity in it or below it.
  out.sort_by(|a, b| (threads[b.at].pinned, b.latest).cmp(&(threads[a.at].pinned, a.latest)));
  out
}

/// Open by the user's choice (`opened`, `closed`), else while something
/// below works or waits on the user.
pub fn is_open(id: &str, b: &Below, opened: &HashSet<String>, closed: &HashSet<String>) -> bool {
  if opened.contains(id) {
    return true;
  }
  if closed.contains(id) {
    return false;
  }
  b.working + b.waiting > 0
}

/// Whether every word of `filter` is in its title or place.
pub fn matches(t: &ThreadItem, words: &[String]) -> bool {
  let hay = format!("{} {}", t.title, t.place.as_deref().unwrap_or("")).to_lowercase();
  words.iter().all(|w| hay.contains(w.as_str()))
}

/// The rows to show: the tree with what is open shown, or with a filter
/// the matches, flat.
pub fn rows(
  threads: &[ThreadItem],
  filter: &str,
  opened: &HashSet<String>,
  closed: &HashSet<String>,
) -> Vec<Row> {
  let parents: HashMap<&str, usize> = threads
    .iter()
    .enumerate()
    .map(|(i, t)| (t.id.as_str(), i))
    .collect();
  let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
  if !words.is_empty() {
    return threads
      .iter()
      .enumerate()
      .filter(|(_, t)| matches(t, &words))
      .map(|(i, t)| Row {
        at: i,
        depth: 0,
        lines: String::new(),
        open: None,
        below: Below::default(),
        under: t
          .parent
          .as_deref()
          .and_then(|p| parents.get(p))
          .map(|&p| threads[p].title.clone()),
      })
      .collect();
  }
  let mut out = Vec::new();
  /// The list, and what the user opened and closed.
  type Ctx<'a> = (&'a [ThreadItem], &'a HashSet<String>, &'a HashSet<String>);
  fn walk(n: &Node, depth: usize, prefix: &str, last: bool, ctx: Ctx, out: &mut Vec<Row>) {
    let (threads, opened, closed) = ctx;
    let b = below(n, threads);
    let id = threads[n.at].id.as_str();
    let open = (!n.children.is_empty()).then(|| is_open(id, &b, opened, closed));
    let lines = if depth == 0 {
      String::new()
    } else {
      format!("{prefix}{}", if last { "└ " } else { "├ " })
    };
    out.push(Row {
      at: n.at,
      depth,
      lines,
      open,
      below: b,
      under: None,
    });
    if open == Some(true) {
      let inner = if depth == 0 {
        String::new()
      } else {
        format!("{prefix}{}", if last { "  " } else { "│ " })
      };
      let k = n.children.len();
      for (j, c) in n.children.iter().enumerate() {
        walk(c, depth + 1, &inner, j + 1 == k, ctx, out);
      }
    }
  }
  for r in roots(threads, &parents) {
    let first = out.len();
    walk(&r, 0, "", true, (threads, opened, closed), &mut out);
    // A pinned spawn, lifted to the top, says what it hangs under.
    let t = &threads[r.at];
    if t.pinned {
      out[first].under = t
        .parent
        .as_deref()
        .filter(|p| *p != t.id)
        .and_then(|p| parents.get(p))
        .map(|&p| threads[p].title.clone());
    }
  }
  out
}

/// The threads above `id`, nearest first (to open them so it shows).
pub fn ancestors(threads: &[ThreadItem], id: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut at = id.to_owned();
  for _ in 0..33 {
    let Some(p) = threads
      .iter()
      .find(|t| t.id == at)
      .filter(|t| !t.pinned)
      .and_then(|t| t.parent.clone())
      .filter(|p| *p != at && threads.iter().any(|t| &t.id == p))
    else {
      break;
    };
    if out.contains(&p) {
      break;
    }
    out.push(p.clone());
    at = p;
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn t(id: &str, parent: Option<&str>, secs: i64, state: Option<&str>) -> ThreadItem {
    ThreadItem {
      id: id.into(),
      title: format!("title {id}"),
      when: String::new(),
      state: state.map(Into::into),
      at: jiff::Timestamp::from_second(secs).unwrap(),
      place: None,
      key: "global".into(),
      pinned: false,
      parent: parent.map(Into::into),
      goal: None,
    }
  }

  fn shown(threads: &[ThreadItem], rows: &[Row]) -> Vec<String> {
    rows
      .iter()
      .map(|r| {
        let mark = match r.open {
          Some(true) => "▾ ",
          Some(false) => "▸ ",
          None => "",
        };
        format!("{}{mark}{}", r.lines, threads[r.at].id)
      })
      .collect()
  }

  #[test]
  fn spawns_hang_under_their_parent_open_while_something_works() {
    let threads = vec![
      t("a", None, 10, None),
      t("a1", Some("a"), 50, Some("done")),
      t("a2", Some("a"), 40, Some("running")),
      t("a21", Some("a2"), 30, Some("waiting")),
      t("b", None, 20, None),
      t("b1", Some("b"), 5, Some("failed")),
      // Its parent is not listed (another place): a root.
      t("c", Some("elsewhere"), 1, None),
    ];
    let none = HashSet::new();
    let rows = rows(&threads, "", &none, &none);
    assert_eq!(
      shown(&threads, &rows),
      vec!["▾ a", "├ a1", "└ ▾ a2", "  └ a21", "▸ b", "c"],
      "a by its latest spawn first; b closed (nothing works below it)"
    );
    assert_eq!(
      rows[0].below,
      Below {
        all: 3,
        working: 1,
        waiting: 1,
        failed: 0
      }
    );
    assert_eq!(rows[4].below.summary(), "1 below · 1 failed");
    // The user's choice wins.
    let closed: HashSet<String> = ["a".to_owned()].into();
    let opened: HashSet<String> = ["b".to_owned()].into();
    let rows2 = super::rows(&threads, "", &opened, &closed);
    assert_eq!(shown(&threads, &rows2), vec!["▸ a", "▾ b", "└ b1", "c"]);
    assert_eq!(ancestors(&threads, "a21"), vec!["a2", "a"]);
  }

  #[test]
  fn a_filter_shows_matches_flat_with_what_they_hang_under() {
    let mut threads = vec![
      t("a", None, 10, None),
      t("a1", Some("a"), 50, None),
      t("b", None, 20, None),
    ];
    threads[0].title = "Thread list tree".into();
    threads[1].title = "Survey the iOS list".into();
    threads[2].title = "Weekend basket".into();
    threads[2].place = Some("~/notes".into());
    let none = HashSet::new();
    let r = rows(&threads, "LIST", &none, &none);
    assert_eq!(r.iter().map(|r| r.at).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(r[1].under.as_deref(), Some("Thread list tree"));
    assert_eq!(rows(&threads, "ios list", &none, &none).len(), 1);
    assert_eq!(
      rows(&threads, "notes", &none, &none)[0].at,
      2,
      "by place too"
    );
    assert!(rows(&threads, "nothing", &none, &none).is_empty());
  }

  #[test]
  fn a_pinned_spawn_is_a_root_with_the_pinned() {
    let mut kid = t("a1", Some("a"), 5, None);
    kid.pinned = true;
    let threads = vec![
      t("b", None, 50, None),
      t("a", None, 10, None),
      kid,
      t("a2", Some("a"), 8, None),
    ];
    let none = HashSet::new();
    let rows = rows(&threads, "", &none, &none);
    assert_eq!(shown(&threads, &rows), vec!["a1", "b", "▸ a"]);
    assert_eq!(rows[0].under.as_deref(), Some("title a"));
    assert_eq!(rows[1].under, None);
    assert!(
      ancestors(&threads, "a1").is_empty(),
      "nothing to open above it"
    );
  }

  #[test]
  fn a_cycle_still_lists_every_thread() {
    let threads = vec![t("a", Some("b"), 1, None), t("b", Some("a"), 2, None)];
    let opened: HashSet<String> = ["a".to_owned(), "b".to_owned()].into();
    let rows = rows(&threads, "", &opened, &HashSet::new());
    let mut ids: Vec<_> = rows.iter().map(|r| threads[r.at].id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["a", "b"]);
  }
}
