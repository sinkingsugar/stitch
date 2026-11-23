//! Tools on other nodes (design §2.14): an MCP server a node shares
//! (`[mcp.servers.<name>] share = true`) is offered to the threads of the
//! user's other nodes, and their calls run on it.
//!
//! - **Published** as the node's `capabilities` record in the home
//!   workspace: the servers it configures, and the tools of those it
//!   shares, written again when they change.
//! - **Called** as a question on the node protocol (`Ask`): the thread,
//!   its repository, the server, the tool, the arguments; it runs there
//!   under that node's rules and approvals (a `question` record in the
//!   thread's repository, synced to the asking node at once), and the
//!   result comes back on the stream.
//! - **Queued** while that node cannot be reached: a `call` record in the
//!   thread's repository, which that node takes (`claim`) after a sync,
//!   runs, and answers with a `report` to the thread.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitch_records::{Author, Draft, RecordId, Ref, kinds};
use stitch_store::RecordRoot;

use crate::api::Reply;
use crate::detached::ForPeer;
use crate::mcp::{Others, Remote, RemoteCall, RemoteServer, RemoteTool};
use crate::node::Shared;
use crate::tools::{BoxFuture, ToolImage, ToolOutcome};
use crate::toolset::Entry;
use crate::{Error, Result};

/// How long reaching a node may take before its tool is said to be
/// unreachable.
const CONNECT: Duration = Duration::from_secs(5);

/// A tool call for another node, as asked on the network.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CallTool {
  pub thread: String,
  /// The thread's repository: where an approval is asked.
  pub repo: String,
  pub server: String,
  pub tool: String,
  /// The arguments, as JSON text.
  pub input: String,
}

/// What this node publishes: the servers it configures, and the tools of
/// those it shares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Capabilities {
  servers: Vec<String>,
  shared: Vec<(String, Vec<RemoteTool>)>,
}

impl Capabilities {
  fn of_record(fields: &Value) -> Capabilities {
    let servers = fields
      .get("servers")
      .and_then(Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(Value::as_str)
          .map(str::to_owned)
          .collect()
      })
      .unwrap_or_default();
    let shared = fields
      .get("shared")
      .and_then(Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(|s| {
            let server = s.get("server")?.as_str()?.to_owned();
            let tools = s
              .get("tools")
              .and_then(Value::as_array)
              .map(|t| {
                t.iter()
                  .filter_map(|t| {
                    Some(RemoteTool {
                      name: t.get("name")?.as_str()?.to_owned(),
                      description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                      schema: t
                        .get("schema")
                        .and_then(Value::as_str)
                        .unwrap_or("{}")
                        .to_owned(),
                    })
                  })
                  .collect()
              })
              .unwrap_or_default();
            Some((server, tools))
          })
          .collect()
      })
      .unwrap_or_default();
    Capabilities { servers, shared }
  }
}

/// The latest `capabilities` record of each node, by node.
fn latest(s: &Shared) -> Vec<(String, RecordId, Capabilities)> {
  let mut records = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .records(Some(kinds::CAPABILITIES))
    .unwrap_or_default();
  records.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
  let mut out: Vec<(String, RecordId, Capabilities)> = Vec::new();
  for r in records {
    let node = r
      .fields
      .get("node")
      .and_then(Value::as_str)
      .unwrap_or(&r.node)
      .to_owned();
    let caps = Capabilities::of_record(&r.fields);
    match out.iter_mut().find(|(n, ..)| *n == node) {
      Some(slot) => *slot = (node, r.id, caps),
      None => out.push((node, r.id, caps)),
    }
  }
  out
}

