//! A shell session on a PTY.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pty_process::Size;
use rustix::process::{Pid, Signal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, watch};
use tokio::task::AbortHandle;

use crate::Result;
use crate::log::{LineLog, Marker};
use crate::remote::{Remote, RemoteConfig, Resumed};
use crate::sandbox::Sandbox;
use crate::spawn::{self, Spawn};

/// Lines kept in the line log.
const LOG_LINES: usize = 20_000;
/// Rows kept above the screen.
const SCROLLBACK: usize = 2_000;
/// How long a new shell has to show its first prompt.
const STARTUP: Duration = Duration::from_secs(10);
/// The same for a remote shell, which reads the user's profile first.
const REMOTE_STARTUP: Duration = Duration::from_secs(30);
/// A command longer than this (bytes), or of several lines (typed, each line
/// would get its own prompt, and the first would end the wait), is written
/// to a file and sourced, not typed: a terminal in canonical mode (bash
/// without line editing) holds at most 1024 bytes of input; macOS keeps 1019
/// of a longer line and drops the rest, its Enter included.
const TYPED_MAX: usize = 768;
/// How long a write may wait for the program to take its input: one that
/// does not read leaves the rest unsent, and the session's input free for
/// what comes next (an interrupt's Ctrl+C).
const WRITE_MAX: Duration = Duration::from_secs(5);

/// The longest a command waits for the prompt to follow its marker (a
/// prompt that prints nothing waits this long each time).
const PROMPT_SHOWN: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("pty: {0}")]
  Pty(#[from] pty_process::Error),
  #[error(transparent)]
  PtyOpen(#[from] crate::pty::OpenError),
  #[error("{0}")]
  Io(#[from] std::io::Error),
  #[error("the shell did not start: {0}")]
  Startup(String),
  #[error("a program is running ({program}): send it input or interrupt it first")]
  Busy { program: String },
  #[error("the session has ended")]
  Closed,
  /// A person typed into the session (attached) and was not told yet:
  /// nothing was sent on top of it.
  #[error("a person typed into the session: {0}")]
  Typed(String),
  #[error("the program took no input for {0:?}: the rest of it was not sent")]
  Stalled(Duration),
}

/// Which shell runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ShellKind {
  /// bash without the user's profile or rc, without line editing: the
  /// predictable default for agents.
  #[default]
  Bash,
  /// The user's `$SHELL` (bash or zsh) with their rc, plus the marker.
  User,
}

#[derive(Clone, Debug)]
pub struct Config {
  pub cwd: PathBuf,
  /// Added to the node's environment; with `clean_env`, the whole of it.
  pub env: Vec<(String, String)>,
  /// The environment is `env` alone (the client's), not the node's.
  pub clean_env: bool,
  /// Variables taken out of it, whoever set them (a fixer's shells: the
  /// SSH agent and the forges' tokens).
  pub env_remove: Vec<String>,
  pub rows: u16,
  pub cols: u16,
  pub shell: ShellKind,
  /// Output longer than this many characters is cut in the middle.
  pub max_output: usize,
  /// The shell helper binary (`stitch-shell-helper`), which forks shells
  /// on macOS; without it they start through `/bin/sh`. Defaults to
  /// `STITCH_SHELL_HELPER`.
  pub helper: Option<PathBuf>,
  /// Where the shell and what it runs may write; none: anywhere.
  pub sandbox: Option<Sandbox>,
  /// Applied to a command's output before it is cut to `max_output`, so
  /// what it hides is never cut in two (the host's secrets).
  pub redact: Option<Redact>,
}

/// What `Config::redact` holds.
#[derive(Clone)]
pub struct Redact(pub std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>);

impl std::fmt::Debug for Redact {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("Redact")
  }
}

impl Config {
  pub fn new(cwd: impl Into<PathBuf>) -> Self {
    Config {
      cwd: cwd.into(),
      env: Vec::new(),
      clean_env: false,
      env_remove: Vec::new(),
      rows: 40,
      cols: 120,
      shell: ShellKind::Bash,
      max_output: 30_000,
      helper: std::env::var_os("STITCH_SHELL_HELPER").map(PathBuf::from),
      sandbox: None,
      redact: None,
    }
  }
}

/// How long a call waits.
#[derive(Clone, Debug)]
pub struct Wait {
  /// The longest it waits for the prompt to come back.
  pub timeout: Duration,
  /// Return early once the screen has been quiet this long (for programs
  /// that stop to ask something); a spinner or a ticking counter is quiet
  /// (see `Session::quiet_view`). `None` waits for the prompt or the timeout.
  pub idle: Option<Duration>,
  /// Return as soon as the screen shows this.
  pub screen: Option<regex::Regex>,
}

impl Wait {
  pub fn prompt(timeout: Duration) -> Self {
    Wait {
      timeout,
      idle: None,
      screen: None,
    }
  }
}

/// Sends `signal` to process group `pgrp` (none: only checks); whether it
/// has a process left. For what is left of a command's group once the
/// command ended: a child that ignored Ctrl+C, orphaned in it.
pub fn signal_group(pgrp: i32, signal: Option<StopSignal>) -> bool {
  let Some(pid) = Pid::from_raw(pgrp).filter(|p| p.as_raw_nonzero().get() > 1) else {
    return false;
  };
  match signal {
    None => rustix::process::test_kill_process_group(pid).is_ok(),
    Some(s) => {
      let sig = match s {
        StopSignal::Term => Signal::TERM,
        StopSignal::Kill => Signal::KILL,
      };
      rustix::process::kill_process_group(pid, sig).is_ok()
    }
  }
}

/// What `Session::signal_command` sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopSignal {
  Term,
  Kill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
  /// The shell waits for a command; the last one exited with `exit`.
  AtPrompt {
    exit: i32,
  },
  /// A program holds the terminal.
  Running {
    pid: i32,
    program: String,
  },
  Closed,
}

#[derive(Clone, Debug)]
pub struct Outcome {
  pub status: Status,
  /// What was printed, as lines; cut in the middle when long.
  pub output: String,
  pub truncated: bool,
  /// The screen, when a full-screen program is showing.
  pub screen: Option<String>,
  pub elapsed: Duration,
}

/// What `wait_for` waits for.
#[derive(Clone, Debug)]
pub enum Until {
  /// The command to end (its prompt back).
  Done,
  /// A line of its output to match.
  Line(regex::Regex),
  /// The screen, every condition given at once (for full-screen programs).
  Screen(ScreenUntil),
}

/// Conditions on what the screen shows; those given must all hold.
#[derive(Clone, Debug, Default)]
pub struct ScreenUntil {
  /// It shows this.
  pub shows: Option<regex::Regex>,
  /// It no longer shows this.
  pub gone: Option<regex::Regex>,
  /// It has not changed for this long (spinners and counters aside).
  pub quiet: Option<Duration>,
}

impl ScreenUntil {
  /// Whether the screen is so; `gone_seen`: what must be gone was shown
  /// (gone from a screen that never showed it is no news). Both look row
  /// by row, the echo of what was sent aside, as a match does.
  fn holds(&self, text: &str, news: &News, quiet_for: Duration, gone_seen: bool) -> bool {
    self
      .shows
      .as_ref()
      .is_none_or(|re| !news.matches(re, text).is_empty())
      && self
        .gone
        .as_ref()
        .is_none_or(|re| gone_seen && news.shown(re, text).is_empty())
      && self.quiet.is_none_or(|q| quiet_for >= q)
  }
}

/// Typed bytes as a person reads them: text as is, Enter `⏎`, keys named.
fn readable(input: &[u8]) -> String {
  let text = String::from_utf8_lossy(input);
  let mut out = String::new();
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    match c {
      '\r' | '\n' => out.push('⏎'),
      '\u{7f}' | '\u{8}' => out.push('⌫'),
      '\t' => out.push('⇥'),
      '\u{1b}' => {
        // An escape sequence: the arrows by name, others as one key.
        let mut seq = String::new();
        if chars.peek() == Some(&'[') || chars.peek() == Some(&'O') {
          seq.push(chars.next().expect("peeked"));
          while let Some(&n) = chars.peek() {
            seq.push(n);
            chars.next();
            if n.is_ascii_alphabetic() || n == '~' {
              break;
            }
          }
        }
        out.push_str(match seq.as_str() {
          "[A" | "OA" => "‹up›",
          "[B" | "OB" => "‹down›",
          "[C" | "OC" => "‹right›",
          "[D" | "OD" => "‹left›",
          "[200~" | "[201~" => "",
          "" => "‹esc›",
          _ => "‹key›",
        });
      }
      c if (1..=26).contains(&(c as u32)) => {
        out.push_str(&format!("‹ctrl-{}›", char::from(b'a' + (c as u8) - 1)));
      }
      c if c.is_control() => out.push_str("‹key›"),
      c => out.push(c),
    }
  }
  out
}

/// Spinner glyphs.
fn spinner(c: char) -> bool {
  ('\u{2800}'..='\u{28FF}').contains(&c) || "◐◓◑◒◴◷◶◵○◉●◌✻✶✳✢✽✺✹✷✸✦✧⋆∗·•".contains(c)
}

