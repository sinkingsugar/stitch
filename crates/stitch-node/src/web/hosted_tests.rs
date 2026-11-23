//! The hosted path (`hosted`: a browser the app lends, the phone's WebKit)
//! checked here, where no WebKit runs: against a scripted host, for what
//! the node makes of each answer, and against a host made of Chrome's
//! pages, for the scripts themselves, on the same fixtures and assertions
//! as Chrome's own path (`live_tests::reads_like_a_browser`). The app's
//! WebKit host runs them in the iOS UI tests (`test9lBrowsesTheWebHere`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chromiumoxide::Browser;
use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, Viewport};
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use serde_json::json;

use super::*;

/// A host made of Chrome's pages: what `PageHost` asks, done over CDP.
struct ChromeHost {
  browser: Arc<Browser>,
  rt: tokio::runtime::Handle,
  pages: Mutex<HashMap<u64, page::Opened>>,
  next: AtomicU64,
}

impl ChromeHost {
  fn page(&self, id: u64) -> Result<chromiumoxide::Page, String> {
    self
      .pages
      .lock()
      .unwrap()
      .get(&id)
      .map(|o| o.page.clone())
      .ok_or_else(|| format!("no page {id}"))
  }
}

impl PageHost for ChromeHost {
  fn open(&self, url: &str, timeout: Duration) -> Result<HostPage, String> {
    match self
      .rt
      .block_on(page::navigate(self.browser.clone(), url, timeout))?
    {
      page::Loaded::File { url } => Ok(HostPage {
        url,
        file: true,
        ..HostPage::default()
      }),
      page::Loaded::Page(opened) => {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let got = HostPage {
          page: id,
          url: opened.url.clone(),
          status: opened.status,
          mime: "text/html".into(),
          file: false,
        };
        self.pages.lock().unwrap().insert(id, opened);
        Ok(got)
      }
    }
  }

  fn state(&self, id: u64) -> Result<HostPage, String> {
    let page = self.page(id)?;
    let (url, status) = self.rt.block_on(async {
      let url = page.url().await.ok().flatten().unwrap_or_default();
      let status = page::eval_value(
        &page,
        "performance.getEntriesByType('navigation')[0]?.responseStatus ?? 0",
      )
      .await
      .ok()
      .and_then(|v| v.as_i64())
      .unwrap_or(0);
      (url, status)
    });
    Ok(HostPage {
      page: id,
      url,
      status,
      mime: "text/html".into(),
      file: false,
    })
  }

  fn call(&self, id: u64, body: &str, timeout: Duration) -> Result<Option<String>, CallError> {
    let page = self.page(id).map_err(CallError::Failed)?;
    let expression = format!("(async () => {{\n{body}\n}})()");
    self.rt.block_on(async {
      let params = EvaluateParams::builder()
        .expression(expression)
        .await_promise(true)
        .return_by_value(true)
        .build()
        .map_err(CallError::Failed)?;
      match tokio::time::timeout(timeout, page.evaluate_expression(params)).await {
        Err(_) => Err(CallError::TimedOut),
        Ok(Ok(r)) => Ok(r.value().and_then(|v| v.as_str()).map(str::to_owned)),
        Ok(Err(chromiumoxide::error::CdpError::JavascriptException(e))) => {
          Err(CallError::Script {
            message: e
              .exception
              .as_ref()
              .and_then(|x| x.description.clone())
              .unwrap_or(e.text.clone()),
            // CDP's lines count from 0, the wrapper's first line its own.
            line: e.line_number,
          })
        }
        Ok(Err(e)) if page::navigated_away(&e.to_string()) => {
          let _ = tokio::time::timeout(Duration::from_secs(15), page.wait_for_navigation()).await;
          Err(CallError::Navigated)
        }
        Ok(Err(e)) => Err(CallError::Failed(e.to_string())),
      }
    })
  }

  fn screenshot(&self, id: u64, area: Option<Area>) -> Result<Vec<u8>, String> {
    let page = self.page(id)?;
    let mut shot =
      chromiumoxide::page::ScreenshotParams::builder().format(CaptureScreenshotFormat::Png);
    if let Some(a) = area {
      shot = shot.clip(Viewport {
        x: a.x,
        y: a.y,
        width: a.width,
        height: a.height,
        scale: 1.0,
      });
    }
    self
      .rt
      .block_on(page.screenshot(shot.build()))
      .map_err(|e| e.to_string())
  }

