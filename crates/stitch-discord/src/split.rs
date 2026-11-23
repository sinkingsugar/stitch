//! A reply cut into Discord messages (at most 2000 characters each): at
//! line breaks where it can, inside a line at a space where it must, and
//! a code block cut in two closed and opened again with its language.

/// What a message may hold, with room to spare for the fences added.
pub const LIMIT: usize = 1990;

fn len(s: &str) -> usize {
  s.chars().count()
}

/// `text` in pieces of at most `limit` characters.
pub fn split(text: &str, limit: usize) -> Vec<String> {
  let text = text.trim_end();
  if len(text) <= limit {
    return if text.trim().is_empty() {
      Vec::new()
    } else {
      vec![text.to_owned()]
    };
  }
  let mut out: Vec<String> = Vec::new();
  let mut cur = String::new();
  // The open code block's opening fence (```rust), if any.
  let mut fence: Option<String> = None;
  let close = |cur: &mut String, fence: &Option<String>, out: &mut Vec<String>| {
    if fence.is_some() {
      cur.push_str("\n```");
    }
    if !cur.trim().is_empty() {
      out.push(std::mem::take(cur));
    }
    cur.clear();
    if let Some(f) = fence {
      cur.push_str(f);
    }
  };
  for line in text.split('\n') {
    let toggles = line.trim_start().starts_with("```");
    // Room for this line and a closing fence after it.
    let reserve = if fence.is_some() || toggles { 4 } else { 0 };
    let sep = usize::from(!cur.is_empty());
    if len(&cur) + sep + len(line) + reserve > limit
      && !cur.is_empty()
      && !is_only_fence(&cur, &fence)
    {
      close(&mut cur, &fence, &mut out);
    }
    if len(&cur) + usize::from(!cur.is_empty()) + len(line) + reserve > limit {
      // A line longer than a message: in pieces, at spaces where any.
      let mut rest = line;
      while len(&cur) + usize::from(!cur.is_empty()) + len(rest) + reserve > limit {
        let room = limit - reserve - len(&cur) - usize::from(!cur.is_empty());
        let cut = cut_at(rest, room.max(1));
        if !cur.is_empty() {
          cur.push('\n');
        }
        cur.push_str(rest[..cut].trim_end());
        rest = rest[cut..].trim_start();
        close(&mut cur, &fence, &mut out);
      }
      if !rest.is_empty() {
        if !cur.is_empty() {
          cur.push('\n');
        }
        cur.push_str(rest);
      }
    } else {
      if !cur.is_empty() {
        cur.push('\n');
      }
      cur.push_str(line);
    }
    if toggles {
      fence = match fence {
        Some(_) => None,
        None => Some(line.trim().to_owned()),
      };
    }
  }
  if fence.is_some() && !is_only_fence(&cur, &fence) {
    cur.push_str("\n```");
  }
  if !cur.trim().is_empty() && !is_only_fence(&cur, &fence) {
    out.push(cur);
  }
  out
}

/// `cur` holds nothing but the fence opened again.
fn is_only_fence(cur: &str, fence: &Option<String>) -> bool {
  fence.as_deref() == Some(cur)
}

/// The byte index to cut `s` at, at most `max` characters in: after the
/// last space there, else at `max`.
fn cut_at(s: &str, max: usize) -> usize {
  let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
  match s[..end].rfind(' ') {
    Some(i) if i > end / 2 => i + 1,
    _ => end,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn short_stays_whole_and_empty_is_nothing() {
    assert_eq!(split("hello", 2000), vec!["hello"]);
    assert!(split("  \n", 2000).is_empty());
  }

  #[test]
  fn cuts_at_lines_under_the_limit() {
    let text: String = (0..300)
      .map(|i| format!("line {i} of the answer\n"))
      .collect();
    let parts = split(&text, 2000);
    assert!(parts.len() > 1);
    for p in &parts {
      assert!(len(p) <= 2000, "{}", len(p));
    }
    assert_eq!(parts.join("\n"), text.trim_end());
  }

  #[test]
  fn long_lines_and_unicode() {
    let word = "épée ";
    let text = word.repeat(1000);
    let parts = split(&text, 100);
    for p in &parts {
      assert!(len(p) <= 100);
    }
    assert_eq!(parts.join(" ").split_whitespace().count(), 1000);
    let solid = "x".repeat(250);
    let parts = split(&solid, 100);
    assert_eq!(
      parts.iter().map(|p| len(p)).collect::<Vec<_>>(),
      vec![100, 100, 50]
    );
  }

  #[test]
  fn code_blocks_close_and_reopen() {
    let mut text = String::from("Here:\n```lua\n");
    for i in 0..100 {
      text.push_str(&format!("print({i}) -- a line of code\n"));
    }
    text.push_str("```\nDone.");
    let parts = split(&text, 500);
    assert!(parts.len() > 2);
    for p in &parts {
      assert!(len(p) <= 500, "{}", len(p));
      assert_eq!(p.matches("```").count() % 2, 0, "unbalanced: {p}");
    }
    assert!(parts[1].starts_with("```lua\n"));
    assert!(parts.last().unwrap().ends_with("Done."));
  }
}
