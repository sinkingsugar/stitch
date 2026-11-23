//! `Tool.Call`: Otter code calling the thread's tools (design §2.17).
//!
//! - **The shard** takes the arguments (a table, or none) as its input and
//!   the tool's name as its parameter, and outputs the tool's text result.
//!   A failed call fails the shard with what the tool said, so `Maybe`
//!   catches it. Images are left out, with a note.
//! - **Dispatch:** `otter_run` inserts a [`ToolCalls`] service into its
//!   mesh: the sending end of a channel. The shard sends each call on it
//!   and waits for the answer; `otter_run`'s own call, still running in
//!   the thread's tool dispatcher, serves the channel ([`serve`]) by
//!   calling the dispatcher, so each call is the thread's own: its tool
//!   and skill sets, its approver, its rules. Where no service is (a
//!   skill's tool, a schedule's check), the shard fails: no tools there.
//! - **Approvals stay per tool:** `otter_run` never asks because of
//!   `Tool.Call` (its approval reads the effects compiled against
//!   [`APPROVAL_CATALOG`]'s `Tool.Call`, which has none); each call asks
//!   as its tool does when the model calls it, the question marked as
//!   coming from a script ([`SCRIPT`]).
//! - **Refused:** `otter_run` itself and what weaves threads or wakes
//!   later ([`REFUSED`]).
//! - **Stopping:** a call the script stops waiting for (cancelled, out of
//!   time) closes its answer's channel, and the call is dropped, as an
//!   interrupted call of the model's is; `otter_run`'s own call dropped
//!   drops them all. A call still waiting on the user's approval when the
//!   run runs out of time is named in the run's error ([`Waiting`]): the
//!   question cannot outlast the run, so the model learns to call the
//!   tool directly, where it waits for the user.

use std::future::Future;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use otter_core::args::Args;
use otter_core::compose::ComposeCtx;
use otter_core::describe::{
  Forms, InputDesc, OutputDesc, ParamDecl, Params, Requirement, ShardDesc, Targets, TypeName,
};
use otter_core::instance::LeafCtx;
use otter_core::shards::Operand;
use otter_core::shards::async_shard::{AsyncShard, async_type};
use otter_core::signature::{Effects, Lifetime};
use otter_core::{Composed, Error, Result, ShardType, Type, Var};
use otter_io::IoTask;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::tools::{BoxFuture, ToolOutcome};

/// The shard's name.
pub(crate) const NAME: &str = "Tool.Call";

/// Tools a script may not call: `otter_run` (no recursion), and those that
/// weave threads or wake this one later: their work outlives the script
/// and belongs to the model's own turn.
pub(crate) const REFUSED: [&str; 9] = [
  "otter_run",
  "spawn",
  "self_improve",
  "send",
  "wait",
  "cancel",
  "wake_at",
  "schedule",
  "subscribe",
];

tokio::task_local! {
  /// Set while a script's call runs: questions it asks say so
  /// (`ApprovalRequest::new`), and the tools waiting on one are kept.
  pub(crate) static SCRIPT: Waiting;
}

/// The tools of one `otter_run` whose calls wait on the user's approval:
/// a name stays when its wait is dropped unanswered (the run out of
/// time), so the run's error can say why it took so long.
pub(crate) type Waiting = Arc<std::sync::Mutex<Vec<String>>>;

/// What a question asked by a script's call starts with.
pub(crate) const FROM_SCRIPT: &str = "[from otter_run, Tool.Call] ";

/// Whether the current task runs a script's call.
pub(crate) fn in_script() -> bool {
  SCRIPT.try_with(|_| ()).is_ok()
}

/// Runs `asking` (a call's wait on the user's approval), its tool kept in
/// the script's [`Waiting`] until answered, when it runs a script's call.
pub(crate) async fn awaiting_approval<T>(name: &str, asking: impl Future<Output = T>) -> T {
  let Ok(waiting) = SCRIPT.try_with(Arc::clone) else {
    return asking.await;
  };
  waiting.lock().unwrap().push(name.to_owned());
  let answer = asking.await;
  let mut w = waiting.lock().unwrap();
  if let Some(i) = w.iter().position(|n| n == name) {
    w.remove(i);
  }
  answer
}

/// What an `otter_run` out of time adds when calls of its script were
/// still waiting on the user's approval.
pub(crate) fn still_waiting(waiting: &Waiting) -> Option<String> {
  let mut names = waiting.lock().unwrap().clone();
  names.sort();
  names.dedup();
  if names.is_empty() {
    return None;
  }
  let list = names.join(", ");
  let verb = if names.len() == 1 { "was" } else { "were" };
  Some(format!(
    "{list} {verb} still waiting for the user's approval (the question is withdrawn): a script's \
     Tool.Call cannot wait past otter_run's time limit. Call {list} directly, where its approval \
     waits for the user, or have it allowed (`[otter] allow`)."
  ))
}

