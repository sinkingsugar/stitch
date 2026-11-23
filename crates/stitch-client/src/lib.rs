//! A client of a Stitch node's events (docs/events.md in the Stitch
//! repository), for programs that are not Stitch: publish an event that
//! wakes the threads subscribed to its topic, or ask them something and
//! wait for the answer until a deadline.
//!
//! It speaks the node's stable events protocol on its Unix socket
//! (`events/1`), so it keeps working across the node's own versions:
//!
//! 1. The client sends one frame of the node's own protocol, `Hello` with
//!    the version `events/1`: a big-endian `u32` length, then the postcard
//!    bytes `00 08 "events/1"`.
//! 2. From then on, both ways, frames of JSON (the same length prefix).
//!    The node first says `{"hello": "events/1", "node": "<name>",
//!    "version": "<its version>"}` (or `{"error": "…"}` and closes).
//! 3. Requests, one at a time, each answered by one frame:
//!    - `{"op": "event", "topic": "a/b", "payload": <JSON>, "ask":
//!      {"deadline_ms": 20000, "schema": <JSON Schema>}, "wait": true}`
//!      (`ask` and `schema` optional; `wait` defaults to true): without
//!      an ask the reply comes at once (`published`); with one and `wait`,
//!      when it is answered, expired, dropped or unheard.
//!    - `{"op": "status", "id": "ev-…", "wait_ms": 5000}`: an event's
//!      state, waiting up to `wait_ms` for a pending ask to change.
//!    - The reply: `{"id", "state", "matched", "answer", "by", "note"}`,
//!      or `{"error": "…"}`. `state` is `published`, `pending`,
//!      `answered` (`answer` the thread's JSON, `null` included; `by` its
//!      ID), `expired`, `dropped` (no subscription woke a thread with it:
//!      filters said nothing, or a limit held it), `unheard` (nothing
//!      subscribes) or `overloaded` (the node's quotas are full: nothing
//!      was kept or routed, `id` is empty; publish again later).
//! 4. Limits: a request frame is at most 84 KB (a 64 KB payload, a 16 KB
//!    schema) and, once begun, read within 10 s; at most 64 connections
//!    at once (past them the node says `{"error": "overloaded: …"}` and
//!    closes). A frame too large, or not JSON-framed, is answered with an
//!    error and the connection closed.
//!
//! A `Client` that met any error after sending a request (a timeout, a
//! broken connection, an unreadable reply) is closed: it cannot tell a
//! late reply from the next one's, so every later call fails at once
//! (`Error::Io` of kind `NotConnected`) until `reconnect`. A refusal
//! (`Error::Refused`) leaves it open.
//!
//! Every ask needs the caller's own fallback: an answer only improves on
//! it. A node that is not running is an error (`Error::NotRunning`), never
//! a wait.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

/// The protocol this crate speaks.
pub const PROTOCOL: &str = "events/1";

/// The longest frame read.
const MAX_FRAME: usize = 16 << 20;

#[derive(Debug)]
pub enum Error {
  /// No node listens on the socket (it is not running).
  NotRunning(PathBuf, std::io::Error),
  Io(std::io::Error),
  /// The node said something this client cannot read (one too old for
  /// events, or not a Stitch node).
  Protocol(String),
  /// The node refused the request (a bad topic, a payload too large…).
  Refused(String),
}

impl std::fmt::Display for Error {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Error::NotRunning(p, e) => write!(f, "no Stitch node at {}: {e}", p.display()),
      Error::Io(e) => write!(f, "node socket: {e}"),
      Error::Protocol(m) => write!(f, "node protocol: {m}"),
      Error::Refused(m) => write!(f, "refused: {m}"),
    }
  }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
  fn from(e: std::io::Error) -> Error {
    Error::Io(e)
  }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What became of an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
  /// Kept and routed (no ask).
  Published,
  /// An ask not answered yet.
  Pending,
  Answered,
  Expired,
  /// An ask no subscription woke a thread with.
  Dropped,
  /// An ask nothing subscribes to.
  Unheard,
  /// The node's quotas are full: nothing was kept or routed.
  Overloaded,
  Other(String),
}