/// What follows the spinner on a line a spinner animates: one whose first
/// non-blank character is a spinner glyph, or that starts, unindented,
/// with an ASCII frame (`|`, `/`, `-` or `\`) and a space. A glyph
/// further in (a `·` between words) or an indented `- ` (a list item) is
/// no spinner's.
fn spun(line: &str) -> Option<&str> {
  let t = line.trim_start();
  let c = t.chars().next()?;
  if spinner(c) {
    return Some(&t[c.len_utf8()..]);
  }
  let b = line.as_bytes();
  (b.len() > 1 && matches!(b[0], b'|' | b'/' | b'-' | b'\\') && b[1] == b' ').then(|| &line[1..])
}

/// The rest of a spinner's line as quiet sees it: without its glyphs and
/// the numbers that tick (elapsed seconds, a token count); a percentage
/// and a count of a total (`45%`, `3/10`) are progress, and kept.
fn unspun(rest: &str, mut keep: impl FnMut(char)) {
  let chars: Vec<char> = rest.chars().collect();
  let mut i = 0;
  while i < chars.len() {
    let c = chars[i];
    if !c.is_ascii_digit() {
      if !spinner(c) {
        keep(c);
      }
      i += 1;
      continue;
    }
    // A number, decimals and all.
    let mut end = i;
    while end < chars.len()
      && (chars[end].is_ascii_digit()
        || chars[end] == '.' && chars.get(end + 1).is_some_and(char::is_ascii_digit))
    {
      end += 1;
    }
    let next = chars.get(end).copied();
    let progress = next == Some('%')
      || next == Some('/') && chars.get(end + 1).is_some_and(char::is_ascii_digit)
      || i >= 2 && chars[i - 1] == '/' && chars[i - 2].is_ascii_digit();
    if progress {
      chars[i..end].iter().copied().for_each(&mut keep);
    }
    i = end;
  }
}

/// The screen's text, and a fingerprint of it that spinners do not move: a
/// line a spinner animates (`spun`) counts without its glyphs and the
/// numbers that tick (the elapsed seconds, a token count), a percentage or
/// a count of a total kept; any other line counts whole, digits included
/// (a test counter, a download percentage), so its progress is no quiet.
fn view_of(screen: &vt100::Screen) -> (String, u64) {
  use std::hash::{Hash, Hasher};
  let text = screen.contents();
  let mut h = std::collections::hash_map::DefaultHasher::new();
  for line in text.lines() {
    match spun(line) {
      Some(rest) => unspun(rest, |c| c.hash(&mut h)),
      None => line.hash(&mut h),
    }
    '\n'.hash(&mut h);
  }
  (text, h.finish())
}

/// How many texts sent are kept to tell their echo (text, then the Enter
/// sent on its own, then a key or two).
const SENT_KEPT: usize = 4;

/// A run of echoed lines this long (letters and digits) on its own line is
/// the echo of what was sent, not an answer quoting a word of it.
const ECHO_LONG: usize = 24;

/// The letters and digits of `text`: what wrapping, indenting, re-rendering
/// and a program's decorations (borders, prompt marks) leave alone.
fn echo_key(text: &str) -> String {
  text.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// What typed bytes say, escape sequences (a paste's markers, keys) left out.
fn typed_text(input: &[u8]) -> String {
  let text = String::from_utf8_lossy(input);
  let mut out = String::new();
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '\u{1b}' {
      out.push(c);
      continue;
    }
    match chars.next() {
      Some('[') => {
        // A CSI sequence: parameters, then its final byte.
        for n in chars.by_ref() {
          if ('@'..='~').contains(&n) {
            break;
          }
        }
      }
      Some('O') => {
        chars.next();
      }
      _ => {}
    }
  }
  out
}

/// A text sent, and where it went: the screen's lines above the cursor's
/// line when it was written, and what that line held then (the prompt).
/// Its echo is looked for from that line on, followed as the screen
/// scrolls.
#[derive(Clone, Debug, Default)]
struct Sent {
  text: String,
  above: Vec<String>,
  at: String,
}

impl Sent {
  /// `text` written with the screen as `screen` shows it.
  fn on(text: String, screen: &vt100::Screen) -> Sent {
    let content = screen.contents();
    let lines: Vec<&str> = content.lines().collect();
    // Wrapped rows are one line of `contents`: the cursor's line is the
    // count of the rows above it that end a line.
    let (row, _) = screen.cursor_position();
    let line = (0..row).filter(|r| !screen.row_wrapped(*r)).count();
    Sent {
      text,
      above: (0..line)
        .map(|i| lines.get(i).copied().unwrap_or("").to_owned())
        .collect(),
      at: lines.get(line).copied().unwrap_or("").trim_end().to_owned(),
    }
  }

  /// The line of `lines` where its echo starts: the cursor's line when it
  /// was written, moved up as far as the screen scrolled since (the lines
  /// above it, the top one perhaps cut by the scrolling, are found again
  /// above it). `None` when that cannot be told (it scrolled away, or a
  /// program redrew the screen).
  fn start(&self, lines: &[&str]) -> Option<usize> {
    let n = self.above.len();
    (0..=n).find_map(|k| {
      let line = n - k;
      let at = lines.get(line).copied().unwrap_or("");
      let kept = &self.above[k..];
      let same = kept.iter().enumerate().all(|(i, was)| {
        let now = lines.get(i).copied().unwrap_or("");
        if i == 0 && k > 0 {
          was.ends_with(now)
        } else {
          now == was
        }
      });
      (same && at.starts_with(&self.at)).then_some(line)
    })
  }
}

/// Which lines of a screen echo what was sent. The echo of a text starts
/// at the line where it was written (`Sent::start`): from there, a run of
/// lines (blank and decoration-only lines aside) whose letters and digits,
/// joined, are a stretch of the text, however the terminal or a program
/// wrapped, indented or framed it, and the runs right after it (a program
/// showing the text again); the first line that is not ends it, and what
/// follows is output, whatever it repeats of the text. A run is echo when
/// it is the whole text, or long and spans lines (the first run may be a
/// line alone); the prompt before a command on its line is part of its
/// echo. Above that line (a program redrawing its input box and history
/// there), or everywhere when where it was written cannot be told, a run
/// is echo when it spans lines, is the whole text, or is long; a short
/// line alone that is a word of the text (`STEP1-DONE` printed as an
/// answer) is news.
fn echoed(lines: &[&str], sent: &[Sent]) -> Vec<bool> {
  let mut echo = vec![false; lines.len()];
  for one in sent {
    let whole = echo_key(&one.text);
    if whole.is_empty() {
      continue;
    }
    let start = one.start(lines);
    let mut keys: Vec<String> = lines.iter().map(|l| echo_key(l)).collect();
    // The prompt and what else was on the line before it is no news.
    if let Some(s) = start
      && let Some(first) = one
        .text
        .split(['\r', '\n'])
        .map(str::trim)
        .find(|l| !l.is_empty())
      && let Some(i) = lines.get(s).and_then(|l| l.find(first))
    {
      keys[s] = echo_key(&lines[s][i..]);
    }
    let above = start.unwrap_or(lines.len());
    // Above where it was written: any run that is surely echo.
    let mut run: Vec<usize> = Vec::new();
    let mut joined = String::new();
    let mut close = |run: &mut Vec<usize>, joined: &mut String| {
      if run.len() > 1 || *joined == whole || joined.chars().count() >= ECHO_LONG {
        for &i in run.iter() {
          echo[i] = true;
        }
      }
      run.clear();
      joined.clear();
    };
    for (i, key) in keys.iter().enumerate().take(above) {
      if key.is_empty() {
        continue;
      }
      if !run.is_empty() && whole.contains(&format!("{joined}{key}")) {
        joined.push_str(key);
        run.push(i);
        continue;
      }
      close(&mut run, &mut joined);
      if whole.contains(key.as_str()) {
        joined.push_str(key);
        run.push(i);
      }
    }
    close(&mut run, &mut joined);
    // From there: the runs that follow one another, up to the first line
    // that is not echo.
    let Some(start) = start else { continue };
    let mut first = true;
    let mut i = start;
    'block: while i < lines.len() {
      let mut run: Vec<usize> = Vec::new();
      let mut joined = String::new();
      while i < lines.len() {
        let key = &keys[i];
        if key.is_empty() {
          i += 1;
          continue;
        }
        if whole.contains(&format!("{joined}{key}")) {
          joined.push_str(key);
          run.push(i);
          i += 1;
          continue;
        }
        break;
      }
      let long = joined.chars().count() >= ECHO_LONG;
      // Still being drawn: the screen read while the terminal echoes the
      // text, its start alone on the last line.
      let drawing = first && i == lines.len() && whole.starts_with(joined.as_str());
      if run.is_empty() || !(joined == whole || drawing || long && (first || run.len() > 1)) {
        break 'block;
      }
      for &r in &run {
        echo[r] = true;
      }
      first = false;
    }
  }
  echo
}

/// What tells a screen match that is news: the texts sent lately and where
/// (their echo is no answer, however wrapped; the command typed at the
/// prompt is one, its line prompt and all), and the screen before the last
/// input (what it matched then is no news).
#[derive(Clone, Debug, Default)]
struct News {
  sent: Vec<Sent>,
  before: String,
}

