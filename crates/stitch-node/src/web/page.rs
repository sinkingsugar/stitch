//! A page in the node's browser: loaded in a fresh browser context, read
//! as Markdown (`browse_web`) or given the model's JavaScript (`web_eval`).

use std::sync::Arc;
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::browser::{
  BrowserContextId, SetDownloadBehaviorBehavior, SetDownloadBehaviorParams,
};
use chromiumoxide::cdp::browser_protocol::network::{EventResponseReceived, ResourceType};
use chromiumoxide::cdp::browser_protocol::page::{
  EventJavascriptDialogOpening, HandleJavaScriptDialogParams,
};
use chromiumoxide::cdp::browser_protocol::target::{
  CreateBrowserContextParams, CreateTargetParams,
};
use chromiumoxide::cdp::js_protocol::runtime::{
  EvaluateParams, EventConsoleApiCalled, RemoteObject,
};
use chromiumoxide::{Browser, Page};
use futures_util::{FutureExt, StreamExt};
use serde_json::Value;

use super::read;

/// A page loaded in its own browser context, disposed with it.
pub(super) struct Opened {
  browser: Arc<Browser>,
  context: Option<BrowserContextId>,
  pub page: Page,
  pub url: String,
  pub status: i64,
}

impl Drop for Opened {
  fn drop(&mut self) {
    if let Some(ctx) = self.context.take() {
      let browser = self.browser.clone();
      tokio::spawn(async move {
        let _ = browser.dispose_browser_context(ctx).await;
      });
    }
  }
}

/// What a URL turned out to be.
pub(super) enum Loaded {
  Page(Opened),
  /// Not a web page (a PDF, an image, a download): fetched as a file.
  File {
    url: String,
  },
}

fn cdp(e: impl std::fmt::Display) -> String {
  e.to_string()
}

/// Opens `url` in a fresh browser context, as far as its load event (a
/// file's response ends it at once): what it is, not yet settled.
pub(super) async fn navigate(
  browser: Arc<Browser>,
  url: &str,
  timeout: Duration,
) -> Result<Loaded, String> {
  let context = browser
    .create_browser_context(CreateBrowserContextParams::default())
    .await
    .map_err(cdp)?;
  browser
    .execute(SetDownloadBehaviorParams {
      behavior: SetDownloadBehaviorBehavior::Deny,
      browser_context_id: Some(context.clone()),
      download_path: None,
      events_enabled: None,
    })
    .await
    .map_err(cdp)?;
  let target = CreateTargetParams::builder()
    .url("about:blank")
    .browser_context_id(context.clone())
    .build()
    .map_err(cdp)?;
  let page = browser.new_page(target).await;
  let mut opened = Opened {
    browser: browser.clone(),
    context: Some(context),
    page: match page {
      Ok(p) => p,
      Err(e) => return Err(cdp(e)),
    },
    url: url.to_owned(),
    status: 0,
  };
  let page = opened.page.clone();
  // Alerts and prompts would block the page: they are dismissed.
  let mut dialogs = page
    .event_listener::<EventJavascriptDialogOpening>()
    .await
    .map_err(cdp)?;
  let dismisser = page.clone();
  tokio::spawn(async move {
    while dialogs.next().await.is_some() {
      let _ = dismisser
        .execute(HandleJavaScriptDialogParams::new(false))
        .await;
    }
  });
  let mut responses = page
    .event_listener::<EventResponseReceived>()
    .await
    .map_err(cdp)?;
  let frame = page.mainframe().await.map_err(cdp)?;
  let page_like = |mime: &str| {
    mime.is_empty()
      || mime == "text/html"
      || mime == "application/xhtml+xml"
      || mime == "text/plain"
  };
  // The main document's response says what the URL is. A file's ends the
  // wait at once: some builds would otherwise sit on it until the timeout.
  let mut main: Option<(String, i64, String)> = None;
  let goto = tokio::time::timeout(timeout, page.goto(url));
  tokio::pin!(goto);
  let navigated = loop {
    tokio::select! {
      done = &mut goto => break done,
      Some(ev) = responses.next() => {
        if ev.r#type == ResourceType::Document && (frame.is_none() || ev.frame_id == frame) {
          let mime = ev.response.mime_type.to_ascii_lowercase();
          let file = !page_like(&mime);
          main = Some((ev.response.url.clone(), ev.response.status, mime));
          if file {
            break Ok(Ok(&page));
          }
        }
      }
    }
  };
  while let Some(Some(ev)) = responses.next().now_or_never() {
    if ev.r#type == ResourceType::Document && (frame.is_none() || ev.frame_id == frame) {
      main = Some((
        ev.response.url.clone(),
        ev.response.status,
        ev.response.mime_type.to_ascii_lowercase(),
      ));
    }
  }
  match (navigated, &main) {
    (_, Some((final_url, _, mime))) if !page_like(mime) => {
      return Ok(Loaded::File {
        url: final_url.clone(),
      });
    }
    (Err(_), _) => {
      return Err(format!(
        "the page did not load within {}s",
        timeout.as_secs()
      ));
    }
    (Ok(Err(e)), _) => {
      // A download aborts navigation before any response is seen.
      let text = e.to_string();
      if text.contains("ERR_ABORTED") {
        return Ok(Loaded::File {
          url: url.to_owned(),
        });
      }
      return Err(format!("could not load {url}: {text}"));
    }
    (Ok(Ok(_)), _) => {}
  }
  if let Some((final_url, status, _)) = main {
    opened.url = final_url;
    opened.status = status;
  }
  if let Ok(Some(u)) = page.url().await {
    opened.url = u;
  }
  Ok(Loaded::Page(opened))
}

