//! A shell session on a host: a `stitch_shell::Session` over an SSH
//! channel, in tmux where the host has it.
//!
//! Opening one runs a short setup script first (its own channel): it makes
//! the session's directory (`~/.cache/stitch/ssh/<name>`, owner only),
//! writes the rc (`stitch_shell::remote_rc`, with the marker's secret) and
//! says whether the host has bash and tmux, and whether a tmux session of
//! that name, started by Stitch, is there to be picked up.
//!
//! - **In tmux**, a control-mode client (`tmux -C new-session -A -s
//!   <name>`) on a channel without a terminal: the pane's output arrives
//!   as the program printed it (`%output`), input goes as `send-keys -H`,
//!   and the size as `refresh-client -C`. A session picked up again draws
//!   the pane as it is (`capture-pane`) and is busy when a command holds
//!   its terminal (its process group, from `ps`), so it carries on where
//!   it was: from any node, after any drop.
//! - **Plain**, a PTY channel running bash: what runs in it ends with the
//!   connection.
//!
//! Signals, long commands to source and closing go through channels of
//! their own (`Connection::exec`).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::ChannelMsg;
use russh::client::Msg;
use stitch_shell::{BoxFuture, Redact, Remote, RemoteConfig, Resumed, Session, StopSignal};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, oneshot, watch};

use crate::client::Connection;
use crate::tmux::{self, Line, quote};
use crate::{Error, Result};

/// How long the small commands (setup, a file, a signal) may take.
const SMALL: Duration = Duration::from_secs(20);

/// What a session is opened with.
#[derive(Clone, Debug)]
pub struct ShellOptions {
  /// Its name there: the tmux session's and the directory's (letters,
  /// digits, `-` and `_`; see `session_name`).
  pub name: String,
  pub rows: u16,
  pub cols: u16,
  pub max_output: usize,
  pub redact: Option<Redact>,
  /// Run it in tmux where the host has it.
  pub tmux: bool,
}

/// `stitch-<thread>-<session>`, as tmux and a shell take it unquoted.
pub fn session_name(thread: &str, session: &str) -> String {
  let clean = |s: &str| -> String {
    s.chars()
      .map(|c| {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
          c
        } else {
          '_'
        }
      })
      .collect()
  };
  format!("stitch-{}-{}", clean(thread), clean(session))
}

/// A session opened on a host.
pub struct Opened {
  pub session: Session,
  pub link: Arc<Link>,
}

/// What the setup script found.
#[derive(Debug, PartialEq, Eq)]
enum Setup {
  NoBash,
  New {
    tmux: bool,
    dir: String,
  },
  Attach {
    dir: String,
    nonce: String,
    running: Option<String>,
  },
}

/// Shell functions: the process group holding a process's terminal
/// (`tpgid`), and a process's command line (`args`); from `/proc` where
/// there is one, else `ps`.
const PROCS: &str = r#"tpgid() { if [ -r "/proc/$1/stat" ]; then sed 's/^.*) //' "/proc/$1/stat" | cut -d' ' -f6; else ps -o tpgid= -p "$1" 2>/dev/null | tr -d ' '; fi; }
args() { if [ -r "/proc/$1/cmdline" ]; then tr '\0' ' ' < "/proc/$1/cmdline"; else ps -o args= -p "$1" 2>/dev/null; fi; }
"#;

