//! `stitch proxy` (design §2.21): an HTTP server on the host that forwards
//! model API requests to the provider with the real API key, so a client
//! that must not hold it (a bot's node in a container) never does. Each
//! client has a token of its own instead, with a daily token budget (all
//! of it, and per end user) and a request rate; usage is counted from the
//! responses (streams pass through unbuffered, metered as they go), kept
//! across restarts and logged per token, end user and thread.
//!
//! Requests are `/<upstream>/<the provider's path>`: a node's provider
//! `base_url` is the proxy's address and the upstream's name
//! (`http://host.docker.internal:7178/anthropic`), its key the token.
//! Only the model endpoints are forwarded.

pub mod config;
pub mod meter;
pub mod usage;

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http_body_util::{BodyExt, Full, Limited, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Method, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;

pub use config::Config;
use meter::{Counts, Meter};
use usage::Book;

/// Largest request body forwarded (images go inline).
const MAX_BODY: usize = 32 << 20;
pub const END_USER: &str = "x-stitch-end-user";
pub const THREAD: &str = "x-stitch-thread";

/// The provider key held for a secret name and upstream kind.
pub type Keys = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

type Body = UnsyncBoxBody<Bytes, std::io::Error>;

struct State {
  config: Config,
  mtime: Option<SystemTime>,
  book: Book,
  /// Each token's requests of the last minute.
  windows: HashMap<String, VecDeque<Instant>>,
}

pub struct Proxy {
  dir: PathBuf,
  keys: Keys,
  client: reqwest::Client,
  state: Mutex<State>,
}

fn mtime(dir: &Path) -> Option<SystemTime> {
  std::fs::metadata(Config::path(dir))
    .and_then(|m| m.modified())
    .ok()
}

impl Proxy {
  pub fn new(dir: &Path, keys: Keys) -> Result<Arc<Proxy>, String> {
    let config = Config::load(dir)?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
      .connect_timeout(Duration::from_secs(30))
      .read_timeout(Duration::from_secs(600))
      .redirect(reqwest::redirect::Policy::none())
      .build()
      .map_err(|e| e.to_string())?;
    Ok(Arc::new(Proxy {
      dir: dir.to_owned(),
      keys,
      client,
      state: Mutex::new(State {
        mtime: mtime(dir),
        config,
        book: Book::load(dir),
        windows: HashMap::new(),
      }),
    }))
  }

  /// The configuration, read again when `proxy.toml` changed (a token
  /// added while it runs counts at once).
  fn config(&self) -> Config {
    let mut s = self.state.lock().expect("proxy state");
    let now = mtime(&self.dir);
    if now != s.mtime {
      match Config::load(&self.dir) {
        Ok(c) => s.config = c,
        Err(e) => eprintln!("proxy: {e} (keeping the configuration read before)"),
      }
      s.mtime = now;
    }
    s.config.clone()
  }

  /// Takes a request from the token's rate; `Err` the seconds to wait.
  fn take_rate(&self, token: &str, per_minute: u32) -> Result<(), u64> {
    let mut s = self.state.lock().expect("proxy state");
    let w = s.windows.entry(token.to_owned()).or_default();
    let now = Instant::now();
    while w
      .front()
      .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
    {
      w.pop_front();
    }
    if w.len() >= per_minute as usize {
      let wait = 60 - now.duration_since(w[0]).as_secs();
      return Err(wait.max(1));
    }
    w.push_back(now);
    Ok(())
  }

  /// Today's budget tokens of a token, and of one of its end users.
  fn spent(&self, token: &str, user: &str) -> (u64, u64) {
    let s = self.state.lock().expect("proxy state");
    match s.book.day(&usage::today(), token) {
      Some(d) => (
        d.total.budget(),
        d.users.get(user).map(Counts::budget).unwrap_or(0),
      ),
      None => (0, 0),
    }
  }

  fn record(&self, line: usage::Line) {
    {
      let mut s = self.state.lock().expect("proxy state");
      s.book
        .record(&usage::today(), &line.token, &line.user, &line.counts);
      if let Err(e) = s.book.save(&self.dir) {
        eprintln!("proxy: saving usage: {e}");
      }
    }
    eprintln!(
      "proxy: {} {} {} {} {} → {} in {} ms: {} in, {} out, {} cache read, {} cache write{}",
      line.token,
      line.user,
      line.thread.as_deref().unwrap_or("-"),
      line.upstream,
      line.path,
      line.status,
      line.ms,
      line.counts.input,
      line.counts.output,
      line.counts.cache_read,
      line.counts.cache_write,
      if line.cut {
        " (cut short by the client)"
      } else {
        ""
      },
    );
    usage::log(&self.dir, &line);
  }
}

fn full(b: impl Into<Bytes>) -> Body {
  Full::new(b.into()).map_err(|e| match e {}).boxed_unsync()
}

/// An error both Anthropic's and OpenAI's clients read (`error.message`).
fn error(status: StatusCode, kind: &str, message: &str) -> Response<Body> {
  let body = json!({"type": "error", "error": {"type": kind, "message": message}});
  Response::builder()
    .status(status)
    .header("content-type", "application/json")
    .body(full(body.to_string()))
    .expect("a response")
}

/// Whether `kind`'s API has this endpoint, and whether its usage counts.
fn endpoint(kind: &str, method: &Method, rest: &str) -> Option<bool> {
  let model_path = |prefix: &str| {
    rest == prefix
      || rest
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('/'))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
  };
  match (kind, method.as_str()) {
    ("anthropic", "POST") if rest == "v1/messages" => Some(true),
    ("anthropic", "POST") if rest == "v1/messages/count_tokens" => Some(false),
    ("anthropic", "GET") if model_path("v1/models") => Some(false),
    ("openai", "POST") if rest == "chat/completions" || rest == "responses" => Some(true),
    ("openai", "GET") if model_path("models") => Some(false),
    _ => None,
  }
}

