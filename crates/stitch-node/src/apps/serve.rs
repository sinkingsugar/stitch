//! What an app's origin answers, whoever asks: the iOS app's URL scheme
//! handler (through the FFI) and the desktop's HTTP server alike.
//!
//! - `/` and the app's files, from its version's tree (fetched from the
//!   user's other nodes when not here); its entry page with the bridge put
//!   first in its `<head>`, and the state it starts with, so
//!   `localStorage` reads at once;
//! - `/_lib/<file>`: the bundled libraries;
//! - `/_stitch/bridge.js`, `/_stitch/stitch.css`;
//! - `/_stitch/state`: GET the thread's state, POST `{"set": {…},
//!   "remove": […]}`;
//! - `/_stitch/events?generation=N`: answers when the state may have
//!   changed since N (a long poll: no event streams through a scheme
//!   handler), with the state and the new generation;
//! - `/_stitch/report`, `/_stitch/error`: POST, to the thread's agent (a
//!   pinned app's: the thread it was pinned from);
//! - `/_stitch/info`: the app's name, version, and whether it is pinned;
//! - `/_stitch/live/threads`, `…/jobs`, `…/attention`, `…/schedules`,
//!   `…/loose-ends`, `…/events`: the node's live state (`live`), for an app the user
//!   let read it; `/_stitch/events` with `live=N` also answers when it
//!   changed since N. A POST to `…/loose-ends` (`check`, `dismiss=<id>`)
//!   from the built-in dashboard alone.
//! - `/_stitch/threads…`: the app's own threads, within the user's grant
//!   (`threads`, design §2.22); `/_stitch/events` with `threads=N`
//!   answers when a thread moved since N.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use stitch_records::BlobHash;

use super::{Host, Manifest, Pinned};
use crate::node::Shared;

/// A request, as either transport has it.
#[derive(Clone, Debug)]
pub struct Request {
  /// The host it was sent to (`<hash>.<hash>`, `p…`, with `.localhost`
  /// and a port on a desktop).
  pub host: String,
  /// The path, with its query.
  pub path: String,
  pub method: String,
  pub body: Vec<u8>,
}

/// What is answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
  pub status: u16,
  pub headers: Vec<(String, String)>,
  pub body: Vec<u8>,
}

impl Response {
  pub(crate) fn new(status: u16, media_type: &str, body: impl Into<Vec<u8>>) -> Response {
    Response {
      status,
      headers: vec![("Content-Type".into(), media_type.into())],
      body: body.into(),
    }
  }

  pub(crate) fn json(v: &Value) -> Response {
    Response::new(200, "application/json", v.to_string()).header("Cache-Control", "no-store")
  }

  pub(crate) fn text(status: u16, text: impl Into<String>) -> Response {
    Response::new(status, "text/plain; charset=utf-8", text.into())
  }

  pub fn header(mut self, name: &str, value: impl Into<String>) -> Response {
    self.headers.push((name.into(), value.into()));
    self
  }

  /// A header's value, by name (case aside).
  pub fn get(&self, name: &str) -> Option<&str> {
    self
      .headers
      .iter()
      .find(|(n, _)| n.eq_ignore_ascii_case(name))
      .map(|(_, v)| v.as_str())
  }
}

/// Which client asks: the CSP and the libraries differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
  /// The phone's web view, through its `stitch-app:` scheme.
  App,
  /// A browser, over HTTP; it takes gzip.
  Browser { gzip: bool },
}

/// The URL scheme the phone serves apps under.
pub const SCHEME: &str = "stitch-app";

/// How long a page's wait for changes lasts at most.
const POLL: Duration = Duration::from_secs(25);

/// An app: its version, and its pin when it is opened as one.
struct Opened {
  manifest: BlobHash,
  m: Manifest,
  pinned: Option<Pinned>,
}

