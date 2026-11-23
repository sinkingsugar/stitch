//! The web client's WebSocket (design §2.24): `/_stitch/ws` on the apps
//! server, for the built-in client's origin alone.
//!
//! **Gate.** The upgrade passes what every request here passes (the Host
//! check and the cookie, in `http`), and a browser applies no CORS to a
//! WebSocket, so this adds the checks that protect it: an `Origin` that is
//! exactly the page's own, `Sec-Fetch-Site: same-origin` when sent, and
//! that origin must be the pin of the built-in client (`builtin =
//! "client"`): a page `show_app` made, or any other app, is refused.
//!
//! **Protocol.** JSON text frames. The node's own connection handler
//! (`node::serve`, the Unix socket's) serves the other end of an in-process
//! pipe, so a request is handled by the same code; this module only turns
//! JSON into the socket's postcard frames and back.
//!
//! Client to node:
//! - `{"id": N, "request": <api::Request>}`: the first must be
//!   `{"Hello": {"version": VERSION}}`, an exact match as on the socket;
//! - `{"follow": {"thread": T, "after": N|null}}` and `{"unfollow":
//!   {"thread": T}}`: stream the live events of a thread's turns here.
//!
//! Node to client:
//! - `{"id": N, "reply": <api::Reply>}`: a reply of request N;
//! - `{"id": N|null, "error": "…"}`: a frame the node could not use (`id`
//!   null when the frame had none);
//! - `{"event": {"thread": T, "event": <live::LiveEvent>}}`: one event a
//!   followed thread's turn streamed (`"Started"`, `{"Text": "…"}`,
//!   `{"ToolCall": "$ ls"}`, `{"ToolResult": {"ok", "output"}}`, `"Ended"`,
//!   …; consecutive text is joined); `{"event": {"thread": T, "gap":
//!   true}}`: some of what happened was not kept, read the transcript
//!   again; `{"event": {"thread": T, "turn": {"running": bool, "queued":
//!   N}}}`: where the thread's turns stand here (`Live::status`), sent when
//!   the follow starts and on each change: a loop running, and `N` turns
//!   accepted whose loop has not begun (a message sent starts one at once,
//!   before its preamble, or waits for the loop running). Following starts
//!   from now; read the transcript (`Request::Transcript`) for before. A
//!   thread compacted into another is followed as its newest (design
//!   §2.15): events are said under the thread asked for; a compaction while
//!   followed comes as `{"Compacted": {from, to, model, window, tokens,
//!   kept, summary}}` (`api::Compaction`), then the new thread's turn
//!   streams on (the client is expected to switch to `to`);
//! - `{"changed": true}`: threads, jobs, approvals or schedules changed on
//!   the node; list again (`Request::Threads`). At most one per
//!   `CHANGED_EVERY` on a connection: changes in between come as one push
//!   after it (a trailing edge, so the last change is never lost).
//!
//! **Backpressure.** A connection runs at most `RUNNING` of its `One`-reply
//! reads at once (`gated`: lists, transcripts, live data; what the user
//! does, a `Send`, an `Answer`, a `Cancel`, runs at once, never behind a
//! burst of refreshes, bounded on its own by `ACTIONS`); up to `QUEUED`
//! more reads wait for a slot (as futures, not threads), and one past that
//! is refused at once with
//! `{"id": N, "error": "busy: …"}`: the client is expected to coalesce its
//! refreshes (the web client does), so a refusal means a client that
//! floods. Chats and shell attachments are long-lived and not counted. The
//! node also bounds the list reads of all connections together
//! (`Shared::list_read`).
//!
//! **Replies.** Each request that has replies coming runs on a lane of its
//! own: a connection to the node's handler with its own pipe, so every frame
//! it produces carries that request's `id` by construction, however many are
//! in flight. One reply for most requests; a `Chat`/`Resume` streams `Reply`s
//! of its own (`Thread`, `Text`, … `Approve`, …) until `ChatDone` (or the
//! `Error` that ends it); a `ShellAttach` streams `ShellOutput` until
//! `ShellEnded`. The requests the socket answers with nothing (`Interject`,
//! `Retract`, `Interrupt`, `Suspend`, `ShellInput`, `Approval`,
//! `BrowserLink`) produce nothing: they are written to the lane they act on
//! (an `Approval` to the chat that asked it; the others to the chat named by
//! an optional `"to": <its request id>` beside `request`, else the newest
//! chat running here; `ShellInput` to the newest shell attached).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use super::Host;
use super::live::is_client;
use super::serve::Response;
use crate::api::{self, Reply, Request};
use crate::live::LiveEvent;
use crate::node::Shared;

/// The path the upgrade is at.
pub(crate) const PATH: &str = "/_stitch/ws";
/// Requests with one reply a connection runs at once.
const RUNNING: usize = 8;
/// More of them that may wait for a slot; past this they are refused.
const QUEUED: usize = 64;
/// The user's actions (requests not `gated`) a connection runs at once;
/// past this they are refused.
const ACTIONS: usize = 32;
/// The least time between two `changed` pushes on a connection.
pub(crate) const CHANGED_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// Largest message taken or sent.
const MAX_MESSAGE: usize = 32 << 20;

fn refused(why: &str) -> Response {
  Response::text(403, why)
}

