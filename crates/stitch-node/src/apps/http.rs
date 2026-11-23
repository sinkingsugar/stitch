//! Apps in a desktop browser (design §2.10): the node serves them over HTTP
//! on this machine only (`127.0.0.1` and `::1`, `[apps] port`), each app at
//! a host of its own under `localhost` (browsers and the system resolve
//! `*.localhost` there), so apps are origins apart.
//!
//! Only the user's own browser gets in: a link the node gives (`stitch app
//! open`, `show_app`'s result) carries the node's token once (`?k=`); the
//! answer sets it as a cookie of that host and drops it from the address.
//! A request must name this server as its host (no DNS rebinding), and a
//! POST must come from the page's own origin.
//!
//! `localhost` itself lists the pinned apps.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;

use super::serve::{self, Client, Request, Response};
use super::ws;
use crate::node::Shared;
use crate::{Error, Result};

/// The port apps are served on, unless `[apps] port` says otherwise
/// (0: not served).
pub const PORT: u16 = 7177;
/// Largest request body taken.
const MAX_BODY: usize = 16 << 20;
const COOKIE: &str = "stitch";

/// Where this node serves apps, and the token a browser needs.
#[derive(Clone)]
pub(crate) struct Desktop {
  pub port: u16,
  token: String,
}

impl Desktop {
  /// A link that lets the browser in: on `host` (an app's, or `""` for
  /// the list of pinned apps), at `path`.
  pub(crate) fn url(&self, host: &str, path: &str) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{}{path}{sep}k={}", self.origin(host), self.token)
  }

  /// The same without the token: what a transcript may keep (it syncs,
  /// and goes to the model's provider).
  pub(crate) fn address(&self, host: &str, path: &str) -> String {
    format!("{}{path}", self.origin(host))
  }

  fn origin(&self, host: &str) -> String {
    if host.is_empty() {
      format!("http://localhost:{}", self.port)
    } else {
      format!("http://{host}.localhost:{}", self.port)
    }
  }

  /// An address of this server's with the token added, for the node's own
  /// browser (`browse_web`, `web_eval`) to get in; another address as it
  /// is.
  pub(crate) fn admit(&self, url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    let ours = u.scheme() == "http"
      && u.port() == Some(self.port)
      && (host == "localhost" || host.ends_with(".localhost"));
    if !ours || u.query_pairs().any(|(k, _)| k == "k") {
      return None;
    }
    let mut u = u;
    u.query_pairs_mut().append_pair("k", &self.token);
    Some(u.to_string())
  }
}

/// The node's token, made once and kept (`apps.token`, readable by the user
/// only).
fn token(data: &Path) -> Result<String> {
  let path = data.join("apps.token");
  if let Ok(t) = std::fs::read_to_string(&path) {
    let t = t.trim();
    if t.len() >= 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
      return Ok(t.to_owned());
    }
  }
  let mut bytes = [0u8; 20];
  ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
    .map_err(|_| Error::Config("no randomness for the apps' token".into()))?;
  let t: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
  std::fs::write(&path, &t).map_err(Error::io(path.display()))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
  }
  Ok(t)
}

/// Serves apps until the node stops; `Ok(None)` when the port is 0.
pub(crate) async fn start(
  s: &Arc<Shared>,
  port: u16,
  stop: watch::Receiver<bool>,
) -> Result<Option<Desktop>> {
  if port == 0 {
    return Ok(None);
  }
  let desktop = Desktop {
    port,
    token: token(&s.data)?,
  };
  let v4 = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
    .await
    .map_err(|e| {
      Error::Config(format!(
        "apps: port {port}: {e} (set another with [apps] port)"
      ))
    })?;
  // `localhost` may resolve to `::1` first.
  let v6 = TcpListener::bind(SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)))
    .await
    .ok();
  for listener in std::iter::once(v4).chain(v6) {
    let (s, d, mut stop) = (s.clone(), desktop.clone(), stop.clone());
    tokio::spawn(async move {
      loop {
        let stream = tokio::select! {
          a = listener.accept() => match a {
            Ok((stream, _)) => stream,
            Err(_) => continue,
          },
          _ = stop.changed() => break,
        };
        let (s, d) = (s.clone(), d.clone());
        tokio::spawn(async move {
          let service = service_fn(move |req| {
            let (s, d) = (s.clone(), d.clone());
            async move { Ok::<_, Infallible>(handle(&s, &d, req).await) }
          });
          let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
            .await;
        });
      }
    });
  }
  Ok(Some(desktop))
}

