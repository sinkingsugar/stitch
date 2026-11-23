//! `browse_web` and `web_eval` in a browser the host app supplies (design
//! §2.3): where the node starts no Chrome (iOS and visionOS), the app
//! lends it pages of its own engine (WebKit, offscreen). The host only
//! loads a page, runs a script in it, screenshots it and closes it; what
//! is read and how (settling, `wait_for`, the main content, the model's
//! code and its console) is the node's, the same scripts as the desktop's
//! Chromium runs (`read`).
//!
//! The host's calls block (the app answers on its main thread): they are
//! made off the node's async threads, with `spawn_blocking`.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;

use super::read;

/// A page the host loaded, or what its main document turned out to be.
#[derive(Clone, Debug, Default)]
pub struct HostPage {
  /// The host's handle of the page (none kept for a file).
  pub page: u64,
  /// Where the page is: after redirects, where a script moved it on.
  pub url: String,
  /// The main document's HTTP status (0 when not known).
  pub status: i64,
  /// The main document's media type, lowercase.
  pub mime: String,
  /// Not a web page (a PDF, an image, a download): the host kept nothing
  /// and the node fetches `url` as a file.
  pub file: bool,
}

/// Why a script did not run to its end.
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
  /// The page went away under it (it navigated, a check that passed and
  /// reloaded): it may be run again in the next document.
  Navigated,
  /// It did not finish within its time.
  TimedOut,
  /// It did not compile or threw outside the node's own wrapper: the
  /// engine's message, and the line of the body (0 when not known).
  Script { message: String, line: i64 },
  /// Anything else (the page is gone, the engine failed).
  Failed(String),
}

/// A region of the page's viewport, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

/// A browser the host app lends the node: each page fresh, with nothing
/// of the user's (no cookies, storage or logins, nothing shared between
/// pages), its downloads refused, its dialogs dismissed, its media
/// silent, about 1280×900 CSS pixels.
pub trait PageHost: Send + Sync + 'static {
  /// Loads `url` in a fresh page; returns once it loaded (or committed
  /// and stopped loading for long: a subresource that never ends holds no
  /// call), or at once when the main document is not a page (`file`).
  fn open(&self, url: &str, timeout: Duration) -> Result<HostPage, String>;
  /// Where the page is now.
  fn state(&self, page: u64) -> Result<HostPage, String>;
  /// Runs `body` as the body of an async function in the page's own world
  /// (its globals seen), awaited; the string it returns (`None` for
  /// anything else).
  fn call(&self, page: u64, body: &str, timeout: Duration) -> Result<Option<String>, CallError>;
  /// A PNG of the viewport, or of `area` of it.
  fn screenshot(&self, page: u64, area: Option<Area>) -> Result<Vec<u8>, String>;
  /// Disposes of the page; does not wait.
  fn close(&self, page: u64);
}

static HOST: RwLock<Option<Arc<dyn PageHost>>> = RwLock::new(None);

/// Lends the node a browser (the app does, before it opens the node).
pub fn set_host(host: Arc<dyn PageHost>) {
  *HOST.write().unwrap_or_else(|e| e.into_inner()) = Some(host);
}

/// The browser the host lent, if any.
pub(super) fn host() -> Option<Arc<dyn PageHost>> {
  HOST.read().unwrap_or_else(|e| e.into_inner()).clone()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
  tokio::task::spawn_blocking(f)
    .await
    .map_err(|e| e.to_string())
}

/// A page loaded by the host, closed when dropped.
pub(super) struct Opened {
  host: Arc<dyn PageHost>,
  id: u64,
  pub url: String,
  pub status: i64,
}

impl Drop for Opened {
  fn drop(&mut self) {
    self.host.close(self.id);
  }
}

pub(super) enum Loaded {
  Page(Opened),
  File { url: String },
}

impl Opened {
  /// Runs an expression (a value or a promise); its value, made JSON.
  async fn value(&self, expression: &str, timeout: Duration) -> Result<Value, CallError> {
    let body = format!(
      "const v = await ({expression});\nreturn JSON.stringify(v === undefined ? null : v);"
    );
    let (host, id) = (self.host.clone(), self.id);
    let text = blocking(move || host.call(id, &body, timeout))
      .await
      .map_err(CallError::Failed)??;
    Ok(
      text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null),
    )
  }

  /// `value`, run again in the next document when the page navigates
  /// while it runs (a few times at most); `reloaded` says one did.
  async fn value_across_reloads(
    &self,
    expression: &str,
    timeout: Duration,
    reloaded: &mut bool,
  ) -> Result<Value, String> {
    for _ in 0..3 {
      match self.value(expression, timeout).await {
        Err(CallError::Navigated) => {
          *reloaded = true;
          // The next document's own load.
          tokio::time::sleep(Duration::from_millis(500)).await;
        }
        done => return done.map_err(|e| describe(&e, timeout, 0)),
      }
    }
    self
      .value(expression, timeout)
      .await
      .map_err(|e| describe(&e, timeout, 0))
  }
}