/// Why the upgrade is not let through, if it is not: `host` is the Host
/// header (with its port), `name` it without; `origin` and `site` are the
/// `Origin` and `Sec-Fetch-Site` headers.
pub(crate) fn refusal(
  s: &Arc<Shared>,
  name: &str,
  host: &str,
  origin: Option<&str>,
  site: Option<&str>,
) -> Option<Response> {
  // A browser always sends Origin on a WebSocket; one that does not is no
  // browser page, and a cookie alone is not enough.
  if origin != Some(format!("http://{host}").as_str()) {
    return Some(refused("not from this app"));
  }
  if site.is_some_and(|f| f != "same-origin") {
    return Some(refused("not from this app"));
  }
  let Some(Host::Pinned(p)) = Host::parse(name) else {
    return Some(refused("the node's WebSocket is for the web client alone"));
  };
  let pinned = super::pinned_all(&s.state().index.lock().expect("index lock")).unwrap_or_default();
  if !pinned.iter().any(|a| a.host == p && is_client(a)) {
    return Some(refused("the node's WebSocket is for the web client alone"));
  }
  None
}

/// Whether `name` (a Host without its port) is the pin of the built-in
/// client: the one origin that may use the node's client-only endpoints.
pub(crate) fn is_client_host(s: &Arc<Shared>, name: &str) -> bool {
  let Some(Host::Pinned(p)) = Host::parse(name) else {
    return false;
  };
  super::pinned_all(&s.state().index.lock().expect("index lock"))
    .unwrap_or_default()
    .iter()
    .any(|a| a.host == p && is_client(a))
}

/// Answers the upgrade (the checks already passed) and serves the
/// connection once it is upgraded.
pub(crate) fn upgrade(
  s: &Arc<Shared>,
  req: &mut hyper::Request<hyper::body::Incoming>,
) -> Response {
  let h = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok());
  let wants = req.method() == hyper::Method::GET
    && h("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
    && h("connection").is_some_and(|c| c.to_ascii_lowercase().contains("upgrade"))
    && h("sec-websocket-version") == Some("13");
  let Some(key) = h("sec-websocket-key").filter(|_| wants).map(str::to_owned) else {
    return Response::text(400, "a WebSocket upgrade, version 13");
  };
  let on = hyper::upgrade::on(&mut *req);
  let s = s.clone();
  tokio::spawn(async move {
    if let Ok(upgraded) = on.await {
      let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
      let ws =
        WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config)).await;
      session(&s, ws).await;
    }
  });
  Response {
    status: 101,
    headers: vec![
      ("Upgrade".into(), "websocket".into()),
      ("Connection".into(), "Upgrade".into()),
      (
        "Sec-WebSocket-Accept".into(),
        derive_accept_key(key.as_bytes()),
      ),
    ],
    body: Vec::new(),
  }
}

#[derive(Deserialize)]
struct In {
  id: Option<u64>,
  request: Option<Request>,
  follow: Option<Follow>,
  unfollow: Option<Follow>,
  /// Deltas in place of `changed` (`feed`): the list, the live sections.
  watch: Option<Watch>,
  /// The threads whose running calls this connection hears.
  calls: Option<Calls>,
  /// For a request that answers nothing and acts on a running chat
  /// (`Interject`, `Retract`, `Interrupt`, `Suspend`): the `id` of the
  /// chat's request. Without it, the newest chat running here.
  to: Option<u64>,
}

#[derive(Deserialize)]
struct Follow {
  thread: String,
  #[serde(default)]
  after: Option<u64>,
}

#[derive(Deserialize)]
struct Watch {
  /// The list's window (`Request::Threads`' `limit`).
  #[serde(default)]
  limit: Option<u32>,
}

#[derive(Deserialize)]
struct Calls {
  threads: Vec<String>,
}

/// What the socket does with a request.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
  /// One reply.
  One,
  /// None: it acts on a lane that is running.
  Silent,
  /// A chat's stream, to `ChatDone` (or the error that ends it).
  Chat,
  /// A shell's output, to `ShellEnded`.
  Shell,
}

fn kind(r: &Request) -> Kind {
  match r {
    Request::Chat { .. } | Request::Resume { .. } => Kind::Chat,
    Request::ShellAttach { .. } => Kind::Shell,
    Request::Interject { .. }
    | Request::Retract { .. }
    | Request::Interrupt
    | Request::Suspend
    | Request::ShellInput(_)
    | Request::Approval { .. }
    | Request::BrowserLink { .. } => Kind::Silent,
    _ => Kind::One,
  }
}

/// The lanes running: each request that has replies coming has a lane of
/// its own, a connection to the node's handler (`node::serve`) with its own
/// pipe, so every frame it sends is tagged with that request's `id` by
/// construction, whatever else is in flight. The requests that answer
/// nothing are written to the lane they act on.
#[derive(Default)]
struct Lanes {
  /// Chats, oldest first: request id and what to write to them.
  chats: Vec<(u64, mpsc::UnboundedSender<Request>)>,
  /// Shell attachments, oldest first.
  shells: Vec<(u64, mpsc::UnboundedSender<Request>)>,
  /// The approvals asked and not answered: approval id, its lane.
  approvals: HashMap<u64, (u64, mpsc::UnboundedSender<Request>)>,
}