/// What this node configures and shares now: shared servers are started
/// (in the background at the node's start) to list their tools. One that
/// does not start keeps the tools last published for it (none at first):
/// it is still shared, its calls failing there, saying why.
async fn now(s: &Shared, before: Option<&Capabilities>) -> Capabilities {
  let settings = s.mcp_settings();
  s.mcp.configure(&settings.servers);
  let mut caps = Capabilities::default();
  for server in s.mcp.servers() {
    caps.servers.push(server.name.clone());
    if server.settings.share != Some(true) {
      continue;
    }
    let Ok(tools) = server.tools().await else {
      // Said in the node's log.
      let last = before
        .and_then(|b| b.shared.iter().find(|(n, _)| *n == server.name))
        .map(|(_, t)| t.clone())
        .unwrap_or_default();
      caps.shared.push((server.name.clone(), last));
      continue;
    };
    let mut tools: Vec<RemoteTool> = tools
      .into_iter()
      .map(|t| RemoteTool {
        description: t
          .description
          .as_deref()
          .filter(|d| !d.trim().is_empty())
          .or(t.title.as_deref())
          .unwrap_or("")
          .to_owned(),
        schema: serde_json::to_string(&*t.input_schema).unwrap_or_else(|_| "{}".into()),
        name: t.name.to_string(),
      })
      .collect();
    // In a stable order, so the same tools are never published twice.
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    caps.shared.push((server.name.clone(), tools));
  }
  caps
}

/// How long the tools of a server that said they changed are left to
/// settle before they are published again.
const SETTLE: Duration = Duration::from_secs(2);

/// Publishes this node's `capabilities` when they changed since it last
/// did; one pass at a time (a pass under way, this one is skipped).
pub(crate) async fn publish(s: &Arc<Shared>) {
  let Ok(one) = s.publishing.try_lock() else {
    return;
  };
  publish_held(s, one).await;
}

/// For as long as the node runs, from its start: publishes what it shares,
/// then again when a server says its tools changed (once they settled),
/// and when a shared server that failed to start or to list its tools is
/// due to be tried again.
pub(crate) async fn follow(s: Arc<Shared>, mut stop: tokio::sync::watch::Receiver<bool>) {
  loop {
    let one = s.publishing.lock().await;
    publish_held(&s, one).await;
    let mut retry: Option<std::time::Instant> = None;
    for server in s.mcp.servers() {
      if server.settings.share == Some(true)
        && let Some(at) = server.retry_at().await
      {
        retry = Some(retry.map_or(at, |r| r.min(at)));
      }
    }
    let due = async {
      match retry {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
      }
    };
    tokio::select! {
      _ = s.mcp.tools_changed.notified() => {
        tokio::select! {
          _ = tokio::time::sleep(SETTLE) => {}
          _ = stop.changed() => return,
        }
      }
      _ = due => {}
      _ = stop.changed() => return,
    }
    if s.mcp.is_closed() {
      return;
    }
  }
}

async fn publish_held(s: &Arc<Shared>, _one: tokio::sync::MutexGuard<'_, ()>) {
  if s.mcp.is_closed() {
    return; // stopping: what it lists now is nothing
  }
  let me = s.state().node.to_string();
  let before = latest(s).into_iter().find(|(n, ..)| *n == me);
  let caps = now(s, before.as_ref().map(|(_, _, c)| c)).await;
  if s.mcp.is_closed() {
    return;
  }
  if before.as_ref().is_some_and(|(_, _, c)| *c == caps)
    || (before.is_none() && caps == Capabilities::default())
  {
    return;
  }
  let Some(home) = s.state().home() else {
    return;
  };
  if let Err(e) = write(s, &home, &caps, before.map(|(_, id, _)| id)) {
    eprintln!("capabilities: not published: {e}");
  }
}