/// A script's failure as a tool says it; `prefix` is the lines the node's
/// wrapper put before the model's code.
fn describe(e: &CallError, timeout: Duration, prefix: i64) -> String {
  match e {
    CallError::Navigated => "the page navigated away while the code ran".into(),
    CallError::TimedOut => format!("the code did not finish within {}s", timeout.as_secs()),
    CallError::Script { message, line } if *line > prefix && prefix > 0 => {
      format!(
        "JavaScript error: {message} (line {} of the code)",
        line - prefix
      )
    }
    CallError::Script { message, .. } => format!("JavaScript error: {message}"),
    CallError::Failed(e) => e.clone(),
  }
}

/// Loads `url`; for a web page, waits until `wait_for` shows (a CSS
/// selector) or the page stops changing.
pub(super) async fn load(
  host: Arc<dyn PageHost>,
  url: &str,
  wait_for: Option<&str>,
  timeout: Duration,
) -> Result<Loaded, String> {
  let (h, u) = (host.clone(), url.to_owned());
  let got = blocking(move || h.open(&u, timeout)).await??;
  if got.file {
    return Ok(Loaded::File {
      url: if got.url.is_empty() {
        url.to_owned()
      } else {
        got.url
      },
    });
  }
  let mut opened = Opened {
    host,
    id: got.page,
    url: got.url,
    status: got.status,
  };
  let mut reloaded = false;
  match wait_for {
    Some(selector) => {
      let found = opened
        .value_across_reloads(&read::wait_for(selector, timeout), timeout, &mut reloaded)
        .await?;
      if found != Value::Bool(true) {
        return Err(format!("`{selector}` did not appear on {}", opened.url));
      }
    }
    None => {
      opened
        .value_across_reloads(&read::settle(), timeout, &mut reloaded)
        .await?;
    }
  }
  // Where it is now: a check that passed reloads into the page asked for,
  // with its own address and status; a script may move on to a file.
  let (h, id) = (opened.host.clone(), opened.id);
  if let Ok(now) = blocking(move || h.state(id)).await? {
    if now.file {
      return Ok(Loaded::File { url: now.url });
    }
    if !now.url.is_empty() {
      opened.url = now.url;
    }
    if now.status > 0 {
      opened.status = now.status;
    }
  }
  Ok(Loaded::Page(opened))
}

/// The page's title and its main content as Markdown, links absolute.
pub(super) async fn markdown(
  opened: &Opened,
  timeout: Duration,
) -> Result<(String, String), String> {
  let v = opened
    .value_across_reloads(read::READ, timeout, &mut false)
    .await?;
  let title = v["title"].as_str().unwrap_or("").trim().to_owned();
  let html = v["html"].as_str().unwrap_or("").to_owned();
  let md = blocking(move || read::markdown(&html)).await??;
  Ok((title, md))
}

/// A screenshot of the viewport, or of the element `selector` finds; PNG.
pub(super) async fn screenshot(
  opened: &Opened,
  selector: Option<&str>,
  timeout: Duration,
) -> Result<Vec<u8>, String> {
  let area = match selector {
    None => None,
    Some(sel) => {
      let b = opened
        .value(&read::element_box(sel), timeout)
        .await
        .map_err(|e| describe(&e, timeout, 0))?;
      let n = |k: &str| b[k].as_f64().unwrap_or(0.0);
      if b.is_null() || n("width") <= 0.0 || n("height") <= 0.0 {
        return Err(format!("no element `{sel}` on the page to screenshot"));
      }
      Some(Area {
        x: n("x"),
        y: n("y"),
        width: n("width"),
        height: n("height"),
      })
    }
  };
  let (h, id) = (opened.host.clone(), opened.id);
  blocking(move || h.screenshot(id, area)).await?
}

/// Runs the model's code (an async function body) in the page; its result
/// as text and what it logged.
pub(super) async fn run(
  opened: &Opened,
  code: &str,
  timeout: Duration,
) -> Result<(String, Vec<String>), String> {
  let body = read::run_body(code);
  let (h, id) = (opened.host.clone(), opened.id);
  let text = blocking(move || h.call(id, &body, timeout))
    .await?
    .map_err(|e| describe(&e, timeout, read::RUN_PREFIX_LINES))?;
  let v: Value = text
    .as_deref()
    .and_then(|t| serde_json::from_str(t).ok())
    .unwrap_or(Value::Null);
  let logged: Vec<String> = v["logs"]
    .as_array()
    .map(|a| {
      a.iter()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect()
    })
    .unwrap_or_default();
  if let Some(e) = v["error"].as_str() {
    let mut out = format!("JavaScript error: {e}");
    if !logged.is_empty() {
      out.push_str("\n\nConsole:\n");
      out.push_str(&logged.join("\n"));
    }
    return Err(out);
  }
  match v["value"].as_str() {
    Some(value) => Ok((value.to_owned(), logged)),
    None => Err("the code's result did not come back from the page".into()),
  }
}