impl Lanes {
  fn end(&mut self, lane: u64) {
    self.chats.retain(|(i, _)| *i != lane);
    self.shells.retain(|(i, _)| *i != lane);
    self.approvals.retain(|_, (i, _)| *i != lane);
  }

  /// Where a request that answers nothing goes, if anywhere.
  fn target(&self, r: &Request, to: Option<u64>) -> Option<mpsc::UnboundedSender<Request>> {
    let newest = |v: &Vec<(u64, mpsc::UnboundedSender<Request>)>| {
      match to {
        Some(t) => v.iter().find(|(i, _)| *i == t),
        None => v.last(),
      }
      .map(|(_, tx)| tx.clone())
    };
    match r {
      Request::Approval { id, .. } => self
        .approvals
        .get(id)
        .map(|(_, tx)| tx.clone())
        .or_else(|| newest(&self.chats)),
      Request::ShellInput(_) => newest(&self.shells),
      Request::BrowserLink { .. } => None,
      _ => newest(&self.chats),
    }
  }
}

type Out = mpsc::UnboundedSender<Message>;

/// Serves one request on a lane of its own: a connection to the node's
/// handler, greeted, the request written, every reply tagged `id`. Ends
/// with the request's last reply; the connection closes with it.
async fn lane(
  s: Arc<Shared>,
  id: u64,
  first: Request,
  out: Out,
  lanes: Arc<Mutex<Lanes>>,
  mut more: mpsc::UnboundedReceiver<Request>,
) {
  let kind = kind(&first);
  let say_error = |why: String| {
    let _ = out.send(text(&json!({"id": id, "error": why})));
  };
  let (ours, theirs) = tokio::io::duplex(1 << 16);
  {
    let s = s.clone();
    tokio::spawn(async move {
      let (r, w) = tokio::io::split(theirs);
      if let Err(e) = crate::node::serve(&s, r, w, crate::peer::Peer::Web).await {
        eprintln!("web client: {e}");
      }
    });
  }
  let (mut from_node, mut to_node) = tokio::io::split(ours);
  let is_hello = matches!(first, Request::Hello { .. });
  if !is_hello {
    let hello = Request::Hello {
      version: crate::VERSION.into(),
    };
    let greeted = api::write(&mut to_node, &hello).await.is_ok()
      && matches!(
        api::read::<Reply, _>(&mut from_node).await,
        Ok(Some(Reply::Hello { .. }))
      );
    if !greeted {
      say_error("the node did not greet".into());
      lanes.lock().expect("lanes lock").end(id);
      return;
    }
  }
  if api::write(&mut to_node, &first).await.is_err() {
    say_error("the node closed the connection".into());
    lanes.lock().expect("lanes lock").end(id);
    return;
  }
  let mut shell_output = false;
  loop {
    let reply = tokio::select! {
      r = api::read::<Reply, _>(&mut from_node) => match r {
        Ok(Some(r)) => r,
        _ => {
          // The node closed this connection (a stop): a request waiting
          // on a reply hears it.
          if kind != Kind::Shell {
            say_error("the node closed the connection".into());
          }
          break;
        }
      },
      Some(r) = more.recv() => {
        if api::write(&mut to_node, &r).await.is_err() {
          break;
        }
        continue;
      }
    };
    if let Reply::Approve { id: a, .. } = &reply {
      let mut l = lanes.lock().expect("lanes lock");
      let tx = l
        .chats
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, tx)| tx.clone());
      if let Some(tx) = tx {
        l.approvals.insert(*a, (id, tx));
      }
    }
    if let Reply::Answered { id: a } = &reply {
      lanes.lock().expect("lanes lock").approvals.remove(a);
    }
    match serde_json::to_value(&reply) {
      Ok(r) => {
        let _ = out.send(text(&json!({"id": id, "reply": r})));
      }
      Err(e) => say_error(e.to_string()),
    }
    let done = match (&reply, kind) {
      (Reply::ChatDone { .. } | Reply::Error(_), Kind::Chat) => true,
      (Reply::ShellEnded, Kind::Shell) => true,
      (Reply::ShellOutput(_), Kind::Shell) => {
        shell_output = true;
        false
      }
      // An attach that failed answers once and nothing follows.
      (Reply::Error(_), Kind::Shell) => !shell_output,
      (_, Kind::One) => true,
      _ => false,
    };
    if done {
      break;
    }
  }
  lanes.lock().expect("lanes lock").end(id);
  if matches!(&first, Request::Hello { version } if version != crate::VERSION) {
    // A Hello of another version ends the connection with the node's
    // answer; the socket ends with it.
    let _ = out.send(Message::Close(None));
  }
}

/// A task that ends with the one holding it (a follow's status watch,
/// with the follow).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
  fn drop(&mut self) {
    self.0.abort();
  }
}