fn write(
  s: &Shared,
  home: &RecordRoot,
  caps: &Capabilities,
  supersedes: Option<RecordId>,
) -> Result<()> {
  let node = s.state().node.clone();
  let shared = toml::Value::Array(
    caps
      .shared
      .iter()
      .map(|(server, tools)| {
        let mut t = toml::Table::new();
        t.insert("server".into(), server.as_str().into());
        t.insert(
          "tools".into(),
          toml::Value::Array(
            tools
              .iter()
              .map(|tool| {
                let mut x = toml::Table::new();
                x.insert("name".into(), tool.name.as_str().into());
                x.insert("description".into(), tool.description.as_str().into());
                x.insert("schema".into(), tool.schema.as_str().into());
                toml::Value::Table(x)
              })
              .collect(),
          ),
        );
        toml::Value::Table(t)
      })
      .collect(),
  );
  let mut draft = Draft::new(kinds::CAPABILITIES, Author::node(node.clone()))?
    .field("node", node.as_str())?
    .field(
      "servers",
      toml::Value::Array(caps.servers.iter().map(|n| n.as_str().into()).collect()),
    )?
    .field("shared", shared)?
    .body(format!("# What {node} offers the user's other nodes\n"));
  if let Some(prev) = supersedes {
    draft = draft.reference("supersedes", Ref::to(prev))?;
  }
  home.publish(&node, draft)?;
  s.state().index.lock().expect("index lock").refresh(home)?;
  Ok(())
}

/// Other nodes' servers as a loop of this node sees them (calls to them
/// made for a thread of `root`).
pub(crate) fn others(s: &Arc<Shared>, root: &RecordRoot) -> Others {
  Others {
    remote: Some(Arc::new(NodeRemote {
      shared: Arc::downgrade(s),
      root: root.clone(),
    })),
    ..listing(s)
  }
}

/// Other nodes' servers: those shared (one per name, the first node by
/// name, none this node configures), and the names of those not shared,
/// from their `capabilities` records.
pub(crate) fn listing(s: &Shared) -> Others {
  let me = s.state().node.to_string();
  let here: Vec<String> = s.mcp.servers().iter().map(|x| x.name.clone()).collect();
  let mut out = Others::default();
  for (node, _, caps) in latest(s) {
    if node == me {
      continue;
    }
    for server in &caps.servers {
      if !caps.shared.iter().any(|(n, _)| n == server) {
        out.unshared.push((node.clone(), server.clone()));
      }
    }
    for (server, tools) in caps.shared {
      out.shared.push(RemoteServer {
        node: node.clone(),
        server,
        tools,
      });
    }
  }
  out.shared.retain(|r| !here.contains(&r.server));
  out
    .shared
    .sort_by(|a, b| (&a.node, &a.server).cmp(&(&b.node, &b.server)));
  let mut seen: Vec<String> = Vec::new();
  out.shared.retain(|r| {
    let first = !seen.contains(&r.server);
    seen.push(r.server.clone());
    first
  });
  out
}

/// The node's side of `mcp::Remote`, for one loop on a thread of `root`.
struct NodeRemote {
  shared: Weak<Shared>,
  root: RecordRoot,
}

impl Remote for NodeRemote {
  fn call<'a>(&'a self, thread: &'a RecordId, call: RemoteCall<'a>) -> BoxFuture<'a, ToolOutcome> {
    Box::pin(async move {
      let Some(s) = self.shared.upgrade() else {
        return ToolOutcome::err("the node is stopping");
      };
      call_on(&s, &self.root, thread, call).await
    })
  }

  fn queue<'a>(&'a self, thread: &'a RecordId, call: RemoteCall<'a>) -> BoxFuture<'a, ToolOutcome> {
    Box::pin(async move {
      let Some(s) = self.shared.upgrade() else {
        return ToolOutcome::err("the node is stopping");
      };
      match queue(&s, &self.root, thread, call) {
        Ok(id) => ToolOutcome::ok(format!(
          "Queued as {id}: {} runs on {} when it is back, under its rules; its result comes \
           to this thread as a report, which wakes it.",
          call.name, call.node
        )),
        Err(e) => ToolOutcome::err(e.to_string()),
      }
    })
  }
}

