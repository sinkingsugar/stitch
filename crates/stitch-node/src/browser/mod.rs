//! Browser control (design §2.18, docs/browser-tool.md): the user's own
//! Chrome, reached through Stitch's extension, which `stitch
//! browser-host` relays to this node as a *browser link* per Chrome
//! profile. The registry here keeps the links and the profiles that have
//! connected; `engine` runs the agents' sessions on them and the
//! `browser_*` tools; `safety` decides what asks; `install` writes the
//! extension's files and the native messaging host manifests.
//!
//! The extension enforces the hard boundaries itself (its tabs only, no
//! cookies, nothing until the user enables the profile in its popup), so
//! nothing here is trusted to keep the user's other tabs out of reach.

pub mod engine;
#[cfg(not(mobile))]
pub mod host;
#[cfg(not(mobile))]
pub mod install;
pub mod safety;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::api::Reply;

/// The protocol the extension speaks (its hello's `proto`).
pub const PROTO: u64 = 1;
/// The extension's pinned ID (from the key in `apps/chrome/manifest.json`).
pub const EXTENSION_ID: &str = "laglcmonaljgkdmikgohcogdmjihlhel";
/// The native messaging host's name.
pub const HOST_NAME: &str = "com.fragcolor.stitch.browser";

/// A Chrome profile as its extension describes itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
  /// A random ID the extension keeps in the profile.
  pub id: String,
  /// The name the user gave it in the popup (may be empty).
  pub name: String,
  /// `chrome`, `brave`, `edge`, `chromium`.
  pub browser: String,
  /// The user switched Stitch on in this profile.
  pub enabled: bool,
}

impl Profile {
  /// How tools and commands name it: `chrome:work`, or `chrome:3f9a1c`
  /// before the user names it.
  pub fn label(&self) -> String {
    let name = if self.name.trim().is_empty() {
      self.id.chars().take(6).collect::<String>()
    } else {
      self
        .name
        .trim()
        .to_lowercase()
        .replace(char::is_whitespace, "-")
    };
    format!("{}:{name}", self.browser)
  }

  /// Whether `wanted` (`chrome:work`, `work`, `chrome`) names it.
  pub fn matches(&self, wanted: &str) -> bool {
    let label = self.label();
    let wanted = wanted.trim().to_lowercase();
    label == wanted
      || label.split_once(':').map(|(_, n)| n) == Some(wanted.as_str())
      || self.browser == wanted
      || self.id == wanted
  }
}

/// The browser a user agent's brands name.
fn browser_of(brand: &str, ua: &str) -> String {
  let b = brand.to_lowercase();
  if b.contains("brave") {
    "brave".into()
  } else if b.contains("edge") || ua.contains("Edg/") {
    "edge".into()
  } else if b.contains("google chrome") && !b.contains("for testing") {
    "chrome".into()
  } else {
    "chromium".into()
  }
}

/// What the extension tells about the user, kept per link (for
/// "watching", §8.2).
#[derive(Clone, Debug, Default)]
pub struct Presence {
  /// The focused window (`-1`: none of Chrome's).
  pub focused: Option<i64>,
  /// The active tab of each window, as last reported.
  pub active: HashMap<i64, i64>,
  /// `active`, `idle` or `locked` (chrome.idle, 60 s).
  pub idle: String,
}

/// One connected profile.
pub struct Link {
  pub id: u64,
  pub profile: Mutex<Profile>,
  pub version: String,
  /// The reload hint, when the extension Chrome runs is older than this
  /// node's files.
  stale: Option<String>,
  tx: mpsc::UnboundedSender<Reply>,
  pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
  next: AtomicU64,
  /// The extension's events, for the engine.
  pub events: broadcast::Sender<Value>,
  pub presence: Mutex<Presence>,
  pub connected_at: Instant,
  /// Per tab, the last note of a CDP command the extension did without
  /// the debugger (`stitch_fallback`: Chrome refused it, another
  /// extension's frame being in the tab), for the tool's result.
  fallback: Mutex<HashMap<i64, String>>,
  /// The sessions the extension has said it knows (its hello's
  /// `sessions`, plus those opened since); none when it does not say.
  /// A reload of the extension forgets them all.
  known: Mutex<Option<HashSet<String>>>,
}

impl Link {
  /// Whether the extension has the session (true when it never said).
  pub(crate) fn knows(&self, session: &str) -> bool {
    self
      .known
      .lock()
      .expect("link lock")
      .as_ref()
      .is_none_or(|k| k.contains(session))
  }

