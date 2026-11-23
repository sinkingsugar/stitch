//! `Decide`: Otter code asking the decision model closed questions about
//! a text (design §2.29), as the thread's `decide` tool: through
//! `otter_run`'s tool calls (`super::calls`), so each call is the thread's
//! own, priced to it, and within an app's grant.
//!
//! `"the text" | Decide({spam: "Is it spam?" lang: {choice: "Which?"
//! options: ["en" "it"]}})` outputs `{answers: {spam: {type: "yes_no" p:
//! 0.93 …} lang: {choice: "it" p: 0.88 …}} calibrated: true model: …}`.

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
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::calls::{Call, ToolCalls};

pub(crate) const NAME: &str = "Decide";

static PARAMS: &[ParamDecl] = &[ParamDecl {
  name: "questions",
  help: "The questions by key: a String is a yes/no question; `{choice: \"…?\" options: [\"a\" \
         \"b\"]}` (or options as a table of when each applies) picks one of up to 255; `{score: \
         \"…?\" levels: [\"low\" \"mid\" \"high\"]}` rates, lowest first. A literal, or a Table \
         variable.",
  forms: Forms::LITERAL.or(Forms::VARIABLE),
  types: &[TypeName::Table],
  requirement: Requirement::Required,
  ty: None,
}];

const SUMMARY: &str = "Answers closed questions about the input text with a decision model: \
  yes/no, a choice or a score, each with a calibrated probability, in one call.";

const HELP: &str = "`text | Decide({spam: \"Is it spam?\"})` outputs `{answers: {spam: {type: \
  \"yes_no\" p: 0.93 confidence: 0.93}} calibrated: true model: \"gemini/gemini-decision-1\" ms: \
  210}`; a choice's answer has `choice`, its `p` and `probabilities`, a score's `score` (the \
  probability-weighted level, 0 for the first) and `probabilities`. About 200 ms and $0.0001 a \
  call, priced to the thread. `calibrated: false`: the decision model was unavailable and a \
  general model guessed. A failed call fails the shard, so `Maybe` catches it. Only in \
  otter_run (it is the thread's `decide` tool): elsewhere it fails.";

const fn desc(effects: Effects) -> ShardDesc {
  ShardDesc {
    name: NAME,
    version: 1,
    summary: SUMMARY,
    help: HELP,
    params: Params::Declared(PARAMS),
    input: InputDesc::Types(&[TypeName::String]),
    output: OutputDesc::Fixed(TypeName::Table),
    targets: Targets::NativeOnly,
    aliases: &[],
    effects,
    lifetime: Lifetime::Stateless,
  }
}

/// The tool's input for a text and the questions (as JSON).
pub(crate) fn tool_input(text: &str, questions: Value) -> Value {
  json!({"input": text, "questions": questions})
}

/// `Decide`. `APPROVING`: no effects of its own (`otter_run`'s approval:
/// the `decide` tool needs none); never run.
struct Decide<const APPROVING: bool>;

impl<const APPROVING: bool> AsyncShard for Decide<APPROVING> {
  type Compiled = Operand;
  type Op = IoTask;
  const DESC: ShardDesc = desc(if APPROVING {
    Effects::NONE
  } else {
    Effects::IO_WAIT
  });

  fn compose(args: &Args, ctx: &mut ComposeCtx<'_>) -> Result<Composed<Operand>> {
    let (questions, _) = Operand::compose_arg(args, "questions", NAME, ctx)?;
    Ok(Composed {
      compiled: questions,
      output: Type::any_table(),
    })
  }

  fn start(questions: &Operand, ctx: &mut impl LeafCtx, input: &Var) -> Result<IoTask> {
    let Some(text) = input.as_str().map(str::to_owned) else {
      return Err(Error::Activation(format!(
        "{NAME}: the input is not a String"
      )));
    };
    let Some(calls) = ctx.services().get_arc::<ToolCalls>() else {
      return Err(Error::Activation(format!(
        "{NAME}: no decisions here: Decide works in otter_run, as the thread's decide tool; a \
         skill's tool or a schedule's check has none"
      )));
    };
    let questions = otter_core::json::write(&questions.get(ctx))
      .ok()
      .and_then(|t| serde_json::from_str::<Value>(&t).ok())
      .ok_or_else(|| Error::Activation(format!("{NAME}: the questions are not JSON")))?;
    let input = tool_input(&text, questions);
    Ok(otter_io::runtime::spawn(move |cancel| async move {
      let (reply, answer) = oneshot::channel();
      calls
        .tx
        .send(Call {
          name: crate::decide::tool::NAME.into(),
          input,
          reply,
        })
        .map_err(|_| format!("{NAME}: otter_run ended"))?;
      tokio::select! {
        a = answer => match a {
          Ok(Ok(text)) => otter_core::json::parse(&text).map_err(|e| format!("{NAME}: {e}")),
          Ok(Err(why)) => Err(format!("{NAME}: {why}")),
          Err(_) => Err(format!("{NAME}: the call was dropped")),
        },
        () = cancel.cancelled() => Err(format!("{NAME}: cancelled")),
      }
    }))
  }
}

pub(crate) static DECIDE: ShardType = async_type::<Decide<false>>();
pub(crate) static DECIDE_APPROVING: ShardType = async_type::<Decide<true>>();
