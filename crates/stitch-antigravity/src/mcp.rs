//! The MCP endpoint agy calls Stitch's tools through: streamable HTTP on
//! 127.0.0.1, plain JSON answers, a random token in a header.
//!
//! A `tools/call` is not run here: it goes to the session it names (agy
//! puts the ACP session in `_meta`), where the provider's stream ends with
//! it as the step's tool call; the HTTP request stays open until the
//! node's loop has run the tool and the next model call brings its result.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use stitch_providers::ToolDef;
use tokio::sync::{mpsc, oneshot};

use crate::acp::RpcError;

/// The header the token goes in.
pub const TOKEN_HEADER: &str = "x-stitch-token";
/// The server's name, as agy prefixes its tools (`mcp_stitch_<tool>`).
pub const SERVER: &str = "stitch";
/// `_meta` key of the ACP session a call belongs to.
const CONVERSATION: &str = "antigravity.google/conversation_id";

/// A tool's result, as the node's loop gave it.
#[derive(Debug)]
pub struct ToolAnswer {
  pub text: String,
  pub is_error: bool,
}

/// What reaches a session's stream.
#[derive(Debug)]
pub enum SessionEvent {
  /// A `session/update`'s `update`.
  Update(Value),
  /// agy calls a tool and waits for `reply`.
  Call {
    name: String,
    input: Value,
    reply: oneshot::Sender<ToolAnswer>,
  },
  /// The prompt's answer.
  Done(Result<Value, RpcError>),
}

/// Sessions by ACP ID, for what agy sends (updates, calls).
#[derive(Default)]
pub struct Router {
  sessions: Mutex<HashMap<String, mpsc::UnboundedSender<SessionEvent>>>,
}

impl Router {
  pub fn register(&self, id: &str, tx: mpsc::UnboundedSender<SessionEvent>) {
    self
      .sessions
      .lock()
      .expect("router lock")
      .insert(id.to_owned(), tx);
  }

  pub fn remove(&self, id: &str) {
    self.sessions.lock().expect("router lock").remove(id);
  }

  /// Sends `event` to session `id`; false when nothing listens.
  pub fn send(&self, id: &str, event: SessionEvent) -> bool {
    let tx = self.sessions.lock().expect("router lock").get(id).cloned();
    tx.is_some_and(|tx| tx.send(event).is_ok())
  }
}

pub struct Mcp {
  pub token: String,
  /// Every tool the node's requests have offered, by name (the last
  /// definition of each): agy lists them once per process.
  tools: Mutex<BTreeMap<String, ToolDef>>,
  router: Arc<Router>,
  /// Calls agy may cancel, by `<Mcp-Session-Id> <JSON-RPC id>`.
  inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl Mcp {
  pub fn new(token: String, router: Arc<Router>) -> Mcp {
    Mcp {
      token,
      tools: Mutex::new(BTreeMap::new()),
      router,
      inflight: Mutex::new(HashMap::new()),
    }
  }

  /// Adds `tools` to those served; true when a name is new.
  pub fn offer(&self, tools: &[ToolDef]) -> bool {
    let mut known = self.tools.lock().expect("tools lock");
    let mut grew = false;
    for t in tools {
      grew |= known.insert(t.name.clone(), t.clone()).is_none();
    }
    grew
  }

  pub fn names(&self) -> Vec<String> {
    self
      .tools
      .lock()
      .expect("tools lock")
      .keys()
      .cloned()
      .collect()
  }

  /// The answer to one JSON-RPC message (`None` for a notification).
  pub async fn handle(&self, session: &str, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = msg.get("id").cloned() else {
      if method == "notifications/cancelled"
        && let Some(r) = params.get("requestId")
      {
        let key = format!("{session} {r}");
        if let Some(tx) = self.inflight.lock().expect("inflight lock").remove(&key) {
          let _ = tx.send(());
        }
      }
      return None;
    };
    let result = match method {
      "initialize" => Ok(json!({
        "protocolVersion": params
          .get("protocolVersion")
          .and_then(Value::as_str)
          .unwrap_or("2025-11-25"),
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")},
      })),
      "ping" => Ok(json!({})),
      "tools/list" => {
        let tools: Vec<Value> = self
          .tools
          .lock()
          .expect("tools lock")
          .values()
          .map(|t| {
            let mut schema = t.input_schema.clone();
            if schema.get("type").is_none()
              && let Some(o) = schema.as_object_mut()
            {
              o.insert("type".into(), json!("object"));
            }
            json!({"name": t.name, "description": t.description, "inputSchema": schema})
          })
          .collect();
        Ok(json!({"tools": tools}))
      }
      "tools/call" => Ok(self.call(session, &id, &params).await),
      // `server/discover` (the stateless lifecycle) included: agy falls
      // back to `initialize`.
      _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
      Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
      Err((code, message)) => {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
      }
    })
  }

