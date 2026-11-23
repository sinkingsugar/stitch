//! Markdown in the TUI: a reply's lines as the terminal shows them.
//!
//! Replies reach the scrollback a finished line at a time, so this renders
//! line by line, keeping only what a line cannot tell alone: whether a code
//! fence is open, and the rows of a table not yet ended (its columns are
//! sized once it is whole). What the terminal copies stays close to what
//! was written: emphasis, code-span and heading markers go, list markers
//! become bullets, a code block keeps its fences (dimmed) and its text as
//! written, and a link keeps its URL after its text (and is clickable where
//! the terminal knows OSC 8).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// A run of text in one style, maybe a link.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seg {
  pub text: String,
  pub style: Style,
  pub link: Option<String>,
}

impl Seg {
  fn new(text: impl Into<String>, style: Style) -> Seg {
    Seg {
      text: text.into(),
      style,
      link: None,
    }
  }
}

/// One rendered line.
pub type MdLine = Vec<Seg>;

/// The text a line shows.
pub fn plain(line: &[Seg]) -> String {
  line.iter().map(|s| s.text.as_str()).collect()
}

/// A line for ratatui (the live area): links as their text.
pub fn to_line(line: MdLine) -> Line<'static> {
  Line::from(
    line
      .into_iter()
      .map(|s| Span::styled(s.text, s.style))
      .collect::<Vec<_>>(),
  )
}

const CODE: Style = Style::new().fg(Color::LightYellow);
const MARK: Style = Style::new().fg(Color::Cyan);
const DIM: Style = Style::new().fg(Color::DarkGray);
const LINK: Style = Style::new()
  .fg(Color::Blue)
  .add_modifier(Modifier::UNDERLINED);

/// A whole text, rendered for a terminal `width` columns wide (0: unknown).
pub fn render(text: &str, width: u16) -> Vec<MdLine> {
  let mut md = Md::new(width);
  let mut out = md.push(text);
  out.extend(md.finish());
  out
}

/// The renderer of one reply, fed its finished lines in order.
#[derive(Clone, Debug, Default)]
pub struct Md {
  /// The terminal's width (0: unknown): tables wider are left as written,
  /// rules are no wider.
  pub width: u16,
  fence: Option<Fence>,
  table: Vec<String>,
}

#[derive(Clone, Debug)]
struct Fence {
  ch: char,
  len: usize,
}

impl Md {
  pub fn new(width: u16) -> Md {
    Md {
      width,
      ..Md::default()
    }
  }