/// Whether a request with one reply waits for one of the connection's
/// `RUNNING` slots: the reads a client refreshes (lists, transcripts,
/// what runs), which can be slow and come in bursts. What the user does
/// (a message sent, an approval answered, a cancel, a rename…) runs at
/// once, never behind them; it still counts toward the `QUEUED` bound.
fn gated(r: &Request) -> bool {
  matches!(
    r,
    Request::Threads { .. }
      | Request::Transcript { .. }
      | Request::Artefacts { .. }
      | Request::Digest {
        update: None | Some(false),
        ..
      }
      | Request::Touched { .. }
      | Request::Live { .. }
      | Request::InFlight { .. }
      | Request::Search { .. }
      | Request::Status
      | Request::Jobs
      | Request::Apps
      | Request::AppThreads { .. }
      | Request::AppLive { .. }
      | Request::Dashboard
      | Request::RunsOn { .. }
      | Request::Runners
      | Request::Models { .. }
      | Request::EffectiveModel { .. }
      | Request::Instructions { .. }
      | Request::MemoryNotes
      | Request::LooseEnds { .. }
      // A room read; making or changing one (`RoomCreate`, `RoomUpdate`)
      // is the user's doing and runs at once.
      | Request::RoomInfo { .. }
  )
}

fn text(v: &Value) -> Message {
  Message::text(v.to_string())
}

/// Pushes `{"changed": true}` to `out` when the node's live state moves,
/// at most once per `every`: while it sleeps after a push, further changes
/// wait and come as one push after it. Ends when `out` closes.
#[cfg(test)]
pub(crate) async fn push_changes(s: Arc<Shared>, out: Out, every: std::time::Duration) {
  push_changes_unless(s, out, every, Arc::default()).await;
}

/// `push_changes`, silent while `watching` (the connection has deltas).
async fn push_changes_unless(
  s: Arc<Shared>,
  out: Out,
  every: std::time::Duration,
  watching: Arc<std::sync::atomic::AtomicBool>,
) {
  let _listening = super::live::Live::listen(&s);
  let mut seen = s.apps.live.generation();
  loop {
    s.apps
      .live
      .wait(seen, std::time::Duration::from_secs(25))
      .await;
    let now = s.apps.live.generation();
    if now != seen {
      seen = now;
      if watching.load(std::sync::atomic::Ordering::SeqCst) {
        continue;
      }
      if out.send(text(&json!({"changed": true}))).is_err() {
        break;
      }
      tokio::time::sleep(every).await;
    } else if out.is_closed() {
      break;
    }
  }
}

/// A connection's deltas (`watch`): the list whole, numbered 1, then each
/// change numbered on; the live sections whole as they move; the calls of
/// the threads it named (`calls`). A push it fell behind on (the channel
/// lagged) or one no delta says (`Reset`) is the list whole again.
async fn deltas(
  s: Arc<Shared>,
  out: Out,
  limit: Option<u32>,
  calls: Arc<Mutex<std::collections::HashSet<String>>>,
) {
  use super::feed::{Feed, Push};
  use tokio::sync::broadcast::error::RecvError;
  // Heard from before the list is read: a change meanwhile comes after it
  // again, and rows are put whole, so applying it twice is the same.
  let _listening = super::live::Live::listen(&s);
  let (mut rx, _subscribed) = Feed::subscribe(&s);
  let query = format!("all=1&every=1&limit={}", limit.unwrap_or(300));
  let snapshot = |s: &Arc<Shared>| {
    let (s, query) = (s.clone(), query.clone());
    async move {
      s.list_read(move |s| super::live::threads(&s, None, &query))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(Value::Null)
    }
  };
  let mut v: u64 = 1;
  if out
    .send(text(
      &json!({"list": {"v": v, "reset": snapshot(&s).await}}),
    ))
    .is_err()
  {
    return;
  }
  loop {
    let frame = match rx.recv().await {
      Ok(p) => match &*p {
        Push::Rows {
          upsert,
          remove,
          places,
        } => {
          v += 1;
          let mut list = json!({"v": v, "upsert": upsert, "remove": remove});
          if let Some(p) = places {
            list["places"] = p.clone();
          }
          json!({ "list": list })
        }
        Push::Reset => {
          v += 1;
          json!({"list": {"v": v, "reset": snapshot(&s).await}})
        }
        Push::Section { what, value } => json!({"live": {"what": what, "value": value}}),
        Push::Call { thread, in_flight } => {
          if !calls.lock().expect("calls lock").contains(thread) {
            continue;
          }
          json!({"call": {"thread": thread, "in_flight": in_flight}})
        }
      },
      Err(RecvError::Lagged(_)) => {
        v += 1;
        json!({"list": {"v": v, "reset": snapshot(&s).await}})
      }
      Err(RecvError::Closed) => return,
    };
    if out.send(text(&frame)).is_err() {
      return;
    }
  }
}