/// One call a script makes.
pub(crate) struct Call {
  pub name: String,
  pub input: Value,
  /// The tool's text, or why it failed.
  pub reply: oneshot::Sender<std::result::Result<String, String>>,
}

/// The mesh service through which `Tool.Call` reaches the thread's tools.
pub(crate) struct ToolCalls {
  pub(super) tx: mpsc::UnboundedSender<Call>,
}

impl ToolCalls {
  /// The service, and the calls it receives, for [`serve`].
  pub(crate) fn new() -> (Arc<ToolCalls>, mpsc::UnboundedReceiver<Call>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(ToolCalls { tx }), rx)
  }
}

/// Calls a tool of the thread by name, as the model would.
pub(crate) type Dispatch<'a> =
  dyn Fn(String, Value) -> BoxFuture<'a, ToolOutcome> + Send + Sync + 'a;

/// Runs `work` (the script) while serving its calls through `dispatch`:
/// several at once when the script makes them so. When `work` ends, calls
/// still in flight are dropped. Those waiting on the user's approval when
/// dropped stay in `waiting`.
pub(crate) async fn serve<T>(
  work: impl Future<Output = T>,
  mut calls: mpsc::UnboundedReceiver<Call>,
  dispatch: &Dispatch<'_>,
  waiting: &Waiting,
) -> T {
  let mut work = std::pin::pin!(work);
  let mut inflight = FuturesUnordered::new();
  loop {
    tokio::select! {
      done = &mut work => return done,
      Some(call) = calls.recv() => inflight.push(answer(call, dispatch, waiting.clone())),
      Some(()) = inflight.next(), if !inflight.is_empty() => {}
    }
  }
}

/// One call: refused, or run with its questions marked, until the script
/// stops waiting for it.
async fn answer(call: Call, dispatch: &Dispatch<'_>, waiting: Waiting) {
  let Call {
    name,
    input,
    mut reply,
  } = call;
  if REFUSED.contains(&name.as_str()) {
    let _ = reply.send(Err(format!(
      "{name} cannot be called from a script (Tool.Call): call it directly. A script may call \
       any other tool the thread has, but not otter_run, nor spawn, self_improve, send, wait, cancel, wake_at \
       or schedule"
    )));
    return;
  }
  let mut out = tokio::select! {
    out = SCRIPT.scope(waiting, dispatch(name.clone(), input)) => out,
    () = reply.closed() => return,
  };
  // What it spent counts with `otter_run`'s own call (`tools::spending`).
  if let Some(s) = out.spent.take() {
    crate::tools::also_spent(s);
  }
  let _ = reply.send(text_of(&name, out));
}

/// What the script gets of an outcome: its text, without the note that
/// the user approved it, and a note for the images it cannot have.
fn text_of(name: &str, out: ToolOutcome) -> std::result::Result<String, String> {
  let mut text = out
    .output
    .strip_prefix("(approved by the user)\n")
    .map(str::to_owned)
    .unwrap_or(out.output);
  let images = out.images.len() + out.generated.len();
  if images > 0 {
    text.push_str(&format!(
      "\n[{images} image{} not shown: call {name} directly to see {}]",
      if images == 1 { "" } else { "s" },
      if images == 1 { "it" } else { "them" }
    ));
  }
  if out.ok { Ok(text) } else { Err(text) }
}

static PARAMS: &[ParamDecl] = &[ParamDecl {
  name: "tool",
  help: "The tool's name, as the model's tool list or a skill gives it (`youtube_video`, \
         `shell_run`, `github__get_issue`): a literal, or a String variable.",
  forms: Forms::LITERAL.or(Forms::VARIABLE),
  types: &[TypeName::String],
  requirement: Requirement::Required,
  ty: None,
}];

const SUMMARY: &str = "Calls one of the thread's tools (any tool the model can call: built-in, \
  MCP, a skill's, lazy ones too) with its input as the arguments, and outputs the tool's text \
  result.";

const HELP: &str = "`{video: \"dQw4w9WgXcQ\"} | Tool.Call(\"youtube_video\")`: the input is \
  the tool's arguments, a table as its schema says (none for no arguments); the output is what \
  the tool returned, as a String (JSON results are text: `FromJson` reads them). A failed call \
  fails the shard with the tool's message, so `Maybe` catches it. Each call is approved as \
  the tool is when the model calls it (the rules, yolo, `[otter] allow`, \"always\" answers, \
  read-only threads), the question saying it comes from otter_run; otter_run itself never \
  asks because of Tool.Call. Images a tool returns are left out, with a note. Not callable: \
  otter_run, spawn, self_improve, send, wait, cancel, wake_at, schedule. Only in otter_run: elsewhere (a \
  skill's tool, a schedule's check) it fails, there being no tools there. Calls on several \
  wires run at once.";

