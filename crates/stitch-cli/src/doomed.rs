//! What deleting threads takes, said plainly (design §2.5, "Deleting a
//! thread"): the node's dry run lists `<id>\t<title>\t<role>`, the role
//! `this` (a thread named), `earlier` or `later` (a part of the same
//! conversation, compacted into it or from it); an older node sends no
//! role.

/// One line of the node's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doomed {
  pub id: String,
  pub title: String,
  /// `this`, `earlier` or `later`; `None` from an older node.
  pub role: Option<String>,
}

pub fn parse(line: &str) -> Doomed {
  let (id, rest) = line.split_once('\t').unwrap_or((line, ""));
  let (title, role) = match rest.rsplit_once('\t') {
    Some((t, r)) if matches!(r, "this" | "earlier" | "later") => (t, Some(r.to_owned())),
    _ => (rest, None),
  };
  Doomed {
    id: id.to_owned(),
    title: title.to_owned(),
    role,
  }
}

/// What a line is, for the list: "this one", "earlier part", "later
/// part"; nothing when the node did not say.
pub fn label(d: &Doomed) -> &'static str {
  match d.role.as_deref() {
    Some("this") => "this one",
    Some("earlier") => "earlier part",
    Some("later") => "later part",
    _ => "",
  }
}

/// The question, in a sentence or two: what goes and why, and what stays.
pub fn question(list: &[Doomed]) -> String {
  if list.iter().any(|d| d.role.is_none()) {
    // An older node: it does not say which is which.
    return format!(
      "Delete {} thread(s) for good, on every node, with what they were summarized from or \
       into? The threads they started and the forks made of them stay.",
      list.len()
    );
  }
  let part = |d: &&Doomed| matches!(d.role.as_deref(), Some("earlier" | "later"));
  let parts = list.iter().filter(part).count();
  let named: Vec<&Doomed> = list.iter().filter(|d| !part(d)).collect();
  let mut s = match named.as_slice() {
    [one] => format!("Delete \"{}\" for good, on every node?", one.title),
    _ => format!(
      "Delete these {} threads for good, on every node?",
      named.len()
    ),
  };
  if parts > 0 && named.len() == 1 {
    s.push_str(&format!(
      " It was summarized and continued in a new thread {} when it grew long, so it is {} parts \
       of one conversation: all {} go.",
      times(parts),
      list.len(),
      list.len()
    ));
  } else if parts > 0 {
    s.push_str(&format!(
      " {parts} more part{} of the same conversations go{} with them (summarized and continued \
       in a new thread when they grew long).",
      if parts == 1 { "" } else { "s" },
      if parts == 1 { "es" } else { "" }
    ));
  }
  s.push_str(if named.len() == 1 {
    " The threads it started and the forks made of it stay."
  } else {
    " The threads they started and the forks made of them stay."
  });
  s
}

fn times(n: usize) -> String {
  match n {
    1 => "once".into(),
    2 => "twice".into(),
    n => format!("{n} times"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn d(id: &str, title: &str, role: Option<&str>) -> Doomed {
    Doomed {
      id: id.into(),
      title: title.into(),
      role: role.map(str::to_owned),
    }
  }

  #[test]
  fn lines_parse_with_and_without_a_role() {
    assert_eq!(parse("a\tTalk\tearlier"), d("a", "Talk", Some("earlier")));
    assert_eq!(parse("a\tTalk"), d("a", "Talk", None));
    assert_eq!(parse("a\tA\tB"), d("a", "A\tB", None));
    assert_eq!(parse("a"), d("a", "", None));
  }

  #[test]
  fn the_question_says_what_goes_and_why() {
    assert_eq!(
      question(&[d("a", "Talk", Some("this"))]),
      "Delete \"Talk\" for good, on every node? The threads it started and the forks made of it stay."
    );
    assert_eq!(
      question(&[
        d("a", "Talk", Some("earlier")),
        d("b", "Talk", Some("earlier")),
        d("c", "Talk", Some("this")),
      ]),
      "Delete \"Talk\" for good, on every node? It was summarized and continued in a new thread \
       twice when it grew long, so it is 3 parts of one conversation: all 3 go. The threads it \
       started and the forks made of it stay."
    );
    assert_eq!(
      question(&[
        d("a", "A", Some("this")),
        d("b", "B", Some("this")),
        d("c", "B", Some("later")),
      ]),
      "Delete these 2 threads for good, on every node? 1 more part of the same conversations goes \
       with them (summarized and continued in a new thread when they grew long). The threads they \
       started and the forks made of them stay."
    );
    // An older node: no roles.
    assert_eq!(
      question(&[d("a", "Talk", None), d("b", "Talk", None)]),
      "Delete 2 thread(s) for good, on every node, with what they were summarized from or into? \
       The threads they started and the forks made of them stay."
    );
  }
}
