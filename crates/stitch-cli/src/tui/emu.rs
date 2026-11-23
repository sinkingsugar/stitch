//! A terminal emulator for tests: Alacritty's own grid
//! (`alacritty_terminal`), with its scrollback and its reflow on resize,
//! fed the bytes the TUI's `Screen` writes. What a user would find in the
//! scrollback after resizing the window is read back from it.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

/// What the terminal answers the program (a cursor position asked).
#[derive(Clone, Default)]
struct Answers(Arc<Mutex<Vec<u8>>>);

impl EventListener for Answers {
  fn send_event(&self, event: Event) {
    if let Event::PtyWrite(text) = event {
      self.0.lock().unwrap().extend_from_slice(text.as_bytes());
    }
  }
}

pub struct Emu {
  term: Term<Answers>,
  parser: Processor,
  answers: Answers,
}

impl Emu {
  pub fn new(rows: u16, cols: u16) -> Emu {
    let config = Config {
      scrolling_history: 100_000,
      ..Config::default()
    };
    let answers = Answers::default();
    Emu {
      term: Term::new(
        config,
        &TermSize::new(cols.into(), rows.into()),
        answers.clone(),
      ),
      parser: Processor::new(),
      answers,
    }
  }

  pub fn feed(&mut self, bytes: &[u8]) {
    self.parser.advance(&mut self.term, bytes);
  }

  /// What the terminal has to answer the program since last asked.
  pub fn answers(&mut self) -> Vec<u8> {
    std::mem::take(&mut *self.answers.0.lock().unwrap())
  }

  /// The window resized: Alacritty reflows what it holds, as it does.
  pub fn resize(&mut self, rows: u16, cols: u16) {
    self.term.resize(TermSize::new(cols.into(), rows.into()));
  }

  /// Rows and columns.
  pub fn size(&self) -> (u16, u16) {
    (self.term.screen_lines() as u16, self.term.columns() as u16)
  }

  /// Row and column.
  pub fn cursor(&self) -> (u16, u16) {
    let p = self.term.grid().cursor.point;
    (p.line.0.max(0) as u16, p.column.0 as u16)
  }

  /// Every row, the scrollback's first, with whether it wraps onto the
  /// next.
  fn rows(&self) -> Vec<(String, bool)> {
    let grid = self.term.grid();
    let cols = grid.columns();
    (-(grid.history_size() as i32)..grid.screen_lines() as i32)
      .map(|line| {
        let row = &grid[Line(line)];
        let mut text = String::new();
        for c in 0..cols {
          let cell = &row[Column(c)];
          if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
          {
            continue;
          }
          text.push(cell.c);
        }
        let wraps = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        (text.trim_end().to_owned(), wraps)
      })
      .collect()
  }

  /// The lines of the scrollback and of the screen above screen row
  /// `upto`, rows the terminal wrapped joined again, trailing blank
  /// lines left out.
  pub fn lines_above(&self, upto: u16) -> Vec<String> {
    let rows = self.rows();
    let keep = rows.len() - (self.term.screen_lines() - usize::from(upto));
    let mut lines = Vec::new();
    let mut cur = String::new();
    for (text, wraps) in &rows[..keep] {
      cur.push_str(text);
      if !wraps {
        lines.push(std::mem::take(&mut cur).trim_end().to_owned());
      }
    }
    if !cur.is_empty() {
      lines.push(cur);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
      lines.pop();
    }
    lines
  }

  /// The screen as it shows, a row per line.
  pub fn screen(&self) -> String {
    let rows = self.rows();
    let n = self.term.screen_lines();
    rows[rows.len() - n..]
      .iter()
      .map(|(t, _)| t.as_str())
      .collect::<Vec<_>>()
      .join("\n")
  }
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use ratatui::layout::{Constraint, Layout};
  use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

  use super::super::{Printed, Screen, Tty, markdown, printed};
  use super::Emu;