/// Runs `call` on its node, asked over the network.
async fn call_on(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  call: RemoteCall<'_>,
) -> ToolOutcome {
  let unreachable = |why: &str| {
    ToolOutcome::err(format!(
      "{} runs on {}, which cannot be reached now ({why}); nothing was run. To run it when {} \
       is back, call queue_call with tool \"{}\" and these arguments: its result comes to this \
       thread as a report.",
      call.name, call.node, call.node, call.name
    ))
  };
  let state = s.state().clone();
  let name = call.node.to_owned();
  let peer = match tokio::task::spawn_blocking(move || state.find_peer(&name)).await {
    Ok(Ok(Some(p))) => p,
    Ok(Ok(None)) => return unreachable("not among this node's peers"),
    Ok(Err(e)) => return unreachable(&e.to_string()),
    Err(e) => return unreachable(&e.to_string()),
  };
  let addr = match peer.addr() {
    Ok(a) => a,
    Err(e) => return unreachable(&e.to_string()),
  };
  let question = crate::node::Question::CallTool(CallTool {
    thread: thread.to_string(),
    repo: root.repo().to_string(),
    server: call.server.to_owned(),
    tool: call.tool.to_owned(),
    input: call.input.to_string(),
  });
  let Ok(bytes) = postcard::to_stdvec(&question) else {
    return ToolOutcome::err("the call could not be encoded");
  };
  let asked = match s.net.ask_within(addr, bytes, CONNECT).await {
    Ok(a) => a,
    Err(stitch_net::AskError::Unreachable(why)) => return unreachable(&why),
    Err(stitch_net::AskError::Failed(e)) => {
      return ToolOutcome::err(format!(
        "the call to {} on {} failed ({e}); it may or may not have run there",
        call.name, call.node
      ));
    }
  };
  match postcard::from_bytes::<Reply>(&asked) {
    Ok(Reply::ToolDone { ok, output, images }) => {
      let scratch = s.data.join("downloads");
      let mut out = ToolOutcome {
        ok,
        output,
        images: Vec::new(),
        spent: None,
        attachments: Vec::new(),
        blobs: Vec::new(),
        generated: Vec::new(),
      };
      for (media_type, bytes) in images {
        let ext = media_type.rsplit('/').next().unwrap_or("bin").to_owned();
        match crate::mcp::save(&scratch, &bytes, &ext) {
          Ok(path) => out.images.push(ToolImage { path, media_type }),
          Err(e) => out
            .output
            .push_str(&format!("\n[an image could not be kept: {e}]")),
        }
      }
      out
    }
    Ok(Reply::Error(e)) => ToolOutcome::err(format!("{}: {e}", call.node)),
    Ok(other) => ToolOutcome::err(format!(
      "{}: an answer of another kind: {other:?}",
      call.node
    )),
    Err(e) => ToolOutcome::err(format!("{}'s answer: {e}", call.node)),
  }
}

/// The repository of a call's thread here: an approval is asked there,
/// where the thread's clients see it. One this node does not hold yet is
/// an error, not a question in another repository that nobody would see.
fn root_for(s: &Shared, repo: &str) -> Result<RecordRoot> {
  let id: stitch_records::RepoId = repo
    .parse()
    .map_err(|e| Error::Config(format!("repository {repo}: {e}")))?;
  s.state().root(&id).ok_or_else(|| {
    Error::Config(format!(
      "the thread's repository {repo} is not on {} yet: sync it there first (`stitch sync`), \
       then call again",
      s.state().node
    ))
  })
}

/// Counts a call run here for another node while it runs (`Shared::busy`).
struct Running<'a>(&'a Shared);

impl<'a> Running<'a> {
  fn new(s: &'a Shared) -> Running<'a> {
    s.remote_calls.fetch_add(1, Ordering::SeqCst);
    Running(s)
  }
}

impl Drop for Running<'_> {
  fn drop(&mut self) {
    self.0.remote_calls.fetch_sub(1, Ordering::SeqCst);
  }
}