/// What a client says of itself in a header: printable, short.
fn label(h: &hyper::HeaderMap, name: &str) -> Option<String> {
  let v = h.get(name)?.to_str().ok()?.trim();
  (!v.is_empty()).then(|| {
    v.chars()
      .filter(|c| !c.is_control())
      .take(100)
      .collect::<String>()
  })
}

fn bearer(h: &hyper::HeaderMap) -> Option<String> {
  if let Some(k) = h.get("x-api-key").and_then(|v| v.to_str().ok()) {
    return Some(k.trim().to_owned());
  }
  let a = h.get("authorization")?.to_str().ok()?;
  a.strip_prefix("Bearer ")
    .or_else(|| a.strip_prefix("bearer "))
    .map(|t| t.trim().to_owned())
}

/// Headers not passed upstream: hop-by-hop ones, the client's
/// credentials and attribution, and `accept-encoding` (the body is read
/// as it passes, to count its usage).
fn forwarded(name: &str) -> bool {
  !matches!(
    name,
    "host"
      | "authorization"
      | "x-api-key"
      | "content-length"
      | "connection"
      | "keep-alive"
      | "transfer-encoding"
      | "te"
      | "trailer"
      | "upgrade"
      | "proxy-authorization"
      | "proxy-connection"
      | "accept-encoding"
  ) && !name.starts_with("x-stitch-")
}