/// Loads `url`; for a web page, waits until `wait_for` shows (a CSS
/// selector) or the page stops changing.
pub(super) async fn load(
  browser: Arc<Browser>,
  url: &str,
  wait_for: Option<&str>,
  timeout: Duration,
) -> Result<Loaded, String> {
  let mut opened = match navigate(browser, url, timeout).await? {
    Loaded::Page(o) => o,
    file => return Ok(file),
  };
  let page = opened.page.clone();
  let mut reloaded = false;
  match wait_for {
    Some(selector) => {
      let found =
        eval_across_reloads(&page, &mut reloaded, &read::wait_for(selector, timeout)).await?;
      if found != Value::Bool(true) {
        // Read what is there; the caller says the selector never showed.
        return Err(format!("`{selector}` did not appear on {}", opened.url));
      }
    }
    None => {
      eval_across_reloads(&page, &mut reloaded, &read::settle()).await?;
    }
  }
  if reloaded {
    // What the page is now: a bot check that passed reloads into the
    // page asked for, with its own address and status.
    if let Ok(Some(u)) = page.url().await {
      opened.url = u;
    }
    let status = eval_value(
      &page,
      "performance.getEntriesByType('navigation')[0]?.responseStatus ?? 0",
    )
    .await;
    if let Some(s) = status.ok().and_then(|v| v.as_i64()).filter(|s| *s > 0) {
      opened.status = s;
    }
  }
  Ok(Loaded::Page(opened))
}

/// Whether an evaluation failed because the page went away under it: it
/// navigated (a bot check passed and reloaded, a script redirected).
pub(super) fn navigated_away(e: &str) -> bool {
  e.contains("Inspected target navigated or closed")
    || e.contains("Execution context was destroyed")
    || e.contains("Cannot find context with specified id")
}

/// `eval_value`, run again in the next document when the page navigates
/// while it runs (a few times at most); `reloaded` says one did.
async fn eval_across_reloads(
  page: &Page,
  reloaded: &mut bool,
  expression: &str,
) -> Result<Value, String> {
  for _ in 0..3 {
    match eval_value(page, expression).await {
      Err(e) if navigated_away(&e) => {
        *reloaded = true;
        let _ = tokio::time::timeout(Duration::from_secs(15), page.wait_for_navigation()).await;
      }
      done => return done,
    }
  }
  eval_value(page, expression).await
}

/// Runs an expression that may return a promise; its value.
pub(super) async fn eval_value(page: &Page, expression: &str) -> Result<Value, String> {
  let params = EvaluateParams::builder()
    .expression(expression)
    .await_promise(true)
    .return_by_value(true)
    .build()
    .map_err(cdp)?;
  match page.evaluate_expression(params).await {
    Ok(r) => Ok(r.value().cloned().unwrap_or(Value::Null)),
    Err(chromiumoxide::error::CdpError::JavascriptException(e)) => Err(format!(
      "JavaScript error: {}",
      e.exception
        .as_ref()
        .and_then(|x| x.description.clone())
        .unwrap_or(e.text.clone())
    )),
    Err(e) => Err(cdp(e)),
  }
}