  async fn call(&self, session: &str, id: &Value, params: &Value) -> Value {
    let result = |text: &str, is_error: bool| json!({"content": [{"type": "text", "text": text}], "isError": is_error});
    let name = params
      .get("name")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_owned();
    let input = params
      .get("arguments")
      .cloned()
      .unwrap_or_else(|| json!({}));
    let Some(conversation) = params
      .pointer("/_meta")
      .and_then(|m| m.get(CONVERSATION))
      .and_then(Value::as_str)
    else {
      return result("This call names no conversation.", true);
    };
    let (tx, rx) = oneshot::channel();
    let sent = self.router.send(
      conversation,
      SessionEvent::Call {
        name,
        input,
        reply: tx,
      },
    );
    if !sent {
      return result("No turn of Stitch's waits on this conversation.", true);
    }
    let key = format!("{session} {id}");
    let (cancel_tx, cancel_rx) = oneshot::channel();
    self
      .inflight
      .lock()
      .expect("inflight lock")
      .insert(key.clone(), cancel_tx);
    let answer = tokio::select! {
      a = rx => a.ok(),
      _ = cancel_rx => None,
    };
    self.inflight.lock().expect("inflight lock").remove(&key);
    match answer {
      Some(a) => result(&a.text, a.is_error),
      None => result("The call was cancelled.", true),
    }
  }
}

/// Serves `mcp` on `listener` until the runtime ends.
pub async fn serve(listener: tokio::net::TcpListener, mcp: Arc<Mcp>) {
  loop {
    let Ok((stream, _)) = listener.accept().await else {
      continue;
    };
    let mcp = mcp.clone();
    tokio::spawn(async move {
      let service = service_fn(move |req| {
        let mcp = mcp.clone();
        async move { Ok::<_, Infallible>(respond(&mcp, req).await) }
      });
      let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await;
    });
  }
}

fn reply(status: u16, body: Bytes, session: Option<&str>) -> hyper::Response<Full<Bytes>> {
  let mut b = hyper::Response::builder()
    .status(status)
    .header("content-type", "application/json");
  if let Some(s) = session {
    b = b.header("mcp-session-id", s);
  }
  b.body(Full::new(body))
    .unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new())))
}