async fn handle(proxy: Arc<Proxy>, req: hyper::Request<Incoming>) -> Response<Body> {
  let started = Instant::now();
  let path = req.uri().path().trim_start_matches('/').to_owned();
  if path == "healthz" {
    return Response::new(full("ok\n"));
  }
  let config = proxy.config();
  let upstreams = config.upstreams();
  let (name, rest) = path.split_once('/').unwrap_or((path.as_str(), ""));
  let Some(upstream) = upstreams.get(name) else {
    return error(
      StatusCode::NOT_FOUND,
      "not_found_error",
      &format!(
        "no upstream `{name}`: requests are /<upstream>/<path>, upstreams {}",
        upstreams.keys().cloned().collect::<Vec<_>>().join(", ")
      ),
    );
  };
  let Some((token, t)) =
    bearer(req.headers()).and_then(|c| config.token_of(&c).map(|(n, t)| (n.clone(), t.clone())))
  else {
    return error(
      StatusCode::UNAUTHORIZED,
      "authentication_error",
      "not a token of this proxy (`stitch proxy token add` makes one)",
    );
  };
  if !t.upstreams.is_empty() && !t.upstreams.iter().any(|u| u == name) {
    return error(
      StatusCode::FORBIDDEN,
      "permission_error",
      &format!("token `{token}` may not use upstream `{name}`"),
    );
  }
  let Some(metered) = endpoint(&upstream.kind, req.method(), rest) else {
    return error(
      StatusCode::NOT_FOUND,
      "not_found_error",
      &format!(
        "{} /{rest} is not a model endpoint this proxy forwards",
        req.method()
      ),
    );
  };
  let user = label(req.headers(), END_USER).unwrap_or_else(|| usage::NOBODY.to_owned());
  let thread = label(req.headers(), THREAD);
  if let Err(wait) = proxy.take_rate(&token, t.requests_per_minute.unwrap_or(config::DEFAULT_RPM)) {
    let mut r = error(
      StatusCode::TOO_MANY_REQUESTS,
      "rate_limit_error",
      &format!("token `{token}` is over its requests a minute; again in {wait} s"),
    );
    r.headers_mut()
      .insert("retry-after", wait.to_string().parse().expect("digits"));
    return r;
  }
  if metered {
    let (all, theirs) = proxy.spent(&token, &user);
    let resets = usage::secs_to_midnight() / 60;
    // Not 429: a client retries those, and a spent budget does not
    // come back for hours.
    if let Some(max) = t.daily_tokens.filter(|m| all >= *m) {
      return error(
        StatusCode::FORBIDDEN,
        "permission_error",
        &format!(
          "today's token budget ({max}) is spent; it starts again at 00:00 UTC, in {}h{:02}",
          resets / 60,
          resets % 60
        ),
      );
    }
    if user != usage::NOBODY
      && let Some(max) = t.daily_tokens_per_user.filter(|m| theirs >= *m)
    {
      return error(
        StatusCode::FORBIDDEN,
        "permission_error",
        &format!(
          "your share of today's token budget ({max}) is spent; it starts again at 00:00 UTC, in {}h{:02}",
          resets / 60,
          resets % 60
        ),
      );
    }
  }
  let secret = upstream.key.clone().unwrap_or_else(|| name.to_owned());
  let Some(key) = (proxy.keys)(&secret, &upstream.kind).filter(|k| !k.is_empty()) else {
    return error(
      StatusCode::BAD_GATEWAY,
      "api_error",
      &format!("the proxy holds no API key `{secret}` for upstream `{name}`"),
    );
  };
  let mut url = format!("{}/{rest}", upstream.base_url.trim_end_matches('/'));
  if let Some(q) = req.uri().query() {
    url.push('?');
    url.push_str(q);
  }
  let method = req.method().clone();
  let mut headers = reqwest::header::HeaderMap::new();
  for (k, v) in req.headers() {
    if forwarded(k.as_str()) {
      headers.append(k.clone(), v.clone());
    }
  }
  match upstream.kind.as_str() {
    "anthropic" => {
      headers.insert("x-api-key", key.parse().expect("a key header"));
    }
    _ => {
      headers.insert(
        "authorization",
        format!("Bearer {key}").parse().expect("a key header"),
      );
    }
  }
  let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
    Ok(b) => b.to_bytes(),
    Err(e) => {
      return error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "invalid_request_error",
        &format!("the request body: {e}"),
      );
    }
  };
  let model = serde_json::from_slice::<Value>(&body)
    .ok()
    .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_owned));
  let sent = proxy
    .client
    .request(method, &url)
    .headers(headers)
    .body(body)
    .send()
    .await;
  let r = match sent {
    Ok(r) => r,
    Err(e) => {
      return error(
        StatusCode::BAD_GATEWAY,
        "api_error",
        &format!("upstream `{name}`: {e}"),
      );
    }
  };
  let status = r.status().as_u16();
  let content_type = r
    .headers()
    .get("content-type")
    .and_then(|v| v.to_str().ok())
    .unwrap_or("")
    .to_owned();
  let mut out = Response::builder().status(status);
  for (k, v) in r.headers() {
    let n = k.as_str();
    if !matches!(
      n,
      "content-length" | "connection" | "keep-alive" | "transfer-encoding" | "content-encoding"
    ) {
      out = out.header(k.clone(), v.clone());
    }
  }
  let line = usage::Line {
    at: jiff::Timestamp::now().to_string(),
    token,
    user,
    thread,
    upstream: name.to_owned(),
    path: format!("/{rest}"),
    model,
    status,
    ms: 0,
    cut: false,
    counts: Counts::default(),
  };
  let metered = Metered {
    inner: Box::pin(r.bytes_stream()),
    meter: Some(Meter::for_content_type(&content_type)),
    done: Some(Box::new({
      let proxy = proxy.clone();
      move |counts, cut| {
        let mut line = line;
        line.counts = counts;
        line.cut = cut;
        line.ms = started.elapsed().as_millis() as u64;
        proxy.record(line);
      }
    })),
  };
  out
    .body(BodyExt::boxed_unsync(StreamBody::new(metered)))
    .unwrap_or_else(|e| error(StatusCode::BAD_GATEWAY, "api_error", &e.to_string()))
}