/// Runs a call of `server`'s `tool` for a thread of another node, under
/// this node's rules: the approval, when one is needed, is asked in the
/// thread's repository and synced to `asker` at once.
async fn run_here(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  server: &str,
  tool: &str,
  input: &Value,
  asker: ForPeer,
) -> std::result::Result<ToolOutcome, String> {
  let settings = s.mcp_settings();
  match settings.servers.get(server) {
    Some(cfg) if cfg.share == Some(true) => {}
    Some(_) => return Err(format!("MCP server {server} is not shared by this node")),
    None => return Err(format!("no MCP server {server} on this node")),
  }
  s.mcp.configure(&settings.servers);
  let mut tools = crate::mcp::McpTools::new(
    &s.mcp,
    &[Entry::on(&format!("{server}__*"))],
    s.data.join("downloads"),
  );
  // Yolo only: in auto mode no guard judges a peer's call here (it has no
  // thread of this node's), so the rules ask.
  // Where the thread works, its place's mode (design §2.28).
  tools.yolo = s.approval_of_thread(thread) == "allow";
  // What "always" answers granted where the thread works, here.
  if let Ok((_, place)) = s.thread_place(thread) {
    tools.live = s.live(&place);
  }
  tools.approver = Some(crate::detached::headless_for(
    s.clone(),
    root.clone(),
    Some(asker),
  ));
  tools.prepare().await;
  // By the name received: two names may read alike once namespaced.
  let name = tools
    .name_of(server, tool)
    .ok_or_else(|| format!("MCP server {server} offers no tool {tool} here"))?;
  Ok(tools.call(thread, &name, input).await)
}

/// Answers `asker`'s `CallTool` question.
pub(crate) async fn answer(s: &Arc<Shared>, asker: &str, call: CallTool) -> Reply {
  if s.mcp.is_closed() {
    return Reply::Error("the node is stopping; nothing was run".into());
  }
  let _running = Running::new(s);
  let run = async {
    let thread: RecordId = call.thread.parse().map_err(|e| format!("{e}"))?;
    let input: Value = serde_json::from_str(&call.input).map_err(|e| e.to_string())?;
    let root = root_for(s, &call.repo).map_err(|e| e.to_string())?;
    run_here(
      s,
      &root,
      &thread,
      &call.server,
      &call.tool,
      &input,
      ForPeer {
        peer: asker.to_owned(),
        withdrawn: "withdrawn: the caller is gone",
      },
    )
    .await
  };
  match run.await {
    Ok(out) => Reply::ToolDone {
      ok: out.ok,
      // Redacted with what this node holds, before it leaves it.
      output: s.vault.redact(&out.output),
      images: out
        .images
        .iter()
        .filter_map(|i| Some((i.media_type.clone(), std::fs::read(&i.path).ok()?)))
        .collect(),
    },
    Err(e) => Reply::Error(e),
  }
}

/// Writes a `call` record for `call`, in the thread's repository, and lets
/// its node know (when it can be reached).
fn queue(
  s: &Arc<Shared>,
  root: &RecordRoot,
  thread: &RecordId,
  call: RemoteCall<'_>,
) -> Result<RecordId> {
  let node = s.state().node.clone();
  let draft = Draft::new(kinds::CALL, Author::node(node.clone()))?
    .field("node", call.node)?
    .field("server", call.server)?
    .field("tool", call.tool)?
    .field("input", call.input.to_string())?
    .reference("thread", Ref::to(thread.clone()))?
    .reference("reports-to", Ref::to(thread.clone()))?
    .body(format!("# {} on {}, queued\n", call.name, call.node));
  let record = root.publish(&node, draft)?;
  s.state().index.lock().expect("index lock").refresh(root)?;
  let (s, to) = (s.clone(), call.node.to_owned());
  tokio::spawn(async move {
    crate::node::sync_all(&s, Some(&to)).await;
  });
  Ok(record.id)
}

