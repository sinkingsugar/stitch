//! The line log: the output stream rendered the way a terminal shows lines
//! (carriage returns overwrite, erase-line erases, escape sequences vanish),
//! so a progress bar ends as its last state and colours leave no trace.
//! Cursor addressing beyond the current line is not followed: full-screen
//! programs are read from the screen instead. Prompt markers are noticed
//! here, at their exact place in the output.

use std::collections::VecDeque;

use vte::{Params, Perform};

/// A prompt the shell printed: the last command's exit status, and where in
/// the log it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
  pub status: i32,
  /// Absolute index of the line the marker arrived on.
  pub line: u64,
  /// That line's text before the marker (output without a final newline).
  pub partial: String,
}

pub(crate) struct LineLog {
  lines: VecDeque<String>,
  /// Lines dropped from the front to bound memory.
  dropped: u64,
  cur: Vec<char>,
  col: usize,
  max_lines: usize,
  nonce: String,
  pub markers: Vec<Marker>,
  /// The first lines from `pin` on, kept when they roll off the log, so a
  /// long command's output keeps its beginning.
  pin: Option<u64>,
  pinned: Vec<String>,
}

/// Lines kept from the start of a pinned range.
const PINNED_LINES: usize = 2_000;

impl LineLog {
  pub fn new(nonce: String, max_lines: usize) -> Self {
    LineLog {
      lines: VecDeque::new(),
      dropped: 0,
      cur: Vec::new(),
      col: 0,
      max_lines,
      nonce,
      markers: Vec::new(),
      pin: None,
      pinned: Vec::new(),
    }
  }

  /// Keeps the beginning of what follows from line `from`.
  pub fn pin(&mut self, from: u64) {
    self.pin = Some(from);
    self.pinned.clear();
  }

  /// The absolute index of the line being written.
  pub fn current_line(&self) -> u64 {
    self.dropped + self.lines.len() as u64
  }

  fn current_text(&self) -> String {
    self.cur.iter().collect::<String>().trim_end().to_owned()
  }

  /// Whether something was shown after `marker`: the prompt itself (the
  /// rc prints the marker first, then the shell its prompt).
  pub fn shown_after(&self, marker: &Marker) -> bool {
    self.current_line() > marker.line || self.current_text().len() > marker.partial.len()
  }

  fn newline(&mut self) {
    let text = self.current_text();
    self.lines.push_back(text);
    self.cur.clear();
    self.col = 0;
    while self.lines.len() > self.max_lines {
      let line = self.lines.pop_front().expect("longer than the limit");
      if let Some(p) = self.pin
        && self.dropped >= p
        && self.pinned.len() < PINNED_LINES
      {
        self.pinned.push(line);
      }
      self.dropped += 1;
    }
  }

  /// Lines `from..to` (absolute; `to` exclusive, `None` up to and including
  /// the current line), joined with newlines. Dropped lines are skipped.
  pub fn text(&self, from: u64, to: Option<u64>) -> String {
    let end = to.unwrap_or(self.current_line() + 1);
    let mut out: Vec<String> = Vec::new();
    if from < self.dropped && self.pin == Some(from) {
      out.extend(self.pinned.iter().cloned());
      let gone = self.dropped - from - self.pinned.len() as u64;
      if gone > 0 {
        out.push(format!("[… {gone} lines omitted …]"));
      }
    }
    let mut i = from.max(self.dropped);
    while i < end {
      let idx = (i - self.dropped) as usize;
      match self.lines.get(idx) {
        Some(l) => out.push(l.clone()),
        None if i == self.current_line() => out.push(self.current_text()),
        None => break,
      }
      i += 1;
    }
    out.join("\n")
  }

  fn put(&mut self, c: char) {
    if self.col < self.cur.len() {
      self.cur[self.col] = c;
    } else {
      while self.cur.len() < self.col {
        self.cur.push(' ');
      }
      self.cur.push(c);
    }
    self.col += 1;
  }
}