fn setup_script(name: &str, nonce: &str, rc: &str, tmux: bool) -> String {
  format!(
    r#"{PROCS}d="$HOME/.cache/stitch/ssh/{name}"
umask 077
mkdir -p "$d" || exit 1
command -v bash >/dev/null 2>&1 || {{ echo "stitch:nobash"; exit 0; }}
cat > "$d/start" <<STITCH_EOF
echo \$\$ > '$d/pid'; exec bash --noprofile --rcfile '$d/rc' --noediting -i
STITCH_EOF
cat > "$d/tmux" <<STITCH_EOF
cd && exec tmux -u -C new-session -A -s {name} -x "\$1" -y "\$2" "sh '$d/start'"
STITCH_EOF
if {tmux} && command -v tmux >/dev/null 2>&1; then
  if tmux has-session -t ={name} 2>/dev/null && [ -f "$d/nonce" ]; then
    p=$(cat "$d/pid" 2>/dev/null)
    g=$(tpgid "$p")
    run=""
    if [ -n "$g" ] && [ "$g" != "$p" ] && [ "$g" -gt 0 ] 2>/dev/null; then
      run=$(args "$g" | head -n 1 | cut -c1-100)
      [ -n "$run" ] || run="a program"
    fi
    printf 'stitch:attach %s %s\n%s\n' "$d" "$(cat "$d/nonce")" "$run"
    exit 0
  fi
  tmux kill-session -t ={name} 2>/dev/null
  mode=tmux
else
  mode=plain
fi
printf '%s' {nonce} > "$d/nonce"
printf '%s' {rc} > "$d/rc"
printf 'stitch:%s %s\n' "$mode" "$d"
"#,
    nonce = quote(nonce),
    rc = quote(rc),
  )
}

fn parse_setup(out: &str) -> Option<Setup> {
  let mut lines = out.lines().skip_while(|l| !l.starts_with("stitch:"));
  let first = lines.next()?;
  let rest = first.strip_prefix("stitch:")?;
  let (mode, args) = rest.split_once(' ').unwrap_or((rest, ""));
  match mode {
    "nobash" => Some(Setup::NoBash),
    "tmux" | "plain" => Some(Setup::New {
      tmux: mode == "tmux",
      dir: args.trim().to_owned(),
    }),
    "attach" => {
      let (dir, nonce) = args.trim().rsplit_once(' ')?;
      let running = lines.next().map(str::trim).filter(|l| !l.is_empty());
      Some(Setup::Attach {
        dir: dir.to_owned(),
        nonce: nonce.to_owned(),
        running: running.map(str::to_owned),
      })
    }
    _ => None,
  }
}

fn nonce() -> String {
  use rand::RngExt;
  let mut rng = rand::rng();
  (0..24)
    .map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.random_range(0..36)]))
    .collect()
}

/// The session's directory as a command line names it there: relative to
/// the home a channel starts in, so no login shell's quoting (fish's
/// differs) is ever needed: what runs goes as `sh -s`'s input, or as a
/// script the setup put in the directory.
fn rel_dir(name: &str) -> String {
  format!(".cache/stitch/ssh/{name}")
}

/// Opens a session on `conn`'s host.
pub async fn open(conn: Arc<Connection>, opts: ShellOptions) -> Result<Opened> {
  let host = conn.target.name.clone();
  let fresh = nonce();
  let rc = stitch_shell::remote_rc(&fresh);
  let script = setup_script(&opts.name, &fresh, &rc, opts.tmux);
  let out = conn.script(&script, SMALL).await?;
  let setup = parse_setup(&out.stdout_text()).ok_or_else(|| Error::Setup {
    host: host.clone(),
    why: format!(
      "its setup said {:?} (exit {:?}){}",
      out.stdout_text().trim(),
      out.status,
      match String::from_utf8_lossy(&out.stderr).trim() {
        "" => String::new(),
        e => format!(", {e:?}"),
      }
    ),
  })?;
  match setup {
    Setup::NoBash => Err(Error::Setup {
      host,
      why: "it has no bash (sessions run bash)".into(),
    }),
    Setup::New { tmux: true, dir } => open_tmux(conn, opts, dir, fresh, None).await,
    Setup::Attach {
      dir,
      nonce,
      running,
    } => open_tmux(conn, opts, dir, nonce, Some(running)).await,
    Setup::New { tmux: false, dir } => open_plain(conn, opts, dir, fresh).await,
  }
}

/// The far end of a session, as `Session` asks of it.
pub struct Link {
  conn: Arc<Connection>,
  /// The session's name there (its directory's, its tmux session's).
  name: String,
  dir: String,
  /// The tmux session, when it runs in one.
  tmux: Option<String>,
  control: Option<Arc<Control>>,
  plain: Option<Arc<russh::ChannelWriteHalf<Msg>>>,
  /// How it ended, once it did.
  end: watch::Receiver<Option<String>>,
  dropped: AtomicBool,
  closed: AtomicBool,
  resumed: bool,
}