type HttpResponse = hyper::Response<Full<Bytes>>;

fn to_http(r: Response) -> HttpResponse {
  let mut b = hyper::Response::builder().status(r.status);
  for (n, v) in &r.headers {
    b = b.header(n.as_str(), v.as_str());
  }
  b.header("X-Content-Type-Options", "nosniff")
    .body(Full::new(Bytes::from(r.body)))
    .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

fn page(status: u16, title: &str, body: &str) -> Response {
  Response {
    status,
    headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
    body: format!(
      "<!doctype html><meta charset=utf-8><meta name=viewport \
       content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
       <link rel=stylesheet href=\"/_stitch/stitch.css\"><body><h1>{title}</h1>{body}</body>"
    )
    .into_bytes(),
  }
}

async fn handle(s: &Arc<Shared>, d: &Desktop, mut req: hyper::Request<Incoming>) -> HttpResponse {
  let header = |n: &str| {
    req
      .headers()
      .get(n)
      .and_then(|v| v.to_str().ok())
      .map(str::to_owned)
  };
  // The host this server is, by name: a page of another site that resolves
  // its own name here (DNS rebinding) is not let in.
  let host = header("host").unwrap_or_default().to_ascii_lowercase();
  let Some(name) = host
    .strip_suffix(&format!(":{}", d.port))
    .map(str::to_owned)
  else {
    return to_http(Response {
      status: 421,
      headers: Vec::new(),
      body: Vec::new(),
    });
  };
  let launcher = matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]");
  if !launcher && !name.ends_with(".localhost") {
    return to_http(Response {
      status: 421,
      headers: Vec::new(),
      body: Vec::new(),
    });
  }
  let path = req
    .uri()
    .path_and_query()
    .map(|p| p.as_str().to_owned())
    .unwrap_or_else(|| "/".into());
  let method = req.method().as_str().to_owned();
  // The token, once in the address: a cookie of this host from then on.
  let (bare, given) = without_token(&path);
  if let Some(k) = given {
    if k != d.token {
      return to_http(denied());
    }
    return to_http(Response {
      status: 303,
      headers: vec![
        ("Location".into(), bare),
        (
          "Set-Cookie".into(),
          // In an iframe (the web client shows apps inline, §2.24) the
          // page is cross-site to its embedder: a Lax cookie is not sent,
          // so it is set partitioned under the embedder, for it alone.
          if header("sec-fetch-dest").as_deref() == Some("iframe") {
            format!(
              "{COOKIE}={}; Path=/; HttpOnly; Secure; SameSite=None; Partitioned; Max-Age=31536000",
              d.token
            )
          } else {
            format!(
              "{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000",
              d.token
            )
          },
        ),
        ("Cache-Control".into(), "no-store".into()),
      ],
      body: Vec::new(),
    });
  }
  let has_cookie = header("cookie").is_some_and(|c| {
    c.split(';')
      .filter_map(|kv| kv.trim().split_once('='))
      .any(|(k, v)| k == COOKIE && v == d.token)
  });
  // The shared files of every app carry nothing of the user's.
  let public = path.starts_with("/_lib/")
    || path.starts_with("/_stitch/stitch.css")
    || path.starts_with("/_stitch/bridge.js");
  if !has_cookie && !public {
    return to_http(denied());
  }
  // The web client's WebSocket: a browser applies no CORS to it, so its
  // Origin is checked here (design §2.24).
  if path.split('?').next() == Some(ws::PATH) {
    if let Some(no) = ws::refusal(
      s,
      &name,
      &host,
      header("origin").as_deref(),
      header("sec-fetch-site").as_deref(),
    ) {
      return to_http(no);
    }
    return to_http(ws::upgrade(s, &mut req));
  }
  if method != "GET" && method != "HEAD" {
    let own = format!("http://{host}");
    if header("origin").is_some_and(|o| o != own)
      || header("sec-fetch-site").is_some_and(|f| f != "same-origin")
    {
      return to_http(Response {
        status: 403,
        headers: Vec::new(),
        body: b"not from this app".to_vec(),
      });
    }
  }
  let (origin, site) = (header("origin"), header("sec-fetch-site"));
  let gzip = header("accept-encoding").is_some_and(|a| a.contains("gzip"));
  // The web client's uploads (any file) have a larger limit of their own.
  let limit = if super::blob::is_blob_path(path.split('?').next().unwrap_or(&path)) {
    super::blob::MAX_UPLOAD
  } else {
    MAX_BODY
  };
  let body = match Limited::new(req.into_body(), limit).collect().await {
    Ok(b) => b.to_bytes().to_vec(),
    Err(_) => {
      return to_http(Response {
        status: 413,
        headers: Vec::new(),
        body: Vec::new(),
      });
    }
  };
  // The web client's image blobs (design §2.24): its origin alone.
  if path
    .split('?')
    .next()
    .is_some_and(super::blob::is_blob_path)
  {
    return to_http(
      super::blob::serve(
        s,
        &name,
        &host,
        &method,
        &path,
        origin.as_deref(),
        site.as_deref(),
        body,
      )
      .await,
    );
  }
  if launcher && !public {
    return to_http(list(s, d));
  }
  let r = serve::serve(
    s,
    Request {
      host: name,
      path,
      method,
      body,
    },
    Client::Browser { gzip },
  )
  .await;
  to_http(r)
}