pub(crate) async fn serve(s: &Arc<Shared>, req: Request, client: Client) -> Response {
  let (path, query) = match req.path.split_once('?') {
    Some((p, q)) => (p.to_owned(), q.to_owned()),
    None => (req.path.clone(), String::new()),
  };
  let path = percent_decode(&path);
  // What every app shares.
  if let Some(lib) = path.strip_prefix("/_lib/") {
    return super::libs::serve(lib, client);
  }
  match path.as_str() {
    "/_stitch/bridge.js" => {
      return Response::new(200, "text/javascript; charset=utf-8", BRIDGE)
        .header("Cache-Control", "no-cache");
    }
    "/_stitch/stitch.css" => {
      return Response::new(200, "text/css; charset=utf-8", STYLE)
        .header("Cache-Control", "no-cache");
    }
    _ => {}
  }
  let opened = match open(s, &req.host).await {
    Ok(o) => o,
    Err(r) => return r,
  };
  let csp = csp(
    &opened.m,
    client,
    opened.pinned.as_ref().is_some_and(super::live::is_client),
  );
  let r = match path.strip_prefix("/_stitch/") {
    Some(api) => self::api(s, &opened, api, &query, &req).await,
    None => file(s, &opened, &path, client).await,
  };
  match csp {
    Some(c) => r.header("Content-Security-Policy", c),
    None => r,
  }
}

/// The app a host names, its manifest fetched when not here.
async fn open(s: &Arc<Shared>, host: &str) -> Result<Opened, Response> {
  let state = s.state();
  let (manifest, pinned) = match Host::parse(host) {
    Some(Host::Version(h)) => (h, None),
    Some(Host::Pinned(p)) => {
      let pinned = super::pinned_all(&state.index.lock().expect("index lock")).unwrap_or_default();
      match pinned.into_iter().find(|a| a.host == p) {
        Some(a) => match super::live::serving_manifest(&state.blobs, &a) {
          Ok(m) => (m, Some(a)),
          Err(e) => return Err(Response::text(500, format!("the web client: {e}"))),
        },
        None => return Err(Response::text(404, "no such pinned app (unpinned?)")),
      }
    }
    None => return Err(Response::text(404, "no app here")),
  };
  if !state.blobs.contains(&manifest) {
    crate::node::fetch_blobs(s, &[manifest]).await;
  }
  match Manifest::read(&state.blobs, &manifest) {
    Some(m) => Ok(Opened {
      manifest,
      m,
      pinned,
    }),
    None => Err(Response::text(
      404,
      "this app is not on this device, and no other of your devices that has it could be \
       reached",
    )),
  }
}

/// What an app's pages may reach: without networking, only this origin
/// (and what it makes: `data:`, `blob:`), WebAssembly included.
fn csp(m: &Manifest, client: Client, web_client: bool) -> Option<String> {
  if m.networking {
    return None;
  }
  let own = match client {
    Client::App => format!("'self' {SCHEME}:"),
    Client::Browser { .. } => "'self'".into(),
  };
  // The web client shows apps inline, in sandboxed iframes on their own
  // origins (design §2.24): `.localhost` names, nothing else.
  let frames = if web_client {
    format!("{own} http://*.localhost:* data: blob:")
  } else {
    format!("{own} data: blob:")
  };
  Some(format!(
    "default-src {own} data: blob:; script-src {own} 'unsafe-inline' 'unsafe-eval' \
     'wasm-unsafe-eval' data: blob:; style-src {own} 'unsafe-inline' data:; connect-src {own} \
     data: blob:; img-src {own} data: blob:; media-src {own} data: blob:; font-src {own} data:; \
     worker-src {own} blob: data:; frame-src {frames}; form-action 'none'"
  ))
}

