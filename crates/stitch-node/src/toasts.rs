//! Notifications on this machine's own desktop (design §2.3, "Notices"):
//! what the node pushes to the user's phones (`node::push`: a notice, a
//! question, loose ends, a schedule paused) it also shows here, through
//! `notify-rust`: D-Bus `org.freedesktop.Notifications` on Linux (GNOME,
//! KDE, mako, dunst…), the Notification Center on macOS, a toast on
//! Windows. Only this node's: a thread's notice shows on the desktop of
//! the node that runs it, as it is pushed from there.
//!
//! On for the user's node (data in `~/.stitch`), off for any other data
//! directory (tests, throwaway nodes), unless `[notifications] desktop`
//! says otherwise. A question's notification stays until it is answered,
//! here or anywhere, and then goes (Linux; macOS's center keeps it). A
//! machine without a desktop (no session bus) shows nothing and says so
//! once in the log.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use notify_rust::{Notification, Timeout};

use crate::apns::Push;
use crate::config::Config;

/// The longest a notification server is waited on: one that never
/// answers (a bus activating a daemon that is not there) holds up no push.
const SHOW_LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct Toasts {
  /// Each open question's notification, to take away once it is answered.
  #[cfg(all(unix, not(target_os = "macos")))]
  questions: Mutex<HashMap<String, notify_rust::NotificationHandle>>,
  #[cfg(not(all(unix, not(target_os = "macos"))))]
  questions: Mutex<HashMap<String, ()>>,
  /// A failure was logged already (a headless machine fails every time).
  failed: AtomicBool,
}

/// Whether this node shows notifications on its desktop.
pub(crate) fn enabled(data: &Path) -> bool {
  let set = Config::load(data)
    .ok()
    .and_then(|c| c.notifications)
    .and_then(|n| n.desktop);
  set.unwrap_or_else(|| is_users_node(data)) && has_desktop()
}

/// The user's own node keeps its data in `~/.stitch`.
fn is_users_node(data: &Path) -> bool {
  let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
    return false;
  };
  let default = Path::new(&home).join(".stitch");
  let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
  canon(data) == canon(&default)
}

/// Linux: a session bus to reach (a user service has it; a headless box
/// or a system service has none). Elsewhere, always.
fn has_desktop() -> bool {
  #[cfg(all(unix, not(target_os = "macos")))]
  {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
      || std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| Path::new(&d).join("bus").exists())
  }
  #[cfg(not(all(unix, not(target_os = "macos"))))]
  {
    true
  }
}

/// What a push shows: its title, and its body under the subtitle (the
/// thread's title) where the desktop has no subtitles.
fn content(p: &Push<'_>) -> (String, Option<String>, String) {
  let title = if p.title.is_empty() {
    "Stitch"
  } else {
    p.title
  };
  let subtitle = p.subtitle.filter(|s| !s.trim().is_empty());
  if cfg!(target_os = "macos") {
    (
      title.to_owned(),
      subtitle.map(str::to_owned),
      p.body.to_owned(),
    )
  } else {
    let body = match subtitle {
      Some(s) => format!("{s}\n{}", p.body),
      None => p.body.to_owned(),
    };
    (title.to_owned(), None, markup_escaped(&body))
  }
}

/// The freedesktop body is markup where the server takes it (GNOME, KDE,
/// mako, dunst do): a command's `<` and `&` must stay text.
fn markup_escaped(s: &str) -> String {
  if cfg!(all(unix, not(target_os = "macos"))) {
    s.replace('&', "&amp;")
      .replace('<', "&lt;")
      .replace('>', "&gt;")
  } else {
    s.to_owned()
  }
}

impl Toasts {
  /// Shows `p` on this desktop (or, for a question answered, takes its
  /// notification away). A line saying what happened, `None` when there
  /// was nothing to do.
  pub(crate) async fn show(&self, data: &Path, node: &str, p: &Push<'_>) -> Option<String> {
    if p.answered {
      self.answered(p.question?).await;
      return None;
    }
    if !enabled(data) {
      return None;
    }
    let (title, subtitle, body) = content(p);
    let mut n = Notification::new();
    n.appname("Stitch").summary(&title).body(&body);
    if let Some(s) = &subtitle {
      n.subtitle(s);
    }
    if p.question.is_some() {
      // Until it is answered: then it is taken away.
      n.timeout(Timeout::Never);
    }
    match self.post(n, p.question).await {
      Ok(()) => {
        self.failed.store(false, Ordering::Relaxed);
        Some(format!("{node}: shown on this desktop"))
      }
      Err(e) => {
        if !self.failed.swap(true, Ordering::Relaxed) {
          eprintln!("desktop notifications: not shown: {e}");
        }
        Some(format!("{node}: not shown on this desktop: {e}"))
      }
    }
  }

