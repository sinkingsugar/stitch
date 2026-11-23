//! What the agent reads and what Discord gets: mentions resolved to names
//! (`<@id>` → `@Name`, `<@&id>` → `@Role`, `<#id>` → `#channel`), the
//! marker of a reply, the pass marker, and `@Name` turned back into a ping
//! for people of the conversation.

use std::collections::HashMap;

/// Names the bridge knows: users (display names), roles and channels.
#[derive(Clone, Debug, Default)]
pub struct Names {
  pub users: HashMap<u64, String>,
  pub roles: HashMap<u64, String>,
  pub channels: HashMap<u64, String>,
}

/// A mention in Discord's markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mention {
  User(u64),
  Role(u64),
  Channel(u64),
}

/// The mention at the start of `s` (`<@id>`, `<@!id>`, `<@&id>`, `<#id>`)
/// and its length.
fn mention_at(s: &str) -> Option<(Mention, usize)> {
  let rest = s.strip_prefix('<')?;
  let (kind, digits) = if let Some(r) = rest.strip_prefix("@&") {
    (2, r)
  } else if let Some(r) = rest.strip_prefix("@!") {
    (1, r)
  } else if let Some(r) = rest.strip_prefix('@') {
    (1, r)
  } else {
    (3, rest.strip_prefix('#')?)
  };
  let end = digits.find('>')?;
  let id: u64 = digits[..end].parse().ok().filter(|&n| n > 0)?;
  if !digits[..end].bytes().all(|b| b.is_ascii_digit()) {
    return None;
  }
  let len = s.len() - digits.len() + end + 1;
  let m = match kind {
    1 => Mention::User(id),
    2 => Mention::Role(id),
    _ => Mention::Channel(id),
  };
  Some((m, len))
}

/// The mentions in `text`, in order.
pub fn mentions(text: &str) -> Vec<Mention> {
  let mut out = Vec::new();
  let mut i = 0;
  while let Some(off) = text[i..].find('<') {
    let at = i + off;
    if let Some((m, len)) = mention_at(&text[at..]) {
      out.push(m);
      i = at + len;
    } else {
      i = at + 1;
    }
  }
  out
}

/// `text` with mentions as names: `@Name`, `@Role`, `#channel`; one not
/// known stays a plain `@user`, `@role` or `#channel`, never an ID.
pub fn resolve(text: &str, names: &Names) -> String {
  let mut out = String::with_capacity(text.len());
  let mut i = 0;
  while let Some(off) = text[i..].find('<') {
    let at = i + off;
    out.push_str(&text[i..at]);
    match mention_at(&text[at..]) {
      Some((m, len)) => {
        let name = match m {
          Mention::User(id) => format!(
            "@{}",
            names.users.get(&id).map(String::as_str).unwrap_or("user")
          ),
          Mention::Role(id) => format!(
            "@{}",
            names.roles.get(&id).map(String::as_str).unwrap_or("role")
          ),
          Mention::Channel(id) => format!(
            "#{}",
            names
              .channels
              .get(&id)
              .map(String::as_str)
              .unwrap_or("channel")
          ),
        };
        out.push_str(&name);
        i = at + len;
      }
      None => {
        out.push('<');
        i = at + 1;
      }
    }
  }
  out.push_str(&text[i..]);
  out
}

/// How much of a replied-to message is shown (characters).
pub const REPLY_SHOWN: usize = 100;

/// `(replying to Name: first words…)`, on one line.
pub fn reply_marker(author: &str, text: &str) -> String {
  let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
  let mut shown: String = one.chars().take(REPLY_SHOWN).collect();
  if one.chars().count() > REPLY_SHOWN {
    shown.push('…');
  }
  format!("(replying to {author}: {shown})")
}

/// What the agent answers to say nothing.
pub const PASS: &str = "[pass]";

/// Whether an answer is the pass marker (alone, trimmed, any case).
pub fn is_pass(answer: &str) -> bool {
  answer.trim().eq_ignore_ascii_case(PASS)
}

/// `@Name` of people in the conversation turned into pings (`<@id>`),
/// longest names first, a name ending where a word does; the IDs pinged,
/// for the message's allowed mentions. Nothing else pings: not
/// `@everyone`, `@here` or roles.
pub fn pings(text: &str, people: &HashMap<u64, String>) -> (String, Vec<u64>) {
  let mut by_len: Vec<(&u64, &String)> = people.iter().filter(|(_, n)| !n.is_empty()).collect();
  by_len.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
  let mut out = String::with_capacity(text.len());
  let mut pinged = Vec::new();
  let mut i = 0;
  'outer: while let Some(off) = text[i..].find('@') {
    let at = i + off;
    out.push_str(&text[i..at]);
    let after = &text[at + 1..];
    // An e-mail address or a word before: not a mention.
    let word_before = text[..at]
      .chars()
      .last()
      .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '`');
    if !word_before {
      for (id, name) in &by_len {
        if let Some(rest) = after.strip_prefix(name.as_str())
          && !rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
          out.push_str(&format!("<@{id}>"));
          if !pinged.contains(*id) {
            pinged.push(**id);
          }
          i = at + 1 + name.len();
          continue 'outer;
        }
      }
    }
    out.push('@');
    i = at + 1;
  }
  out.push_str(&text[i..]);
  (out, pinged)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names() -> Names {
    Names {
      users: [(1, "Ann".to_owned()), (2, "Bo B".to_owned())].into(),
      roles: [(7, "mods".to_owned())].into(),
      channels: [(9, "general".to_owned())].into(),
    }
  }

  #[test]
  fn mentions_become_names() {
    let n = names();
    assert_eq!(
      resolve("<@1> and <@!2>, ask <@&7> in <#9>", &n),
      "@Ann and @Bo B, ask @mods in #general"
    );
    assert_eq!(
      resolve("<@3> <@&8> <#10> <b> <@x> 1<2", &n),
      "@user @role #channel <b> <@x> 1<2"
    );
    assert_eq!(
      mentions("hi <@1> <#9> <@&7> <@!2> <@>"),
      vec![
        Mention::User(1),
        Mention::Channel(9),
        Mention::Role(7),
        Mention::User(2)
      ]
    );
  }

  #[test]
  fn a_reply_is_marked_short() {
    assert_eq!(
      reply_marker("Ann", "see\n  this"),
      "(replying to Ann: see this)"
    );
    let long = "x".repeat(150);
    let m = reply_marker("Ann", &long);
    assert!(m.ends_with("…)") && m.chars().count() < 130, "{m}");
  }

  #[test]
  fn the_pass_marker() {
    assert!(is_pass("[pass]") && is_pass("  [PASS]\n") && !is_pass("[pass] ok") && !is_pass(""));
  }

  #[test]
  fn people_of_the_conversation_are_pinged_and_no_one_else() {
    let people: HashMap<u64, String> = [(1, "Ann".to_owned()), (2, "Ann Lee".to_owned())].into();
    let (t, ids) = pings(
      "@Ann Lee and @Ann, not @Annie, @everyone or a@Ann.com",
      &people,
    );
    assert_eq!(t, "<@2> and <@1>, not @Annie, @everyone or a@Ann.com");
    assert_eq!(ids, vec![2, 1]);
  }
}