  /// Renders finished lines (`text` split on every newline). Table rows
  /// are held until the table ends.
  pub fn push(&mut self, text: &str) -> Vec<MdLine> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
      self.line(raw, &mut out);
    }
    out
  }

  /// The end of the reply: a table still held is rendered.
  pub fn finish(&mut self) -> Vec<MdLine> {
    let mut out = Vec::new();
    self.flush_table(&mut out);
    self.fence = None;
    out
  }

  /// What the live area shows: a held table, then the line still being
  /// written, as they would render now.
  pub fn preview(&self, partial: &str) -> Vec<MdLine> {
    let mut md = self.clone();
    let mut out = if partial.is_empty() {
      Vec::new()
    } else {
      md.push(partial)
    };
    out.extend(md.finish());
    out
  }

  fn line(&mut self, raw: &str, out: &mut Vec<MdLine>) {
    if let Some(f) = &self.fence {
      if closes(f, raw) {
        self.fence = None;
        out.push(vec![Seg::new(raw, DIM)]);
      } else {
        out.push(vec![Seg::new(raw, CODE)]);
      }
      return;
    }
    if raw.trim_start().starts_with('|') {
      self.table.push(raw.to_owned());
      return;
    }
    self.flush_table(out);
    if let Some(f) = opens(raw) {
      self.fence = Some(f);
      out.push(vec![Seg::new(raw, DIM)]);
      return;
    }
    out.push(self.block(raw));
  }

  /// A line outside code and tables.
  fn block(&self, raw: &str) -> MdLine {
    let indent = raw.len() - raw.trim_start_matches(' ').len();
    let t = &raw[indent..];
    let mut out = Vec::new();
    if indent <= 3 {
      let hashes = t.chars().take_while(|&c| c == '#').count();
      if (1..=6).contains(&hashes) && (t.len() == hashes || t[hashes..].starts_with(' ')) {
        let mut style = Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD);
        if hashes == 1 {
          style = style.add_modifier(Modifier::UNDERLINED);
        }
        inline(t[hashes..].trim(), style, &mut out);
        return out;
      }
      if is_rule(t) {
        let n = match self.width {
          0 => 40,
          w => usize::from(w).min(60),
        };
        return vec![Seg::new("─".repeat(n), DIM)];
      }
      if let Some(rest) = t.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        out.push(Seg::new("│ ", DIM));
        out.extend(self.block(rest));
        return out;
      }
    }
    if indent > 0 {
      out.push(Seg::new(" ".repeat(indent), Style::new()));
    }
    let bullet = ["- ", "* ", "+ "].iter().find_map(|b| t.strip_prefix(b));
    if let Some(rest) = bullet {
      let task = [("[ ] ", false), ("[x] ", true), ("[X] ", true)]
        .iter()
        .find_map(|(m, done)| rest.strip_prefix(m).map(|r| (r, *done)));
      match task {
        Some((rest, false)) => {
          out.push(Seg::new("☐ ", MARK));
          inline(rest, Style::new(), &mut out);
        }
        Some((rest, true)) => {
          out.push(Seg::new("☑ ", Style::new().fg(Color::Green)));
          inline(rest, Style::new().fg(Color::DarkGray), &mut out);
        }
        None => {
          out.push(Seg::new("• ", MARK));
          inline(rest, Style::new(), &mut out);
        }
      }
      return out;
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits)
      && matches!(t[digits..].chars().next(), Some('.' | ')'))
      && t[digits + 1..].starts_with(' ')
    {
      out.push(Seg::new(&t[..digits + 2], MARK));
      inline(&t[digits + 2..], Style::new(), &mut out);
      return out;
    }
    inline(t, Style::new(), &mut out);
    out
  }

  fn flush_table(&mut self, out: &mut Vec<MdLine>) {
    let rows = std::mem::take(&mut self.table);
    if rows.is_empty() {
      return;
    }
    match self.table_lines(&rows) {
      Some(lines) => out.extend(lines),
      // Not a table, or too wide for the terminal: as written.
      None => out.extend(rows.iter().map(|r| self.block(r))),
    }
  }

  fn table_lines(&self, rows: &[String]) -> Option<Vec<MdLine>> {
    if rows.len() < 2 {
      return None;
    }
    let aligns: Vec<Align> = cells(&rows[1])
      .iter()
      .map(|c| align(c))
      .collect::<Option<_>>()?;
    let body: Vec<Vec<MdLine>> = rows
      .iter()
      .enumerate()
      .filter(|(i, _)| *i != 1)
      .map(|(i, r)| {
        let style = if i == 0 {
          Style::new().add_modifier(Modifier::BOLD)
        } else {
          Style::new()
        };
        cells(r)
          .iter()
          .map(|c| {
            let mut segs = Vec::new();
            inline(c, style, &mut segs);
            segs
          })
          .collect()
      })
      .collect();
    let n = body
      .iter()
      .map(Vec::len)
      .max()
      .unwrap_or(0)
      .max(aligns.len());
    let mut widths = vec![0usize; n];
    for row in &body {
      for (i, c) in row.iter().enumerate() {
        widths[i] = widths[i].max(plain(c).width());
      }
    }
    let total = widths.iter().sum::<usize>() + 3 * n.saturating_sub(1);
    if self.width != 0 && total > usize::from(self.width) {
      return None;
    }
    let mut out = Vec::new();
    for (r, row) in body.iter().enumerate() {
      let mut line = Vec::new();
      for (i, w) in widths.iter().enumerate() {
        if i > 0 {
          line.push(Seg::new(" │ ", DIM));
        }
        let cell = row.get(i).cloned().unwrap_or_default();
        let pad = w - plain(&cell).width();
        let (left, right) = match aligns.get(i).copied().unwrap_or(Align::Left) {
          Align::Left => (0, pad),
          Align::Right => (pad, 0),
          Align::Center => (pad / 2, pad - pad / 2),
        };
        // The last column is not padded on the right (no trailing blanks).
        let right = if i + 1 == n { 0 } else { right };
        if left > 0 {
          line.push(Seg::new(" ".repeat(left), Style::new()));
        }
        line.extend(cell);
        if right > 0 {
          line.push(Seg::new(" ".repeat(right), Style::new()));
        }
      }
      out.push(line);
      if r == 0 {
        let rule = widths
          .iter()
          .map(|w| "─".repeat(*w))
          .collect::<Vec<_>>()
          .join("─┼─");
        out.push(vec![Seg::new(rule, DIM)]);
      }
    }
    Some(out)
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
  Left,
  Right,
  Center,
}