/// A file of the app's tree; its entry page with the bridge.
async fn file(s: &Arc<Shared>, o: &Opened, path: &str, client: Client) -> Response {
  let state = s.state();
  let rel = path.trim_start_matches('/');
  let rel = if rel.is_empty() || rel.ends_with('/') {
    format!(
      "{rel}{}",
      if rel.is_empty() {
        &o.m.entry
      } else {
        "index.html"
      }
    )
  } else {
    rel.to_owned()
  };
  if rel
    .split('/')
    .any(|p| p == ".." || p == "." || p.is_empty())
  {
    return Response::text(404, "no such file");
  }
  // Trees missing here are fetched, a level at a time.
  let flat = loop {
    match crate::folders::load_tree(&state.blobs, &o.m.tree) {
      Ok(f) => break f,
      Err(missing) => {
        if crate::node::fetch_blobs(s, &missing).await == 0 {
          return Response::text(
            404,
            "this app's files are not on this device, and no other of your devices that has \
             them could be reached",
          );
        }
      }
    }
  };
  let hash = match flat.get(&rel) {
    Some(crate::folders::Leaf::File { hash, .. }) => *hash,
    _ => return Response::text(404, format!("{rel}: no such file in the app")),
  };
  if !state.blobs.contains(&hash) {
    crate::node::fetch_blobs(s, &[hash]).await;
  }
  let Ok(mut bytes) = std::fs::read(state.blobs.path(&hash)) else {
    return Response::text(
      404,
      format!(
        "{rel} is not on this device, and no other of your devices that has it could be reached"
      ),
    );
  };
  let media_type = media_type(&rel);
  if rel == o.m.entry {
    let thread_state = match &o.m.thread {
      Some(t) => s
        .apps
        .state(&state.index.lock().expect("index lock"), t)
        .unwrap_or_default(),
      None => BTreeMap::new(),
    };
    bytes = with_bridge(&bytes, &start(s, o, &thread_state), o, client);
  }
  Response::new(200, media_type, bytes).header("Cache-Control", "no-cache")
}

/// What the bridge starts with.
fn start(s: &Arc<Shared>, o: &Opened, state: &BTreeMap<String, String>) -> Value {
  json!({
    "name": o.m.name,
    "version": &o.manifest.hex()[..8],
    "pinned": o.pinned.is_some(),
    "thread":o.m.thread.as_ref().map(|t| t.to_string()),
    "state": state,
    "generation": s.apps.generation(),
    "live": live_said(s, o),
    "threads": threads_said(s, o),
  })
}

/// Where an app stands for threads of its own, with the generation its
/// long poll waits past when it may run them.
fn threads_said(s: &Arc<Shared>, o: &Opened) -> Value {
  let mut v = super::threads::said(s, &o.m);
  if v["consent"] != "none"
    && let Some(t) = &o.m.thread
  {
    // The family's own mark: it moves only when one of its threads does.
    let owner = super::threads::Owner {
      thread: t.clone(),
      name: o.m.name.clone(),
    };
    v["generation"] = json!(super::threads::family_mark(s, &owner));
  }
  v
}

/// Whether an app may read live state (`allowed`, `denied`, `ask`), with
/// the generations of each kind when it may.
fn live_said(s: &Arc<Shared>, o: &Opened) -> Value {
  match super::live::consent(s, &o.m, o.pinned.as_ref()) {
    Some(true) => {
      let mut v = s.apps.live.said();
      v["consent"] = json!("allowed");
      v
    }
    Some(false) => json!({"consent": "denied"}),
    None => json!({"consent": "ask"}),
  }
}