type Done = Box<dyn FnOnce(Counts, bool) + Send>;

/// The upstream's body, passed on chunk by chunk as it arrives and
/// metered on the way; its usage recorded when it ends, or when the
/// client goes away first (`cut`).
struct Metered {
  inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
  meter: Option<Meter>,
  done: Option<Done>,
}

impl Metered {
  fn finish(&mut self, cut: bool) {
    if let (Some(m), Some(done)) = (self.meter.take(), self.done.take()) {
      done(m.finish(), cut);
    }
  }
}

impl Stream for Metered {
  type Item = Result<Frame<Bytes>, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    match self.inner.poll_next_unpin(cx) {
      Poll::Ready(Some(Ok(b))) => {
        if let Some(m) = &mut self.meter {
          m.feed(&b);
        }
        Poll::Ready(Some(Ok(Frame::data(b))))
      }
      Poll::Ready(Some(Err(e))) => {
        self.finish(true);
        Poll::Ready(Some(Err(std::io::Error::other(e))))
      }
      Poll::Ready(None) => {
        self.finish(false);
        Poll::Ready(None)
      }
      Poll::Pending => Poll::Pending,
    }
  }
}

impl Drop for Metered {
  fn drop(&mut self) {
    self.finish(true);
  }
}

/// Binds what it can of `addrs` (an address this machine lacks, such as
/// Docker's bridge before Docker runs, is skipped with a note); fails
/// when none binds.
pub async fn bind(addrs: &[String]) -> Result<Vec<TcpListener>, String> {
  let mut out = Vec::new();
  let mut failed = Vec::new();
  for a in addrs {
    let addr: SocketAddr = a
      .parse()
      .map_err(|e| format!("listen address `{a}`: {e}"))?;
    match TcpListener::bind(addr).await {
      Ok(l) => out.push(l),
      Err(e) => failed.push(format!("{a}: {e}")),
    }
  }
  for f in &failed {
    eprintln!("proxy: not listening on {f}");
  }
  if out.is_empty() {
    return Err(format!("could not listen anywhere: {}", failed.join("; ")));
  }
  Ok(out)
}

async fn accept(proxy: Arc<Proxy>, l: TcpListener) {
  if let Ok(a) = l.local_addr() {
    eprintln!("proxy: listening on http://{a}");
  }
  loop {
    let Ok((stream, _)) = l.accept().await else {
      continue;
    };
    let proxy = proxy.clone();
    tokio::spawn(async move {
      let svc = service_fn(move |req| {
        let proxy = proxy.clone();
        async move { Ok::<_, Infallible>(handle(proxy, req).await) }
      });
      let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), svc)
        .await;
    });
  }
}