  /// The session was opened in the extension.
  pub(crate) fn opened(&self, session: &str) {
    if let Some(k) = self.known.lock().expect("link lock").as_mut() {
      k.insert(session.to_owned());
    }
  }

  /// Set when Chrome runs an older extension than this node's files.
  pub fn stale_hint(&self) -> Option<String> {
    self.stale.clone()
  }

  /// Sends a request to the extension and waits for its answer.
  pub async fn req(&self, op: &str, mut args: Value, timeout: Duration) -> Result<Value, String> {
    let id = self.next.fetch_add(1, Ordering::SeqCst);
    let (done, wait) = oneshot::channel();
    self.pending.lock().expect("link lock").insert(id, done);
    if !args.is_object() {
      args = json!({});
    }
    args["t"] = "req".into();
    args["id"] = id.into();
    args["op"] = op.into();
    if self.tx.send(Reply::BrowserMsg(args.to_string())).is_err() {
      self.pending.lock().expect("link lock").remove(&id);
      return Err("the browser disconnected".into());
    }
    match tokio::time::timeout(timeout, wait).await {
      Ok(Ok(r)) => r,
      Ok(Err(_)) => Err("the browser disconnected".into()),
      Err(_) => {
        self.pending.lock().expect("link lock").remove(&id);
        Err(format!(
          "the browser did not answer `{op}` within {}s",
          timeout.as_secs()
        ))
      }
    }
  }

  /// A CDP command on a tab, through the extension.
  pub async fn cdp(&self, tab: i64, method: &str, params: Value) -> Result<Value, String> {
    let r = self
      .req(
        "cdp",
        json!({"tab": tab, "method": method, "params": params}),
        Duration::from_secs(30),
      )
      .await?;
    if let Some(note) = r.get("stitch_fallback").and_then(Value::as_str) {
      self
        .fallback
        .lock()
        .expect("link lock")
        .insert(tab, note.to_owned());
    }
    Ok(r)
  }

  /// The note of the last command on `tab` done without the debugger
  /// since the last call, if any (see `fallback`).
  pub fn take_fallback(&self, tab: i64) -> Option<String> {
    self.fallback.lock().expect("link lock").remove(&tab)
  }

  /// A message to the page's script in a tab; its `result`.
  pub async fn page(&self, tab: i64, msg: Value) -> Result<Value, String> {
    let r = self
      .req(
        "page",
        json!({"tab": tab, "msg": msg}),
        Duration::from_secs(20),
      )
      .await?;
    if r.get("ok").and_then(Value::as_bool) == Some(true) {
      Ok(r.get("result").cloned().unwrap_or(Value::Null))
    } else if r.is_null() {
      Err("the page's Stitch script did not answer (the page is still loading, or Chrome does not let extensions in it)".into())
    } else {
      Err(
        r.get("error")
          .and_then(Value::as_str)
          .unwrap_or("the page's script failed")
          .to_owned(),
      )
    }
  }

  fn deliver(&self, msg: Value) {
    match msg.get("t").and_then(Value::as_str) {
      Some("res") => {
        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
          return;
        };
        if let Some(done) = self.pending.lock().expect("link lock").remove(&id) {
          let r = if msg.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(msg.get("result").cloned().unwrap_or(Value::Null))
          } else {
            Err(
              msg
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("failed")
                .to_owned(),
            )
          };
          let _ = done.send(r);
        }
      }
      Some("event") => {
        self.note_presence(&msg);
        let _ = self.events.send(msg);
      }
      Some("hello") => {
        if let Ok(h) = parse_hello(&msg) {
          *self.profile.lock().expect("link lock") = h.0;
          *self.known.lock().expect("link lock") = hello_sessions(&msg);
        }
      }
      _ => {}
    }
  }

  fn note_presence(&self, msg: &Value) {
    let mut p = self.presence.lock().expect("link lock");
    match msg.get("kind").and_then(Value::as_str) {
      Some("focus") => p.focused = msg.get("window").and_then(Value::as_i64),
      Some("activated") => {
        if let (Some(w), Some(t)) = (
          msg.get("window").and_then(Value::as_i64),
          msg.get("tab").and_then(Value::as_i64),
        ) {
          p.active.insert(w, t);
        }
      }
      Some("idle") => {
        p.idle = msg
          .get("state")
          .and_then(Value::as_str)
          .unwrap_or("")
          .to_owned()
      }
      Some("user_input") => p.idle = "active".into(),
      _ => {}
    }
  }
}

/// The sessions a hello lists, if it does.
fn hello_sessions(v: &Value) -> Option<HashSet<String>> {
  Some(
    v.get("sessions")?
      .as_array()?
      .iter()
      .filter_map(|s| s.as_str().map(str::to_owned))
      .collect(),
  )
}