/// `/_stitch/live/<kind>`: the node's live state, for an app allowed to
/// read it; else 403 saying whether the user may still be asked.
async fn live(s: &Arc<Shared>, o: &Opened, kind: &str, query: &str, post: bool) -> Response {
  // The one write: loose ends, from the built-in dashboard alone (a pass
  // spends; a dismissal is the user's).
  let builtin = o
    .pinned
    .as_ref()
    .and_then(|p| p.builtin.as_ref())
    .is_some_and(|(k, _)| k == super::live::DASHBOARD_KEY);
  if post && !(kind == "loose-ends" && builtin) {
    return Response::text(405, "live state is read with GET, and only read");
  }
  let consent = super::live::consent(s, &o.m, o.pinned.as_ref());
  if consent != Some(true) {
    let (name, thread) = match &o.pinned {
      Some(p) => (p.name.as_str(), None),
      None => (o.m.name.as_str(), o.m.thread.as_ref()),
    };
    let mut r = Response::json(&super::live::refused(name, thread, consent));
    r.status = 403;
    return r;
  }
  // The first read starts the watch, so changes after it are heard.
  let _listening = super::live::Live::listen(s);
  match kind {
    "threads" => {
      let s2 = s.clone();
      let thread = o.m.thread.clone();
      let query = query.to_owned();
      match tokio::task::spawn_blocking(move || super::live::threads(&s2, thread.as_ref(), &query))
        .await
      {
        Ok(Ok(v)) => Response::json(&v),
        Ok(Err(e)) => Response::text(400, e),
        Err(e) => Response::text(500, e.to_string()),
      }
    }
    "attention" => {
      let s2 = s.clone();
      match tokio::task::spawn_blocking(move || super::live::attention(&s2)).await {
        Ok(v) => Response::json(&v),
        Err(e) => Response::text(500, e.to_string()),
      }
    }
    "jobs" => Response::json(&super::live::jobs(s).await),
    "schedules" => {
      let s2 = s.clone();
      let query = query.to_owned();
      match tokio::task::spawn_blocking(move || super::live::schedules(&s2, &query)).await {
        Ok(v) => Response::json(&v),
        Err(e) => Response::text(500, e.to_string()),
      }
    }
    "loose-ends" if post => {
      let param = |k: &str| {
        query
          .split('&')
          .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
          .find(|(key, _)| *key == k)
          .map(|(_, v)| percent_decode(v))
      };
      if let Some(t) = param("dismiss") {
        let Ok(thread) = t.parse::<stitch_records::RecordId>() else {
          return Response::text(400, format!("`{t}` is not a thread's ID"));
        };
        return match crate::loose::dismiss(s, &thread) {
          Ok(()) => Response::json(&json!({"ok": true, "dismissed": t})),
          Err(e) => Response::text(400, e.to_string()),
        };
      }
      if param("check").is_some() {
        // In the background: the page hears of it through `threads`.
        let s2 = s.clone();
        tokio::spawn(async move {
          match crate::loose::pass(
            &s2,
            &crate::loose::Judge::All,
            jiff::SignedDuration::ZERO,
            false,
          )
          .await
          {
            Ok(p) => eprintln!("loose ends (dashboard): {}", crate::loose::said(&p)),
            Err(e) => eprintln!("loose ends (dashboard): no pass: {e}"),
          }
        });
        return Response::json(&json!({"ok": true, "checking": true}));
      }
      Response::text(400, "loose ends: `check`, or `dismiss=<thread>`")
    }
    "loose-ends" => {
      let s2 = s.clone();
      match tokio::task::spawn_blocking(move || crate::loose::live(&s2)).await {
        Ok(v) => Response::json(&v),
        Err(e) => Response::text(500, e.to_string()),
      }
    }
    "events" => {
      let s2 = s.clone();
      let query = query.to_owned();
      match tokio::task::spawn_blocking(move || crate::events::live(&s2, &query)).await {
        Ok(v) => Response::json(&v),
        Err(e) => Response::text(500, e.to_string()),
      }
    }
    _ => Response::text(
      404,
      "no such live state: threads, jobs, attention, schedules, loose-ends or events",
    ),
  }
}