impl News {
  /// The lines of `text` that `re` matches, the echo of what was sent left
  /// out.
  fn shown(&self, re: &regex::Regex, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let echo = echoed(&lines, &self.sent);
    lines
      .iter()
      .zip(echo)
      .filter(|(l, echo)| !echo && re.is_match(l))
      .map(|(l, _)| l.trim_end().to_owned())
      .collect()
  }

  /// `shown` for what was printed since the last input (the line log
  /// from the input's line): the texts sent are echoed from its start.
  fn shown_since(&self, re: &regex::Regex, printed: &str) -> Vec<String> {
    News {
      sent: self
        .sent
        .iter()
        .map(|s| Sent {
          text: s.text.clone(),
          ..Sent::default()
        })
        .collect(),
      before: String::new(),
    }
    .shown(re, printed)
  }

  /// The new matches: lines of `text` that `re` matches, not echo, and
  /// more of them than the screen before showed (count by count).
  fn matches(&self, re: &regex::Regex, text: &str) -> Vec<String> {
    let mut before = self.shown(re, &self.before);
    self
      .shown(re, text)
      .into_iter()
      .filter(|l| match before.iter().position(|b| b == l) {
        Some(i) => {
          before.remove(i);
          false
        }
        None => true,
      })
      .collect()
  }
}

#[derive(Clone, Debug)]
pub struct Waited {
  /// The line that matched.
  pub matched: Option<String>,
  /// The command ended (or the session did).
  pub done: bool,
  /// What it printed so far, and the session's state.
  pub outcome: Outcome,
}

/// What a call writes.
enum Input {
  /// Sent as is.
  Bytes(Vec<u8>),
  /// Typed at the prompt (or sourced, when long).
  Command(String),
}

/// The command last entered at the prompt.
#[derive(Clone, Debug)]
pub struct Command {
  /// Its first line.
  pub text: String,
  /// It has not ended.
  pub running: bool,
  pub started: Instant,
}

/// Where the command last entered at the prompt began.
#[derive(Clone, Debug)]
struct Entered {
  text: String,
  /// Its first output line.
  from: u64,
  /// Prompts seen before it: one more and it ended.
  prompts: u64,
  started: Instant,
}

#[derive(Clone, Debug)]
pub struct Screen {
  pub text: String,
  pub cursor: (u16, u16),
  pub alternate: bool,
  pub status: Status,
}

struct State {
  log: LineLog,
  log_parser: vte::Parser,
  screen: vt100::Parser,
  prompts: u64,
  last_prompt: Option<Marker>,
  ended: bool,
  /// A line was entered at the prompt and its prompt has not come back:
  /// the shell is busy even when it holds the terminal itself (a builtin
  /// such as `read` waiting for input).
  pending: bool,
  entered: Option<Entered>,
  /// What a person typed (attached), not yet told to the agent.
  typed: String,
  /// The command typed before its Enter (`name_command`).
  named: Option<String>,
  /// Why its output ended: the end of output, or a read error.
  end_why: Option<String>,
  /// How the shell ended, once reaped: exit status or signal.
  exited: Option<String>,
  /// The log line when input was last written: what was shown after it
  /// counts as seen by a wait that starts later (`ScreenUntil::gone`).
  input_line: u64,
  /// The texts sent lately, the command typed, and the screen before the
  /// last input: what a screen match must not be to be news.
  news: News,
  /// Files long commands are sourced from, with the prompts seen before
  /// each was entered: removed once one more came.
  sourced: Vec<(PathBuf, u64)>,
}

/// Where the shell runs: here, on a PTY of this node's, or elsewhere
/// (`Remote`: an SSH channel), where only the marker says what it does.
enum Backend {
  Local(Local),
  Remote(Arc<dyn Remote>),
}

/// What `Session::start` starts from.
struct Start {
  reader: Reader,
  writer: Writer,
  backend: Backend,
  ender: Ender,
  nonce: String,
  rows: u16,
  cols: u16,
  max_output: usize,
  redact: Option<Redact>,
  resumed: Option<Resumed>,
}

/// A shell of this node's.
struct Local {
  /// A duplicate of the PTY master, for terminal queries and resizing.
  control: OwnedFd,
  shell: Pid,
  /// The shell was reaped: its pid, and its group's, may belong to another
  /// process now. Set the moment it is known (`spawn::Child::reaped`).
  reaped: Arc<std::sync::atomic::AtomicBool>,
  /// The session's own directory (owner only), for its lifetime: the rc
  /// file, and the long commands to source.
  dir: tempfile::TempDir,
  /// Started in a sandbox (`Config::sandbox`).
  sandboxed: bool,
}

impl Local {
  fn reaped(&self) -> bool {
    self.reaped.load(std::sync::atomic::Ordering::SeqCst)
  }
}

/// How the reader learns how the shell ended, once its output ended.
enum Ender {
  Local(spawn::Child),
  Remote(Arc<dyn Remote>),
}

impl Ender {
  async fn wait(self) -> String {
    match self {
      Ender::Local(child) => match child.wait().await {
        Ok(status) => exit_text(status),
        Err(e) => format!("waiting for it failed: {e}"),
      },
      Ender::Remote(r) => r.ended().await,
    }
  }
}

type Writer = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;
type Reader = Box<dyn tokio::io::AsyncRead + Send + Unpin>;

struct Inner {
  writer: tokio::sync::Mutex<Writer>,
  backend: Backend,
  state: Mutex<State>,
  /// Bumped on every chunk of output and at the end.
  changed: watch::Sender<u64>,
  output: broadcast::Sender<Vec<u8>>,
  reader: AbortHandle,
  max_output: usize,
  redact: Option<Redact>,
  /// Long commands sourced so far: each one's file is named by its number.
  sourced: std::sync::atomic::AtomicU64,
}

impl Inner {
  fn local(&self) -> Option<&Local> {
    match &self.backend {
      Backend::Local(l) => Some(l),
      Backend::Remote(_) => None,
    }
  }
}

impl Drop for Inner {
  fn drop(&mut self) {
    // Never wait here: signal and let the reader see the end. A shell
    // already reaped is not signalled: its pid, and its group's, may be
    // another process's by now. A remote one ends with its channel.
    if let Some(l) = self.local()
      && !l.reaped()
    {
      let _ = rustix::process::kill_process(l.shell, Signal::HUP);
      let _ = rustix::process::kill_process_group(l.shell, Signal::KILL);
    }
    self.reader.abort();
  }
}

/// How a process ended, as said.
fn exit_text(status: std::process::ExitStatus) -> String {
  use std::os::unix::process::ExitStatusExt;
  match (status.code(), status.signal()) {
    (Some(code), _) => format!("the shell exited with status {code}"),
    (None, Some(sig)) => format!("the shell was killed by signal {sig}{}", signal_name(sig)),
    _ => format!("the shell ended ({status})"),
  }
}

/// " (SIGSEGV)" for 11, for the signals a dying shell is likely to show.
fn signal_name(sig: i32) -> &'static str {
  match sig {
    libc::SIGHUP => " (SIGHUP)",
    libc::SIGINT => " (SIGINT)",
    libc::SIGQUIT => " (SIGQUIT)",
    libc::SIGILL => " (SIGILL)",
    libc::SIGTRAP => " (SIGTRAP)",
    libc::SIGABRT => " (SIGABRT)",
    libc::SIGBUS => " (SIGBUS)",
    libc::SIGFPE => " (SIGFPE)",
    libc::SIGKILL => " (SIGKILL)",
    libc::SIGSEGV => " (SIGSEGV)",
    libc::SIGPIPE => " (SIGPIPE)",
    libc::SIGTERM => " (SIGTERM)",
    _ => "",
  }
}

/// A shell session; clones share it.
#[derive(Clone)]
pub struct Session {
  inner: Arc<Inner>,
}

fn nonce() -> String {
  use rand::RngExt;
  let mut rng = rand::rng();
  (0..24)
    .map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.random_range(0..36)]))
    .collect()
}

// History expansion is off: `!` in a command is just a character, and a
// line is never rejected before it runs for a history reference.
const BASH_RC: &str = r#"PS1='$ '
PS2=''
set +H
unset HISTFILE
export PAGER=cat GIT_PAGER=cat
__stitch_n="$STITCH_SHELL_NONCE"; unset STITCH_SHELL_NONCE
__stitch_prompt() { local s=$?; printf '\033]777;stitch;%s;prompt;%s\007' "$__stitch_n" "$s"; }
PROMPT_COMMAND=__stitch_prompt
"#;

const USER_BASH_RC: &str = r#"[ -f ~/.bashrc ] && . ~/.bashrc
set +H
__stitch_n="$STITCH_SHELL_NONCE"; unset STITCH_SHELL_NONCE
__stitch_prompt() { local s=$?; printf '\033]777;stitch;%s;prompt;%s\007' "$__stitch_n" "$s"; }
PROMPT_COMMAND="__stitch_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#;

const USER_ZSH_RC: &str = r#"[ -f "$HOME/.zshrc" ] && ZDOTDIR="$HOME" . "$HOME/.zshrc"
unsetopt BANG_HIST
__stitch_n="$STITCH_SHELL_NONCE"; unset STITCH_SHELL_NONCE
__stitch_prompt() { local s=$?; printf '\033]777;stitch;%s;prompt;%s\007' "$__stitch_n" "$s"; }
precmd_functions=(__stitch_prompt $precmd_functions)
"#;