impl State {
  fn of(s: &str) -> State {
    match s {
      "published" => State::Published,
      "pending" => State::Pending,
      "answered" => State::Answered,
      "expired" => State::Expired,
      "dropped" => State::Dropped,
      "unheard" => State::Unheard,
      "overloaded" => State::Overloaded,
      o => State::Other(o.to_owned()),
    }
  }
}

/// An event as the node answers for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
  pub id: String,
  pub state: State,
  /// The subscriptions its topic matched.
  pub matched: u32,
  /// The answer (`Answered`; `Some(Value::Null)` when the thread
  /// answered `null`).
  pub answer: Option<Value>,
  /// The thread that answered.
  pub by: Option<String>,
  /// Why (`Dropped`, `Unheard`, `Expired`).
  pub note: Option<String>,
}

impl Event {
  /// The answer as `T`, when there is one that reads as `T`.
  pub fn answer_as<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
    self
      .answer
      .clone()
      .and_then(|a| serde_json::from_value(a).ok())
  }
}

/// An ask's options.
#[derive(Clone, Debug)]
pub struct Ask {
  /// How long the publisher waits; past it the ask is expired and the
  /// publisher uses its fallback.
  pub deadline: Duration,
  /// A JSON Schema the answer must match (the node refuses others).
  pub schema: Option<Value>,
}

/// A connection to a node's events.
pub struct Client {
  stream: UnixStream,
  socket: PathBuf,
  /// Closed after an error mid-request: its stream may hold a late reply.
  closed: bool,
  node: String,
  version: String,
  timeout: Duration,
}

/// The node's socket as `stitch` finds it: `$STITCH_SOCKET`, else
/// `node.sock` in `$STITCH_DATA` or `~/.stitch`.
pub fn default_socket() -> PathBuf {
  if let Some(p) = std::env::var_os("STITCH_SOCKET") {
    return PathBuf::from(p);
  }
  let data = std::env::var_os("STITCH_DATA")
    .map(PathBuf::from)
    .unwrap_or_else(|| {
      std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".stitch")
    });
  data.join("node.sock")
}

impl Client {
  /// Connects to the node at `default_socket()`.
  pub fn connect() -> Result<Client> {
    Client::connect_to(default_socket())
  }

  /// Connects to the node listening at `socket`.
  pub fn connect_to(socket: impl AsRef<Path>) -> Result<Client> {
    let path = socket.as_ref();
    let stream = UnixStream::connect(path).map_err(|e| Error::NotRunning(path.to_path_buf(), e))?;
    let mut c = Client {
      stream,
      socket: path.to_path_buf(),
      closed: false,
      node: String::new(),
      version: String::new(),
      timeout: Duration::from_secs(10),
    };
    c.stream.set_write_timeout(Some(c.timeout))?;
    // `Request::Hello { version: "events/1" }` in postcard: variant 0, the
    // string's length, its bytes.
    let mut hello = vec![0u8, PROTOCOL.len() as u8];
    hello.extend_from_slice(PROTOCOL.as_bytes());
    c.write_frame(&hello)?;
    let said = c.read_json(Some(c.timeout))?;
    if let Some(e) = said.get("error").and_then(Value::as_str) {
      return Err(Error::Refused(e.to_owned()));
    }
    if said.get("hello").and_then(Value::as_str) != Some(PROTOCOL) {
      return Err(Error::Protocol(format!("unexpected hello: {said}")));
    }
    c.node = said["node"].as_str().unwrap_or("").to_owned();
    c.version = said["version"].as_str().unwrap_or("").to_owned();
    Ok(c)
  }

  /// Whether an earlier error closed it (`reconnect` opens it again).
  pub fn is_closed(&self) -> bool {
    self.closed
  }