/// The entry page with the bridge first in its `<head>` (or first of all),
/// the state it starts with inlined, and on a desktop a web app manifest
/// for a pinned app (installed, it opens in a window of its own).
fn with_bridge(page: &[u8], start: &Value, o: &Opened, client: Client) -> Vec<u8> {
  let page = String::from_utf8_lossy(page);
  // `</` never ends the script early.
  let data = start.to_string().replace("</", "<\\/");
  let mut head = format!(
    "<meta charset=\"utf-8\"><script>window.__stitch={data};</script>\
     <script src=\"/_stitch/bridge.js\"></script>"
  );
  if !page.contains("name=\"viewport\"") && !page.contains("name='viewport'") {
    head.push_str(
      "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">",
    );
  }
  if let Some(map) = super::libs::import_map(&page) {
    head.push_str(&map);
  }
  if let (Some(p), Client::Browser { .. }) = (&o.pinned, client) {
    head.push_str(&format!(
      // A manifest is fetched without cookies unless asked: it would be
      // refused (403) without the token's.
      "<link rel=\"manifest\" href=\"/_stitch/manifest.webmanifest\" \
       crossorigin=\"use-credentials\"><title>{}</title>",
      escape(&p.name)
    ));
  }
  let lower = page.to_ascii_lowercase();
  let at = match lower.find("<head") {
    Some(i) => lower[i..].find('>').map(|j| i + j + 1),
    None => None,
  };
  let at = at.unwrap_or_else(|| {
    // After a doctype, else first.
    lower
      .find("<!doctype")
      .and_then(|i| lower[i..].find('>').map(|j| i + j + 1))
      .unwrap_or(0)
  });
  let mut out = String::with_capacity(page.len() + head.len());
  out.push_str(&page[..at]);
  out.push_str(&head);
  out.push_str(&page[at..]);
  out.into_bytes()
}

fn escape(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
}

#[derive(Deserialize)]
struct Writes {
  #[serde(default)]
  set: BTreeMap<String, Value>,
  #[serde(default)]
  remove: Vec<String>,
}