/// A delimiter row's cell (`---`, `:--`, `--:`, `:-:`), as its alignment.
fn align(cell: &str) -> Option<Align> {
  let c = cell.trim();
  let (l, r) = (c.starts_with(':'), c.ends_with(':') && c.len() > 1);
  let dashes = c.trim_start_matches(':').trim_end_matches(':');
  if dashes.is_empty() || !dashes.chars().all(|ch| ch == '-') {
    return None;
  }
  Some(match (l, r) {
    (true, true) => Align::Center,
    (false, true) => Align::Right,
    _ => Align::Left,
  })
}

/// A table row's cells: split on `|`, not an escaped one or one in code.
fn cells(row: &str) -> Vec<String> {
  let t = row.trim();
  let t = t.strip_prefix('|').unwrap_or(t);
  let t = if t.ends_with('|') && !t.ends_with("\\|") {
    &t[..t.len() - 1]
  } else {
    t
  };
  let mut out = Vec::new();
  let mut cell = String::new();
  let (mut code, mut escaped) = (false, false);
  for c in t.chars() {
    match c {
      '|' if !code && !escaped => out.push(std::mem::take(&mut cell).trim().to_owned()),
      c => {
        if c == '`' && !escaped {
          code = !code;
        }
        escaped = c == '\\' && !escaped;
        cell.push(c);
      }
    }
  }
  out.push(cell.trim().to_owned());
  out
}

fn opens(raw: &str) -> Option<Fence> {
  let indent = raw.len() - raw.trim_start_matches(' ').len();
  if indent > 3 {
    return None;
  }
  let t = &raw[indent..];
  let ch = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
  let len = t.chars().take_while(|&c| c == ch).count();
  // A backtick fence's info string has no backtick.
  (len >= 3 && !(ch == '`' && t[len..].contains('`'))).then_some(Fence { ch, len })
}

fn closes(f: &Fence, raw: &str) -> bool {
  let t = raw.trim_start_matches(' ');
  if raw.len() - t.len() > 3 {
    return false;
  }
  let len = t.chars().take_while(|&c| c == f.ch).count();
  len >= f.len && t[len..].trim().is_empty()
}

fn is_rule(t: &str) -> bool {
  let t = t.trim_end();
  let Some(ch) = t.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else {
    return false;
  };
  t.chars().all(|c| c == ch || c == ' ') && t.chars().filter(|&c| c == ch).count() >= 3
}