/// The path without `k=`, and the token it had.
fn without_token(path: &str) -> (String, Option<String>) {
  let Some((p, q)) = path.split_once('?') else {
    return (path.to_owned(), None);
  };
  let mut token = None;
  let rest: Vec<&str> = q
    .split('&')
    .filter(|kv| match kv.strip_prefix("k=") {
      Some(t) => {
        token = Some(t.to_owned());
        false
      }
      None => !kv.is_empty(),
    })
    .collect();
  let bare = if rest.is_empty() {
    p.to_owned()
  } else {
    format!("{p}?{}", rest.join("&"))
  };
  (bare, token)
}

fn denied() -> Response {
  page(
    403,
    "Not let in",
    "<p>Open apps with a link from Stitch: <code>stitch app open</code> (the pinned apps), \
     <code>stitch app open &lt;thread&gt;</code> (the latest app a thread showed), or the link \
     the chat prints when an agent shows one (<code>/app</code> in the TUI opens it).</p>",
  )
}

/// The pinned apps, as links (each with the token: they are other hosts).
fn list(s: &Arc<Shared>, d: &Desktop) -> Response {
  let pinned = super::pinned(&s.state().index.lock().expect("index lock")).unwrap_or_default();
  let mut body = String::new();
  if pinned.is_empty() {
    body.push_str(
      "<p>No pinned apps yet. Pin one from the phone, or with <code>stitch app pin</code>.</p>",
    );
  } else {
    body.push_str("<ul style=\"list-style:none;padding:0\">");
    for p in &pinned {
      body.push_str(&format!(
        "<li style=\"margin:10px 0\"><a href=\"{}\" style=\"display:flex;align-items:center;\
         gap:12px;text-decoration:none;color:inherit\"><img src=\"data:image/svg+xml;base64,{}\" \
         width=40 height=40 alt=\"\"><span>{}</span></a></li>",
        d.url(&p.host, "/"),
        base64::Engine::encode(
          &base64::engine::general_purpose::STANDARD,
          serve::icon(&p.name)
        ),
        p.name
          .replace('&', "&amp;")
          .replace('<', "&lt;")
          .replace('>', "&gt;")
      ));
    }
    body.push_str("</ul><p style=\"color:var(--fg2)\">An app's page can be installed as an app of its own (the browser's Install, or Add to Dock).</p>");
  }
  page(200, "Apps", &body)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_token_leaves_the_address() {
    assert_eq!(
      without_token("/x?k=abc&y=1"),
      ("/x?y=1".into(), Some("abc".into()))
    );
    assert_eq!(without_token("/?k=abc"), ("/".into(), Some("abc".into())));
    assert_eq!(without_token("/x?y=1"), ("/x?y=1".into(), None));
    let d = Desktop {
      port: 7177,
      token: "t".into(),
    };
    assert_eq!(d.url("", "/"), "http://localhost:7177/?k=t");
    assert_eq!(
      d.url("pab", "/a?b=1"),
      "http://pab.localhost:7177/a?b=1&k=t"
    );
    assert_eq!(d.address("pab", "/"), "http://pab.localhost:7177/");
    // The node's own browser gets in where this server is, nowhere else.
    assert_eq!(
      d.admit("http://pab.localhost:7177/x?y=1").as_deref(),
      Some("http://pab.localhost:7177/x?y=1&k=t")
    );
    assert_eq!(d.admit("http://pab.localhost:7178/"), None);
    assert_eq!(d.admit("https://example.com/"), None);
    assert_eq!(d.admit("http://evil.com:7177/"), None);
  }
}