impl Link {
  /// It runs in tmux (so it outlives a dropped connection).
  pub fn in_tmux(&self) -> bool {
    self.tmux.is_some()
  }

  /// It was picked up where it was (a tmux session already there).
  pub fn resumed(&self) -> bool {
    self.resumed
  }

  /// It ended because the connection dropped (not the shell's exit, nor
  /// a close).
  pub fn dropped(&self) -> bool {
    self.dropped.load(Ordering::SeqCst)
  }

  pub fn host(&self) -> &str {
    &self.conn.target.name
  }

  fn set_end(&self, end: &watch::Sender<Option<String>>, exited: Option<String>) {
    let why = match exited {
      Some(why) => why,
      None if self.closed.load(Ordering::SeqCst) => "the session was closed".into(),
      None => {
        self.dropped.store(true, Ordering::SeqCst);
        format!("the connection to {} dropped", self.conn.target.name)
      }
    };
    let _ = end.send(Some(why));
  }

  async fn run_small(&self, script: &str) -> Result<crate::client::Exec> {
    self.conn.script(script, SMALL).await
  }
}

impl Remote for Link {
  fn resize(&self, rows: u16, cols: u16) {
    if let Some(c) = &self.control {
      let c = c.clone();
      tokio::spawn(async move {
        let _ = c.send(&format!("refresh-client -C {cols}x{rows}\n")).await;
      });
    } else if let Some(w) = &self.plain {
      let w = w.clone();
      tokio::spawn(async move {
        let _ = w
          .window_change(u32::from(cols), u32::from(rows), 0, 0)
          .await;
      });
    }
  }

  fn put_file(&self, name: String, data: Vec<u8>) -> BoxFuture<'_, std::io::Result<String>> {
    Box::pin(async move {
      let path = format!("{}/{name}", self.dir);
      // Unquoted (`rel_dir`): a login shell of any kind takes it.
      let out = self
        .conn
        .exec(
          &format!("cat > {}/{name}", rel_dir(&self.name)),
          Some(&data),
          SMALL,
        )
        .await
        .map_err(std::io::Error::other)?;
      if !out.ok() {
        return Err(std::io::Error::other(format!(
          "writing {path} on {} failed: {}",
          self.host(),
          String::from_utf8_lossy(&out.stderr).trim()
        )));
      }
      Ok(path)
    })
  }

  fn signal(&self, signal: StopSignal) -> BoxFuture<'_, Option<i32>> {
    Box::pin(async move {
      let sig = match signal {
        StopSignal::Term => "TERM",
        StopSignal::Kill => "KILL",
      };
      let d = quote(&self.dir);
      // No `--` before the group: dash's `kill` (Debian's and Ubuntu's
      // /bin/sh) takes it for a process ID and fails; with the signal
      // given first, `-<group>` is read as the group everywhere.
      let script = format!(
        r#"{PROCS}p=$(cat {d}/pid 2>/dev/null)
g=$(tpgid "$p")
if [ -n "$g" ] && [ "$g" != "$p" ] && [ "$g" -gt 1 ] 2>/dev/null; then kill -{sig} "-$g" && echo "$g"; fi"#
      );
      let out = self.run_small(&script).await.ok()?;
      out.stdout_text().trim().parse().ok()
    })
  }

  fn close(&self) -> BoxFuture<'_, ()> {
    Box::pin(async move {
      self.closed.store(true, Ordering::SeqCst);
      let d = quote(&self.dir);
      let tmux = match &self.tmux {
        Some(name) => format!("tmux kill-session -t ={name} 2>/dev/null; "),
        None => String::new(),
      };
      let script = format!(
        r#"p=$(cat {d}/pid 2>/dev/null); [ -n "$p" ] && kill -HUP "$p" 2>/dev/null; {tmux}rm -rf {d}"#
      );
      if !self.conn.is_closed() {
        let _ = self.run_small(&script).await;
      }
      if let Some(w) = &self.plain {
        let _ = w.close().await;
      }
      // Its end, said once the reader saw it (a moment).
      let mut end = self.end.clone();
      let _ = tokio::time::timeout(Duration::from_secs(2), end.wait_for(Option::is_some)).await;
    })
  }

  fn ended(&self) -> BoxFuture<'_, String> {
    Box::pin(async move {
      let mut end = self.end.clone();
      match end.wait_for(Option::is_some).await {
        Ok(v) => v.clone().unwrap_or_default(),
        Err(_) => "its channel closed".into(),
      }
    })
  }
}

