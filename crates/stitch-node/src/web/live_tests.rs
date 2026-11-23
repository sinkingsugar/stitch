//! The web tools against a real headless Chromium and local fixtures, no
//! outside sites. Skipped where no browser is found.

use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

/// A one-page PDF: with a line of text, or only a filled square.
fn pdf(text: bool) -> Vec<u8> {
  let stream = if text {
    "BT /F1 8 Tf 10 100 Td (PDF fixture text, long enough to count as a text page.) Tj ET"
  } else {
    "0 0 1 rg 20 20 160 160 re f"
  };
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
  ];
  let mut out = String::from("%PDF-1.4\n");
  let mut offsets = Vec::new();
  for (i, o) in objects.iter().enumerate() {
    offsets.push(out.len());
    out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
  }
  let xref = out.len();
  out.push_str("xref\n0 6\n0000000000 65535 f \n");
  for o in offsets {
    out.push_str(&format!("{o:010} 00000 n \n"));
  }
  out.push_str(&format!(
    "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
  ));
  out.into_bytes()
}

const PNG: &[u8] = &[
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
  0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
  0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xff, 0x1f, 0x00,
  0x03, 0x03, 0x02, 0x00, 0xef, 0xa3, 0x67, 0x5b, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
  0xae, 0x42, 0x60, 0x82,
];

fn page(path: &str) -> Option<(&'static str, Vec<u8>)> {
  let html = |s: &str| Some(("text/html", s.as_bytes().to_vec()));
  match path {
    "/dynamic" => html(
      r#"<title>Dynamic fixture</title><nav>Navigation noise</nav><main><p>Initial</p></main>
      <script>setTimeout(() => { document.querySelector("main").innerHTML =
        `<h1>Rendered by JavaScript</h1><p>A <a href="/next">next page</a>.</p><span style="display:none">Hidden noise</span>`; }, 700)</script>"#,
    ),
    "/late" => html(
      r#"<title>Late</title><main id="m">Waiting</main>
      <script>setTimeout(() => { document.getElementById("m").innerHTML = '<p id="ready">Arrived late</p>'; }, 1500)</script>"#,
    ),
    "/app" => html(
      r#"<title>App</title><main>App</main><script>window.APP = {items: [3, 1, 2]}; alert("blocking?");</script>"#,
    ),
    // A bot check that passes: it moves on to the page by itself, while
    // the page is being waited for.
    "/check" => html(
      r#"<title>Checking</title><main>One moment</main>
      <script>setTimeout(() => location.replace("/dynamic"), 400)</script>"#,
    ),
    // An app (Stitch's bridge first) that reloads itself once, guarded by
    // `sessionStorage`, as the web client does on a version mismatch.
    "/reload-once" => Some((
      "text/html",
      format!(
        r#"<title>Once</title><script>{}</script><main id="m">Loading</main>
        <script>const n = Number(sessionStorage.getItem("reloads") || 0);
        if (n < 1) {{ sessionStorage.setItem("reloads", String(n + 1)); location.reload(); }}
        else document.getElementById("m").innerHTML = `<p id="done">Reloaded ${{n}}</p>`;</script>"#,
        include_str!("../apps/bridge.js")
      )
      .into_bytes(),
    )),
    // A page that reloads itself for ever: every evaluation in it is cut
    // short.
    "/reloads" => html(
      r#"<title>Reloads</title><main>Again</main>
      <script>setTimeout(() => location.reload(), 150)</script>"#,
    ),
    "/redirect" => None,
    "/long" => Some(("text/plain", "hello world ".repeat(4000).into_bytes())),
    "/report.pdf" => Some(("application/pdf", pdf(true))),
    "/scan" => Some(("application/pdf", pdf(false))),
    "/dot.png" => Some(("image/png", PNG.to_vec())),
    "/data.csv" => Some(("text/csv", b"Name,Count\nApples,7\n".to_vec())),
    "/archive.zip" => Some(("application/zip", vec![1, 2, 3, 4])),
    _ => Some(("text/html", b"<main>Missing</main>".to_vec())),
  }
}

pub(super) async fn serve() -> String {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  tokio::spawn(async move {
    loop {
      let Ok((mut sock, _)) = listener.accept().await else {
        return;
      };
      tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
        // The request as it came, for what the browser tells sites.
        let echoed = format!("<title>Headers</title><main><pre>{head}</pre></main>");
        let found = match path.as_str() {
          "/headers" => Some(("text/html", echoed.into_bytes())),
          "/blocked" => Some((
            "text/html",
            b"<title>Just a moment...</title><main>Checking your browser</main>".to_vec(),
          )),
          _ => page(&path),
        };
        let response = match found {
          None => b"HTTP/1.1 302 Found\r\nlocation: /dynamic\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
          Some((kind, body)) => {
            let status = match path.as_str() {
              "/missing" => "404 Not Found",
              "/blocked" => "403 Forbidden",
              _ => "200 OK",
            };
            let mut r = format!(
              "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
              body.len()
            )
            .into_bytes();
            r.extend(body);
            r
          }
        };
        let _ = sock.write_all(&response).await;
        let _ = sock.shutdown().await;
      });
    }
  });
  base
}