  fn close(&self, id: u64) {
    let opened = self.pages.lock().unwrap().remove(&id);
    // Its context is disposed of on the runtime.
    let _guard = self.rt.enter();
    drop(opened);
  }
}

fn web_with(dir: &Path, host: Arc<dyn PageHost>) -> Web {
  let mut web = Web::new(dir, BrowserSettings::default(), SearchSettings::default());
  web.host = Some(host);
  web
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lent_browser_reads_as_chrome_does() {
  let dir = tempfile::tempdir().unwrap();
  let chrome_web = Web::new(
    dir.path(),
    BrowserSettings::default(),
    SearchSettings::default(),
  );
  let Ok(browser) = chrome_web.chrome.browser().await else {
    eprintln!("no Chrome or Chromium here: skipped");
    return;
  };
  let opens = tokio::time::timeout(Duration::from_secs(10), browser.new_page("about:blank")).await;
  match opens {
    Ok(Ok(p)) => {
      let _ = p.close().await;
    }
    _ => {
      eprintln!("the browser opens no page here (a sandbox?): skipped");
      return;
    }
  }
  let host = Arc::new(ChromeHost {
    browser,
    rt: tokio::runtime::Handle::current(),
    pages: Mutex::new(HashMap::new()),
    next: AtomicU64::new(0),
  });
  let web = web_with(dir.path(), host.clone());
  let base = live_tests::serve().await;
  live_tests::reads_like_a_browser(&web, &base).await;

  // A syntax error says where in the model's code it is.
  let out = web
    .call(
      "web_eval",
      &json!({"url": format!("{base}/app"), "code": "const a = 1;\nreturn a +;"}),
      None,
    )
    .await;
  assert!(
    !out.ok && out.output.starts_with("JavaScript error: SyntaxError"),
    "{}",
    out.output
  );
  // Console output survives a throw, and the page's console is its own
  // again after the code.
  let out = web
    .call(
      "web_eval",
      &json!({"url": format!("{base}/app"), "code": "console.warn('careful'); throw new TypeError('bad')"}),
      None,
    )
    .await;
  assert!(
    !out.ok
      && out.output.contains("TypeError: bad")
      && out.output.contains("Console:\n[warning] careful"),
    "{}",
    out.output
  );
  // Every page was closed.
  assert!(host.pages.lock().unwrap().is_empty());
  tokio::time::timeout(Duration::from_secs(10), chrome_web.shutdown())
    .await
    .unwrap();
}

/// A host that answers from a script: what the node does with each answer.
#[derive(Default)]
struct Scripted {
  /// What `open` gives.
  opened: Mutex<Option<HostPage>>,
  /// Answers to calls, in turn; the last one repeats.
  answers: Mutex<Vec<Result<Option<String>, CallError>>>,
  bodies: Mutex<Vec<String>>,
  closed: Mutex<Vec<u64>>,
}

impl PageHost for Scripted {
  fn open(&self, url: &str, _: Duration) -> Result<HostPage, String> {
    Ok(self.opened.lock().unwrap().clone().unwrap_or(HostPage {
      page: 7,
      url: url.into(),
      status: 200,
      mime: "text/html".into(),
      file: false,
    }))
  }
  fn state(&self, page: u64) -> Result<HostPage, String> {
    Ok(HostPage {
      page,
      url: "https://moved.example/here".into(),
      status: 201,
      mime: "text/html".into(),
      file: false,
    })
  }
  fn call(&self, _: u64, body: &str, _: Duration) -> Result<Option<String>, CallError> {
    self.bodies.lock().unwrap().push(body.to_owned());
    let mut a = self.answers.lock().unwrap();
    if a.len() > 1 {
      a.remove(0)
    } else {
      a[0].clone()
    }
  }
  fn screenshot(&self, _: u64, _: Option<Area>) -> Result<Vec<u8>, String> {
    Err("no screen".into())
  }
  fn close(&self, page: u64) {
    self.closed.lock().unwrap().push(page);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lent_browser_s_answers_become_the_tools() {
  let dir = tempfile::tempdir().unwrap();
  let host = Arc::new(Scripted::default());
  let web = web_with(dir.path(), host.clone());
  assert!(
    web
      .definitions(false)
      .iter()
      .any(|d| d.name == "browse_web")
  );

  // A page that navigates while it settles is waited for again, then read
  // where it ended.
  *host.answers.lock().unwrap() = vec![
    Err(CallError::Navigated),
    Ok(Some("true".into())),
    Ok(Some(
      json!({"title": "Moved", "html": "<main><h1>Here</h1></main>"}).to_string(),
    )),
  ];
  let out = web
    .call(
      "browse_web",
      &json!({"url": "https://start.example/"}),
      None,
    )
    .await;
  assert!(out.ok, "{}", out.output);
  assert!(
    out
      .output
      .starts_with("https://moved.example/here\nTitle: Moved\nHTTP 201\n\n# Here"),
    "{}",
    out.output
  );
  assert_eq!(host.bodies.lock().unwrap().len(), 3);
  assert_eq!(*host.closed.lock().unwrap(), vec![7]);

  // A selector that never shows.
  *host.answers.lock().unwrap() = vec![Ok(Some("false".into()))];
  let out = web
    .call(
      "browse_web",
      &json!({"url": "https://start.example/", "wait_for": "#never"}),
      None,
    )
    .await;
  assert!(
    !out.ok
      && out
        .output
        .contains("`#never` did not appear on https://start.example/"),
    "{}",
    out.output
  );

  // The model's code: its value and console; an error in it by its own
  // lines; a timeout.
  *host.answers.lock().unwrap() = vec![
    Ok(Some("true".into())),
    Ok(Some(
      json!({"value": "42", "logs": ["[log] hi"]}).to_string(),
    )),
  ];
  let out = web
    .call(
      "web_eval",
      &json!({"url": "https://start.example/", "code": "return 42"}),
      None,
    )
    .await;
  assert_eq!(
    out.output,
    "https://moved.example/here (HTTP 201)\n\nResult:\n42\n\nConsole:\n[log] hi"
  );
  assert!(
    host
      .bodies
      .lock()
      .unwrap()
      .last()
      .unwrap()
      .contains("\nreturn 42\n")
  );
  *host.answers.lock().unwrap() = vec![
    Ok(Some("true".into())),
    Err(CallError::Script {
      message: "SyntaxError: Unexpected token ';'".into(),
      line: read::RUN_PREFIX_LINES + 2,
    }),
  ];
  let out = web
    .call(
      "web_eval",
      &json!({"url": "https://start.example/", "code": "let a = 1;\nreturn a +;"}),
      None,
    )
    .await;
  assert_eq!(
    out.output,
    "JavaScript error: SyntaxError: Unexpected token ';' (line 2 of the code)"
  );
  *host.answers.lock().unwrap() = vec![Ok(Some("true".into())), Err(CallError::TimedOut)];
  let out = web
    .call(
      "web_eval",
      &json!({"url": "https://start.example/", "code": "await new Promise(() => {})", "timeout_secs": 3}),
      None,
    )
    .await;
  assert_eq!(out.output, "the code did not finish within 3s");

  // A screenshot the host cannot take fails the call, said.
  *host.answers.lock().unwrap() = vec![
    Ok(Some("true".into())),
    Ok(Some(json!({"title": "T", "html": "<p>x</p>"}).to_string())),
  ];
  let out = web
    .call(
      "browse_web",
      &json!({"url": "https://start.example/", "screenshot": true}),
      None,
    )
    .await;
  assert!(!out.ok && out.output == "no screen", "{}", out.output);

  // A file is the node's to fetch: web_eval says to read it.
  *host.opened.lock().unwrap() = Some(HostPage {
    url: "https://start.example/report".into(),
    mime: "application/pdf".into(),
    file: true,
    ..HostPage::default()
  });
  let out = web
    .call(
      "web_eval",
      &json!({"url": "https://start.example/x", "code": "return 1"}),
      None,
    )
    .await;
  assert_eq!(
    out.output,
    "https://start.example/report is a file, not a web page: read it with browse_web"
  );
}
