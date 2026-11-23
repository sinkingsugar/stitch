//! What reading a page takes whatever browser loaded it (design §2.3): the
//! scripts that wait for a page to settle or for a selector, read its main
//! content, find an element to screenshot and run the model's code, and
//! what is made of their results. The desktop's Chromium (`page`) and a
//! browser the host app supplies (`hosted`, the phone's WebKit) run the
//! same scripts, so a page reads the same on both.

use std::time::Duration;

/// How long a page may keep changing after load before it is read.
pub(super) const SETTLE_CAP_MS: u64 = 5000;

/// An expression: a promise of `true` once `selector` shows (an element
/// with a box), `false` once `timeout` (at most 20 s) passes first.
pub(super) fn wait_for(selector: &str, timeout: Duration) -> String {
  format!(
    "new Promise(r => {{ const t0 = Date.now(); (function poll() {{ const el = document.querySelector({}); \
     if (el && el.getClientRects().length) return r(true); if (Date.now() - t0 > {}) return r(false); \
     setTimeout(poll, 100); }})(); }})",
    serde_json::to_string(selector).expect("a string serializes"),
    timeout.as_millis().min(20_000)
  )
}

/// An expression: a promise that resolves once the page settled, a second
/// after load and half a second without changes (scripts that render late
/// usually do it within that), at most `SETTLE_CAP_MS`.
pub(super) fn settle() -> String {
  format!(
    "new Promise(r => {{ const t0 = Date.now(); let last = t0; \
     const o = new MutationObserver(() => {{ last = Date.now(); }}); \
     o.observe(document, {{subtree: true, childList: true, characterData: true, attributes: true}}); \
     (function check() {{ const now = Date.now(); \
       if ((now - t0 >= 1000 && now - last >= 500) || now - t0 >= {SETTLE_CAP_MS}) {{ o.disconnect(); return r(true); }} \
       setTimeout(check, 100); }})(); }})"
  )
}

/// An expression: the page's title and the HTML of its main content
/// (`main`, `article`, else the body), hidden elements and noise left out,
/// links and images absolute.
pub(super) const READ: &str = r#"(() => {
    const visible = el => { const s = getComputedStyle(el); return s.display !== 'none' && s.visibility !== 'hidden' && el.getClientRects().length > 0; };
    const all = document.body ? document.body.querySelectorAll('*') : [];
    if (all.length <= 30000) for (const el of all) if (!['BR', 'WBR', 'OPTION'].includes(el.tagName) && !visible(el)) el.setAttribute('data-stitch-hidden', '');
    const root = [...document.querySelectorAll('main, article, [role="main"]')].find(visible) || document.body;
    if (!root) return { title: document.title, html: '' };
    const copy = root.cloneNode(true);
    const noise = 'script, style, noscript, template, iframe, svg, canvas, nav, footer, aside, button, [aria-hidden="true"], [data-stitch-hidden]' + (root === document.body ? ', header' : '');
    copy.querySelectorAll(noise).forEach(e => e.remove());
    copy.querySelectorAll('a[href]').forEach(a => a.setAttribute('href', a.href));
    copy.querySelectorAll('img[src]').forEach(i => i.setAttribute('src', i.src));
    return { title: document.title, html: copy.outerHTML.slice(0, 4000000) };
  })()"#;

/// An expression: the box of the element `selector` finds, scrolled into
/// view (CSS pixels, from the viewport's corner); `null` when none.
pub(super) fn element_box(selector: &str) -> String {
  format!(
    "(() => {{ const el = document.querySelector({}); if (!el) return null; \
     el.scrollIntoView({{block: 'center', inline: 'center'}}); const r = el.getBoundingClientRect(); \
     return {{ x: r.left, y: r.top, width: r.width, height: r.height }}; }})()",
    serde_json::to_string(selector).expect("a string serializes")
  )
}

/// Lines of `run_body` before the model's code, to say where in it an
/// error is.
pub(super) const RUN_PREFIX_LINES: i64 = 8;