#[tokio::test(flavor = "multi_thread")]
async fn the_web_tools_read_pages_files_and_run_code() {
  let dir = tempfile::tempdir().unwrap();
  let web = Web::new(
    dir.path(),
    BrowserSettings::default(),
    SearchSettings::default(),
  );
  let Ok(browser) = web.chrome.browser().await else {
    eprintln!("no Chrome or Chromium here: skipped");
    return;
  };
  // A browser that starts but never opens a page: one run where it may
  // not (an agent's sandbox, as macOS's seatbelt confines it). Every call
  // would time out.
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
  drop(browser);
  let base = serve().await;
  let call = |name: &'static str, input: serde_json::Value| {
    let web = &web;
    async move {
      let out = web.call(name, &input, None).await;
      assert!(out.ok, "{name} {input}: {}", out.output);
      out
    }
  };
  // The browser passes for one with a window: the agent it sends and
  // shows, no automation flag, a screen around its window.
  let out = call("browse_web", json!({"url": format!("{base}/headers")})).await;
  assert!(
    out.output.contains("Chrome/") && !out.output.contains("Headless"),
    "{}",
    out.output
  );
  let out = call(
    "web_eval",
    json!({"url": format!("{base}/app"), "code": "return [navigator.userAgent.includes('Headless'), navigator.webdriver, screen.width >= outerWidth, innerWidth, innerHeight];"}),
  )
  .await;
  assert!(
    out
      .output
      .contains("Result:\n[\n  false,\n  false,\n  true,\n  1280,\n  900\n]"),
    "{}",
    out.output
  );

  reads_like_a_browser(&web, &base).await;

  // An app's `sessionStorage` lasts across its reloads, as a tab's does:
  // the web client's "reload once" guard held in it (a page kept in memory
  // only had every tab of the fleet reloading for ever on a mismatch, and
  // the node serving each reload, a core of it).
  let out = call(
    "browse_web",
    json!({"url": format!("{base}/reload-once"), "wait_for": "#done", "timeout_secs": 10}),
  )
  .await;
  assert!(out.output.contains("Reloaded 1"), "{}", out.output);

  // The browser is shared, and stops with the node.
  let first = web.chrome.browser().await.unwrap();
  let again = web.chrome.browser().await.unwrap();
  assert!(Arc::ptr_eq(&first, &again));
  drop((first, again));
  tokio::time::timeout(Duration::from_secs(10), web.shutdown())
    .await
    .unwrap();
}