  /// The TUI's screen on an emulated Alacritty, driven as the event loop
  /// drives it, with a live area standing in for the app's.
  struct Sim {
    emu: Arc<Mutex<Emu>>,
    screen: Screen,
    /// The lines printed so far, as the scrollback should hold them.
    expected: Vec<String>,
    /// What the live area shows above the input box.
    live: Vec<String>,
    n: usize,
  }

  impl Sim {
    fn new(rows: u16, cols: u16) -> Sim {
      let emu = Arc::new(Mutex::new(Emu::new(rows, cols)));
      let screen = Screen::new(Tty::Emu(emu.clone()), 4).unwrap();
      let mut s = Sim {
        emu,
        screen,
        expected: Vec::new(),
        live: Vec::new(),
        n: 0,
      };
      s.draw();
      s
    }

    fn want(&self) -> u16 {
      (self.live.len() as u16 + 4)
        .min(self.screen.rows.saturating_sub(1))
        .max(3)
    }

    fn draw(&mut self) {
      let live = self.live.join("\n");
      self
        .screen
        .draw(|f| {
          let [l, input, status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
          ])
          .areas(f.area());
          f.render_widget(Paragraph::new(live).wrap(Wrap { trim: false }), l);
          f.render_widget(
            Paragraph::new("typing▏").block(
              Block::new()
                .borders(Borders::TOP | Borders::BOTTOM)
                .title("INPUT Enter send · Esc Esc messages"),
            ),
            input,
          );
          f.render_widget(Paragraph::new("STATUS gamer · model · thread"), status);
        })
        .unwrap();
    }

    /// One turn of the event loop: what is new printed, the live area
    /// laid out and drawn.
    fn turn(&mut self, items: Vec<Printed>) {
      let mut md = markdown::Md::default();
      for item in &items {
        let (_, shown) = printed(item, &mut md, self.screen.cols);
        self
          .expected
          .extend(shown.split('\n').map(|l| l.trim_end().to_owned()));
      }
      let want = self.want();
      if !items.is_empty() || want != self.screen.height {
        self.screen.relayout(&items, want).unwrap();
      }
      self.draw();
    }

    /// Messages: short ones, and long ones that wrap and reflow.
    fn messages(&mut self, count: usize) {
      let items = (0..count)
        .map(|_| {
          self.n += 1;
          let n = self.n;
          if n.is_multiple_of(3) {
            Printed::You(format!(
              "message {n}: {}",
              "a long line that wraps and reflows ".repeat(4)
            ))
          } else {
            Printed::Notice(format!("message {n}"))
          }
        })
        .collect();
      self.turn(items);
    }

    /// The window resized: the terminal reflows, the TUI draws once more
    /// before its event loop looks (a spinner's frame), then lays out
    /// the live area for the new size.
    fn resize(&mut self, rows: u16, cols: u16) {
      self.emu.lock().unwrap().resize(rows, cols);
      self.draw();
      self.turn(Vec::new());
    }

    /// The scrollback and the screen above the live area hold what was
    /// printed, each line once, in order.
    fn check(&self, what: &str) -> Result<(), String> {
      let emu = self.emu.lock().unwrap();
      let got = emu.lines_above(self.screen.rows - self.screen.height);
      let mut want = self.expected.clone();
      while want.last().is_some_and(|l| l.is_empty()) {
        want.pop();
      }
      if got == want {
        return Ok(());
      }
      let at = got
        .iter()
        .zip(&want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
      let from = at.saturating_sub(3);
      Err(format!(
        "{what}: the scrollback differs from what was printed at line {at} \
         ({} lines, {} printed)\n  got:  {:?}\n  want: {:?}\nscreen:\n{}",
        got.len(),
        want.len(),
        &got[from..(at + 5).min(got.len())],
        &want[from..(at + 5).min(want.len())],
        emu.screen()
      ))
    }
  }

  #[derive(Clone, Copy, Debug)]
  enum Live {
    /// The input and the status line.
    Idle,
    /// A reply streaming: its lines so far, then printed.
    Streaming,
    /// The Esc Esc picker: a taller live area, a message's preview.
    Picker,
  }