fn first_param(params: &Params, default: usize) -> usize {
  params
    .iter()
    .next()
    .and_then(|p| p.first().copied())
    .map(|v| if v == 0 { default } else { v as usize })
    .unwrap_or(default)
}

impl Perform for LineLog {
  fn print(&mut self, c: char) {
    self.put(c);
  }

  fn execute(&mut self, byte: u8) {
    match byte {
      b'\n' => self.newline(),
      b'\r' => self.col = 0,
      0x08 => self.col = self.col.saturating_sub(1),
      b'\t' => {
        let next = (self.col / 8 + 1) * 8;
        while self.col < next {
          self.put(' ');
        }
      }
      _ => {}
    }
  }

  fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
    match action {
      // Erase in line: to the end, to the start, or all of it.
      'K' => match params
        .iter()
        .next()
        .and_then(|p| p.first().copied())
        .unwrap_or(0)
      {
        0 => self.cur.truncate(self.col),
        1 => {
          for c in self.cur.iter_mut().take(self.col + 1) {
            *c = ' ';
          }
        }
        _ => self.cur.clear(),
      },
      'G' => self.col = first_param(params, 1) - 1,
      'C' => self.col += first_param(params, 1),
      'D' => self.col = self.col.saturating_sub(first_param(params, 1)),
      _ => {}
    }
  }

  fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
    // `777;stitch;<nonce>;prompt;<status>`, printed by the session's rc.
    if let [b"777", b"stitch", nonce, b"prompt", status] = params
      && *nonce == self.nonce.as_bytes()
    {
      let status = std::str::from_utf8(status)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);
      self.markers.push(Marker {
        status,
        line: self.current_line(),
        partial: self.current_text(),
      });
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn feed(log: &mut LineLog, bytes: &[u8]) {
    let mut p = vte::Parser::new();
    p.advance(log, bytes);
  }

  #[test]
  fn carriage_returns_and_erases_leave_the_last_state() {
    let mut log = LineLog::new("n".into(), 100);
    feed(
      &mut log,
      b"\x1b[32mgreen\x1b[0m\n10%\r50%\r100%\ndone\x1b[2K\rcleared\nab\x08c\ttab\n",
    );
    assert_eq!(log.text(0, None), "green\n100%\ncleared\nac      tab\n");
  }

  #[test]
  fn markers_need_the_nonce_and_record_their_place() {
    let mut log = LineLog::new("s3cret".into(), 100);
    feed(&mut log, b"out\npartial\x1b]777;stitch;guess;prompt;0\x07");
    assert!(log.markers.is_empty());
    feed(&mut log, b"\x1b]777;stitch;s3cret;prompt;3\x07");
    assert_eq!(
      log.markers,
      vec![Marker {
        status: 3,
        line: 1,
        partial: "partial".into()
      }]
    );
    let m = log.markers[0].clone();
    assert!(!log.shown_after(&m), "the prompt comes after its marker");
    feed(&mut log, b"$ ");
    assert!(log.shown_after(&m));
  }

  #[test]
  fn old_lines_are_dropped_and_indices_stay_absolute() {
    let mut log = LineLog::new("n".into(), 3);
    for i in 0..10 {
      feed(&mut log, format!("line {i}\n").as_bytes());
    }
    assert_eq!(log.current_line(), 10);
    assert_eq!(log.text(0, Some(10)), "line 7\nline 8\nline 9");
    assert_eq!(log.text(8, Some(9)), "line 8");
  }

  #[test]
  fn a_pinned_range_keeps_its_beginning() {
    let mut log = LineLog::new("n".into(), 3);
    feed(&mut log, b"before\n");
    log.pin(1);
    for i in 0..10 {
      feed(&mut log, format!("out {i}\n").as_bytes());
    }
    let text = log.text(1, Some(log.current_line()));
    assert!(text.starts_with("out 0\nout 1\n"), "{text}");
    assert!(text.ends_with("out 8\nout 9"), "{text}");
  }
}