  #[cfg(all(unix, not(target_os = "macos")))]
  async fn post(&self, n: Notification, question: Option<&str>) -> Result<(), String> {
    let handle = tokio::time::timeout(SHOW_LIMIT, n.show_async())
      .await
      .map_err(|_| "the notification server did not answer".to_owned())?
      .map_err(|e| e.to_string())?;
    if let Some(q) = question {
      let old = self
        .questions
        .lock()
        .expect("toasts lock")
        .insert(q.to_owned(), handle);
      if let Some(old) = old {
        old.close_async().await;
      }
    }
    Ok(())
  }

  /// macOS and Windows show synchronously (no D-Bus): off the runtime.
  #[cfg(not(all(unix, not(target_os = "macos"))))]
  async fn post(&self, n: Notification, _question: Option<&str>) -> Result<(), String> {
    let shown = tokio::task::spawn_blocking(move || {
      // Left to itself, mac-notification-sys finds the app to post as with
      // an AppleScript (`id of application "use_default"`), which hangs
      // asking where that app is. A node posts as the Stitch app where it
      // is installed (its icon, its own switch in Settings), else as
      // Terminal. It can be set once only, failure included, so the app is
      // looked for first.
      #[cfg(target_os = "macos")]
      {
        static APP: std::sync::Once = std::sync::Once::new();
        APP.call_once(|| {
          let app = if Path::new("/Applications/Stitch.app").exists() {
            "com.fragcolor.stitch"
          } else {
            "com.apple.Terminal"
          };
          if let Err(e) = notify_rust::set_application(app) {
            eprintln!("desktop notifications: not posting as {app}: {e}");
          }
        });
      }
      // The handle posts as it drops (waiting up to 2 s for delivery).
      n.show().map(drop).map_err(|e| e.to_string())
    });
    tokio::time::timeout(SHOW_LIMIT, shown)
      .await
      .map_err(|_| "the notification center did not answer".to_owned())?
      .map_err(|e| e.to_string())?
  }

  /// A question is answered (or withdrawn): its notification goes.
  async fn answered(&self, question: &str) {
    let handle = self.questions.lock().expect("toasts lock").remove(question);
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(h) = handle {
      h.close_async().await;
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = handle;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn push<'a>(title: &'a str, subtitle: Option<&'a str>, body: &'a str) -> Push<'a> {
    Push {
      title,
      subtitle,
      body,
      badge: None,
      question: None,
      node: "n",
      answered: false,
      thread: None,
    }
  }

  #[test]
  fn a_push_reads_as_a_notification() {
    let (title, subtitle, body) = content(&push("", Some("Fix CI"), "echo a < b && c"));
    assert_eq!(title, "Stitch");
    if cfg!(target_os = "macos") {
      assert_eq!(subtitle.as_deref(), Some("Fix CI"));
      assert_eq!(body, "echo a < b && c");
    } else {
      assert_eq!(subtitle, None);
      assert_eq!(body, "Fix CI\necho a &lt; b &amp;&amp; c");
    }
    let (_, _, body) = content(&push("Approve on gamer?", Some("  "), "ls"));
    assert_eq!(body, "ls");
  }

  /// Shows a notice and a question on this desktop, then takes the
  /// question away: `cargo test -p stitch-node toasts -- --ignored`, and
  /// look.
  #[tokio::test]
  #[ignore = "shows notifications on the desktop"]
  async fn shows_on_this_desktop() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
      dir.path().join("config.toml"),
      "[notifications]\ndesktop = true\n",
    )
    .unwrap();
    let t = Toasts::default();
    let line = t
      .show(
        dir.path(),
        "n",
        &push("Stitch", Some("A thread"), "A notice <b>&</b>"),
      )
      .await;
    assert_eq!(line.as_deref(), Some("n: shown on this desktop"));
    let mut q = push("Approve on n?", Some("A thread"), "rm -rf target");
    q.question = Some("q1");
    let line = t.show(dir.path(), "n", &q).await;
    assert_eq!(line.as_deref(), Some("n: shown on this desktop"));
    tokio::time::sleep(Duration::from_secs(4)).await;
    q.answered = true;
    assert_eq!(t.show(dir.path(), "n", &q).await, None);
  }

  #[test]
  fn only_the_users_node_shows_them_by_default() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!is_users_node(dir.path()));
    assert!(!enabled(dir.path()));
  }
}