  /// Connects again to the same socket, after an error closed it (or to
  /// pick up a node that restarted).
  pub fn reconnect(&mut self) -> Result<()> {
    let mut fresh = Client::connect_to(&self.socket)?;
    fresh.timeout = self.timeout;
    *self = fresh;
    Ok(())
  }

  /// The node's name.
  pub fn node(&self) -> &str {
    &self.node
  }

  /// The node's version (its build's protocol, for logs).
  pub fn version(&self) -> &str {
    &self.version
  }

  /// How long a request without an ask waits for its reply (default 10 s).
  pub fn set_timeout(&mut self, timeout: Duration) {
    self.timeout = timeout;
  }

  /// Publishes an event; it returns once the node kept and routed it.
  pub fn publish(&mut self, topic: &str, payload: &Value) -> Result<Event> {
    self.request(
      &json!({"op": "event", "topic": topic, "payload": payload}),
      self.timeout,
    )
  }

  /// Publishes an ask and waits for its answer until its deadline (a
  /// little after, for the node's own word): `Answered` with the answer,
  /// else `Expired`, `Dropped` or `Unheard` (use the fallback).
  pub fn ask(&mut self, topic: &str, payload: &Value, ask: &Ask) -> Result<Event> {
    let wait = ask.deadline + self.timeout;
    self.request(&ask_request(topic, payload, ask, true), wait)
  }

  /// Publishes an ask and returns at once (`Pending`, with its id), for
  /// `status` to follow.
  pub fn ask_later(&mut self, topic: &str, payload: &Value, ask: &Ask) -> Result<Event> {
    self.request(&ask_request(topic, payload, ask, false), self.timeout)
  }

  /// An event's state, waiting up to `wait` for a pending ask to change.
  pub fn status(&mut self, id: &str, wait: Duration) -> Result<Event> {
    self.request(
      &json!({"op": "status", "id": id, "wait_ms": wait.as_millis() as u64}),
      wait + self.timeout,
    )
  }

  fn request(&mut self, req: &Value, wait: Duration) -> Result<Event> {
    if self.closed {
      return Err(Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "closed after an earlier error: reconnect",
      )));
    }
    let bytes = serde_json::to_vec(req).map_err(|e| Error::Protocol(e.to_string()))?;
    let v = match self
      .write_frame(&bytes)
      .and_then(|()| self.read_json(Some(wait)))
    {
      Ok(v) => v,
      Err(e) => {
        // Its reply may still come: nothing on this stream can be trusted
        // to answer the next request.
        self.closed = true;
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        return Err(e);
      }
    };
    if let Some(e) = v.get("error").and_then(Value::as_str) {
      return Err(Error::Refused(e.to_owned()));
    }
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let state = State::of(&s("state").unwrap_or_default());
    let answer = match (&state, v.get("answer")) {
      // A thread may answer `null`: still an answer.
      (State::Answered, a) => Some(a.cloned().unwrap_or(Value::Null)),
      (_, a) => a.filter(|a| !a.is_null()).cloned(),
    };
    Ok(Event {
      id: s("id").ok_or_else(|| {
        self.closed = true;
        Error::Protocol(format!("no id in {v}"))
      })?,
      state,
      matched: v.get("matched").and_then(Value::as_u64).unwrap_or(0) as u32,
      answer,
      by: s("by"),
      note: s("note"),
    })
  }

  fn write_frame(&mut self, bytes: &[u8]) -> Result<()> {
    self.stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    self.stream.write_all(bytes)?;
    self.stream.flush()?;
    Ok(())
  }

  fn read_json(&mut self, wait: Option<Duration>) -> Result<Value> {
    self.stream.set_read_timeout(wait)?;
    let mut len = [0u8; 4];
    self.stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
      return Err(Error::Protocol(format!("a frame of {len} bytes")));
    }
    let mut buf = vec![0u8; len];
    self.stream.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
      .map_err(|_| Error::Protocol("the node does not speak the events protocol (too old?)".into()))
  }
}