/// The page's title and its main content as Markdown, links absolute.
pub(super) async fn markdown(opened: &Opened) -> Result<(String, String), String> {
  let v = eval_across_reloads(&opened.page, &mut false, read::READ).await?;
  let title = v["title"].as_str().unwrap_or("").trim().to_owned();
  let html = v["html"].as_str().unwrap_or("").to_owned();
  let md = tokio::task::spawn_blocking(move || read::markdown(&html))
    .await
    .map_err(cdp)??;
  Ok((title, md))
}

/// Runs the model's code (an async function body) in the page; its result
/// as text and what it logged.
pub(super) async fn run(
  opened: &Opened,
  code: &str,
  timeout: Duration,
) -> Result<(String, Vec<String>), String> {
  let mut console = opened
    .page
    .event_listener::<EventConsoleApiCalled>()
    .await
    .map_err(cdp)?;
  // The console's own `debug` kept first: the code may replace it, and the
  // end marker below is logged with it.
  let expression = format!(
    "(async () => {{ if (!globalThis.__stitchDebug) Object.defineProperty(globalThis, \
     '__stitchDebug', {{ value: console.debug.bind(console) }}); \
     const value = await (async () => {{\n{code}\n}})(); \
     if (value === undefined) return 'undefined'; if (typeof value === 'string') return value; \
     try {{ return JSON.stringify(value, null, 2); }} catch (e) {{ return String(value); }} }})()"
  );
  let result = tokio::time::timeout(timeout, eval_value(&opened.page, &expression))
    .await
    .map_err(|_| format!("the code did not finish within {}s", timeout.as_secs()))?;
  // What the code logged may still be on its way when its result is in
  // (the console's events come apart from the reply; under load the last
  // ones came after it and were missed): a marker logged after it comes
  // after them, so they are read up to it, for at most two seconds.
  let marker = format!("__stitch_console_end_{}", std::process::id());
  let marked = tokio::time::timeout(
    Duration::from_secs(2),
    eval_value(
      &opened.page,
      &format!("(globalThis.__stitchDebug || console.debug)({marker:?}); 0"),
    ),
  )
  .await
  .is_ok_and(|r| r.is_ok());
  let until = tokio::time::Instant::now() + Duration::from_secs(2);
  let mut logged = Vec::new();
  loop {
    let ev = if marked {
      match tokio::time::timeout_at(until, console.next()).await {
        Ok(Some(ev)) => ev,
        _ => break,
      }
    } else {
      match console.next().now_or_never() {
        Some(Some(ev)) => ev,
        _ => break,
      }
    };
    let args: Vec<String> = ev.args.iter().map(describe).collect();
    if args.len() == 1 && args[0] == marker {
      break;
    }
    if logged.len() < 200 {
      let kind = format!("{:?}", ev.r#type).to_lowercase();
      logged.push(format!("[{kind}] {}", args.join(" ")));
    }
  }
  let value = result?;
  let text = match value {
    Value::String(s) => s,
    other => other.to_string(),
  };
  Ok((text, logged))
}

fn describe(o: &RemoteObject) -> String {
  match (&o.value, &o.description) {
    (Some(Value::String(s)), _) => s.clone(),
    (Some(v), _) => v.to_string(),
    (None, Some(d)) => d.clone(),
    _ => format!("{:?}", o.r#type),
  }
}

/// A screenshot of the page's first screen, or of the element `selector`
/// finds; PNG.
pub(super) async fn screenshot(opened: &Opened, selector: Option<&str>) -> Result<Vec<u8>, String> {
  use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
  match selector {
    Some(sel) => {
      let element = opened
        .page
        .find_element(sel)
        .await
        .map_err(|_| format!("no element `{sel}` on the page to screenshot"))?;
      element
        .screenshot(CaptureScreenshotFormat::Png)
        .await
        .map_err(cdp)
    }
    None => opened
      .page
      .screenshot(
        chromiumoxide::page::ScreenshotParams::builder()
          .format(CaptureScreenshotFormat::Png)
          .build(),
      )
      .await
      .map_err(cdp),
  }
}