/// Inline Markdown: code spans, emphasis, strikethrough, links, escapes.
fn inline(s: &str, base: Style, out: &mut Vec<Seg>) {
  let chars: Vec<char> = s.chars().collect();
  let mut buf = String::new();
  let flush = |buf: &mut String, out: &mut Vec<Seg>| {
    if !buf.is_empty() {
      out.push(Seg::new(std::mem::take(buf), base));
    }
  };
  let mut i = 0;
  'outer: while i < chars.len() {
    let c = chars[i];
    if c == '\\' && chars.get(i + 1).is_some_and(char::is_ascii_punctuation) {
      buf.push(chars[i + 1]);
      i += 2;
      continue;
    }
    if c == '`' {
      let n = run(&chars, i, '`');
      match find_run(&chars, i + n, '`', n) {
        Some(j) => {
          flush(&mut buf, out);
          let mut code: String = chars[i + n..j].iter().collect();
          if code.len() > 2
            && code.starts_with(' ')
            && code.ends_with(' ')
            && !code.trim().is_empty()
          {
            code = code[1..code.len() - 1].to_owned();
          }
          out.push(Seg::new(code, base.patch(CODE)));
          i = j + n;
        }
        None => {
          buf.extend(&chars[i..i + n]);
          i += n;
        }
      }
      continue;
    }
    if c == '<'
      && let Some(len) = chars[i + 1..].iter().position(|&c| c == '>')
    {
      let inner: String = chars[i + 1..i + 1 + len].iter().collect();
      if ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| inner.starts_with(p))
        && !inner.contains(char::is_whitespace)
      {
        flush(&mut buf, out);
        out.push(Seg {
          text: inner.clone(),
          style: base.patch(LINK),
          link: Some(inner),
        });
        i += len + 2;
        continue;
      }
    }
    let image = c == '!' && chars.get(i + 1) == Some(&'[');
    if (c == '[' || image)
      && let Some((text, url, end)) = link(&chars, if image { i + 1 } else { i })
    {
      flush(&mut buf, out);
      let start = out.len();
      inline(&text, base.patch(LINK), out);
      for seg in &mut out[start..] {
        seg.link = Some(url.clone());
      }
      if plain(&out[start..]) != url {
        out.push(Seg {
          text: format!(" ({url})"),
          style: base.patch(DIM),
          link: Some(url),
        });
      }
      i = end;
      continue;
    }
    for (delim, m) in [
      ("**", Modifier::BOLD),
      ("__", Modifier::BOLD),
      ("~~", Modifier::CROSSED_OUT),
      ("*", Modifier::ITALIC),
      ("_", Modifier::ITALIC),
    ] {
      if let Some((inner, end)) = emphasis(&chars, i, delim) {
        flush(&mut buf, out);
        inline(&inner, base.add_modifier(m), out);
        i = end;
        continue 'outer;
      }
    }
    buf.push(c);
    i += 1;
  }
  flush(&mut buf, out);
}

fn run(chars: &[char], i: usize, c: char) -> usize {
  chars[i..].iter().take_while(|&&x| x == c).count()
}

/// Where a run of exactly `n` `c`s starts, from `from` on.
fn find_run(chars: &[char], from: usize, c: char, n: usize) -> Option<usize> {
  let mut j = from;
  while j < chars.len() {
    if chars[j] == c {
      let len = run(chars, j, c);
      if len == n {
        return Some(j);
      }
      j += len;
    } else {
      j += 1;
    }
  }
  None
}

/// `[text](url)` at `i`: its text, URL, and where it ends.
fn link(chars: &[char], i: usize) -> Option<(String, String, usize)> {
  let close = i + 1 + chars[i + 1..].iter().position(|&c| c == ']')?;
  if chars.get(close + 1) != Some(&'(') || close == i + 1 {
    return None;
  }
  let end = close + 2 + chars[close + 2..].iter().position(|&c| c == ')')?;
  let inside: String = chars[close + 2..end].iter().collect();
  // A title after the URL is dropped.
  let url = inside.split_whitespace().next()?.to_owned();
  let url = url
    .strip_prefix('<')
    .and_then(|u| u.strip_suffix('>'))
    .map_or(url.clone(), str::to_owned);
  Some((chars[i + 1..close].iter().collect(), url, end + 1))
}