fn program_name(pid: Pid) -> String {
  spawn::program_name(pid).unwrap_or_else(|| "a program".into())
}

/// Cuts `text` in the middle when longer than `max` characters.
fn clip(text: &str, max: usize) -> (String, bool) {
  let count = text.chars().count();
  if count <= max {
    return (text.to_owned(), false);
  }
  let head: String = text.chars().take(max / 3).collect();
  let tail: String = text.chars().skip(count - (max - max / 3)).collect();
  let omitted = text.lines().count() - head.lines().count() - tail.lines().count();
  (
    format!("{head}\n[… {omitted} lines omitted …]\n{tail}"),
    true,
  )
}

impl Session {
  /// Starts a shell and waits for its first prompt.
  pub async fn spawn(cfg: Config) -> Result<Session> {
    let rc = tempfile::tempdir()?;
    let nonce = nonce();
    let user_shell = std::env::var("SHELL").unwrap_or_default();
    let mut env: BTreeMap<OsString, OsString> = if cfg.clean_env {
      BTreeMap::new()
    } else {
      std::env::vars_os().collect()
    };
    let (program, args): (&str, Vec<OsString>) = match cfg.shell {
      ShellKind::User if user_shell.ends_with("zsh") => {
        std::fs::write(rc.path().join(".zshrc"), USER_ZSH_RC)?;
        env.insert("ZDOTDIR".into(), rc.path().into());
        (&user_shell, vec!["-i".into()])
      }
      ShellKind::User if user_shell.ends_with("bash") => {
        let path = rc.path().join("bashrc");
        std::fs::write(&path, USER_BASH_RC)?;
        (
          &user_shell,
          vec!["--rcfile".into(), path.into(), "-i".into()],
        )
      }
      _ => {
        let path = rc.path().join("bashrc");
        std::fs::write(&path, BASH_RC)?;
        (
          "bash",
          vec![
            "--noprofile".into(),
            "--rcfile".into(),
            path.into(),
            "--noediting".into(),
            "-i".into(),
          ],
        )
      }
    };
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("STITCH_SHELL_NONCE".into(), nonce.clone().into());
    env.extend(cfg.env.iter().map(|(k, v)| (k.into(), v.into())));
    for k in &cfg.env_remove {
      env.remove(std::ffi::OsStr::new(k));
    }
    #[cfg(target_os = "macos")]
    if cfg.sandbox.is_some() {
      crate::sandbox::prepare(rc.path(), &cfg.cwd, &mut env)?;
    }
    let crate::pty::Opened { pty, pts, tty } = crate::pty::open().await?;
    pty.resize(Size::new(cfg.rows, cfg.cols))?;
    // Close-on-exec: no shell, nor anything it runs, holds the master open
    // (it would keep a closed session's terminal from hanging up).
    let control = rustix::io::fcntl_dupfd_cloexec(pty.as_fd(), 0).map_err(std::io::Error::from)?;
    let spec = Spawn {
      program: program.into(),
      args,
      env,
      cwd: cfg.cwd.clone(),
      tty,
      helper: cfg.helper.clone(),
      sandbox: cfg.sandbox.clone(),
    };
    // It blocks (looking the program up, the helper's report): off the
    // async threads, and not forever.
    let child = match tokio::time::timeout(
      STARTUP,
      tokio::task::spawn_blocking(move || spawn::spawn(&spec)),
    )
    .await
    {
      Ok(Ok(started)) => started?,
      Ok(Err(e)) => return Err(Error::Io(std::io::Error::other(e))),
      Err(_) => {
        return Err(Error::Startup(format!(
          "{program} was not started within {STARTUP:?} (the shell helper did not answer)"
        )));
      }
    };
    let reaped = child.reaped();
    // The shell has the terminal now: its end must be seen when it ends.
    drop(pts);
    let shell = Pid::from_raw(child.id()).ok_or_else(|| Error::Startup("no process ID".into()))?;
    let (reader, writer) = pty.into_split();
    let session = Session::start(Start {
      reader: Box::new(reader),
      writer: Box::new(writer),
      backend: Backend::Local(Local {
        control,
        shell,
        reaped,
        dir: rc,
        sandboxed: cfg.sandbox.is_some(),
      }),
      ender: Ender::Local(child),
      nonce,
      rows: cfg.rows,
      cols: cfg.cols,
      max_output: cfg.max_output,
      redact: cfg.redact.clone(),
      resumed: None,
    });
    session.first_prompt(program, STARTUP).await
  }

  /// A session over a terminal elsewhere (`Remote`, an SSH channel): its
  /// shell started with `remote_rc`'s marker. A new one waits for its first
  /// prompt; one `resumed` (a tmux session picked up again) starts from the
  /// screen it showed, busy when a program runs there.
  pub async fn remote(cfg: RemoteConfig) -> Result<Session> {
    let resumed = cfg.resumed.is_some();
    let session = Session::start(Start {
      reader: cfg.output,
      writer: cfg.input,
      backend: Backend::Remote(cfg.remote.clone()),
      ender: Ender::Remote(cfg.remote),
      nonce: cfg.nonce,
      rows: cfg.rows,
      cols: cfg.cols,
      max_output: cfg.max_output,
      redact: cfg.redact,
      resumed: cfg.resumed,
    });
    if resumed {
      return Ok(session);
    }
    session
      .first_prompt("the remote shell", REMOTE_STARTUP)
      .await
  }