async fn respond(mcp: &Mcp, req: hyper::Request<Incoming>) -> hyper::Response<Full<Bytes>> {
  let authorized = req
    .headers()
    .get(TOKEN_HEADER)
    .and_then(|v| v.to_str().ok())
    .is_some_and(|t| t == mcp.token);
  if !req.uri().path().starts_with("/mcp") {
    return reply(404, Bytes::new(), None);
  }
  if !authorized {
    return reply(401, Bytes::new(), None);
  }
  match *req.method() {
    hyper::Method::POST => {}
    hyper::Method::DELETE => return reply(200, Bytes::new(), None),
    _ => return reply(405, Bytes::new(), None),
  }
  let session = req
    .headers()
    .get("mcp-session-id")
    .and_then(|v| v.to_str().ok())
    .map(str::to_owned);
  let Ok(body) = req.into_body().collect().await else {
    return reply(400, Bytes::new(), None);
  };
  let Ok(msg) = serde_json::from_slice::<Value>(&body.to_bytes()) else {
    let e =
      json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
    return reply(400, Bytes::from(e.to_string()), None);
  };
  // A new session at `initialize`.
  let new = (msg.get("method").and_then(Value::as_str) == Some("initialize"))
    .then(|| crate::random_hex(16));
  let session = new.clone().or(session).unwrap_or_default();
  let answer = match &msg {
    Value::Array(batch) => {
      let mut out = Vec::new();
      for m in batch {
        if let Some(a) = mcp.handle(&session, m).await {
          out.push(a);
        }
      }
      (!out.is_empty()).then_some(Value::Array(out))
    }
    m => mcp.handle(&session, m).await,
  };
  match answer {
    Some(a) => reply(200, Bytes::from(a.to_string()), new.as_deref()),
    None => reply(202, Bytes::new(), None),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn mcp() -> (Mcp, Arc<Router>) {
    let router = Arc::new(Router::default());
    (Mcp::new("tok".into(), router.clone()), router)
  }

  #[tokio::test]
  async fn the_handshake_and_the_tool_list() {
    let (m, _) = mcp();
    let tool = |n: &str| ToolDef {
      name: n.into(),
      description: format!("does {n}"),
      input_schema: json!({"properties": {}}),
    };
    assert!(m.offer(&[tool("b"), tool("a")]));
    assert!(!m.offer(&[tool("a")]));
    let discover = m
      .handle(
        "s",
        &json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}),
      )
      .await
      .unwrap();
    assert_eq!(discover["error"]["code"], -32601);
    let init = m
      .handle(
        "s",
        &json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}}),
      )
      .await
      .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(
      m.handle(
        "s",
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
      )
      .await
      .is_none()
    );
    let list = m
      .handle(
        "s",
        &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
      )
      .await
      .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "a");
    assert_eq!(tools[0]["inputSchema"]["type"], "object");
    assert_eq!(m.names(), ["a", "b"]);
  }

  #[tokio::test]
  async fn a_call_goes_to_its_session_and_waits_for_the_answer() {
    let (m, router) = mcp();
    let m = Arc::new(m);
    let (tx, mut rx) = mpsc::unbounded_channel();
    router.register("conv-1", tx);
    let call = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
      "name": "shell_run", "arguments": {"command": "ls"},
      "_meta": {"antigravity.google/conversation_id": "conv-1"}}});
    let m2 = m.clone();
    let pending = tokio::spawn(async move { m2.handle("s", &call).await.unwrap() });
    let Some(SessionEvent::Call { name, input, reply }) = rx.recv().await else {
      panic!("no call");
    };
    assert_eq!(
      (name.as_str(), &input),
      ("shell_run", &json!({"command": "ls"}))
    );
    reply
      .send(ToolAnswer {
        text: "a b".into(),
        is_error: false,
      })
      .unwrap();
    let out = pending.await.unwrap();
    assert_eq!(out["result"]["content"][0]["text"], "a b");
    assert_eq!(out["result"]["isError"], false);

    // Cancelled by agy: the call ends at once.
    let call = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
      "name": "x", "_meta": {"antigravity.google/conversation_id": "conv-1"}}});
    let m2 = m.clone();
    let pending = tokio::spawn(async move { m2.handle("s", &call).await.unwrap() });
    let _held = rx.recv().await;
    m.handle(
      "s",
      &json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 5}}),
    )
    .await;
    let out = pending.await.unwrap();
    assert_eq!(out["result"]["isError"], true);

    // A conversation nobody waits on.
    let out = m
      .handle(
        "s",
        &json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {
          "name": "x", "_meta": {"antigravity.google/conversation_id": "nobody"}}}),
      )
      .await
      .unwrap();
    assert_eq!(out["result"]["isError"], true);
  }
}