/// Emphasis by `delim` at `i`: its inner text and where it ends. Opening
/// is followed by, and closing preceded by, a non-blank; `_` is never
/// inside a word (snake_case stays as written).
fn emphasis(chars: &[char], i: usize, delim: &str) -> Option<(String, usize)> {
  let d: Vec<char> = delim.chars().collect();
  let (n, ch) = (d.len(), d[0]);
  let at = |j: usize| chars.get(j).copied();
  let is = |j: usize| chars.get(j..j + n) == Some(&d[..]);
  if !is(i) {
    return None;
  }
  let single = n == 1;
  let before = i.checked_sub(1).and_then(at);
  if at(i + n).is_none_or(char::is_whitespace)
    || (single && (at(i + 1) == Some(ch) || before == Some(ch)))
    || (ch == '_' && before.is_some_and(char::is_alphanumeric))
  {
    return None;
  }
  let mut j = i + n + 1;
  while j + n <= chars.len() {
    if is(j)
      && !chars[j - 1].is_whitespace()
      && !(single && (at(j + 1) == Some(ch) || chars[j - 1] == ch))
      && !(ch == '_' && at(j + n).is_some_and(char::is_alphanumeric))
    {
      return Some((chars[i + n..j].iter().collect(), j + n));
    }
    j += 1;
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  fn shown(text: &str) -> Vec<String> {
    render(text, 80).iter().map(|l| plain(l)).collect()
  }

  fn styled(text: &str) -> Vec<(String, Style)> {
    render(text, 80)
      .into_iter()
      .flatten()
      .map(|s| (s.text, s.style))
      .collect()
  }

  #[test]
  fn emphasis_and_code_lose_their_markers() {
    assert_eq!(
      shown("a **bold** and *it* and `co*de*` and ~~gone~~"),
      ["a bold and it and co*de* and gone"]
    );
    let s = styled("**b** `c`");
    assert!(s[0].1.add_modifier.contains(Modifier::BOLD));
    assert_eq!(s[2], ("c".into(), CODE));
  }

  #[test]
  fn what_is_not_emphasis_stays_as_written() {
    for t in [
      "snake_case_name and 2 * 3 * 4",
      "a ** b",
      "unclosed **bold and `code",
      "file_name.rs",
    ] {
      assert_eq!(shown(t), [t], "{t}");
    }
    assert_eq!(shown(r"\*not\* it"), ["*not* it"]);
  }

  #[test]
  fn blocks_render() {
    assert_eq!(
      shown(
        "# Title\n## Sub\n- one\n  * two\n- [ ] todo\n- [x] done\n3. three\n> quoted *it*\n---"
      ),
      [
        "Title",
        "Sub",
        "• one",
        "  • two",
        "☐ todo",
        "☑ done",
        "3. three",
        "│ quoted it",
        &"─".repeat(60),
      ]
    );
  }

  #[test]
  fn code_blocks_keep_their_text_and_fences() {
    let text = "```rust\nlet x = **y**; # no\n- no\n```\n**yes**";
    assert_eq!(
      shown(text),
      ["```rust", "let x = **y**; # no", "- no", "```", "yes"]
    );
    let lines = render(text, 80);
    assert_eq!(lines[0][0].style, DIM);
    assert_eq!(lines[1][0].style, CODE);
  }

  #[test]
  fn a_fence_spans_pushes() {
    let mut md = Md::new(80);
    md.push("~~~~");
    let code: Vec<_> = md
      .push("# still code\n~~~")
      .iter()
      .map(|l| plain(l))
      .collect();
    assert_eq!(code, ["# still code", "~~~"]);
    assert_eq!(plain(&md.push("~~~~")[0]), "~~~~");
    assert_eq!(plain(&md.push("# heading")[0]), "heading");
  }

  #[test]
  fn links_keep_their_url() {
    let l = &render("see [the docs](https://x.y/z \"t\") or <https://a.b>", 80)[0];
    assert_eq!(plain(l), "see the docs (https://x.y/z) or https://a.b");
    assert_eq!(l[1].link.as_deref(), Some("https://x.y/z"));
    assert_eq!(shown("[https://a.b](https://a.b)"), ["https://a.b"]);
  }

  #[test]
  fn tables_are_held_till_whole_then_aligned() {
    let mut md = Md::new(80);
    assert!(md.push("| name | n |\n|:--|--:|\n| **a** | 1 |").is_empty());
    assert_eq!(
      md.preview("| bb | 22")
        .iter()
        .map(|l| plain(l))
        .collect::<Vec<_>>(),
      ["name │  n", "─────┼───", "a    │  1", "bb   │ 22"]
    );
    let out: Vec<_> = md
      .push("| bb | 22 |\nafter")
      .iter()
      .map(|l| plain(l))
      .collect();
    assert_eq!(
      out,
      ["name │  n", "─────┼───", "a    │  1", "bb   │ 22", "after"]
    );
  }

  #[test]
  fn a_table_too_wide_or_without_its_delimiter_is_left_as_written() {
    let wide = format!("| {} | b |\n|---|---|\n| c | d |", "a".repeat(90));
    assert_eq!(shown(&wide).len(), 3);
    assert!(shown(&wide)[1].starts_with("|---"));
    assert_eq!(shown("| just | pipes |"), ["| just | pipes |"]);
  }

  #[test]
  fn cells_split_outside_code_and_escapes() {
    assert_eq!(cells("| a `b|c` | d\\|e | f |"), ["a `b|c`", "d\\|e", "f"]);
  }
}