  /// A conversation, a resize (or several) while the live area shows
  /// `live`, then more of the conversation.
  fn run(live: Live, first: usize, sizes: &[(u16, u16)]) -> Result<(), String> {
    let mut s = Sim::new(30, 80);
    s.messages(first);
    match live {
      Live::Idle => {}
      Live::Streaming => {
        s.live = (1..=8)
          .map(|i| format!("streamed line {i} of the reply"))
          .chain(["⠋ writing · 3s · Esc interrupts".to_owned()])
          .collect();
        s.turn(Vec::new());
      }
      Live::Picker => {
        s.live = (1..=12)
          .map(|i| format!("preview {i}: message 39 says this"))
          .collect();
        s.turn(Vec::new());
      }
    }
    for &(rows, cols) in sizes {
      s.resize(rows, cols);
    }
    match live {
      Live::Idle => {}
      Live::Streaming => {
        s.live.clear();
        let text = (1..=8)
          .map(|i| format!("streamed line {i} of the reply"))
          .collect::<Vec<_>>()
          .join("\n");
        s.turn(vec![Printed::Reply {
          model: "model".into(),
          text,
        }]);
      }
      Live::Picker => {
        s.live.clear();
        s.turn(Vec::new());
      }
    }
    s.messages(10);
    s.check(&format!("{live:?} through {sizes:?}"))
  }