const fn desc(effects: Effects) -> ShardDesc {
  ShardDesc {
    name: NAME,
    version: 1,
    summary: SUMMARY,
    help: HELP,
    params: Params::Declared(PARAMS),
    input: InputDesc::Types(&[TypeName::Table, TypeName::None]),
    output: OutputDesc::Fixed(TypeName::String),
    targets: Targets::NativeOnly,
    aliases: &[],
    effects,
    lifetime: Lifetime::Stateless,
  }
}

/// `Tool.Call`. `APPROVING`: the form `otter_run`'s approval compiles
/// against, with no effects of its own, since each call is approved as its
/// tool is; never run.
struct ToolCall<const APPROVING: bool>;

impl<const APPROVING: bool> AsyncShard for ToolCall<APPROVING> {
  type Compiled = Operand;
  type Op = IoTask;
  const DESC: ShardDesc = desc(if APPROVING {
    Effects::NONE
  } else {
    Effects::IO_WAIT
  });

  fn compose(args: &Args, ctx: &mut ComposeCtx<'_>) -> Result<Composed<Operand>> {
    let (tool, _) = Operand::compose_arg(args, "tool", NAME, ctx)?;
    Ok(Composed {
      compiled: tool,
      output: Type::string(),
    })
  }

  fn start(tool: &Operand, ctx: &mut impl LeafCtx, input: &Var) -> Result<IoTask> {
    let Some(name) = tool.get(ctx).as_str().map(str::to_owned) else {
      return Err(Error::Activation(format!("{NAME}: tool is not a String")));
    };
    let Some(calls) = ctx.services().get_arc::<ToolCalls>() else {
      return Err(Error::Activation(format!(
        "{NAME}({name}): no tools here: Tool.Call works in otter_run, which calls the thread's \
         tools; a skill's tool or a schedule's check has none"
      )));
    };
    let input = match input {
      Var::None => Value::Object(Default::default()),
      v => {
        let text = otter_core::json::write(v)
          .map_err(|e| Error::Activation(format!("{NAME}({name}): the arguments: {e}")))?;
        serde_json::from_str(&text)
          .map_err(|e| Error::Activation(format!("{NAME}({name}): the arguments: {e}")))?
      }
    };
    Ok(otter_io::runtime::spawn(move |cancel| async move {
      let (reply, answer) = oneshot::channel();
      calls
        .tx
        .send(Call {
          name: name.clone(),
          input,
          reply,
        })
        .map_err(|_| format!("{NAME}({name}): the tools are gone (otter_run ended)"))?;
      tokio::select! {
        a = answer => match a {
          Ok(Ok(text)) => Ok(Var::string(text.as_str())),
          Ok(Err(why)) => Err(format!("{name}: {why}")),
          Err(_) => Err(format!("{NAME}({name}): the call was dropped")),
        },
        () = cancel.cancelled() => Err(format!("{NAME}({name}): cancelled")),
      }
    }))
  }
}

static TOOL_CALL: ShardType = async_type::<ToolCall<false>>();
static TOOL_CALL_APPROVING: ShardType = async_type::<ToolCall<true>>();

/// The shard as scripts run it.
pub(crate) static CATALOG: &[&ShardType] = &[&TOOL_CALL, &super::decide::DECIDE];

/// The shard as `otter_run`'s approval compiles it: no effects of its own.
pub(crate) static APPROVAL_CATALOG: &[&ShardType] =
  &[&TOOL_CALL_APPROVING, &super::decide::DECIDE_APPROVING];

#[cfg(test)]
mod tests {
  use super::*;
  use crate::tools::ToolImage;

  #[test]
  fn a_script_gets_the_text_without_images_or_the_approval_note() {
    let mut out = ToolOutcome::ok("(approved by the user)\nseen");
    out.images.push(ToolImage {
      path: "/x.png".into(),
      media_type: "image/png".into(),
    });
    assert_eq!(
      text_of("view_file", out.clone()),
      Ok("seen\n[1 image not shown: call view_file directly to see it]".into())
    );
    out.images.push(out.images[0].clone());
    out.ok = false;
    assert_eq!(
      text_of("view_file", out),
      Err("seen\n[2 images not shown: call view_file directly to see them]".into())
    );
    assert_eq!(text_of("x", ToolOutcome::ok("plain")), Ok("plain".into()));
  }
}