async fn open_plain(
  conn: Arc<Connection>,
  opts: ShellOptions,
  dir: String,
  nonce: String,
) -> Result<Opened> {
  let host = conn.target.name.clone();
  let channel = conn.channel().await?;
  let failed = |e: russh::Error| Error::Channel {
    host: host.clone(),
    why: e.to_string(),
  };
  channel
    .request_pty(
      false,
      "xterm-256color",
      u32::from(opts.cols),
      u32::from(opts.rows),
      0,
      0,
      &[],
    )
    .await
    .map_err(failed)?;
  channel
    .exec(false, format!("sh {}/start", rel_dir(&opts.name)))
    .await
    .map_err(failed)?;
  let (mut read, write) = channel.split();
  let input = write.make_writer();
  let write = Arc::new(write);
  let (end_tx, end_rx) = watch::channel(None);
  let link = Arc::new(Link {
    conn,
    name: opts.name.clone(),
    dir,
    tmux: None,
    control: None,
    plain: Some(write),
    end: end_rx,
    dropped: AtomicBool::new(false),
    closed: AtomicBool::new(false),
    resumed: false,
  });
  let (mut out_w, out_r) = tokio::io::duplex(64 * 1024);
  let l = link.clone();
  tokio::spawn(async move {
    let mut exited = None;
    while let Some(msg) = read.wait().await {
      match msg {
        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
          if out_w.write_all(&data).await.is_err() {
            break;
          }
        }
        ChannelMsg::ExitStatus { exit_status } => {
          exited = Some(format!("the shell exited with status {exit_status}"));
        }
        ChannelMsg::ExitSignal { signal_name, .. } => {
          exited = Some(format!("the shell was killed by {signal_name:?}"));
        }
        _ => {}
      }
    }
    l.set_end(&end_tx, exited);
    drop(out_w);
  });
  let session = Session::remote(RemoteConfig {
    nonce,
    output: Box::new(out_r),
    input: Box::new(input),
    remote: link.clone(),
    rows: opts.rows,
    cols: opts.cols,
    max_output: opts.max_output,
    redact: opts.redact,
    resumed: None,
  })
  .await?;
  Ok(Opened { session, link })
}

/// A command's reply awaited, and whether it lets the pane's output
/// through (`capture-pane` when picking a session up: what came before is
/// in the capture).
struct Pending {
  reply: Option<oneshot::Sender<std::result::Result<Vec<String>, String>>>,
  opens: bool,
}

/// The control-mode client: commands written in order, their replies
/// matched in that order.
pub(crate) struct Control {
  write: russh::ChannelWriteHalf<Msg>,
  /// Held while a command is queued and written: commands go in the
  /// order their replies are queued. The reader never takes it (a write
  /// may wait for the reader to make room).
  sending: Mutex<()>,
  queue: std::sync::Mutex<VecDeque<Pending>>,
  /// The pane's output goes to the session (off until a picked-up
  /// session's capture is in).
  forward: AtomicBool,
  pane: std::sync::Mutex<Option<String>>,
}

impl Control {
  async fn write(&self, line: &str, pending: Pending) -> std::io::Result<()> {
    let _sending = self.sending.lock().await;
    self.queue.lock().expect("queue lock").push_back(pending);
    self
      .write
      .data_bytes(line.as_bytes().to_vec())
      .await
      .map_err(std::io::Error::other)
  }