/// Takes the queued calls for this node that nobody took: each is claimed,
/// run and answered with a report to its thread. A call this node claimed
/// that has no report and is not running (the node stopped meanwhile) is
/// reported as interrupted, not run again.
pub(crate) fn claim_calls(s: &Arc<Shared>) {
  if s.mcp.is_closed() {
    return;
  }
  let me = s.state().node.to_string();
  let calls: Vec<(stitch_store::IndexedRecord, bool)> = {
    let index = s.state().index.lock().expect("index lock");
    index
      .records(Some(kinds::CALL))
      .unwrap_or_default()
      .into_iter()
      .filter(|c| c.fields.get("node").and_then(Value::as_str) == Some(me.as_str()))
      .filter(|c| {
        index
          .sources(kinds::REPORT, "call", &c.id, None)
          .is_ok_and(|r| r.is_empty())
      })
      .map(|c| {
        let claimed = index
          .sources(kinds::CLAIM, "claims", &c.id, None)
          .is_ok_and(|cl| !cl.is_empty());
        (c, claimed)
      })
      .collect()
  };
  for (c, claimed) in calls {
    if !s
      .detached
      .claimed
      .lock()
      .expect("claimed lock")
      .insert(c.id.to_string())
    {
      continue; // running here
    }
    let s = s.clone();
    tokio::spawn(async move {
      if let Err(e) = take(&s, &c, claimed).await {
        eprintln!("call {}: {e}", c.id);
      }
    });
  }
}

/// Takes the queued call `c`: claims it, runs it, reports. What fails
/// once its repository and record are read is reported as `failed`.
async fn take(s: &Arc<Shared>, c: &stitch_store::IndexedRecord, claimed: bool) -> Result<()> {
  let state = s.state();
  let root = state
    .root(&c.repo)
    .ok_or_else(|| Error::Config(format!("repository {} is not held here", c.repo)))?;
  let record = root.read_record(&c.id)?;
  let thread = record.refs.first("thread").map(|r| r.id.clone());
  let Some(to) = record
    .refs
    .first("reports-to")
    .map(|r| r.id.clone())
    .or_else(|| thread.clone())
  else {
    return Err(Error::Config("a call without its thread".into()));
  };
  let server = record.field_str("server").unwrap_or("").to_owned();
  let tool = record.field_str("tool").unwrap_or("").to_owned();
  let asker = c.node.clone();
  let (outcome, text) = if claimed {
    (
      "interrupted",
      format!(
        "{server}__{tool} was taken by {} but the node stopped before it answered; it may or \
         may not have run.",
        state.node
      ),
    )
  } else {
    let _running = Running::new(s);
    let run = async {
      let thread = thread.ok_or_else(|| Error::Config("a call without its thread".into()))?;
      let claim = Draft::new(kinds::CLAIM, Author::node(state.node.clone()))?
        .reference("claims", Ref::to(c.id.clone()))?
        .body(format!("# taken by {}\n", state.node));
      root.publish(&state.node, claim)?;
      state.index.lock().expect("index lock").refresh(&root)?;
      let input: Value = serde_json::from_str(record.field_str("input").unwrap_or("{}"))
        .unwrap_or_else(|_| serde_json::json!({}));
      Ok::<_, Error>(
        run_here(
          s,
          &root,
          &thread,
          &server,
          &tool,
          &input,
          ForPeer {
            peer: asker.clone(),
            withdrawn: "withdrawn: the queued call was interrupted",
          },
        )
        .await,
      )
    };
    let ran = run.await;
    if s.mcp.is_closed() {
      // The node is stopping: claimed and unreported, it is reported as
      // interrupted when the node starts again.
      return Ok(());
    }
    match ran {
      Ok(Ok(out)) => {
        let mut text = out.output;
        if !out.images.is_empty() {
          text.push_str(&format!(
            "\n\n[{} image(s) the call returned are not carried by a report]",
            out.images.len()
          ));
        }
        (if out.ok { "done" } else { "failed" }, text)
      }
      Ok(Err(e)) => ("failed", e),
      Err(e) => (
        "failed",
        format!("{server}__{tool} was not run on {}: {e}", state.node),
      ),
    }
  };
  let report = Draft::new(kinds::REPORT, Author::node(state.node.clone()))?
    .field("outcome", outcome)?
    .reference("call", Ref::to(c.id.clone()))?
    .reference("reports-to", Ref::to(to))?
    .body(text);
  root.publish(&state.node, report)?;
  state.index.lock().expect("index lock").refresh(&root)?;
  crate::node::sync_all(s, Some(&asker)).await;
  Ok(())
}