/// What a page reads as, whichever browser renders it: Chrome here, the
/// browser an app lends (`hosted_tests`).
pub(super) async fn reads_like_a_browser(web: &Web, base: &str) {
  let call = |name: &'static str, input: serde_json::Value| async move {
    let out = web.call(name, &input, None).await;
    assert!(out.ok, "{name} {input}: {}", out.output);
    out
  };

  // A page rendered by its script, read after it settles, through a
  // redirect; noise and hidden text left out, links absolute.
  let out = call("browse_web", json!({"url": format!("{base}/redirect")})).await;
  assert!(
    out
      .output
      .starts_with(&format!("{base}/dynamic\nTitle: Dynamic fixture\nHTTP 200")),
    "{}",
    out.output
  );
  assert!(
    out.output.contains("# Rendered by JavaScript"),
    "{}",
    out.output
  );
  assert!(
    out.output.contains(&format!("[next page]({base}/next)")),
    "{}",
    out.output
  );
  assert!(!out.output.contains("Navigation noise") && !out.output.contains("Hidden noise"));

  // Screenshots: the first screen, or one element.
  let out = call(
    "browse_web",
    json!({"url": format!("{base}/dynamic"), "screenshot": true}),
  )
  .await;
  let shot = std::fs::read(&out.images[0].path).unwrap();
  assert_eq!(crate::images::dimensions(&shot), Some((1280, 900)));
  let out = call(
    "browse_web",
    json!({"url": format!("{base}/dynamic"), "screenshot_selector": "h1"}),
  )
  .await;
  let (w, h) = crate::images::dimensions(&std::fs::read(&out.images[0].path).unwrap()).unwrap();
  assert!(w < 1280 && h < 200, "{w}×{h}");

  // A page that moves on while it settles is read where it ends.
  let out = call("browse_web", json!({"url": format!("{base}/check")})).await;
  assert!(
    out
      .output
      .starts_with(&format!("{base}/dynamic\nTitle: Dynamic fixture\nHTTP 200")),
    "{}",
    out.output
  );
  assert!(
    out.output.contains("# Rendered by JavaScript"),
    "{}",
    out.output
  );

  // A bot wall is said to be one.
  let out = call("browse_web", json!({"url": format!("{base}/blocked")})).await;
  assert!(
    out
      .output
      .contains("[Blocked: this is Cloudflare's challenge, not the page."),
    "{}",
    out.output
  );

  // Content that comes late is waited for.
  let out = call(
    "browse_web",
    json!({"url": format!("{base}/late"), "wait_for": "#ready"}),
  )
  .await;
  assert!(out.output.contains("Arrived late"), "{}", out.output);

  // The model's code runs in the page; alerts do not block it.
  let out = call(
    "web_eval",
    json!({"url": format!("{base}/app"), "code": "console.log('items', window.APP.items.length); return window.APP.items.sort();"}),
  )
  .await;
  assert!(
    out.output.contains("Result:\n[\n  1,\n  2,\n  3\n]"),
    "{}",
    out.output
  );
  assert!(
    out.output.contains("Console:\n[log] items 3"),
    "{}",
    out.output
  );
  let failed = web
    .call(
      "web_eval",
      &json!({"url": format!("{base}/app"), "code": "throw new Error('nope')"}),
      None,
    )
    .await;
  assert!(
    !failed.ok && failed.output.contains("nope"),
    "{}",
    failed.output
  );

  // Files: a PDF by its name, a scanned one without (the browser finds it
  // is not a page) rendered as an image, an image, a table, a download.
  let out = call("browse_web", json!({"url": format!("{base}/report.pdf")})).await;
  assert!(
    out
      .output
      .contains("## Page 1\n\nPDF fixture text, long enough"),
    "{}",
    out.output
  );
  assert!(out.images.is_empty());
  let out = call("browse_web", json!({"url": format!("{base}/scan")})).await;
  assert!(out.output.contains("(no text layer)"), "{}", out.output);
  assert_eq!(out.images.len(), 1, "{}", out.output);
  assert!(
    std::fs::read(&out.images[0].path)
      .unwrap()
      .starts_with(b"\x89PNG")
  );
  let out = call("browse_web", json!({"url": format!("{base}/dot.png")})).await;
  assert_eq!(out.images[0].media_type, "image/png");
  let out = call("browse_web", json!({"url": format!("{base}/data.csv")})).await;
  assert!(
    out
      .output
      .contains("| Name | Count |\n| --- | --- |\n| Apples | 7 |"),
    "{}",
    out.output
  );
  let out = call("browse_web", json!({"url": format!("{base}/archive.zip")})).await;
  assert!(
    out.output.contains("saved to ") && out.output.contains("use the shell"),
    "{}",
    out.output
  );

  // Long text is cut, and kept whole.
  let out = call("browse_web", json!({"url": format!("{base}/long")})).await;
  assert!(
    out
      .output
      .contains("[… cut at 32000 characters; all of it is in "),
    "{}",
    &out.output[out.output.len() - 200..]
  );
}

/// The pages of the browser at `path`, as Chrome has them (not as the
/// driver remembers them).
async fn pages_at(browser: &chromiumoxide::Browser, path: &str) -> Vec<String> {
  use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
  let targets = browser.execute(GetTargetsParams::default()).await.unwrap();
  targets
    .result
    .target_infos
    .iter()
    .filter(|t| t.r#type == "page" && t.url.contains(path))
    .map(|t| t.url.clone())
    .collect()
}

/// A call that fails in a page that never stops reloading leaves no page
/// behind: one left reloads on, every second, until the browser idles out
/// (the node's web client looping on a version mismatch kept a core busy,
/// serving it again and again).
#[tokio::test(flavor = "multi_thread")]
async fn a_page_that_reloads_for_ever_is_closed_after_the_call() {
  let dir = tempfile::tempdir().unwrap();
  let web = Web::new(
    dir.path(),
    BrowserSettings::default(),
    SearchSettings::default(),
  );
  let Ok(browser) = web.chrome.browser().await else {
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
  let base = serve().await;
  let url = format!("{base}/reloads");
  let read = web
    .call("browse_web", &json!({"url": url, "timeout_secs": 10}), None)
    .await;
  let eval = web
    .call(
      "web_eval",
      &json!({"url": url, "code": "await new Promise(r => setTimeout(r, 1000)); return 1;", "timeout_secs": 10}),
      None,
    )
    .await;
  let mut left = pages_at(&browser, "/reloads").await;
  for _ in 0..50 {
    if left.is_empty() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    left = pages_at(&browser, "/reloads").await;
  }
  assert!(
    left.is_empty(),
    "pages left reloading: {left:?} (browse_web: {}; web_eval: {})",
    read.output,
    eval.output
  );
  drop(browser);
  tokio::time::timeout(Duration::from_secs(10), web.shutdown())
    .await
    .unwrap();
}