fn parse_hello(v: &Value) -> Result<(Profile, String), String> {
  let p = v.get("profile").ok_or("a hello without a profile")?;
  let id = p
    .get("id")
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty() && s.len() <= 64)
    .ok_or("a hello without a profile ID")?;
  let b = v.get("browser").cloned().unwrap_or(Value::Null);
  let browser = browser_of(
    b.get("brand").and_then(Value::as_str).unwrap_or(""),
    b.get("ua").and_then(Value::as_str).unwrap_or(""),
  );
  let name: String = p
    .get("name")
    .and_then(Value::as_str)
    .unwrap_or("")
    .chars()
    .filter(|c| !c.is_control())
    .take(40)
    .collect();
  Ok((
    Profile {
      id: id.to_owned(),
      name,
      browser,
      enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(false),
    },
    v.get("version")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_owned(),
  ))
}

/// A profile that connected once, as `profiles.json` keeps it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Known {
  #[serde(flatten)]
  profile: Profile,
  last_seen: i64,
  /// `stitch browser profiles disable` while it was away: switched off at
  /// its next connection.
  #[serde(default)]
  disable_pending: bool,
}

/// The node's browsers: the links connected now, and every profile that
/// connected once.
pub struct Browsers {
  file: PathBuf,
  ext_dir: PathBuf,
  links: Mutex<Vec<Arc<Link>>>,
  known: Mutex<Vec<Known>>,
  next: AtomicU64,
  pub engine: engine::Engine,
  /// Told when a link comes or goes.
  pub changed: broadcast::Sender<()>,
}

impl Browsers {
  pub fn new(data: &Path, settings: crate::config::BrowserSettings) -> Arc<Browsers> {
    let file = data.join("browser").join("profiles.json");
    let known = std::fs::read(&file)
      .ok()
      .and_then(|b| serde_json::from_slice(&b).ok())
      .unwrap_or_default();
    #[cfg(not(mobile))]
    match install::refresh(data) {
      Ok(true) => eprintln!("stitch: browser extension files refreshed"),
      Ok(false) => {}
      Err(e) => eprintln!("stitch: could not refresh the browser extension files: {e}"),
    }
    Arc::new(Browsers {
      ext_dir: data.join("browser").join("extension"),
      file,
      links: Mutex::default(),
      known: Mutex::new(known),
      next: AtomicU64::new(1),
      engine: engine::Engine::new(settings),
      changed: broadcast::channel(16).0,
    })
  }

  /// A browser link's start: its hello, and where its messages go.
  pub fn connect(&self, hello: &str, tx: mpsc::UnboundedSender<Reply>) -> crate::Result<Arc<Link>> {
    let v: Value = serde_json::from_str(hello)
      .map_err(|e| crate::Error::Protocol(format!("browser hello: {e}")))?;
    let proto = v.get("proto").and_then(Value::as_u64).unwrap_or(0);
    if proto != PROTO {
      return Err(crate::Error::Protocol(format!(
        "the Stitch extension speaks protocol {proto}, this node {PROTO}: run `stitch browser install` and reload the extension"
      )));
    }
    let (profile, version) = parse_hello(&v).map_err(crate::Error::Protocol)?;
    // No extension is installed from a phone's node.
    #[cfg(mobile)]
    let stale: Option<String> = {
      let _ = &self.ext_dir;
      None
    };
    #[cfg(not(mobile))]
    let stale = match v.get("build").and_then(Value::as_str) {
      Some("dev") => None,
      Some(b) if b == install::build_id() => None,
      _ => Some(format!(
        "The Stitch extension running in Chrome is older than this node's and was asked to reload itself; if this persists, open chrome://extensions and press reload on Stitch's card (its files are already updated at {}).",
        self.ext_dir.display()
      )),
    };
    // Ask a stale extension to re-read its files; one that does not know
    // the message ignores it.
    #[cfg(not(mobile))]
    if stale.is_some() {
      let _ = tx.send(Reply::BrowserMsg(
        json!({"t": "reload", "build": install::build_id()}).to_string(),
      ));
    }
    let link = Arc::new(Link {
      id: self.next.fetch_add(1, Ordering::SeqCst),
      profile: Mutex::new(profile.clone()),
      version,
      stale,
      tx,
      pending: Mutex::default(),
      next: AtomicU64::new(1),
      events: broadcast::channel(256).0,
      presence: Mutex::new(Presence {
        idle: "active".into(),
        ..Presence::default()
      }),
      connected_at: Instant::now(),
      fallback: Mutex::default(),
      known: Mutex::new(hello_sessions(&v)),
    });
    let disable = {
      let mut links = self.links.lock().expect("browsers lock");
      // The same profile again (its host restarted): the old link is gone.
      links.retain(|l| l.profile.lock().expect("link lock").id != profile.id);
      links.push(link.clone());
      self.remember(&profile)
    };
    if disable {
      let l = link.clone();
      tokio::spawn(async move {
        let _ = l.req("disable", json!({}), Duration::from_secs(10)).await;
      });
    }
    self.engine.linked(&link);
    let _ = self.changed.send(());
    Ok(link)
  }