  /// The session over a started shell's terminal, its reader running.
  fn start(start: Start) -> Session {
    let Start {
      mut reader,
      writer,
      backend,
      ender,
      nonce,
      rows,
      cols,
      max_output,
      redact,
      resumed,
    } = start;
    let mut state = State {
      log: LineLog::new(nonce, LOG_LINES),
      log_parser: vte::Parser::new(),
      screen: vt100::Parser::new(nonzero(rows), nonzero(cols), SCROLLBACK),
      prompts: 0,
      last_prompt: None,
      ended: false,
      pending: false,
      entered: None,
      typed: String::new(),
      named: None,
      end_why: None,
      exited: None,
      input_line: 0,
      news: News::default(),
      sourced: Vec::new(),
    };
    if let Some(r) = resumed {
      state.resume(r);
    }
    let state = Mutex::new(state);
    let (changed, _) = watch::channel(0u64);
    let (output, _) = broadcast::channel(256);
    // The reader starts once the session is built: before, its `Weak` does
    // not upgrade (`Arc::new_cyclic`), which would read as the session
    // dropped, and the shell's first output would end the reader.
    let (start, built) = tokio::sync::oneshot::channel::<()>();
    let inner = Arc::new_cyclic(|weak: &std::sync::Weak<Inner>| {
      let weak = weak.clone();
      let task = tokio::spawn(async move {
        if built.await.is_err() {
          return;
        }
        let mut buf = vec![0u8; 16 * 1024];
        loop {
          // EOF or EIO (the slave side closed) end the session; another
          // read error does too, and is said.
          let (n, why) = match reader.read(&mut buf).await {
            Ok(0) => (0, "its output ended".to_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.raw_os_error() == Some(rustix::io::Errno::IO.raw_os_error()) => {
              (0, "its terminal closed (EIO)".to_owned())
            }
            Err(e) => (0, format!("reading its terminal failed: {e}")),
            Ok(n) => (n, String::new()),
          };
          let Some(inner) = weak.upgrade() else { return };
          if n == 0 {
            {
              let mut s = inner.state.lock().expect("state lock");
              s.ended = true;
              s.end_why = Some(why);
            }
            inner.changed.send_modify(|v| *v += 1);
            drop(inner);
            let exited = ender.wait().await;
            if let Some(inner) = weak.upgrade() {
              let mut s = inner.state.lock().expect("state lock");
              s.exited = Some(exited);
              drop(s);
              inner.changed.send_modify(|v| *v += 1);
            }
            return;
          }
          let bytes = &buf[..n];
          let done: Vec<PathBuf> = {
            let mut s = inner.state.lock().expect("state lock");
            let State {
              log,
              log_parser,
              screen,
              ..
            } = &mut *s;
            log_parser.advance(log, bytes);
            screen.process(bytes);
            let markers = std::mem::take(&mut s.log.markers);
            for m in markers {
              s.prompts += 1;
              s.last_prompt = Some(m);
              s.pending = false;
            }
            let prompts = s.prompts;
            let (done, left) = std::mem::take(&mut s.sourced)
              .into_iter()
              .partition(|(_, before)| *before < prompts);
            s.sourced = left;
            done.into_iter().map(|(file, _)| file).collect()
          };
          for file in done {
            let _ = std::fs::remove_file(file);
          }
          let _ = inner.output.send(bytes.to_vec());
          inner.changed.send_modify(|v| *v += 1);
        }
      });
      #[cfg(test)]
      std::thread::sleep(tests::BUILD_DELAY.get());
      Inner {
        writer: tokio::sync::Mutex::new(writer),
        backend,
        state,
        changed,
        output,
        reader: task.abort_handle(),
        max_output,
        redact,
        sourced: std::sync::atomic::AtomicU64::new(0),
      }
    });
    let _ = start.send(());
    Session { inner }
  }

  /// Waits for the new shell's first prompt, at most `limit`.
  async fn first_prompt(self, program: &str, limit: Duration) -> Result<Session> {
    let session = self;
    let mut rx = session.inner.changed.subscribe();
    let ready = tokio::time::timeout(limit, async {
      loop {
        {
          let s = session.inner.state.lock().expect("state lock");
          if s.prompts > 0 {
            return Ok(());
          }
          if s.ended {
            break;
          }
        }
        if rx.changed().await.is_err() {
          return Err(Error::Closed);
        }
      }
      // Ended before its first prompt: how, once the shell is reaped (a
      // moment; a shell still there after a read error is said so).
      let _ = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
          if session
            .inner
            .state
            .lock()
            .expect("state lock")
            .exited
            .is_some()
          {
            return;
          }
          if rx.changed().await.is_err() {
            return;
          }
        }
      })
      .await;
      let s = session.inner.state.lock().expect("state lock");
      let printed = s.log.text(0, None);
      let printed = if printed.trim().is_empty() {
        "nothing".to_owned()
      } else {
        format!("\"{}\"", printed.trim())
      };
      let end = s.end_why.as_deref().unwrap_or("its output ended");
      Err(Error::Startup(match session.inner.local() {
        // It was executed: posix_spawn returns a pid for nothing less.
        Some(l) => format!(
          "{program} (pid {}) was executed, then {end}; {}; it printed {printed}",
          l.shell.as_raw_nonzero(),
          s.exited.clone().unwrap_or_else(|| format!(
            "the shell (pid {}) had not exited",
            l.shell.as_raw_nonzero()
          )),
        ),
        None => format!(
          "{program} started, then {end}; {}; it printed {printed}",
          s.exited
            .clone()
            .unwrap_or_else(|| "its end was not said".into()),
        ),
      }))
    })
    .await;
    match ready {
      Ok(Ok(())) => Ok(session),
      Ok(Err(e)) => Err(e),
      Err(_) => {
        let seen = session
          .inner
          .state
          .lock()
          .expect("state lock")
          .log
          .text(0, None);
        Err(Error::Startup(format!(
          "no prompt within {limit:?}; saw: {seen}"
        )))
      }
    }
  }

  /// The shell's process, while it runs here (not reaped): a session
  /// leader, so its session ID too. What its commands descend from.
  pub fn shell_pid(&self) -> Option<i32> {
    let l = self.inner.local()?;
    (!l.reaped()).then(|| l.shell.as_raw_nonzero().get())
  }

  /// Whether the shell runs in a sandbox (`Config::sandbox`).
  pub fn sandboxed(&self) -> bool {
    self.inner.local().is_some_and(|l| l.sandboxed)
  }

  /// Whether the shell runs elsewhere (`Session::remote`).
  pub fn is_remote(&self) -> bool {
    matches!(self.inner.backend, Backend::Remote(_))
  }

  /// The program the remote shell runs, as `status` says it: the command
  /// entered last.
  fn remote_program(s: &State) -> String {
    match &s.entered {
      Some(e) if !e.text.is_empty() => e.text.clone(),
      _ => "a program".into(),
    }
  }

  pub fn status(&self) -> Status {
    let (ended, exit, pending, program) = {
      let s = self.inner.state.lock().expect("state lock");
      (
        s.ended,
        s.last_prompt.as_ref().map_or(0, |m| m.status),
        s.pending,
        (s.pending && self.is_remote()).then(|| Self::remote_program(&s)),
      )
    };
    if ended {
      return Status::Closed;
    }
    let Some(local) = self.inner.local() else {
      // Elsewhere, a line entered and its prompt not back is all there is
      // to go by.
      return match program {
        Some(program) => Status::Running { pid: 0, program },
        None => Status::AtPrompt { exit },
      };
    };
    // No foreground group: the shell has gone, and its end is being read.
    match spawn::foreground(local.control.as_fd()).map(|pgrp| pgrp.unwrap_or(local.shell)) {
      Ok(pgrp) if pgrp != local.shell => Status::Running {
        pid: pgrp.as_raw_nonzero().get(),
        program: program_name(pgrp),
      },
      Ok(pgrp) if pending => Status::Running {
        pid: pgrp.as_raw_nonzero().get(),
        program: format!(
          "{} (the shell itself, e.g. a builtin such as `read`)",
          program_name(pgrp)
        ),
      },
      Ok(_) => Status::AtPrompt { exit },
      Err(_) => Status::Closed,
    }
  }

  /// Types a command at the prompt and waits. Fails if a program holds the
  /// terminal (told as the command is written, under the input's lock: of
  /// two at once, one is typed).
  pub async fn run(&self, command: &str, wait: Wait) -> Result<Outcome> {
    if self.status() == Status::Closed {
      return Err(Error::Closed);
    }
    let command = command.trim_end_matches('\n');
    self
      .exchange(Input::Command(command.to_owned()), wait, true)
      .await
  }

  /// Sends input (text or key bytes) to whatever holds the terminal, and
  /// waits.
  pub async fn send(&self, input: &[u8], wait: Wait) -> Result<Outcome> {
    self.send_with(input, wait, false).await
  }

  /// `send`, unless a person typed into the session since it was last
  /// asked (`Error::Typed`, with what they typed, taken): an agent's input
  /// never goes on top of theirs. `run` minds it too.
  pub async fn send_minding(&self, input: &[u8], wait: Wait) -> Result<Outcome> {
    self.send_with(input, wait, true).await
  }

  async fn send_with(&self, input: &[u8], wait: Wait, mind: bool) -> Result<Outcome> {
    if self.status() == Status::Closed {
      return Err(Error::Closed);
    }
    self
      .exchange(Input::Bytes(input.to_vec()), wait, mind)
      .await
  }

  /// At the prompt: what `status` says without naming the program.
  fn at_prompt(&self) -> bool {
    let (ended, pending) = {
      let s = self.inner.state.lock().expect("state lock");
      (s.ended, s.pending)
    };
    !ended
      && !pending
      && self.inner.local().is_none_or(|l| {
        spawn::foreground(l.control.as_fd()).is_ok_and(|pgrp| pgrp.unwrap_or(l.shell) == l.shell)
      })
  }

  /// Waits, at most `limit`, until the prompt is shown after its marker.
  async fn prompt_shown(&self, limit: Duration) {
    let mut rx = self.inner.changed.subscribe();
    let shown = || {
      let s = self.inner.state.lock().expect("state lock");
      s.last_prompt.as_ref().is_none_or(|m| s.log.shown_after(m))
    };
    let _ = tokio::time::timeout(limit, async {
      while !shown() {
        if rx.changed().await.is_err() {
          return;
        }
      }
    })
    .await;
  }

  fn current_line(&self) -> u64 {
    self
      .inner
      .state
      .lock()
      .expect("state lock")
      .log
      .current_line()
  }

  /// A command as typed at the prompt: the bytes, the line its output
  /// starts on, and the file it is sourced from when too long or of several
  /// lines to type (a
  /// file of its own in the session's directory, owner only, so it runs in
  /// this shell as typed would). Called under the writer's lock.
  async fn typed_command(&self, command: &str) -> Result<(Vec<u8>, u64, Option<PathBuf>)> {
    if command.len() <= TYPED_MAX && !command.contains('\n') {
      // One line: the echoed command takes it; output starts after it.
      let mut input = command.as_bytes().to_vec();
      input.push(b'\r');
      return Ok((input, self.current_line() + 1, None));
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let n = self
      .inner
      .sourced
      .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let local = match &self.inner.backend {
      Backend::Local(l) => l,
      Backend::Remote(r) => {
        // Put in the session's own directory there, through a channel of
        // its own; removed with the directory.
        let mut data = command.as_bytes().to_vec();
        data.push(b'\n');
        let path = r.put_file(format!("command-{n}"), data).await?;
        let mut input = format!(". '{}'", path.replace('\'', "'\\''")).into_bytes();
        input.push(b'\r');
        return Ok((input, self.current_line() + 1, None));
      }
    };
    let path = local.dir.path().join(format!("command-{n}"));
    let mut f = std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .mode(0o600)
      .open(&path)?;
    f.write_all(command.as_bytes())?;
    f.write_all(b"\n")?;
    let mut input =
      format!(". '{}'", path.display().to_string().replace('\'', "'\\''")).into_bytes();
    input.push(b'\r');
    Ok((input, self.current_line() + 1, Some(path)))
  }

  /// Writes `input` and waits; `mind`: nothing is sent on top of a
  /// person's typing (`Error::Typed`).
  async fn exchange(&self, input: Input, wait: Wait, mind: bool) -> Result<Outcome> {
    let started = Instant::now();
    let mut rx = self.inner.changed.subscribe();
    // Written whole by a task of its own, so a call cut short leaves no
    // input half sent (a paste left open swallows the next Ctrl+C). What
    // goes with it is decided under the writer's lock, which a person's
    // typing and every other call take too: their input is either before
    // it (and minded) or after it, and a command is typed only at the
    // prompt, which the one typed before has left busy.
    let session = self.clone();
    let written = tokio::spawn(async move {
      let inner = &session.inner;
      let mut w = inner.writer.lock().await;
      let at_prompt = session.at_prompt();
      if mind {
        let mut st = inner.state.lock().expect("state lock");
        if !st.typed.is_empty() {
          return Err(Error::Typed(std::mem::take(&mut st.typed)));
        }
      }
      let (input, from, command, file) = match input {
        Input::Bytes(input) => (input, session.current_line(), None, None),
        Input::Command(command) => {
          match session.status() {
            Status::Running { program, .. } => return Err(Error::Busy { program }),
            Status::Closed => return Err(Error::Closed),
            Status::AtPrompt { .. } => {}
          }
          // The marker comes before the prompt: typed in between, the
          // command's echo would take the prompt's line and the prompt its
          // output's (`$ hello`). So the prompt is waited for, a while.
          session.prompt_shown(PROMPT_SHOWN).await;
          let (input, from, file) = session.typed_command(&command).await?;
          (input, from, Some(command), file)
        }
      };
      let (prompts_before, seen) = {
        let mut st = inner.state.lock().expect("state lock");
        // What was sent is kept a while, with where it was written: the
        // Enter sent on its own after a text is followed by the text shown
        // again, re-wrapped.
        let typed = typed_text(&input);
        if !echo_key(&typed).is_empty() {
          if st.news.sent.len() == SENT_KEPT {
            st.news.sent.remove(0);
          }
          let sent = Sent::on(typed, st.screen.screen());
          st.news.sent.push(sent);
        }
        // What the screen showed before the input, taken before it is
        // written: a fast answer must not slip into it.
        let (before, seen) = view_of(st.screen.screen());
        st.news.before = before;
        st.log.pin(from);
        st.input_line = st.log.current_line();
        // A sourced file goes once the command's prompt is back.
        if let Some(file) = file {
          let prompts = st.prompts;
          st.sourced.push((file, prompts));
        }
        (st.prompts, seen)
      };
      let wrote = tokio::time::timeout(WRITE_MAX, async {
        w.write_all(&input).await?;
        w.flush().await
      })
      .await;
      match wrote {
        Err(_) => return Err(Error::Stalled(WRITE_MAX)),
        Ok(Err(e)) => return Err(e.into()),
        Ok(Ok(())) => {}
      }
      // Written: a line entered at the prompt keeps the shell busy until
      // its prompt, unless that came back already; said before the
      // writer's lock is let go, so the next input finds it busy.
      let mut st = inner.state.lock().expect("state lock");
      let named = st.named.take();
      if at_prompt && (input.contains(&b'\r') || input.contains(&b'\n')) {
        st.pending = st.prompts == prompts_before;
        let text = String::from_utf8_lossy(&input);
        let typed = command
          .as_deref()
          .unwrap_or(&text)
          .lines()
          .next()
          .unwrap_or("")
          .trim()
          .to_owned();
        st.entered = Some(Entered {
          text: if typed.is_empty() {
            named.unwrap_or_default()
          } else {
            typed
          },
          from,
          prompts: prompts_before,
          started: Instant::now(),
        });
      }
      let news = st.news.clone();
      drop(st);
      drop(w);
      Ok((from, prompts_before, news, seen))
    });
    let (from, prompts_before, news, mut seen) = written
      .await
      .map_err(|e| Error::Io(std::io::Error::other(e)))??;
    let deadline = tokio::time::Instant::now() + wait.timeout;
    let mut quiet_since = tokio::time::Instant::now();
    loop {
      {
        let s = self.inner.state.lock().expect("state lock");
        if s.prompts > prompts_before || s.ended {
          break;
        }
      }
      let (text, now_seen) = self.quiet_view();
      if now_seen != seen {
        seen = now_seen;
        quiet_since = tokio::time::Instant::now();
      }
      // The screen as it was is no answer: a match counts when what
      // matches is not what matched before the input (an always-visible
      // prompt box is not news), nor the echo of what was sent.
      if let Some(re) = &wait.screen
        && !news.matches(re, &text).is_empty()
      {
        break;
      }
      let idle_at = wait.idle.map(|i| quiet_since + i);
      if idle_at.is_some_and(|t| t <= tokio::time::Instant::now()) {
        break;
      }
      let until = idle_at.map_or(deadline, |i| i.min(deadline));
      match tokio::time::timeout_at(until, rx.changed()).await {
        Ok(Ok(())) => continue,
        Ok(Err(_)) => break,
        // Past the deadline, or time to look at the quiet again.
        Err(_) if tokio::time::Instant::now() >= deadline => break,
        Err(_) => continue,
      }
    }
    Ok(self.outcome(from, prompts_before, started.elapsed()))
  }

  fn outcome(&self, from: u64, prompts_before: u64, elapsed: Duration) -> Outcome {
    let status = self.status();
    let s = self.inner.state.lock().expect("state lock");
    let finished = s.prompts > prompts_before;
    let raw = match (&s.last_prompt, finished) {
      // Everything up to the line the prompt came on, and what that line
      // held before it (output without a final newline).
      (Some(m), true) => {
        let mut text = s.log.text(from, Some(m.line));
        if !m.partial.is_empty() {
          if !text.is_empty() {
            text.push('\n');
          }
          text.push_str(&m.partial);
        }
        text
      }
      _ => s.log.text(from, None),
    };
    let raw = match &self.inner.redact {
      Some(redact) => (redact.0)(&raw),
      None => raw,
    };
    let (output, truncated) = clip(raw.trim_end(), self.inner.max_output);
    let screen = s.screen.screen();
    let shown = (screen.alternate_screen() && !matches!(status, Status::AtPrompt { .. }))
      .then(|| screen.contents());
    Outcome {
      status,
      output,
      truncated,
      screen: shown,
      elapsed,
    }
  }

  /// What a person types (an attached terminal): sent as is, and kept,
  /// readable, until `take_typed`.
  pub async fn type_in(&self, input: &[u8]) -> Result<()> {
    if self.status() == Status::Closed {
      return Err(Error::Closed);
    }
    // Written whole by a task of its own and at most for `WRITE_MAX`, as
    // an agent's input is; kept while the writer's lock is held: an
    // agent's input written after this finds it (see `exchange`).
    let session = self.clone();
    let input = input.to_vec();
    tokio::spawn(async move {
      let mut w = session.inner.writer.lock().await;
      let wrote = tokio::time::timeout(WRITE_MAX, async {
        w.write_all(&input).await?;
        w.flush().await
      })
      .await;
      // Some of it may have gone in even when the rest could not: it is
      // told either way.
      let shown = readable(&input);
      let mut s = session.inner.state.lock().expect("state lock");
      s.typed.push_str(&shown);
      s.input_line = s.log.current_line();
      drop(s);
      drop(w);
      match wrote {
        Err(_) => Err(Error::Stalled(WRITE_MAX)),
        Ok(Err(e)) => Err(e.into()),
        Ok(Ok(())) => Ok(()),
      }
    })
    .await
    .map_err(|e| Error::Io(std::io::Error::other(e)))?
  }

  /// Puts back what `take_typed` took and could not be told.
  pub fn untake_typed(&self, typed: &str) {
    let mut s = self.inner.state.lock().expect("state lock");
    s.typed.insert_str(0, typed);
  }

  /// Takes what a person typed when `tell` tells it (returns `true`),
  /// under the session's lock, so no one else takes it in between;
  /// whether it was told. `tell` must not touch this session.
  pub fn take_typed_if(&self, tell: impl FnOnce(&str) -> bool) -> bool {
    let mut s = self.inner.state.lock().expect("state lock");
    if s.typed.is_empty() || !tell(&s.typed) {
      return false;
    }
    s.typed.clear();
    true
  }

  /// The command the next Enter at the prompt runs, when it was typed
  /// before the Enter (in its own write): its name in `command`.
  pub fn name_command(&self, text: &str) {
    let first = text.lines().next().unwrap_or("").trim();
    if !first.is_empty() {
      self.inner.state.lock().expect("state lock").named = Some(first.to_owned());
    }
  }

  /// What a person typed since this was last asked, readable.
  pub fn take_typed(&self) -> Option<String> {
    let typed = std::mem::take(&mut self.inner.state.lock().expect("state lock").typed);
    (!typed.is_empty()).then_some(typed)
  }

  /// The screen as escape sequences that draw it on a terminal (cleared
  /// first), cursor included: what an attached terminal starts from.
  pub fn screen_drawn(&self) -> Vec<u8> {
    let s = self.inner.state.lock().expect("state lock");
    let screen = s.screen.screen();
    let mut out = b"\x1b[H\x1b[2J".to_vec();
    out.extend(screen.contents_formatted().into_bytes());
    let (row, col) = screen.cursor_position();
    out.extend(format!("\x1b[{};{}H", row + 1, col + 1).into_bytes());
    out
  }

  /// The screen's text, and a fingerprint that spinner glyphs and digits
  /// do not change: the same fingerprint, the screen is quiet.
  pub fn quiet_view(&self) -> (String, u64) {
    view_of(self.inner.state.lock().expect("state lock").screen.screen())
  }

  /// Waits until the screen has been quiet for `quiet` (spinners and
  /// counters aside), at most `max`.
  pub async fn settle(&self, quiet: Duration, max: Duration) {
    let _ = self
      .wait_for(
        &Until::Screen(ScreenUntil {
          quiet: Some(quiet),
          ..ScreenUntil::default()
        }),
        Some(max),
      )
      .await;
  }

  /// Resolves once the session has ended (its shell gone).
  pub async fn ended(&self) {
    let mut rx = self.inner.changed.subscribe();
    loop {
      if self.inner.state.lock().expect("state lock").ended {
        return;
      }
      if rx.changed().await.is_err() {
        return;
      }
    }
  }

  /// Whether the program asked for bracketed paste.
  pub fn bracketed_paste(&self) -> bool {
    self
      .inner
      .state
      .lock()
      .expect("state lock")
      .screen
      .screen()
      .bracketed_paste()
  }

  /// The command last entered at the prompt, if any.
  pub fn command(&self) -> Option<Command> {
    let s = self.inner.state.lock().expect("state lock");
    s.entered.as_ref().map(|e| Command {
      text: e.text.clone(),
      running: s.prompts <= e.prompts && !s.ended,
      started: e.started,
    })
  }

  /// What the command last entered printed, whole as the log keeps it (its
  /// first 2,000 lines and its last 20,000), with when it was entered:
  /// up to its prompt once it ended, else up to now.
  pub fn command_output(&self) -> Option<(Instant, String)> {
    let s = self.inner.state.lock().expect("state lock");
    let e = s.entered.as_ref()?;
    let finished = s.prompts > e.prompts;
    let text = match (&s.last_prompt, finished) {
      (Some(m), true) => {
        let mut text = s.log.text(e.from, Some(m.line));
        if !m.partial.is_empty() {
          if !text.is_empty() {
            text.push('\n');
          }
          text.push_str(&m.partial);
        }
        text
      }
      _ => s.log.text(e.from, None),
    };
    Some((e.started, text.trim_end().to_owned()))
  }

  /// The process group that holds the terminal, when it is a command's
  /// (not the shell's own).
  pub fn command_group(&self) -> Option<i32> {
    self.command_pgrp().map(|p| p.as_raw_nonzero().get())
  }

  /// Never a remote one: its number means another process here.
  fn command_pgrp(&self) -> Option<Pid> {
    let l = self.inner.local()?;
    if l.reaped() {
      return None;
    }
    spawn::foreground(l.control.as_fd())
      .ok()
      .flatten()
      .filter(|p| *p != l.shell)
  }

  /// `signal_command` for a remote session (where `command_group` has
  /// none): the group that holds its terminal there, signalled through a
  /// channel of its own; the group's number there.
  pub async fn signal_remote(&self, signal: StopSignal) -> Option<i32> {
    match &self.inner.backend {
      Backend::Remote(r) => r.signal(signal).await,
      Backend::Local(_) => None,
    }
  }

  /// Sends `signal` to the process group that holds the terminal, when it
  /// is a command's (not the shell's own: the shell is never signalled
  /// here); the group signalled. Every process of that group gets it: a
  /// build's compilers and test binaries with it, unless one moved to a
  /// group of its own.
  pub fn signal_command(&self, signal: StopSignal) -> Option<i32> {
    let pgrp = self.command_pgrp()?;
    let sig = match signal {
      StopSignal::Term => Signal::TERM,
      StopSignal::Kill => Signal::KILL,
    };
    rustix::process::kill_process_group(pgrp, sig).ok()?;
    Some(pgrp.as_raw_nonzero().get())
  }

  /// Stops the command last entered, unless it is the one entered at
  /// `before` (its `Command::started`; `None`: none was): a command
  /// entered before the caller's own is not its to stop. Ctrl+C, then
  /// again while that same command runs (one that lands while bash forks
  /// a list's next command is taken by bash, and the command runs on), up
  /// to `tries` times, each waiting `each` for its prompt. Whether it no
  /// longer runs (ended, or none was entered); a command entered after it
  /// is not touched. A plain `send` of Ctrl+C stays one keypress.
  pub async fn stop_command(
    &self,
    before: Option<Instant>,
    tries: u32,
    each: Duration,
  ) -> Result<bool> {
    // A line still being written (a call cut short) is entered first.
    drop(self.inner.writer.lock().await);
    let Some(started) = self
      .command()
      .map(|c| c.started)
      .filter(|s| Some(*s) != before)
    else {
      return Ok(true);
    };
    let runs = || {
      self
        .command()
        .is_some_and(|c| c.started == started && c.running)
    };
    for _ in 0..tries {
      if !runs() {
        return Ok(true);
      }
      match self.send(b"\x03", Wait::prompt(each)).await {
        Ok(_) => {}
        Err(Error::Closed) => return Ok(true),
        Err(e) => return Err(e),
      }
    }
    Ok(!runs())
  }

  /// Waits, without sending anything, for the command last entered to end
  /// or for a line of its output to match (lines it printed before count),
  /// up to `timeout` (`None`: no limit). Output is scanned once, as it comes.
  pub async fn wait_for(&self, until: &Until, timeout: Option<Duration>) -> Result<Waited> {
    let started = Instant::now();
    let mut rx = self.inner.changed.subscribe();
    let (from, prompts) = {
      let s = self.inner.state.lock().expect("state lock");
      match &s.entered {
        Some(e) => (e.from, e.prompts),
        // Nothing entered yet: nothing runs, and all output counts.
        None => (0, s.prompts.saturating_sub(1)),
      }
    };
    let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
    let mut scanned = from;
    let (_, mut seen) = self.quiet_view();
    let news = self.inner.state.lock().expect("state lock").news.clone();
    let mut quiet_since = tokio::time::Instant::now();
    // What must be gone counts as seen when it was shown since the last
    // input (a watch set right after sending, before the program drew it).
    let mut gone_seen = match until {
      Until::Screen(ScreenUntil { gone: Some(re), .. }) => {
        let s = self.inner.state.lock().expect("state lock");
        !news
          .shown_since(re, &s.log.text(s.input_line, None))
          .is_empty()
      }
      _ => false,
    };
    let (matched, done) = loop {
      if let Until::Screen(on) = until {
        let (text, now_seen) = self.quiet_view();
        if now_seen != seen {
          seen = now_seen;
          quiet_since = tokio::time::Instant::now();
        }
        gone_seen |= on
          .gone
          .as_ref()
          .is_some_and(|re| !news.shown(re, &text).is_empty());
        let ended = self.inner.state.lock().expect("state lock").ended;
        if ended {
          break (None, true);
        }
        if on.holds(&text, &news, quiet_since.elapsed(), gone_seen) {
          let shown = on
            .shows
            .as_ref()
            .and_then(|re| news.matches(re, &text).into_iter().next());
          break (Some(shown.unwrap_or_default()), false);
        }
        // Quiet long enough already (what else it waits for does not hold
        // yet): only a change can make it hold, so no time to wake for.
        let quiet_at = on
          .quiet
          .map(|q| quiet_since + q)
          .filter(|at| *at > tokio::time::Instant::now());
        let until_at = match (deadline, quiet_at) {
          (Some(d), Some(q)) => Some(d.min(q)),
          (d, q) => d.or(q),
        };
        let changed = rx.changed();
        let woke = match until_at {
          Some(at) => tokio::time::timeout_at(at, changed).await.ok(),
          None => Some(changed.await),
        };
        match woke {
          Some(Ok(())) => continue,
          Some(Err(_)) => break (None, true),
          None if deadline.is_some_and(|d| tokio::time::Instant::now() >= d) => {
            break (None, false);
          }
          None => continue,
        }
      }
      {
        let s = self.inner.state.lock().expect("state lock");
        let done = s.prompts > prompts || s.ended;
        let mut matched = None;
        if let Until::Line(re) = until {
          let current = s.log.current_line();
          if current > scanned {
            matched = s
              .log
              .text(scanned, Some(current))
              .lines()
              .find(|l| re.is_match(l))
              .map(str::to_owned);
            scanned = current;
          }
          // The line being written, without passing it: it may grow.
          if matched.is_none() {
            let partial = s.log.text(current, None);
            if re.is_match(&partial) {
              matched = Some(partial);
            }
          }
        }
        let finished = match until {
          Until::Done => done,
          Until::Line(_) => matched.is_some() || done,
          Until::Screen(_) => unreachable!("handled above"),
        };
        if finished {
          break (matched, done);
        }
      }
      let changed = rx.changed();
      let woke = match deadline {
        Some(d) => tokio::time::timeout_at(d, changed).await.ok(),
        None => Some(changed.await),
      };
      match woke {
        Some(Ok(())) => continue,
        // The session is gone, or the time is up.
        Some(Err(_)) => break (None, true),
        None => break (None, false),
      }
    };
    Ok(Waited {
      matched,
      done,
      outcome: self.outcome(from, prompts, started.elapsed()),
    })
  }

  /// The terminal screen as it shows now.
  pub fn screen(&self) -> Screen {
    let status = self.status();
    let s = self.inner.state.lock().expect("state lock");
    let screen = s.screen.screen();
    Screen {
      text: screen.contents(),
      cursor: screen.cursor_position(),
      alternate: screen.alternate_screen(),
      status,
    }
  }

  pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
    match &self.inner.backend {
      Backend::Local(l) => rustix::termios::tcsetwinsize(
        &l.control,
        rustix::termios::Winsize {
          ws_row: rows,
          ws_col: cols,
          ws_xpixel: 0,
          ws_ypixel: 0,
        },
      )
      .map_err(std::io::Error::from)?,
      Backend::Remote(r) => r.resize(rows, cols),
    }
    self
      .inner
      .state
      .lock()
      .expect("state lock")
      .screen
      .screen_mut()
      .set_size(nonzero(rows), nonzero(cols));
    Ok(())
  }

  /// Raw output as it arrives, for showing the session live.
  pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
    self.inner.output.subscribe()
  }

  /// Waits (at most `limit`) until the reader saw the session's output end.
  async fn until_ended(&self, limit: Duration) {
    let mut rx = self.inner.changed.subscribe();
    let _ = tokio::time::timeout(limit, async {
      loop {
        if self.inner.state.lock().expect("state lock").ended {
          return;
        }
        if rx.changed().await.is_err() {
          return;
        }
      }
    })
    .await;
  }

  /// Ends the session: the shell gets SIGHUP (and passes it to its jobs),
  /// then whatever is left of its process group, and of the group of the
  /// command running in it (a process that ignores SIGHUP), is killed.
  pub async fn close(&self) {
    let l = match &self.inner.backend {
      Backend::Local(l) => l,
      // Elsewhere: the remote ends it (its tmux session too), then its
      // output ends, which this session's reader sees a moment after the
      // remote says it ended: waited for, so `status` says `Closed` once
      // `close` returns.
      Backend::Remote(r) => {
        r.close().await;
        self.until_ended(Duration::from_secs(2)).await;
        return;
      }
    };
    if l.reaped() {
      return;
    }
    let command = self.command_pgrp();
    let _ = rustix::process::kill_process(l.shell, Signal::HUP);
    self.until_ended(Duration::from_millis(500)).await;
    // Taken before the shell went, so it was the command's: a group with
    // no member left fails (ESRCH), and its number comes back only after
    // the pids wrap.
    if let Some(pgrp) = command {
      let _ = rustix::process::kill_process_group(pgrp, Signal::KILL);
    }
    if !l.reaped() {
      let _ = rustix::process::kill_process_group(l.shell, Signal::KILL);
    }
  }
}