async fn api(s: &Arc<Shared>, o: &Opened, api: &str, query: &str, req: &Request) -> Response {
  let post = req.method.eq_ignore_ascii_case("POST");
  let thread = o.m.thread.clone();
  if let Some(kind) = api.strip_prefix("live/") {
    return live(s, o, kind, query, post).await;
  }
  if let Some(rest) = api.strip_prefix("threads")
    && (rest.is_empty() || rest.starts_with('/'))
  {
    return super::threads::api(s, &o.m, rest, query, req).await;
  }
  if let Some(rest) = api.strip_prefix("files")
    && (rest.is_empty() || rest.starts_with('/'))
  {
    return super::files::api(s, &o.m, rest, query, req).await;
  }
  match (api, post) {
    ("info", false) => Response::json(&json!({
      "name": o.m.name,
      "version": &o.manifest.hex()[..8],
      "pinned": o.pinned.is_some(),
      "networking": o.m.networking,
    })),
    ("state", false) => {
      let Some(t) = thread else {
        return Response::json(&json!({"state": {}, "generation": 0}));
      };
      let generation = s.apps.generation();
      let st = s
        .apps
        .state(&s.state().index.lock().expect("index lock"), &t)
        .unwrap_or_default();
      Response::json(&json!({"state": st, "generation": generation}))
    }
    ("state", true) => {
      let Some(t) = thread else {
        return Response::text(409, "this app keeps no state: it has no thread");
      };
      let w: Writes = match serde_json::from_slice(&req.body) {
        Ok(w) => w,
        Err(e) => return Response::text(400, e.to_string()),
      };
      match super::write(s, &t, w.set, w.remove) {
        Ok(()) => Response::json(&json!({"ok": true})),
        Err(e) => Response::text(400, e.to_string()),
      }
    }
    ("events", false) => {
      let number = |key: &str| {
        query
          .split('&')
          .find_map(|kv| kv.strip_prefix(key))
          .and_then(|v| v.parse::<u64>().ok())
      };
      let seen = number("generation=").unwrap_or(0);
      // Live state's changes too, for an app allowed to read it.
      let live_seen =
        number("live=").filter(|_| super::live::consent(s, &o.m, o.pinned.as_ref()) == Some(true));
      // The app's threads moving too, for an app that has any: the same
      // watch, its `threads` mark (design §2.22).
      let threads_seen = number("threads=").filter(|_| {
        o.m.thread.is_some()
          && super::threads::consent(&s.state().index.lock().expect("index lock"), &o.m)
            != super::threads::Consent::None
      });
      let _listening = live_seen
        .or(threads_seen)
        .map(|_| super::live::Live::listen(s));
      let live_wait = async {
        match live_seen {
          Some(g) => s.apps.live.wait(g, POLL).await,
          None => std::future::pending().await,
        }
      };
      // Its threads: answered only when one of its family moved (its own
      // mark), never for the user's other threads.
      let threads_wait = async {
        let (Some(mark), Some(t)) = (threads_seen, &o.m.thread) else {
          return std::future::pending().await;
        };
        let owner = super::threads::Owner {
          thread: t.clone(),
          name: o.m.name.clone(),
        };
        let deadline = tokio::time::Instant::now() + POLL;
        loop {
          let g = s.apps.live.generation();
          let (s2, o2) = (s.clone(), owner.clone());
          let now = tokio::task::spawn_blocking(move || super::threads::family_mark(&s2, &o2))
            .await
            .unwrap_or(mark);
          if now != mark {
            return;
          }
          let left = deadline.saturating_duration_since(tokio::time::Instant::now());
          if left.is_zero() {
            return std::future::pending().await;
          }
          s.apps.live.wait(g, left).await;
        }
      };
      tokio::select! {
        () = s.apps.wait(seen, POLL) => {}
        () = live_wait => {}
        () = threads_wait => {}
      }
      let generation = s.apps.generation();
      let st = match &thread {
        Some(t) => s
          .apps
          .state(&s.state().index.lock().expect("index lock"), t)
          .unwrap_or_default(),
        None => BTreeMap::new(),
      };
      Response::json(&json!({
        "state": st,
        "generation": generation,
        "live": live_said(s, o),
        "threads": threads_said(s, o),
      }))
    }
    ("report", true) => {
      // A pinned app reports to the thread it was pinned from, as its
      // version; an app in its conversation, to its thread.
      let (thread, from) = match &o.pinned {
        Some(p) => match &p.thread {
          Some(t) if s.thread_place(t).is_ok() => (
            t.clone(),
            format!(
              "Pinned app \"{}\" (version {})",
              p.name,
              &o.manifest.hex()[..8]
            ),
          ),
          Some(t) => {
            return Response::text(
              409,
              format!(
                "this pinned app's conversation ({t}) is not on this device (deleted?): its \
                 reports have nowhere to go"
              ),
            );
          }
          None => {
            return Response::text(
              409,
              "this pinned app has no conversation to report to: pin it from the conversation \
               that shows it",
            );
          }
        },
        None => match &thread {
          Some(t) => (t.clone(), format!("App \"{}\"", o.m.name)),
          None => return Response::text(409, "this app has no thread to report to"),
        },
      };
      match super::report(s, &thread, &from, &String::from_utf8_lossy(&req.body)).await {
        Ok(said) => Response::json(&json!({"ok": true, "said": said})),
        Err((status, e)) => Response::text(status, e),
      }
    }
    ("error", true) => {
      // Pinned or not: once per version, of its thread's latest only
      // (design §2.10).
      super::error(
        s,
        &o.manifest,
        &o.m,
        o.pinned.is_some(),
        &String::from_utf8_lossy(&req.body),
      );
      Response::json(&json!({"ok": true}))
    }
    ("manifest.webmanifest", false) => {
      let name = o
        .pinned
        .as_ref()
        .map_or(o.m.name.clone(), |p| p.name.clone());
      Response::new(
        200,
        "application/manifest+json",
        json!({
          "name": name,
          "short_name": name,
          "start_url": "/",
          "display": "standalone",
          "icons": [{"src": "/_stitch/icon.svg", "sizes": "any", "type": "image/svg+xml"}],
        })
        .to_string(),
      )
    }
    ("icon.svg", false) => {
      let name = o.pinned.as_ref().map_or(&o.m.name, |p| &p.name);
      Response::new(200, "image/svg+xml", icon(name))
    }
    _ => Response::text(404, "no such call"),
  }
}