  /// A message from a link's extension.
  pub fn incoming(&self, link: &Arc<Link>, msg: &str) {
    let Ok(v) = serde_json::from_str::<Value>(msg) else {
      return;
    };
    let hello = v.get("t").and_then(Value::as_str) == Some("hello");
    link.deliver(v);
    if hello {
      let p = link.profile.lock().expect("link lock").clone();
      self.remember(&p);
      let _ = self.changed.send(());
    }
  }

  pub fn disconnect(&self, link: &Arc<Link>) {
    self
      .links
      .lock()
      .expect("browsers lock")
      .retain(|l| l.id != link.id);
    for (_, done) in link.pending.lock().expect("link lock").drain() {
      let _ = done.send(Err("the browser disconnected".into()));
    }
    let _ = self.changed.send(());
  }

  /// Records a profile; whether a disable waits for it.
  fn remember(&self, p: &Profile) -> bool {
    let mut known = self.known.lock().expect("browsers lock");
    let now = jiff::Timestamp::now().as_second();
    let disable = match known.iter_mut().find(|k| k.profile.id == p.id) {
      Some(k) => {
        let d = k.disable_pending && p.enabled;
        k.profile = p.clone();
        k.last_seen = now;
        k.disable_pending = false;
        d
      }
      None => {
        known.push(Known {
          profile: p.clone(),
          last_seen: now,
          disable_pending: false,
        });
        false
      }
    };
    self.save(&known);
    disable
  }

  fn save(&self, known: &[Known]) {
    if let Some(dir) = self.file.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(known) {
      let _ = std::fs::write(&self.file, bytes);
    }
  }

  /// The links connected now.
  pub fn links(&self) -> Vec<Arc<Link>> {
    self.links.lock().expect("browsers lock").clone()
  }

  /// Whether any profile is connected and enabled (the `browser` tools
  /// are offered then).
  pub fn any_enabled(&self) -> bool {
    self
      .links
      .lock()
      .expect("browsers lock")
      .iter()
      .any(|l| l.profile.lock().expect("link lock").enabled)
  }