async fn session<S>(s: &Arc<Shared>, ws: WebSocketStream<S>)
where
  S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
  let (mut sink, mut incoming) = ws.split();
  let (out, mut queue) = mpsc::unbounded_channel::<Message>();
  let writer = tokio::spawn(async move {
    while let Some(m) = queue.recv().await {
      let close = matches!(m, Message::Close(_));
      if sink.send(m).await.is_err() || close {
        break;
      }
    }
    let _ = sink.close().await;
  });
  let lanes = Arc::new(Mutex::new(Lanes::default()));
  let mut running = tokio::task::JoinSet::new();
  // The threads, jobs or approvals changed: the client lists again.
  let watching: Arc<std::sync::atomic::AtomicBool> = Arc::default();
  let changes = tokio::spawn(push_changes_unless(
    s.clone(),
    out.clone(),
    CHANGED_EVERY,
    watching.clone(),
  ));
  let calls: Arc<Mutex<std::collections::HashSet<String>>> = Arc::default();
  let mut watch: Option<tokio::task::JoinHandle<()>> = None;
  let gate = Arc::new(tokio::sync::Semaphore::new(RUNNING));
  let waiting = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let acting = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let mut following: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
  let mut greeted = false;
  while let Some(Ok(m)) = incoming.next().await {
    let frame = match m {
      Message::Text(t) => t,
      Message::Close(_) => break,
      Message::Binary(_) => {
        let _ = out.send(text(&json!({"id": null, "error": "text frames only"})));
        continue;
      }
      _ => continue,
    };
    let say = |id: Option<u64>, why: String| {
      let _ = out.send(text(&json!({"id": id, "error": why})));
    };
    let raw: Value = serde_json::from_str(frame.as_str()).unwrap_or(Value::Null);
    let m: In = match serde_json::from_value(raw.clone()) {
      Ok(m) => m,
      Err(e) => {
        say(
          raw["id"].as_u64(),
          format!("not a message of the protocol: {e}"),
        );
        continue;
      }
    };
    if let Some(request) = m.request {
      let Some(id) = m.id else {
        say(None, "a request needs an `id`".into());
        continue;
      };
      if !greeted && !matches!(request, Request::Hello { .. }) {
        say(Some(id), "the first message is `Hello`".into());
        break;
      }
      greeted = true;
      if kind(&request) == Kind::Silent {
        // Nothing comes back: it goes to the chat or shell it acts on.
        let target = lanes.lock().expect("lanes lock").target(&request, m.to);
        if let Some(tx) = target {
          let _ = tx.send(request);
        }
        continue;
      }
      let (tx, rx) = mpsc::unbounded_channel();
      {
        let mut l = lanes.lock().expect("lanes lock");
        match kind(&request) {
          Kind::Chat => l.chats.push((id, tx)),
          Kind::Shell => l.shells.push((id, tx)),
          _ => {}
        }
      }
      if kind(&request) == Kind::One {
        use std::sync::atomic::Ordering::SeqCst;
        // Reads and the user's actions are bounded apart: a flood of
        // refreshes neither delays nor refuses a message sent.
        let gated = gated(&request);
        let (count, bound) = if gated {
          (&waiting, RUNNING + QUEUED)
        } else {
          (&acting, ACTIONS)
        };
        if count.fetch_add(1, SeqCst) >= bound {
          count.fetch_sub(1, SeqCst);
          say(
            Some(id),
            if gated {
              format!("busy: {RUNNING} requests running and {QUEUED} waiting on this connection")
            } else {
              format!("busy: {ACTIONS} actions running on this connection")
            },
          );
          continue;
        }
        let (s, out, lanes) = (s.clone(), out.clone(), lanes.clone());
        let (gate, count) = (gate.clone(), count.clone());
        running.spawn(async move {
          let permit = if gated {
            Some(gate.acquire().await)
          } else {
            None
          };
          lane(s, id, request, out, lanes, rx).await;
          drop(permit);
          count.fetch_sub(1, SeqCst);
        });
      } else {
        running.spawn(lane(s.clone(), id, request, out.clone(), lanes.clone(), rx));
      }
    } else if let Some(f) = m.follow {
      if !greeted {
        say(None, "the first message is `Hello`".into());
        break;
      }
      let (s, out, thread) = (s.clone(), out.clone(), f.thread.clone());
      let after = f.after;
      let task = tokio::spawn(async move {
        let mut after = after;
        // A thread compacted into another is that one now (design §2.15,
        // "One conversation"): its feed is followed, said as the thread
        // asked for, so a client open on an old ID (a link, a
        // notification) sees the turn running.
        let mut feed = match thread.parse::<stitch_records::RecordId>() {
          Ok(t) => crate::fork::current(&s, &t).to_string(),
          Err(_) => thread.clone(),
        };
        if feed != thread {
          after = None;
        }
        // Where its turns stand, at once and on each change.
        let status = {
          let (s, out, thread, feed) = (s.clone(), out.clone(), thread.clone(), feed.clone());
          async move {
            let mut seen = s.live.status(&feed);
            loop {
              let said = json!({"event": {"thread": thread, "turn": seen}});
              if out.send(text(&said)).is_err() {
                return;
              }
              seen = loop {
                let now = s
                  .live
                  .status_change(&feed, seen, std::time::Duration::from_secs(25))
                  .await;
                if now != seen {
                  break now;
                }
                if out.is_closed() {
                  return;
                }
              };
            }
          }
        };
        let _status = AbortOnDrop(tokio::spawn(status));
        loop {
          let f = crate::elsewhere::follow_here(&s, &feed, after, None).await;
          after = Some(f.next);
          // Compacted meanwhile: said (the client switches to the new
          // thread), then the new thread's feed is followed from its
          // start, which holds the same `Compacted` first (not said again).
          let mut moved = None;
          // The first answer (`after` none) only says where the feed stands.
          for event in f.events {
            if let LiveEvent::Compacted(c) = &event {
              if c.to == feed {
                continue;
              }
              moved = Some(c.to.clone());
            }
            let sent = out.send(text(&json!({"event": {"thread": thread, "event": event}})));
            if sent.is_err() {
              return;
            }
          }
          if let Some(to) = moved {
            feed = to;
            after = Some(0);
            continue;
          }
          if f.gap {
            let sent = out.send(text(&json!({"event": {"thread": thread, "gap": true}})));
            if sent.is_err() {
              return;
            }
          }
        }
      });
      if let Some(old) = following.insert(f.thread, task) {
        old.abort();
      }
    } else if let Some(f) = m.unfollow {
      if let Some(old) = following.remove(&f.thread) {
        old.abort();
      }
    } else if let Some(w) = m.watch {
      if !greeted {
        say(None, "the first message is `Hello`".into());
        break;
      }
      // Again: a resync, from a snapshot numbered 1.
      watching.store(true, std::sync::atomic::Ordering::SeqCst);
      if let Some(old) = watch.replace(tokio::spawn(deltas(
        s.clone(),
        out.clone(),
        w.limit,
        calls.clone(),
      ))) {
        old.abort();
      }
    } else if let Some(c) = m.calls {
      let added: Vec<String> = {
        let mut set = calls.lock().expect("calls lock");
        let added = c
          .threads
          .iter()
          .filter(|t| !set.contains(*t))
          .cloned()
          .collect();
        *set = c.threads.into_iter().collect();
        added
      };
      for (thread, in_flight) in s.apps.live.feed.calls_of(&added) {
        let _ = out.send(text(
          &json!({"call": {"thread": thread, "in_flight": in_flight}}),
        ));
      }
    } else {
      say(
        m.id,
        "nothing to do: `request`, `follow`, `unfollow`, `watch` or `calls`".into(),
      );
    }
  }
  for (_, t) in following {
    t.abort();
  }
  changes.abort();
  if let Some(w) = watch {
    w.abort();
  }
  // Its lanes close their connections to the node's handler; chats there
  // go on to their end, as when a socket's client leaves.
  running.abort_all();
  let _ = out.send(Message::Close(None));
  drop(out);
  let _ = writer.await;
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
  use std::time::Duration;

  use super::*;

  async fn node(dir: &std::path::Path) -> crate::node::NodeHandle {
    let c = crate::config::Config {
      node: Some("mbp".into()),
      discovery: Some("local".into()),
      sync_interval_secs: Some(0),
      ..Default::default()
    };
    c.save(dir).unwrap();
    crate::start(dir).await.unwrap()
  }

  /// The next frame of `rx` that is a `list` push, within `wait`.
  async fn next_list(rx: &mut mpsc::UnboundedReceiver<Message>, wait: Duration) -> Option<Value> {
    let end = tokio::time::Instant::now() + wait;
    loop {
      let m = tokio::time::timeout_at(end, rx.recv()).await.ok()??;
      if let Message::Text(t) = m {
        let v: Value = serde_json::from_str(t.as_str()).unwrap();
        if v.get("list").is_some() {
          return Some(v["list"].clone());
        }
      }
    }
  }

  /// Deltas (`watch`): a call starting in a running thread pushes nothing to
  /// the list (only to who watches that thread's calls), and a change of a
  /// row is one delta with that row alone.
  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_running_call_is_no_list_traffic_and_a_row_change_is_one_delta() {
    use stitch_records::{Author, Draft};
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let root = stitch_store::RecordRoot::init(
      &dir.path().join("home"),
      "home".parse().unwrap(),
      stitch_store::Role::Home,
    )
    .unwrap();
    stitch_store::Index::open(&data.join("index.sqlite"))
      .unwrap()
      .refresh(&root)
      .unwrap();
    let node = node(&data).await;
    let s = node.shared().clone();
    let me = s.state().node.clone();
    let home = s.state().home().unwrap();
    let mut ids = Vec::new();
    for title in ["one", "two"] {
      let r = home
        .publish(
          &me,
          Draft::new("thread", Author::user(me.clone()))
            .unwrap()
            .body(format!("# {title}\n")),
        )
        .unwrap();
      ids.push(r.id);
    }
    s.state().index.lock().unwrap().refresh(&home).unwrap();
    let thread = ids[0].clone();
    // Its loop runs here (a turn starting): a row change, said once.
    s.detached
      .starting
      .lock()
      .unwrap()
      .insert(thread.to_string(), String::new());

    let (out, mut rx) = mpsc::unbounded_channel::<Message>();
    let calls: Arc<Mutex<std::collections::HashSet<String>>> = Arc::default();
    let task = tokio::spawn(deltas(s.clone(), out, Some(300), calls.clone()));
    let first = next_list(&mut rx, Duration::from_secs(10))
      .await
      .expect("the list");
    assert_eq!(first["v"], 1);
    let rows = first["reset"]["threads"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{first}");
    // Settle: what the first passes see (the loop starting) comes now.
    let mut last = 1;
    while let Some(l) = next_list(&mut rx, Duration::from_millis(2500)).await {
      assert_eq!(l["v"].as_u64(), Some(last + 1), "numbered on: {l}");
      last += 1;
    }

    // A call starts and ends, several times: no list traffic.
    let call = |n: u32| {
      let ev = stitch_records::Event::ToolCall(stitch_records::ToolCall {
        id: format!("c{n}"),
        at: jiff::Timestamp::now(),
        name: "shell_run".into(),
        input: json!({"command": format!("sleep {n}")}),
      });
      json!({
        "pid": std::process::id(), "prev": null, "msg_n": 0,
        "events": [ev.to_value()],
        "running": {"id": format!("c{n}"), "since": jiff::Timestamp::now().to_string()},
      })
      .to_string()
    };
    let key = format!("pending-step:{thread}");
    for n in 0..3 {
      s.state()
        .index
        .lock()
        .unwrap()
        .set_local(&key, Some(&call(n)))
        .unwrap();
      tokio::time::sleep(Duration::from_millis(1200)).await;
    }
    assert_eq!(
      next_list(&mut rx, Duration::from_millis(1500)).await,
      None,
      "a running call is not a list change"
    );
    // Watched, the thread's call is heard.
    calls.lock().unwrap().insert(thread.to_string());
    s.state()
      .index
      .lock()
      .unwrap()
      .set_local(&key, Some(&call(9)))
      .unwrap();
    let mut heard = false;
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Ok(Some(Message::Text(t))) = tokio::time::timeout_at(end, rx.recv()).await {
      let v: Value = serde_json::from_str(t.as_str()).unwrap();
      assert!(v.get("list").is_none(), "{v}");
      if v["call"]["thread"] == thread.to_string() {
        assert_eq!(v["call"]["in_flight"]["name"], "shell_run", "{v}");
        heard = true;
        break;
      }
    }
    assert!(heard, "the watched thread's call");

    // The other thread renamed: one delta, its row alone.
    crate::meta::set(
      &s,
      &ids[1],
      "title",
      "renamed".into(),
      Author::user(me.clone()),
    )
    .unwrap();
    let d = next_list(&mut rx, Duration::from_secs(5))
      .await
      .expect("a delta");
    assert_eq!(d["v"].as_u64(), Some(last + 1), "{d}");
    let up = d["upsert"].as_array().unwrap();
    assert_eq!(up.len(), 1, "{d}");
    assert_eq!(up[0]["id"], ids[1].to_string());
    assert_eq!(up[0]["title"], "renamed");
    assert!(d.get("reset").is_none());
    assert_eq!(
      next_list(&mut rx, Duration::from_millis(2500)).await,
      None,
      "one delta"
    );
    task.abort();
    node.stop().await.unwrap();
  }

  /// Before and after deltas, on a copy of a node's data
  /// (`STITCH_PROFILE_DATA`, as `tests/list_profile.rs` makes it): a client
  /// on the home view while a thread works (a call starting every 1.5 s, a
  /// row change every 5 s), re-listing on `changed` as the client did
  /// (debounced 500 ms to 2 s: `Threads` 300 and the four live sections),
  /// then on deltas. Prints the process's CPU and the bytes pushed.
  /// `cargo test --release -p stitch-node --lib bench_home_view -- --ignored --nocapture`
  #[cfg(target_os = "linux")]
  #[ignore]
  #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
  async fn bench_home_view_while_a_thread_works() {
    use stitch_records::Author;
    let Ok(src) = std::env::var("STITCH_PROFILE_DATA") else {
      return;
    };
    let secs: u64 = std::env::var("STITCH_BENCH_SECS")
      .ok()
      .and_then(|v| v.parse().ok())
      .unwrap_or(40);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::copy(
      std::path::Path::new(&src).join("index.sqlite"),
      data.join("index.sqlite"),
    )
    .unwrap();
    let node = node(&data).await;
    let s = node.shared().clone();
    let me = s.state().node.clone();
    let thread = s.state().index.lock().unwrap().listed_threads().unwrap()[0]
      .0
      .id
      .clone();
    s.detached
      .starting
      .lock()
      .unwrap()
      .insert(thread.to_string(), String::new());
    let cpu = || {
      let f: Vec<u64> = std::fs::read_to_string("/proc/self/stat")
        .unwrap()
        .rsplit(')')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|v| v.parse().unwrap_or(0))
        .collect();
      f[11] + f[12]
    };
    let work = |s: Arc<Shared>, until: tokio::time::Instant| {
      let (thread, me) = (thread.clone(), me.clone());
      tokio::spawn(async move {
        let key = format!("pending-step:{thread}");
        let mut n = 0u32;
        while tokio::time::Instant::now() < until {
          n += 1;
          let ev = stitch_records::Event::ToolCall(stitch_records::ToolCall {
            id: format!("c{n}"),
            at: jiff::Timestamp::now(),
            name: "shell_run".into(),
            input: json!({"command": format!("step {n}")}),
          });
          let v = json!({"pid": std::process::id(), "prev": null, "msg_n": 0, "events": [ev.to_value()],
            "running": {"id": format!("c{n}"), "since": jiff::Timestamp::now().to_string()}});
          s.state()
            .index
            .lock()
            .unwrap()
            .set_local(&key, Some(&v.to_string()))
            .unwrap();
          if n.is_multiple_of(3) {
            let s2 = s.clone();
            let (t, me) = (thread.clone(), me.clone());
            let _ = tokio::task::spawn_blocking(move || {
              crate::meta::set(
                &s2,
                &t,
                "note",
                format!("step {n}").into(),
                Author::user(me),
              )
            })
            .await;
          }
          tokio::time::sleep(Duration::from_millis(1500)).await;
        }
      })
    };
    // Settle after the start (schedules, resumes).
    tokio::time::sleep(Duration::from_secs(10)).await;
    for mode in ["changed", "deltas"] {
      let (out, mut rx) = mpsc::unbounded_channel::<Message>();
      let until = tokio::time::Instant::now() + Duration::from_secs(secs);
      let bytes = Arc::new(AtomicUsize::new(0));
      let client = if mode == "changed" {
        let (s, bytes) = (s.clone(), bytes.clone());
        tokio::spawn(async move {
          let pushes = tokio::spawn(push_changes(s.clone(), out, CHANGED_EVERY));
          loop {
            // The client's debounce: 500 ms after the last, 2 s at most.
            let Some(_) = rx.recv().await else { break };
            let first = tokio::time::Instant::now();
            loop {
              match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Some(_)) if first.elapsed() < Duration::from_secs(2) => continue,
                _ => break,
              }
            }
            let q = "all=1&every=1&limit=300".to_owned();
            let mut n = s
              .list_read(move |s| crate::apps::live::threads(&s, None, &q))
              .await
              .unwrap()
              .unwrap()
              .to_string()
              .len();
            n += s
              .list_read(|s| crate::apps::live::attention(&s))
              .await
              .unwrap()
              .to_string()
              .len();
            n += crate::apps::live::jobs(&s).await.to_string().len();
            n += s
              .list_read(|s| crate::apps::live::schedules(&s, ""))
              .await
              .unwrap()
              .to_string()
              .len();
            n += s
              .list_read(|s| crate::loose::live(&s))
              .await
              .unwrap()
              .to_string()
              .len();
            bytes.fetch_add(n, SeqCst);
          }
          pushes.abort();
        })
      } else {
        let calls: Arc<Mutex<std::collections::HashSet<String>>> = Arc::default();
        calls.lock().unwrap().insert(thread.to_string());
        let task = tokio::spawn(deltas(s.clone(), out, Some(300), calls));
        let bytes = bytes.clone();
        tokio::spawn(async move {
          let mut first = true;
          while let Some(m) = rx.recv().await {
            if let Message::Text(t) = m {
              if first {
                first = false;
                continue;
              } // the snapshot, on connecting
              bytes.fetch_add(t.len(), SeqCst);
            }
          }
          task.abort();
        })
      };
      tokio::time::sleep(Duration::from_secs(3)).await;
      let (c0, t0) = (cpu(), std::time::Instant::now());
      bytes.store(0, SeqCst);
      let w = work(s.clone(), until);
      let _ = w.await;
      let dt = t0.elapsed().as_secs_f64();
      let ticks = (cpu() - c0) as f64;
      println!(
        "{mode:8}: {:5.1}% of a core, {:8.0} B/s to the client",
        ticks / dt,
        bytes.load(SeqCst) as f64 / dt
      );
      client.abort();
      tokio::time::sleep(Duration::from_secs(3)).await;
    }
    node.stop().await.unwrap();
  }

  /// A burst of changes comes as a push at once and one more after the
  /// quiet interval, never one per change; the last change is not lost.
  #[tokio::test]
  async fn changed_pushes_are_coalesced() {
    let dir = tempfile::tempdir().unwrap();
    let node = node(&dir.path().join("data")).await;
    let s = node.shared().clone();
    let (out, mut rx) = mpsc::unbounded_channel::<Message>();
    let every = Duration::from_millis(300);
    let task = tokio::spawn(push_changes(s.clone(), out, every));
    tokio::time::sleep(Duration::from_millis(100)).await;
    // 100 changes in about 200 ms.
    for _ in 0..100 {
      s.apps.live.bump(&["threads"]);
      tokio::time::sleep(Duration::from_millis(2)).await;
    }
    tokio::time::sleep(every * 3).await;
    let mut got = 0;
    while rx.try_recv().is_ok() {
      got += 1;
    }
    assert!(
      (2..=3).contains(&got),
      "100 changes in 200 ms with a 300 ms interval: a push now and one after, got {got}"
    );
    // Quiet: nothing more; then one change is one push, at once.
    s.apps.live.bump(&["jobs"]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err());
    task.abort();
    node.stop().await.unwrap();
  }

  /// List reads of the node run on the blocking pool a few at a time: the
  /// rest wait as futures, so a flood cannot fill the pool.
  #[tokio::test]
  async fn list_reads_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let node = node(&dir.path().join("data")).await;
    let s = node.shared().clone();
    let (now, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let mut all = Vec::new();
    for _ in 0..200 {
      let (s, now, peak) = (s.clone(), now.clone(), peak.clone());
      all.push(tokio::spawn(async move {
        s.list_read(move |_| {
          let n = now.fetch_add(1, SeqCst) + 1;
          peak.fetch_max(n, SeqCst);
          std::thread::sleep(Duration::from_millis(5));
          now.fetch_sub(1, SeqCst);
        })
        .await
        .unwrap();
      }));
    }
    for t in all {
      t.await.unwrap();
    }
    let peak = peak.load(SeqCst);
    assert!(
      (1..=crate::node::LIST_READS).contains(&peak),
      "at most {} at once, saw {peak}",
      crate::node::LIST_READS
    );
    node.stop().await.unwrap();
  }
}