  /// Sends a command, its reply not awaited.
  async fn send(&self, line: &str) -> std::io::Result<()> {
    self
      .write(
        line,
        Pending {
          reply: None,
          opens: false,
        },
      )
      .await
  }

  /// Sends a command and waits for its reply's lines.
  async fn command(&self, line: &str, opens: bool) -> std::result::Result<Vec<String>, String> {
    let (tx, rx) = oneshot::channel();
    self
      .write(
        line,
        Pending {
          reply: Some(tx),
          opens,
        },
      )
      .await
      .map_err(|e| e.to_string())?;
    match tokio::time::timeout(SMALL, rx).await {
      Ok(Ok(r)) => r,
      Ok(Err(_)) => Err("tmux ended".into()),
      Err(_) => Err(format!("tmux did not answer `{}`", line.trim())),
    }
  }
}

async fn open_tmux(
  conn: Arc<Connection>,
  opts: ShellOptions,
  dir: String,
  nonce: String,
  resumed: Option<Option<String>>,
) -> Result<Opened> {
  let host = conn.target.name.clone();
  let channel = conn.channel().await?;
  let failed = |e: russh::Error| Error::Channel {
    host: host.clone(),
    why: e.to_string(),
  };
  let name = opts.name.clone();
  let start = format!("sh {}/tmux {} {}", rel_dir(&name), opts.cols, opts.rows);
  channel.exec(false, start).await.map_err(failed)?;
  let (mut read, write) = channel.split();
  let control = Arc::new(Control {
    write,
    sending: Mutex::new(()),
    queue: std::sync::Mutex::new(VecDeque::new()),
    forward: AtomicBool::new(resumed.is_none()),
    pane: std::sync::Mutex::new(None),
  });
  let (end_tx, end_rx) = watch::channel(None);
  let link = Arc::new(Link {
    conn,
    name: name.clone(),
    dir,
    tmux: Some(name.clone()),
    control: Some(control.clone()),
    plain: None,
    end: end_rx,
    dropped: AtomicBool::new(false),
    closed: AtomicBool::new(false),
    resumed: resumed.is_some(),
  });
  let (mut out_w, out_r) = tokio::io::duplex(64 * 1024);
  // What tmux writes: the pane's output to the session, replies to who
  // asked.
  let l = link.clone();
  let c = control.clone();
  tokio::spawn(async move {
    let mut buf: Vec<u8> = Vec::new();
    let mut exited = None;
    let mut reply: Option<(Pending, Vec<String>)> = None;
    let mut in_block = false;
    'read: while let Some(msg) = read.wait().await {
      let data = match msg {
        ChannelMsg::Data { data } => data,
        ChannelMsg::ExitStatus { .. } if exited.is_none() => {
          exited = Some(format!(
            "tmux ended (its session {} is gone)",
            l.tmux.as_deref().unwrap_or("")
          ));
          continue;
        }
        _ => continue,
      };
      buf.extend_from_slice(&data);
      while let Some(at) = buf.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buf.drain(..=at).collect();
        let line = &line[..line.len() - 1];
        match tmux::parse(line) {
          Line::Output { pane, bytes } => {
            let ours = c
              .pane
              .lock()
              .expect("pane lock")
              .as_ref()
              .is_none_or(|p| *p == pane);
            if ours && c.forward.load(Ordering::SeqCst) && out_w.write_all(&bytes).await.is_err() {
              break 'read;
            }
          }
          Line::Begin { ours } => {
            in_block = true;
            if ours {
              reply = c
                .queue
                .lock()
                .expect("queue lock")
                .pop_front()
                .map(|p| (p, Vec::new()));
            }
          }
          Line::End { ok } => {
            in_block = false;
            if let Some((p, lines)) = reply.take() {
              if p.opens {
                c.forward.store(true, Ordering::SeqCst);
              }
              if let Some(tx) = p.reply {
                let _ = tx.send(if ok { Ok(lines) } else { Err(lines.join("\n")) });
              }
            }
          }
          Line::Exit(why) => {
            exited = Some(if why.is_empty() {
              "the shell ended (its tmux session closed)".to_owned()
            } else {
              format!("tmux ended: {why}")
            });
          }
          Line::Other(text) if in_block => {
            if let Some((_, lines)) = reply.as_mut() {
              lines.push(String::from_utf8_lossy(&text).into_owned());
            }
          }
          Line::Other(_) => {}
        }
      }
    }
    // Who waits for a reply hears it will not come.
    c.queue.lock().expect("queue lock").clear();
    l.set_end(&end_tx, exited);
    drop(out_w);
  });
  let fail = |why: String| Error::Setup {
    host: host.clone(),
    why: format!("tmux: {why}"),
  };
  control
    .send(&format!("refresh-client -C {}x{}\n", opts.cols, opts.rows))
    .await
    .map_err(|e| fail(e.to_string()))?;
  let info = control
    .command(
      &format!(
        // The client's own session (a target of `=name` finds no pane).
        "display-message -p {}\n",
        quote("#{pane_id} #{alternate_on} #{cursor_x} #{cursor_y}")
      ),
      false,
    )
    .await
    .map_err(fail)?;
  let info = info.first().cloned().unwrap_or_default();
  let mut parts = info.split_whitespace();
  let pane = parts.next().unwrap_or("").to_owned();
  let alternate = parts.next() == Some("1");
  let x: u16 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
  let y: u16 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
  if !pane.starts_with('%') {
    return Err(fail(format!("no pane in {info:?}")));
  }
  *control.pane.lock().expect("pane lock") = Some(pane.clone());
  let resumed = match resumed {
    None => None,
    Some(running) => {
      // The pane as it is; its output flows from right after.
      let lines = control
        .command(&format!("capture-pane -p -e -t {pane}\n"), true)
        .await
        .map_err(fail)?;
      let mut screen = Vec::new();
      if alternate {
        screen.extend_from_slice(b"\x1b[?1049h");
      }
      screen.extend_from_slice(b"\x1b[H\x1b[2J");
      screen.extend_from_slice(lines.join("\r\n").as_bytes());
      screen.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
      Some(Resumed { screen, running })
    }
  };
  // Input: what the session writes, as keys.
  let (in_w, mut in_r) = tokio::io::duplex(16 * 1024);
  let c = control.clone();
  tokio::spawn(async move {
    let mut buf = vec![0u8; 4096];
    loop {
      let n = match in_r.read(&mut buf).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
      };
      for line in tmux::send_keys(&pane, &buf[..n]) {
        if c.send(&line).await.is_err() {
          return;
        }
      }
    }
  });
  let session = Session::remote(RemoteConfig {
    nonce,
    output: Box::new(out_r),
    input: Box::new(in_w),
    remote: link.clone(),
    rows: opts.rows,
    cols: opts.cols,
    max_output: opts.max_output,
    redact: opts.redact,
    resumed,
  })
  .await?;
  Ok(Opened { session, link })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_are_safe_unquoted() {
    assert_eq!(
      session_name("2026-10-21T060706Z-mbp-9jdz", "main"),
      "stitch-2026-10-21T060706Z-mbp-9jdz-main"
    );
    assert_eq!(session_name("t", "a b;'c"), "stitch-t-a_b__c");
  }

  #[test]
  fn the_setup_is_read() {
    assert_eq!(parse_setup("stitch:nobash\n"), Some(Setup::NoBash));
    assert_eq!(
      parse_setup("motd\nstitch:tmux /home/u/.cache/stitch/ssh/x\n"),
      Some(Setup::New {
        tmux: true,
        dir: "/home/u/.cache/stitch/ssh/x".into()
      })
    );
    assert_eq!(
      parse_setup("stitch:attach /d n0nce\nsleep 100\n"),
      Some(Setup::Attach {
        dir: "/d".into(),
        nonce: "n0nce".into(),
        running: Some("sleep 100".into())
      })
    );
    assert_eq!(
      parse_setup("stitch:attach /d n0nce\n\n"),
      Some(Setup::Attach {
        dir: "/d".into(),
        nonce: "n0nce".into(),
        running: None
      })
    );
    assert_eq!(parse_setup("nothing"), None);
  }
}