impl State {
  /// Picks up a remote shell where it was (`Resumed`): its screen drawn,
  /// at its prompt or busy with what runs there. A prompt is counted for
  /// the one it showed (or will show once what runs ends).
  fn resume(&mut self, r: Resumed) {
    let State {
      log,
      log_parser,
      screen,
      ..
    } = self;
    log_parser.advance(log, &r.screen);
    screen.process(&r.screen);
    self.log.markers.clear();
    self.prompts = 1;
    if let Some(program) = r.running {
      self.pending = true;
      self.entered = Some(Entered {
        text: program,
        from: self.log.current_line(),
        prompts: 1,
        started: Instant::now(),
      });
    }
  }
}

/// A terminal dimension as vt100 takes it: at least one row or column.
fn nonzero(n: u16) -> std::num::NonZeroU16 {
  std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::os::unix::process::ExitStatusExt;

  thread_local! {
    /// How long building a session takes after its reader is spawned, on
    /// this thread: room for the reader to run first.
    pub(super) static BUILD_DELAY: std::cell::Cell<Duration> =
      const { std::cell::Cell::new(Duration::ZERO) };
  }

  /// The shell's first output read while the session is still being
  /// built is not taken for the session dropped: its prompt is seen.
  #[tokio::test(flavor = "multi_thread")]
  async fn output_read_before_the_session_is_built_is_kept() {
    BUILD_DELAY.set(Duration::from_millis(500));
    let s = Session::spawn(Config::new(std::env::temp_dir())).await;
    BUILD_DELAY.set(Duration::ZERO);
    let s = s.unwrap();
    let o = s
      .run("echo built", Wait::prompt(Duration::from_secs(10)))
      .await
      .unwrap();
    assert_eq!(o.output, "built");
    s.close().await;
  }

  #[test]
  fn how_a_shell_ended_is_said() {
    let killed = std::process::ExitStatus::from_raw(9);
    assert_eq!(
      exit_text(killed),
      "the shell was killed by signal 9 (SIGKILL)"
    );
    let exited = std::process::ExitStatus::from_raw(1 << 8);
    assert_eq!(exit_text(exited), "the shell exited with status 1");
  }

  #[test]
  fn a_partly_drawn_echo_is_no_news() {
    // The screen read mid-echo: `READY-2` is in what is drawn so far.
    let sent = vec![Sent {
      text: "sleep 1; echo READY-2; sleep 30\r".into(),
      above: vec!["READY-1".into()],
      at: "$".into(),
    }];
    let screen = ["READY-1", "$ sleep 1; echo READY-2"];
    assert_eq!(echoed(&screen, &sent), [false, true]);
    let re = regex::Regex::new("READY-\\d").unwrap();
    let news = News {
      sent: sent.clone(),
      before: "READY-1\n$".into(),
    };
    assert!(news.matches(&re, &screen.join("\n")).is_empty());
    // The output after the whole echo is news.
    let screen = ["READY-1", "$ sleep 1; echo READY-2; sleep 30", "READY-2"];
    assert_eq!(echoed(&screen, &sent), [false, true, false]);
  }

  #[test]
  fn the_echo_is_told_however_wrapped_and_a_word_of_it_is_news() {
    // Written on the screen's first line.
    let sent = vec![Sent {
      text: "Fix the bug, then say STEP1-DONE when the first step is finished and wait.".into(),
      ..Sent::default()
    }];
    let screen = [
      "╭──────────────────────────────╮",
      "│ > Fix the bug, then say      │",
      "│   STEP1-DONE when the first  │",
      "│   step is finished and wait. │",
      "╰──────────────────────────────╯",
      "",
      "⏺ STEP1-DONE",
    ];
    assert_eq!(
      echoed(&screen, &sent),
      [false, true, true, true, false, false, false]
    );
    // Wrapped mid-word by the terminal, the marker on a line of its own.
    let screen = [
      "$ Fix the bug, then say STEP1-",
      "DONE when the first step",
      "x",
    ];
    assert_eq!(echoed(&screen, &sent), [true, true, false]);
    // Shown before, a match is no news; the echo never is; a second answer
    // is.
    let re = regex::Regex::new("STEP1-DONE").unwrap();
    let news = News {
      sent,
      before: "⏺ STEP1-DONE\n".into(),
    };
    assert!(
      news
        .matches(
          &re,
          "> Fix the bug, then say\nSTEP1-DONE when the first\n⏺ STEP1-DONE"
        )
        .is_empty()
    );
    assert_eq!(
      news.matches(&re, "⏺ STEP1-DONE\n⏺ STEP1-DONE"),
      ["⏺ STEP1-DONE"]
    );
    assert_eq!(typed_text(b"\x1b[200~a b\x1b[201~\r\x1b[A"), "a b\r");
  }

  #[test]
  fn a_spinner_ticks_quietly_and_progress_shows() {
    let quiet = |line: &str| {
      let mut kept = String::new();
      match spun(line) {
        Some(rest) => unspun(rest, |c| kept.push(c)),
        None => kept.push_str(line),
      }
      kept
    };
    let same = |a: &str, b: &str| quiet(a) == quiet(b);
    assert!(same("⠋ thinking (12s)", "⠙ thinking (13s)"));
    assert!(same(
      "✻ Working… (4s · ↑ 1.2k tokens)",
      "✶ Working… (5s · ↑ 1.3k tokens)"
    ));
    assert!(same("| working 3s", "/ working 4s"));
    assert!(!same("• downloading 5%", "• downloading 6%"));
    assert!(!same("⠋ 45.5% (3s)", "⠋ 45.6% (3s)"));
    assert!(!same("✻ step 3/10 (4s)", "✻ step 4/10 (4s)"));
    assert!(!same("copied · 3 files", "copied · 4 files"));
    assert!(!same("  - step 3", "  - step 4"));
  }

  #[test]
  fn the_echo_is_where_it_was_written_and_output_after_it_is_news() {
    let sent = |text: &str, above: &[&str], at: &str| Sent {
      text: text.into(),
      above: above.iter().map(|l| l.to_string()).collect(),
      at: at.into(),
    };
    // A prompt of the user's (letters and digits of its own) and a
    // program that names itself in its output.
    let build = sent("./build\r", &["earlier output"], "me@box ~/src %");
    let screen = [
      "earlier output",
      "me@box ~/src % ./build",
      "./build: Error in step 2",
    ];
    assert_eq!(
      echoed(&screen, std::slice::from_ref(&build)),
      [false, true, false]
    );
    // Scrolled since: the echo is followed up; output repeating a long
    // stretch of the command, or short lines that join into one, is news.
    let cmd = "sleep 1; echo 'All tests passed successfully'; for i in 1 2 3; do echo $i; done\r";
    let run = sent(cmd, &["one", "two", "three"], "$");
    let screen = [
      "three",
      &format!("$ {}", cmd.trim_end()),
      "All tests passed successfully",
      "1",
      "2",
      "3",
    ];
    assert_eq!(
      echoed(&screen, std::slice::from_ref(&run)),
      [false, true, false, false, false, false]
    );
    assert_eq!(run.start(&screen), Some(1));
    // Its line scrolled away (or the screen redrawn): as it was, a run
    // that is surely echo, anywhere.
    assert_eq!(run.start(&["2", "3"]), None);
    assert_eq!(build.start(&["x", "y", "z"]), None);
  }
}