/// The model's code (an async function body) as a whole async function
/// body of its own: run in the page, it returns, as JSON, `{value}` (the
/// result as text: a string as it is, anything else as indented JSON) or
/// `{error}`, with `logs`, what the code logged (`[log] …`, at most 200
/// lines). The console is the page's again when it is done. The code is
/// inlined, not given to `eval` or `Function`, so pages whose content
/// security policy forbids those run it too (edge talk's lesson).
pub(super) fn run_body(code: &str) -> String {
  // RUN_PREFIX_LINES lines before the code.
  format!(
    "const __stitchLogs = [], __stitchConsole = {{}};\n\
     const __stitchShow = a => {{ if (typeof a === 'string') return a; try {{ const s = JSON.stringify(a); return s === undefined ? String(a) : s; }} catch (e) {{ return String(a); }} }};\n\
     for (const [k, kind] of [['log', 'log'], ['info', 'info'], ['warn', 'warning'], ['error', 'error'], ['debug', 'debug']]) {{\n\
       __stitchConsole[k] = console[k];\n\
       console[k] = (...args) => {{ if (__stitchLogs.length < 200) __stitchLogs.push('[' + kind + '] ' + args.map(__stitchShow).join(' ')); return __stitchConsole[k].apply(console, args); }};\n\
     }}\n\
     try {{\n\
       const value = await (async () => {{\n\
     {code}\n\
       }})();\n\
       let text;\n\
       if (value === undefined) text = 'undefined';\n\
       else if (typeof value === 'string') text = value;\n\
       else {{ try {{ text = JSON.stringify(value, null, 2); }} catch (e) {{ text = undefined; }} if (text === undefined) text = String(value); }}\n\
       return JSON.stringify({{ value: text, logs: __stitchLogs }});\n\
     }} catch (e) {{\n\
       const stack = e && e.stack ? String(e.stack) : '';\n\
       return JSON.stringify({{ error: String(e) + (stack && !stack.startsWith(String(e)) ? '\\n' + stack : ''), logs: __stitchLogs }});\n\
     }} finally {{ Object.assign(console, __stitchConsole); }}"
  )
}

/// A page's main content, as `READ` gave its HTML, made Markdown.
pub(super) fn markdown(html: &str) -> Result<String, String> {
  htmd::HtmlToMarkdown::builder()
    .skip_tags(vec!["script", "style", "noscript"])
    .build()
    .convert(html)
    .map(|md| tidy(&md))
    .map_err(|e| e.to_string())
}

/// At most one blank line in a row.
pub(super) fn tidy(md: &str) -> String {
  let mut out = String::with_capacity(md.len());
  let mut blank = 0;
  for line in md.lines() {
    let line = line.trim_end();
    if line.is_empty() {
      blank += 1;
      if blank > 1 {
        continue;
      }
    } else {
      blank = 0;
    }
    out.push_str(line);
    out.push('\n');
  }
  out.trim().to_owned()
}

/// What `browse_web` adds to a page that is a bot check standing in for
/// the page asked for (`wall`).
pub(super) fn blocked(kind: &str) -> String {
  format!(
    "\n\n[Blocked: this is {kind}, not the page. The site refused this headless browser, \
     and retrying it here will not help. The user's own browser (the browser_* tools, \
     where available) usually gets through; or find the same content elsewhere.]"
  )
}

/// What a page is when it is a bot check that stood in for the page
/// asked for: its kind, from the walls' own titles, or from a refusal
/// with next to nothing in it (DataDome's captcha page is an empty one).
pub(super) fn wall(status: i64, title: &str, text: &str) -> Option<&'static str> {
  const TITLES: &[(&str, &str)] = &[
    ("just a moment", "Cloudflare's challenge"),
    ("attention required! | cloudflare", "Cloudflare's block"),
    ("the request could not be satisfied", "CloudFront's block"),
    ("access to this page has been denied", "PerimeterX's block"),
    ("pardon our interruption", "Imperva's block"),
    ("prove your humanity", "a human check"),
    ("verify you are human", "a human check"),
    ("are you a robot", "a human check"),
    ("let's get your identity verified", "a human check"),
  ];
  let t = title.to_lowercase();
  if let Some((_, kind)) = TITLES.iter().find(|(m, _)| t.contains(m)) {
    return Some(kind);
  }
  // Not 401: a login asked for is no bot check.
  let refused = matches!(status, 403 | 429);
  (refused && text.trim().len() < 1500).then_some("a refusal with no content")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_code_starts_where_its_lines_are_counted_from() {
    let body = run_body("MARK");
    let at = body.lines().position(|l| l.trim() == "MARK").unwrap();
    assert_eq!(at as i64, RUN_PREFIX_LINES);
  }
}
