//! Sessions whose shell runs elsewhere (an SSH channel, design §2.20).
//!
//! The session is the same (its line log, screen, marker, waits and
//! jobs); only the terminal is somewhere else. What a local session asks
//! of its PTY and processes (the foreground group, signals, a file to
//! source, a resize) a remote one asks of its `Remote`. Whether a program
//! runs comes from the marker alone: a line entered and its prompt not
//! back.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::session::{Redact, StopSignal};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A terminal elsewhere, as a remote session needs it.
pub trait Remote: Send + Sync + 'static {
  /// Resizes the terminal.
  fn resize(&self, rows: u16, cols: u16);
  /// Writes `data` to a file `name` of the session's own there (owner
  /// only); its path, as the shell there names it.
  fn put_file(&self, name: String, data: Vec<u8>) -> BoxFuture<'_, std::io::Result<String>>;
  /// Signals the process group that holds the terminal there, when it is a
  /// command's (never the shell's own); its number there.
  fn signal(&self, signal: StopSignal) -> BoxFuture<'_, Option<i32>>;
  /// Ends the shell (and what keeps it, a tmux session).
  fn close(&self) -> BoxFuture<'_, ()>;
  /// How it ended, once its output did: the shell's exit, or the
  /// connection dropped.
  fn ended(&self) -> BoxFuture<'_, String>;
}

/// A shell elsewhere picked up where it was (a tmux session after a
/// reconnect).
#[derive(Clone, Debug, Default)]
pub struct Resumed {
  /// What draws its screen as it is (escape sequences), cursor included.
  pub screen: Vec<u8>,
  /// What runs there, when the shell is not at its prompt.
  pub running: Option<String>,
}

/// What `Session::remote` starts from.
pub struct RemoteConfig {
  /// The marker's secret, as `remote_rc` was given it.
  pub nonce: String,
  /// The terminal's output.
  pub output: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
  /// Its input.
  pub input: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
  pub remote: Arc<dyn Remote>,
  pub rows: u16,
  pub cols: u16,
  pub max_output: usize,
  pub redact: Option<Redact>,
  pub resumed: Option<Resumed>,
}

/// The rc of a remote bash: the user's login profile read quietly (their
/// `PATH`), then what the local default has (a plain prompt, no history
/// file, no pager) and the marker, its secret written in (no environment
/// passes over SSH). The rc is owner only, in the session's directory.
pub fn remote_rc(nonce: &str) -> String {
  format!(
    r#"{{ [ -f /etc/profile ] && . /etc/profile; if [ -f "$HOME/.bash_profile" ]; then . "$HOME/.bash_profile"; elif [ -f "$HOME/.profile" ]; then . "$HOME/.profile"; fi; }} </dev/null >/dev/null 2>&1
PS1='$ '
PS2=''
set +H
unset HISTFILE
unset PROMPT_COMMAND
export PAGER=cat GIT_PAGER=cat
__stitch_n='{nonce}'
__stitch_prompt() {{ local s=$?; printf '\033]777;stitch;%s;prompt;%s\007' "$__stitch_n" "$s"; }}
PROMPT_COMMAND=__stitch_prompt
"#
  )
}