  /// `stitch browser profiles`: every profile that connected, and
  /// `disable` turns one off first.
  pub async fn profiles(&self, disable: Option<&str>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    if let Some(name) = disable {
      let link = self
        .links()
        .into_iter()
        .find(|l| l.profile.lock().expect("link lock").matches(name));
      match link {
        Some(l) => {
          l.req("disable", json!({}), Duration::from_secs(10)).await?;
          l.profile.lock().expect("link lock").enabled = false;
          out.push(format!(
            "disabled {}",
            l.profile.lock().expect("link lock").label()
          ));
        }
        None => {
          let mut known = self.known.lock().expect("browsers lock");
          let Some(k) = known.iter_mut().find(|k| k.profile.matches(name)) else {
            return Err(format!(
              "no browser profile {name} has connected to this node"
            ));
          };
          k.disable_pending = true;
          k.profile.enabled = false;
          out.push(format!(
            "{} is not connected: switched off when it next connects",
            k.profile.label()
          ));
          let snapshot = known.clone();
          drop(known);
          self.save(&snapshot);
        }
      }
    }
    let links = self.links();
    let known = self.known.lock().expect("browsers lock").clone();
    if known.is_empty() {
      out.push(
        "no browser has connected: run `stitch browser install`, load the extension, and switch it on in its popup".into(),
      );
    }
    for k in known {
      let live = links
        .iter()
        .find(|l| l.profile.lock().expect("link lock").id == k.profile.id);
      let p = live
        .map(|l| l.profile.lock().expect("link lock").clone())
        .unwrap_or(k.profile.clone());
      let sessions = live.map(|l| self.engine.sessions_on(l.id)).unwrap_or(0);
      out.push(format!(
        "{}  {}  {}{}",
        p.label(),
        if p.enabled { "enabled" } else { "disabled" },
        if live.is_some() {
          "connected"
        } else {
          "not connected"
        },
        if sessions > 0 {
          format!(", {sessions} session(s)")
        } else {
          String::new()
        }
      ));
    }
    Ok(out)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn profiles_are_named_by_browser_and_name() {
    let mut p = Profile {
      id: "3f9a1c2b-0000".into(),
      name: String::new(),
      browser: "chrome".into(),
      enabled: true,
    };
    assert_eq!(p.label(), "chrome:3f9a1c");
    p.name = "Work Stuff".into();
    assert_eq!(p.label(), "chrome:work-stuff");
    assert!(p.matches("work-stuff"));
    assert!(p.matches("chrome:work-stuff"));
    assert!(p.matches("chrome"));
    assert!(!p.matches("brave"));
    assert_eq!(browser_of("Brave 153, Chromium 153", ""), "brave");
    assert_eq!(browser_of("Google Chrome 153, Chromium 153", ""), "chrome");
    assert_eq!(browser_of("Google Chrome for Testing 153", ""), "chromium");
  }

  #[tokio::test]
  async fn a_link_answers_requests_and_refuses_old_protocols() {
    let dir = tempfile::tempdir().unwrap();
    let b = Browsers::new(dir.path(), Default::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    assert!(
      b.connect(
        r#"{"t":"hello","proto":0,"profile":{"id":"x"}}"#,
        tx.clone()
      )
      .is_err()
    );
    let link = b
      .connect(
        r#"{"t":"hello","proto":1,"build":"dev","version":"0.1.0","profile":{"id":"abc","name":"","enabled":true},"browser":{"brand":"Google Chrome 153"}}"#,
        tx,
      )
      .unwrap();
    assert!(b.any_enabled());
    let l = link.clone();
    let asking =
      tokio::spawn(async move { l.req("tabs", json!({}), Duration::from_secs(5)).await });
    let Some(Reply::BrowserMsg(m)) = rx.recv().await else {
      panic!("no request")
    };
    let m: Value = serde_json::from_str(&m).unwrap();
    assert_eq!(m["op"], "tabs");
    b.incoming(
      &link,
      &json!({"t":"res","id":m["id"],"ok":true,"result":[1,2]}).to_string(),
    );
    assert_eq!(asking.await.unwrap().unwrap(), json!([1, 2]));
    let lines = b.profiles(None).await.unwrap();
    assert!(
      lines[0].starts_with("chrome:abc  enabled  connected"),
      "{lines:?}"
    );
    b.disconnect(&link);
    assert!(!b.any_enabled());
    // Remembered across restarts.
    let again = Browsers::new(dir.path(), Default::default());
    let lines = again.profiles(Some("chrome:abc")).await.unwrap();
    assert!(lines[0].contains("not connected"), "{lines:?}");
    assert!(lines[1].starts_with("chrome:abc  disabled"), "{lines:?}");
  }

  #[cfg(not(mobile))]
  #[tokio::test]
  async fn a_stale_extension_build_is_flagged_with_a_reload_hint() {
    let dir = tempfile::tempdir().unwrap();
    let b = Browsers::new(dir.path(), Default::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let hello = |build: Option<&str>, id: &str| {
      let build = build
        .map(|b| format!(r#""build":"{b}","#))
        .unwrap_or_default();
      format!(
        r#"{{"t":"hello","proto":1,{build}"version":"0.1.0","profile":{{"id":"{id}","name":"","enabled":true}},"browser":{{"brand":"Google Chrome 153"}}}}"#
      )
    };
    let current = install::build_id();
    for (build, id) in [(None, "a"), (Some("0000other"), "b")] {
      let l = b.connect(&hello(build, id), tx.clone()).unwrap();
      let h = l.stale_hint().expect("stale");
      let Ok(Reply::BrowserMsg(m)) = rx.try_recv() else {
        panic!("a stale hello must make the node send a reload")
      };
      let m: Value = serde_json::from_str(&m).unwrap();
      assert_eq!(m["t"], "reload");
      assert_eq!(m["build"], current.as_str());
      assert!(
        h.contains("chrome://extensions") && h.contains("reload"),
        "{h}"
      );
      assert!(
        h.contains(&dir.path().join("browser/extension").display().to_string()),
        "{h}"
      );
    }
    for (build, id) in [(Some(current.as_str()), "c"), (Some("dev"), "d")] {
      let l = b.connect(&hello(build, id), tx.clone()).unwrap();
      assert!(l.stale_hint().is_none());
      assert!(
        rx.try_recv().is_err(),
        "no reload for a current or dev build"
      );
    }
  }
}