fn ask_request(topic: &str, payload: &Value, ask: &Ask, wait: bool) -> Value {
  let mut a = json!({"deadline_ms": ask.deadline.as_millis() as u64});
  if let Some(s) = &ask.schema {
    a["schema"] = s.clone();
  }
  json!({"op": "event", "topic": topic, "payload": payload, "ask": a, "wait": wait})
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn states_read() {
    assert_eq!(State::of("answered"), State::Answered);
    assert_eq!(State::of("x"), State::Other("x".into()));
  }

  /// A fake node: says hello, then answers each request with what
  /// `answer` gives (`None`: never answers).
  fn fake(answer: fn(&Value) -> Option<Value>) -> (PathBuf, std::thread::JoinHandle<()>) {
    let dir = std::env::temp_dir().join(format!(
      "stitch-client-fake-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("node.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let h = std::thread::spawn(move || {
      for conn in listener.incoming().take(2) {
        let mut c = conn.unwrap();
        let read = |c: &mut UnixStream| -> Option<Vec<u8>> {
          let mut len = [0u8; 4];
          c.read_exact(&mut len).ok()?;
          let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
          c.read_exact(&mut buf).ok()?;
          Some(buf)
        };
        let write = |c: &mut UnixStream, v: &Value| {
          let b = serde_json::to_vec(v).unwrap();
          let _ = c.write_all(&(b.len() as u32).to_be_bytes());
          let _ = c.write_all(&b);
        };
        read(&mut c).unwrap();
        write(
          &mut c,
          &json!({"hello": PROTOCOL, "node": "fake", "version": "x"}),
        );
        while let Some(req) = read(&mut c) {
          let req: Value = serde_json::from_slice(&req).unwrap();
          if let Some(v) = answer(&req) {
            write(&mut c, &v);
          }
        }
      }
    });
    (path, h)
  }

  #[test]
  fn a_null_answer_is_an_answer() {
    let (path, _h) = fake(|_| {
      Some(json!({"id": "ev-1", "state": "answered", "matched": 1, "answer": null, "by": "t"}))
    });
    let mut c = Client::connect_to(&path).unwrap();
    let e = c
      .ask(
        "a/b",
        &json!({}),
        &Ask {
          deadline: Duration::from_millis(100),
          schema: None,
        },
      )
      .unwrap();
    assert_eq!(e.state, State::Answered);
    assert_eq!(e.answer, Some(Value::Null));
  }

  #[test]
  fn a_timeout_closes_the_client_until_it_reconnects() {
    // The first request is never answered; the second (on a new
    // connection) is.
    let (path, _h) = fake(|req| {
      (req["topic"] == "second").then(|| json!({"id": "ev-2", "state": "published", "matched": 0}))
    });
    let mut c = Client::connect_to(&path).unwrap();
    c.set_timeout(Duration::from_millis(200));
    assert!(matches!(c.publish("first", &json!(1)), Err(Error::Io(_))));
    assert!(c.is_closed());
    // Refused at once, not read from the stream a late reply may be on.
    let started = std::time::Instant::now();
    match c.publish("second", &json!(2)) {
      Err(Error::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotConnected),
      other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < Duration::from_millis(100));
    c.reconnect().unwrap();
    assert!(!c.is_closed());
    let e = c.publish("second", &json!(2)).unwrap();
    assert_eq!((e.id.as_str(), e.state), ("ev-2", State::Published));
  }

  #[test]
  fn overloaded_reads() {
    assert_eq!(State::of("overloaded"), State::Overloaded);
  }

  #[test]
  fn a_missing_node_is_not_running() {
    let dir = std::env::temp_dir().join(format!("stitch-client-{}", std::process::id()));
    let r = Client::connect_to(dir.join("node.sock"));
    assert!(matches!(r, Err(Error::NotRunning(..))));
  }
}
