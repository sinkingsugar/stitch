//! tmux's control mode (`tmux -C`), as a remote session speaks it.
//!
//! A tmux client in control mode reads commands, one a line, and writes
//! notifications: `%output %<pane> <bytes>` is what the pane's program
//! printed, unchanged except that bytes below a space and `\` are written
//! as `\ooo` (octal); a command's reply is the lines between `%begin` and
//! `%end` (or `%error`), whose third number is 1 for a command this client
//! sent; `%exit` ends it. So a session in tmux reads the program's own
//! output (its marker included), not tmux's redrawing of the pane.

/// A line of what tmux writes, parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
  /// A pane's output, decoded.
  Output { pane: String, bytes: Vec<u8> },
  /// A reply starts; `ours`: to a command this client sent.
  Begin { ours: bool },
  /// It ended, `ok` or with an error.
  End { ok: bool },
  /// The client is done (the session ended, or it was detached).
  Exit(String),
  /// Another notification, or a line of a reply.
  Other(Vec<u8>),
}

pub fn parse(line: &[u8]) -> Line {
  let line = line.strip_suffix(b"\r").unwrap_or(line);
  if let Some(rest) = line.strip_prefix(b"%output ") {
    let at = rest.iter().position(|b| *b == b' ').unwrap_or(rest.len());
    let pane = String::from_utf8_lossy(&rest[..at]).into_owned();
    let data = rest.get(at + 1..).unwrap_or(&[]);
    return Line::Output {
      pane,
      bytes: unescape(data),
    };
  }
  if let Some(rest) = line.strip_prefix(b"%begin ") {
    // `%begin <time> <number> <flags>`: flags 1 for this client's.
    let ours = String::from_utf8_lossy(rest)
      .split_whitespace()
      .nth(2)
      .is_none_or(|f| f.parse::<u32>().is_ok_and(|f| f & 1 == 1));
    return Line::Begin { ours };
  }
  if line.starts_with(b"%end ") || line == b"%end" {
    return Line::End { ok: true };
  }
  if line.starts_with(b"%error ") || line == b"%error" {
    return Line::End { ok: false };
  }
  if line == b"%exit" || line.starts_with(b"%exit ") {
    let why = String::from_utf8_lossy(line.get(6..).unwrap_or(&[]))
      .trim()
      .to_owned();
    return Line::Exit(why);
  }
  Line::Other(line.to_vec())
}

/// `\ooo` back to the byte.
pub fn unescape(data: &[u8]) -> Vec<u8> {
  let mut out = Vec::with_capacity(data.len());
  let mut i = 0;
  while i < data.len() {
    if data[i] == b'\\'
      && i + 4 <= data.len()
      && data[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b))
    {
      let v = data[i + 1..i + 4]
        .iter()
        .fold(0u32, |v, d| v * 8 + u32::from(d - b'0'));
      out.push((v & 0xff) as u8);
      i += 4;
    } else {
      out.push(data[i]);
      i += 1;
    }
  }
  out
}

/// `send-keys` for `bytes`, as lines of at most a few hundred bytes each:
/// each byte a key in hex (`-H`), so nothing is read as a key name.
pub fn send_keys(pane: &str, bytes: &[u8]) -> Vec<String> {
  bytes
    .chunks(256)
    .map(|chunk| {
      let mut line = format!("send-keys -t {pane} -H");
      for b in chunk {
        line.push_str(&format!(" {b:02x}"));
      }
      line.push('\n');
      line
    })
    .collect()
}

/// A word quoted for a tmux command line (or a POSIX shell): single
/// quotes, with any inside closed, escaped and opened again.
pub fn quote(word: &str) -> String {
  format!("'{}'", word.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn control_mode_lines_are_read() {
    assert_eq!(
      parse(b"%output %0 echo hi\\015\\012\\134x\\033]777;"),
      Line::Output {
        pane: "%0".into(),
        bytes: b"echo hi\r\n\\x\x1b]777;".to_vec()
      }
    );
    assert_eq!(
      parse("%output %3 é".as_bytes()),
      Line::Output {
        pane: "%3".into(),
        bytes: "é".as_bytes().to_vec()
      }
    );
    assert_eq!(
      parse(b"%begin 1792563380 310 0"),
      Line::Begin { ours: false }
    );
    assert_eq!(
      parse(b"%begin 1792563380 317 1"),
      Line::Begin { ours: true }
    );
    assert_eq!(parse(b"%end 1792563380 317 1"), Line::End { ok: true });
    assert_eq!(parse(b"%error 1792563380 317 1"), Line::End { ok: false });
    assert_eq!(parse(b"%exit"), Line::Exit(String::new()));
    assert_eq!(parse(b"%exit detached"), Line::Exit("detached".into()));
    assert_eq!(
      parse(b"%window-add @0"),
      Line::Other(b"%window-add @0".to_vec())
    );
    // A backslash not followed by three octal digits is itself.
    assert_eq!(unescape(b"a\\9b\\01"), b"a\\9b\\01");
  }

  #[test]
  fn keys_are_sent_as_hex() {
    assert_eq!(send_keys("%1", b"ls\r"), ["send-keys -t %1 -H 6c 73 0d\n"]);
    assert_eq!(send_keys("%1", &[b'x'; 600]).len(), 3);
    assert_eq!(quote("it's"), "'it'\\''s'");
  }
}