/// A pinned app's icon on a desktop: its initial on a colour of its name.
pub fn icon(name: &str) -> String {
  let hue = blake3::hash(name.as_bytes()).as_bytes()[0] as u32 * 360 / 256;
  let initial = name
    .chars()
    .find(|c| c.is_alphanumeric())
    .map(|c| c.to_uppercase().to_string())
    .unwrap_or_else(|| "·".into());
  format!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"><rect width=\"64\" \
     height=\"64\" rx=\"14\" fill=\"hsl({hue},55%,45%)\"/><text x=\"32\" y=\"43\" \
     font-family=\"-apple-system,Segoe UI,sans-serif\" font-size=\"32\" font-weight=\"600\" \
     fill=\"#fff\" text-anchor=\"middle\">{}</text></svg>",
    escape(&initial)
  )
}

/// A file's media type, by its extension.
pub fn media_type(path: &str) -> &'static str {
  let ext = path
    .rsplit_once('.')
    .map(|(_, e)| e.to_ascii_lowercase())
    .unwrap_or_default();
  match ext.as_str() {
    "html" | "htm" => "text/html; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "json" | "map" => "application/json",
    "webmanifest" => "application/manifest+json",
    "txt" | "md" => "text/plain; charset=utf-8",
    "csv" => "text/csv; charset=utf-8",
    "xml" => "application/xml",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "avif" => "image/avif",
    "ico" => "image/x-icon",
    "wasm" => "application/wasm",
    "woff2" => "font/woff2",
    "woff" => "font/woff",
    "ttf" => "font/ttf",
    "otf" => "font/otf",
    "mp3" => "audio/mpeg",
    "wav" => "audio/wav",
    "ogg" => "audio/ogg",
    "m4a" => "audio/mp4",
    "mp4" => "video/mp4",
    "webm" => "video/webm",
    "glb" => "model/gltf-binary",
    "gltf" => "model/gltf+json",
    "stl" => "model/stl",
    "pdf" => "application/pdf",
    _ => "application/octet-stream",
  }
}

/// `%xx` decoded; what is not valid stays as it is.
pub(crate) fn percent_decode(s: &str) -> String {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%'
      && i + 2 < b.len()
      && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
    {
      out.push(v);
      i += 3;
      continue;
    }
    out.push(b[i]);
    i += 1;
  }
  String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

const BRIDGE: &str = include_str!("bridge.js");
const STYLE: &str = include_str!("stitch.css");

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_bridge_goes_first_in_the_head() {
    let m = Manifest {
      app: 1,
      name: "T".into(),
      tree: BlobHash::of(b"t"),
      entry: "index.html".into(),
      networking: false,
      thread: None,
      needs: None,
    };
    let o = Opened {
      manifest: BlobHash::of(b"m"),
      m,
      pinned: None,
    };
    let start = json!({"state": {"k": "</script>"}});
    let page = with_bridge(
      b"<!DOCTYPE html><html><HEAD><title>x</title></head><body></body></html>",
      &start,
      &o,
      Client::App,
    );
    let page = String::from_utf8(page).unwrap();
    assert!(
      page
        .starts_with("<!DOCTYPE html><html><HEAD><meta charset=\"utf-8\"><script>window.__stitch=")
    );
    assert!(page.contains("<\\/script>"));
    assert!(page.contains("name=\"viewport\""));
    assert!(page.ends_with("<title>x</title></head><body></body></html>"));

    let bare = String::from_utf8(with_bridge(b"<p>hi</p>", &start, &o, Client::App)).unwrap();
    assert!(bare.starts_with("<meta charset"));
    assert!(bare.ends_with("<p>hi</p>"));
  }

  #[test]
  fn decoding_and_types() {
    assert_eq!(percent_decode("/a%20b/%E2%9C%93"), "/a b/✓");
    assert_eq!(percent_decode("/100%"), "/100%");
    assert_eq!(media_type("x/y.MJS"), "text/javascript; charset=utf-8");
    assert_eq!(media_type("noext"), "application/octet-stream");
  }
}