  /// Resizing the window, in every direction and whatever the live area
  /// shows, leaves the conversation in the scrollback once, in order:
  /// nothing of the live area left behind, no line of text lost.
  #[test]
  fn resizing_the_terminal_keeps_the_scrollback_whole() {
    let resizes: &[(&str, &[(u16, u16)])] = &[
      ("wider", &[(30, 120)]),
      ("narrower", &[(30, 50)]),
      ("shorter", &[(18, 80)]),
      ("taller", &[(45, 80)]),
      (
        "dragged",
        &[(28, 76), (25, 70), (22, 64), (26, 90), (34, 110)],
      ),
      ("back and forth", &[(20, 60), (30, 80)]),
    ];
    let mut failed = Vec::new();
    for live in [Live::Idle, Live::Streaming, Live::Picker] {
      for (name, sizes) in resizes {
        // A conversation with scrollback, and one that does not fill the
        // screen yet.
        for first in [40, 2] {
          if let Err(e) = run(live, first, sizes) {
            failed.push(format!("{name}, {first} messages first, {e}"));
          }
        }
      }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n\n"));
  }

  /// The real TUI (the built `stitch`, or `STITCH_BIN`) in a pseudo
  /// terminal on Alacritty's emulator, which answers its cursor queries
  /// as Alacritty does, the window resized under it: each resize is taken
  /// at once (the cursor's answer read, not waited two seconds for), and
  /// the scrollback holds what was printed once. It starts a node of its
  /// own in a temporary directory. `cargo build -p stitch-cli && cargo
  /// test -p stitch-cli the_tui_in_alacritty -- --ignored --nocapture`.
  #[test]
  #[ignore]
  fn the_tui_in_alacritty_takes_resizes() {
    use alacritty_terminal::event::{OnResize, WindowSize};
    use alacritty_terminal::tty::{self, EventedReadWrite, Options, Shell};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let bin = std::env::var("STITCH_BIN")
      .map(std::path::PathBuf::from)
      .unwrap_or_else(|_| {
        let exe = std::env::current_exe().unwrap();
        exe.parent().unwrap().parent().unwrap().join("stitch")
      });
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let set_up = std::process::Command::new(&bin)
      .args(["setup", "--name", "e2e", "--local"])
      .env("STITCH_DATA", &data)
      .current_dir(&work)
      .output()
      .unwrap();
    assert!(set_up.status.success(), "{set_up:?}");
    let size = |rows: u16, cols: u16| WindowSize {
      num_lines: rows,
      num_cols: cols,
      cell_width: 8,
      cell_height: 16,
    };
    let options = Options {
      shell: Some(Shell::new(bin.display().to_string(), Vec::new())),
      working_directory: Some(work.clone()),
      drain_on_exit: false,
      env: [
        ("STITCH_DATA".to_owned(), data.display().to_string()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
      ]
      .into(),
    };
    let mut pty = tty::new(&options, size(30, 80), 0).unwrap();
    let emu = Arc::new(Mutex::new(Emu::new(30, 80)));
    let mut keys = pty.writer().try_clone().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let reading = {
      let (emu, stop) = (emu.clone(), stop.clone());
      let mut reader = pty.reader().try_clone().unwrap();
      let mut writer = pty.writer().try_clone().unwrap();
      std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        while !stop.load(Ordering::Relaxed) {
          match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
              let answers = {
                let mut e = emu.lock().unwrap();
                e.feed(&buf[..n]);
                e.answers()
              };
              if !answers.is_empty() {
                let _ = writer.write_all(&answers);
              }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
              std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
          }
        }
      })
    };
    // Waits for the screen to show what `ready` wants and then not to
    // change for a moment; how long it took.
    let settle = |ready: &dyn Fn(&str) -> bool, what: &str| -> Duration {
      let start = Instant::now();
      let (mut last, mut since) = (String::new(), Instant::now());
      loop {
        let screen = emu.lock().unwrap().screen();
        if screen != last {
          (last, since) = (screen.clone(), Instant::now());
        } else if ready(&screen) && since.elapsed() > Duration::from_millis(300) {
          return since.duration_since(start);
        }
        assert!(
          start.elapsed() < Duration::from_secs(30),
          "{what}: not ready:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(10));
      }
    };
    let status = |s: &str| s.lines().last().is_some_and(|l| l.starts_with(" e2e "));
    settle(&status, "started");
    // A line of `/help`'s list, and the last.
    const HELP: &str = "/help                     this list";
    let helps = |emu: &Emu| {
      let (rows, _) = emu.size();
      emu
        .lines_above(rows)
        .iter()
        .filter(|l| l.contains(HELP))
        .count()
    };
    let mut asked = 0;
    let mut help = |keys: &mut std::fs::File| {
      keys.write_all(b"/help\r").unwrap();
      asked += 1;
      let start = Instant::now();
      // (Fewer when the scrollback lost one: counted at the end.)
      while helps(&emu.lock().unwrap()) < asked && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
      }
      settle(&status, "help");
    };
    for _ in 0..3 {
      help(&mut keys);
    }
    let mut took = Vec::new();
    for (rows, cols) in [(20, 60), (40, 120), (24, 70), (36, 100), (30, 80)] {
      emu.lock().unwrap().resize(rows, cols);
      pty.on_resize(size(rows, cols));
      let fits = move |s: &str| {
        status(s)
          && s
            .lines()
            .nth(usize::from(rows) - 4)
            .is_some_and(|l| l.contains("Enter send"))
      };
      took.push(((rows, cols), settle(&fits, &format!("{rows}x{cols}"))));
    }
    help(&mut keys);
    let _ = keys.write_all(b"\x03");
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let _ = reading.join();
    let _ = std::process::Command::new(&bin)
      .args(["node", "stop"])
      .env("STITCH_DATA", &data)
      .output();
    let emu = emu.lock().unwrap();
    let (rows, _) = emu.size();
    let all = emu.lines_above(rows);
    eprintln!("resizes taken in {took:?}");
    let count = |p: &dyn Fn(&str) -> bool| all.iter().filter(|l| p(l)).count();
    let mut wrong = Vec::new();
    let helps = count(&|l| l.contains(HELP));
    if helps != 4 {
      wrong.push(format!("/help's list {helps} times, not 4"));
    }
    // The live area, left only where it is drawn.
    let inputs = count(&|l| l.starts_with("Enter send"));
    if inputs > 1 {
      wrong.push(format!("the input box {inputs} times"));
    }
    let statuses = count(&|l| l.starts_with(" e2e "));
    if statuses > 1 {
      wrong.push(format!("the status line {statuses} times"));
    }
    for ((r, c), t) in took {
      if t > Duration::from_millis(1500) {
        wrong.push(format!("{r}x{c} took {t:?}"));
      }
    }
    assert!(wrong.is_empty(), "{}\n{all:#?}", wrong.join("\n"));
  }
}