/// Serves on `listeners` until the task is dropped.
pub async fn serve(proxy: Arc<Proxy>, listeners: Vec<TcpListener>) {
  let tasks: Vec<_> = listeners
    .into_iter()
    .map(|l| tokio::spawn(accept(proxy.clone(), l)))
    .collect();
  for t in tasks {
    let _ = t.await;
  }
}

/// Serves on what binds of `addrs`, trying the others again every 30
/// seconds (Docker's bridge appears when Docker starts, which may be
/// after the proxy at boot); fails when none binds at first.
pub async fn run(proxy: Arc<Proxy>, addrs: &[String]) -> Result<(), String> {
  let mut pending = Vec::new();
  let mut tasks = Vec::new();
  for a in addrs {
    let addr: SocketAddr = a
      .parse()
      .map_err(|e| format!("listen address `{a}`: {e}"))?;
    match TcpListener::bind(addr).await {
      Ok(l) => tasks.push(tokio::spawn(accept(proxy.clone(), l))),
      Err(e) => {
        eprintln!("proxy: not listening on {a} yet ({e}): trying again every 30 s");
        pending.push(addr);
      }
    }
  }
  if tasks.is_empty() {
    return Err(format!("could not listen on any of {}", addrs.join(", ")));
  }
  while !pending.is_empty() {
    tokio::time::sleep(Duration::from_secs(30)).await;
    let mut still = Vec::new();
    for addr in pending {
      match TcpListener::bind(addr).await {
        Ok(l) => tasks.push(tokio::spawn(accept(proxy.clone(), l))),
        Err(_) => still.push(addr),
      }
    }
    pending = still;
  }
  for t in tasks {
    let _ = t.await;
  }
  Ok(())
}

/// Adds a token named `name` (replacing one of that name) and returns
/// its clear text, which is not kept.
pub fn add_token(dir: &Path, name: &str, mut token: config::Token) -> Result<String, String> {
  if name.is_empty()
    || !name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
  {
    return Err(format!(
      "`{name}`: a token's name is letters, digits, `-` and `_`"
    ));
  }
  let mut c = Config::load(dir)?;
  let clear = config::new_token();
  token.hash = config::hash(&clear);
  token.created = Some(jiff::Timestamp::now().to_string());
  c.tokens.insert(name.to_owned(), token);
  c.save(dir)?;
  Ok(clear)
}

pub fn remove_token(dir: &Path, name: &str) -> Result<(), String> {
  let mut c = Config::load(dir)?;
  if c.tokens.remove(name).is_none() {
    return Err(format!("no token `{name}`"));
  }
  c.save(dir)
}

/// Lines saying each token's limits and what it and its users used on
/// the last `days` days.
pub fn report(dir: &Path, days: usize) -> Result<Vec<String>, String> {
  let c = Config::load(dir)?;
  let book = Book::load(dir);
  let mut out = Vec::new();
  for (name, t) in &c.tokens {
    let limit = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "none".into());
    out.push(format!(
      "{name}: daily {} tokens, {} per user, {} requests a minute, upstreams {}",
      limit(t.daily_tokens),
      limit(t.daily_tokens_per_user),
      t.requests_per_minute.unwrap_or(config::DEFAULT_RPM),
      if t.upstreams.is_empty() {
        "all".into()
      } else {
        t.upstreams.join(", ")
      },
    ));
  }
  for (day, tokens) in book.days.iter().rev().take(days) {
    for (token, d) in tokens {
      let c = &d.total;
      out.push(format!(
        "{day} {token}: {} requests, {} budget tokens ({} in, {} out, {} cache read, {} cache write)",
        c.requests,
        c.budget(),
        c.input,
        c.output,
        c.cache_read,
        c.cache_write
      ));
      for (user, u) in &d.users {
        out.push(format!(
          "  {user}: {} requests, {} budget tokens",
          u.requests,
          u.budget()
        ));
      }
    }
  }
  Ok(out)
}

#[cfg(test)]
mod tests;
