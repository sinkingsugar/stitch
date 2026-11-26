//! A thread the user had run on another node (design §2.7, "Continue on
//! another node"): a chat typed into it here is passed on to that node as
//! an assignment (`chat = true`, `interject = true`: woven into the loop
//! running there, else a turn of its own), and this node follows that
//! turn live (`Question::Follow`, a long poll on its `live::Live`),
//! streaming it to the client as a chat here would stream, until it ends;
//! then the thread is synced from there, so the transcript has it. The
//! phone becomes a remote for a thread that runs on the Linux box.

use std::sync::Arc;
use std::time::Duration;

use stitch_records::RecordId;
use tokio::sync::mpsc;

use crate::api::Reply;
use crate::chat::{self, ChatContext};
use crate::live::{Followed, LiveEvent};
use crate::node::{Question, Shared};
use crate::{Error, Result};

/// How long the node that runs it holds a follower's question when
/// nothing comes.
const HOLD: Duration = Duration::from_secs(20);
/// How long reaching it may take.
const CONNECT: Duration = Duration::from_secs(8);
/// How long a message may wait to be taken there before the chat ends
/// saying so (it is taken all the same, once that node has it).
const UNTAKEN: Duration = Duration::from_secs(60);
/// Asks in a row that may fail before the chat ends (the turn goes on
/// there).
const TRIES: usize = 3;

/// Why a question was not answered.
enum Unasked {
  /// Not reached (offline, asleep, no address).
  Unreachable,
  /// A node from before the question.
  Unknown,
  Failed(String),
}

/// Asks peer `node` a question, reaching it within `CONNECT`.
async fn ask(s: &Arc<Shared>, node: &str, q: &Question) -> std::result::Result<Reply, Unasked> {
  ask_how(s, node, q, None).await
}

/// How long a question that may be asked twice waits for its answer on a
/// kept connection before it is asked again on a fresh one.
const QUICK_REPLY: Duration = Duration::from_secs(6);

/// Asks peer `node` a question, reaching it within `CONNECT`; `again`: one
/// that may be asked twice and is answered at once, its answer waited for
/// that long on a kept connection before a fresh one is dialed.
async fn ask_how(
  s: &Arc<Shared>,
  node: &str,
  q: &Question,
  again: Option<Duration>,
) -> std::result::Result<Reply, Unasked> {
  let state = s.state().clone();
  let name = node.to_owned();
  let peer = tokio::task::spawn_blocking(move || state.find_peer(&name))
    .await
    .ok()
    .and_then(|r| r.ok())
    .flatten()
    .ok_or(Unasked::Unreachable)?;
  let addr = peer.addr().map_err(|_| Unasked::Unreachable)?;
  let question = postcard::to_stdvec(q).map_err(|e| Unasked::Failed(e.to_string()))?;
  let asked = match again {
    None => s.net.ask_within(addr, question, CONNECT).await,
    Some(reply) => s.net.ask_again_within(addr, question, CONNECT, reply).await,
  }
  .map_err(|e| match e {
    stitch_net::AskError::Unreachable(_) => Unasked::Unreachable,
    stitch_net::AskError::Failed(e) => Unasked::Failed(e.to_string()),
  })?;
  match postcard::from_bytes::<Reply>(&asked) {
    Ok(Reply::Error(e)) if e.starts_with("a question this node does not know") => {
      Err(Unasked::Unknown)
    }
    Ok(r) => Ok(r),
    Err(e) => Err(Unasked::Failed(format!("{UNREADABLE}: {e}"))),
  }
}

/// Why a peer's answer was not read: it does not decode as a `Reply`.
const UNREADABLE: &str = "an answer this build cannot read (another build?)";

async fn follow(
  s: &Arc<Shared>,
  node: &str,
  thread: &RecordId,
  after: Option<u64>,
  assignment: Option<&RecordId>,
) -> std::result::Result<Followed, Unasked> {
  let q = Question::Follow {
    thread: thread.to_string(),
    after,
    assignment: assignment.map(|a| a.to_string()),
  };
  match ask(s, node, &q).await? {
    Reply::Lines(lines) => lines
      .first()
      .and_then(|l| serde_json::from_str(l).ok())
      .ok_or_else(|| Unasked::Failed("an answer it could not read".into())),
    Reply::Error(e) => Err(Unasked::Failed(e)),
    other => Err(Unasked::Failed(format!("unexpected answer: {other:?}"))),
  }
}

/// A follower's question, answered (`Question::Follow`): what came since
/// `after`, gathered for a moment once something came, so a streaming
/// turn is not asked for once per token.
pub(crate) async fn answer_follow(
  s: &Arc<Shared>,
  thread: &str,
  after: Option<u64>,
  assignment: Option<&str>,
) -> Reply {
  let f = follow_here(s, thread, after, assignment).await;
  Reply::Lines(vec![serde_json::to_string(&f).unwrap_or_default()])
}

/// `answer_follow` as the `Followed` itself (the web client's WebSocket
/// streams it, `apps::ws`).
pub(crate) async fn follow_here(
  s: &Arc<Shared>,
  thread: &str,
  after: Option<u64>,
  assignment: Option<&str>,
) -> Followed {
  let mut f = s.live.follow(thread, after, assignment, HOLD).await;
  if after.is_some() && !f.events.is_empty() && !f.done {
    tokio::time::sleep(Duration::from_millis(200)).await;
    f = s
      .live
      .follow(thread, after, assignment, Duration::ZERO)
      .await;
  }
  f
}

/// The step `thread`'s loop runs the calls of here now
/// (`chat::step_in_flight`, this process's loops only).
fn in_flight_here(s: &Shared, thread: &RecordId) -> Option<chat::StepInFlight> {
  chat::step_in_flight(&s.state().index, thread, Some(std::process::id()))
}

fn in_flight_lines(step: Option<chat::StepInFlight>) -> Reply {
  Reply::Lines(
    step
      .and_then(|s| serde_json::to_string(&s).ok())
      .into_iter()
      .collect(),
  )
}

/// `Request::InFlight`: the step whose calls run now on `thread`, read
/// here, else asked of the node that runs the thread (`weave::runner`);
/// none when that node is not reached, or is from before the question.
pub(crate) async fn in_flight(s: &Arc<Shared>, thread: &str) -> Result<Reply> {
  // A thread compacted into another runs as that one.
  let t = crate::fork::current(s, &thread.parse()?);
  if let Some(step) = in_flight_here(s, &t) {
    return Ok(in_flight_lines(Some(step)));
  }
  let started_on = s
    .state()
    .index
    .lock()
    .expect("index lock")
    .record(&t)
    .ok()
    .flatten()
    .map(|r| r.node.to_string());
  let Some(started_on) = started_on else {
    return Ok(in_flight_lines(None));
  };
  let runner = crate::weave::runner(s, &t, &started_on);
  if runner == s.state().node.as_str() {
    return Ok(in_flight_lines(None));
  }
  // Clients ask every few seconds while a thread is open: its node is
  // asked again soon only while a call runs there.
  let now = std::time::Instant::now();
  if IN_FLIGHT_ASKED
    .lock()
    .expect("in-flight lock")
    .get(thread)
    .is_some_and(|until| now < *until)
  {
    return Ok(in_flight_lines(None));
  }
  let (reply, quiet) = match ask(s, &runner, &Question::InFlight(t.to_string())).await {
    Ok(Reply::Lines(lines)) if !lines.is_empty() => (Reply::Lines(lines), Duration::ZERO),
    Ok(_) => (in_flight_lines(None), IN_FLIGHT_NONE),
    Err(_) => (in_flight_lines(None), IN_FLIGHT_UNREACHED),
  };
  let mut asked = IN_FLIGHT_ASKED.lock().expect("in-flight lock");
  asked.retain(|_, until| now < *until);
  if !quiet.is_zero() {
    asked.insert(thread.to_owned(), now + quiet);
  }
  Ok(reply)
}

/// When each thread's node may be asked again for its call in flight,
/// after it had none or was not reached.
static IN_FLIGHT_ASKED: std::sync::LazyLock<
  std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);
/// How long a thread whose node said no call runs is not asked again.
const IN_FLIGHT_NONE: Duration = Duration::from_secs(8);
/// How long one whose node was not reached is not asked again.
const IN_FLIGHT_UNREACHED: Duration = Duration::from_secs(60);

/// A peer's `Question::InFlight`, answered from this node's loops.
pub(crate) fn answer_in_flight(s: &Shared, thread: &str) -> Reply {
  in_flight_lines(
    thread
      .parse::<RecordId>()
      .ok()
      .and_then(|t| in_flight_here(s, &t)),
  )
}

/// An action button another node offered (design §2.30): pressed there,
/// where its call runs (`Question::Press`), the answer waited for however
/// long the call takes (its approvals included).
pub(crate) async fn press(
  s: &Arc<Shared>,
  node: &str,
  thread: &RecordId,
  set: &str,
  item: usize,
) -> Result<crate::actions::Pressed> {
  let q = Question::Press {
    thread: thread.to_string(),
    set: set.to_owned(),
    item: item as u32,
  };
  match ask(s, node, &q).await {
    Ok(Reply::Lines(l)) if !l.is_empty() => {
      serde_json::from_str(&l[0]).map_err(|e| Error::Config(format!("{node}: {e}")))
    }
    Ok(Reply::Error(e)) => Err(Error::Config(e)),
    Ok(_) => Err(Error::Config(format!(
      "{node} answered the press with nothing"
    ))),
    Err(Unasked::Unreachable) => Err(Error::Config(format!(
      "these buttons run on {node}, which is not reachable now"
    ))),
    Err(Unasked::Unknown) => Err(Error::Config(format!(
      "these buttons run on {node}, whose Stitch is too old to press them"
    ))),
    Err(Unasked::Failed(e)) => Err(Error::Config(format!("{node}: {e}"))),
  }
}

/// Tells peer `node` how a call it rang went (`Question::Ring`).
pub(crate) async fn ring_outcome(
  s: &Arc<Shared>,
  node: &str,
  call: &str,
  outcome: &str,
) -> Result<()> {
  let q = Question::Ring {
    call: call.to_owned(),
    outcome: outcome.to_owned(),
  };
  // Asked again on a fresh connection when the kept one is dead: a phone
  // woken by the call's push holds connections its peers dropped while it
  // was suspended, and the call's outcome cannot wait for them to time out.
  match ask_how(s, node, &q, Some(QUICK_REPLY)).await {
    Ok(Reply::Error(e)) => Err(Error::Config(e)),
    Ok(_) => Ok(()),
    Err(Unasked::Unreachable) => Err(Error::Config(format!("{node} is not reachable now"))),
    Err(Unasked::Unknown) => Err(Error::Config(format!(
      "{node}'s Stitch is too old to hear of calls"
    ))),
    Err(Unasked::Failed(e)) => Err(Error::Config(format!("{node}: {e}"))),
  }
}

/// A peer's `Question::Press`, run here.
pub(crate) async fn answer_press(s: &Arc<Shared>, thread: &str, set: &str, item: u32) -> Reply {
  let run = async {
    let t: RecordId = thread.parse()?;
    let p = crate::actions::press_here(s, &t, set, item as usize).await?;
    Ok::<_, Error>(serde_json::json!(p).to_string())
  };
  match run.await {
    Ok(line) => Reply::Lines(vec![line]),
    Err(e) => Reply::Error(e.to_string()),
  }
}

/// Why this node runs no programs (`Question::Runs`), so no thread can be
/// set to run on it; `None` when it does (shells start here).
pub(crate) fn no_programs(s: &Shared) -> Option<String> {
  if !stitch_shell::CAN_SPAWN {
    Some("no programs run there (no shells: a phone)".into())
  } else if s.approval() == "off" {
    Some("its shell is off ([shell] approval = \"off\")".into())
  } else {
    None
  }
}

/// Whether peer `node` runs programs, as it says (`Question::Runs`):
/// `Ok(None)` when it cannot be asked (not reached, or a node from before
/// the question), `Err` with why when it does not.
pub(crate) async fn runs_programs(
  s: &Arc<Shared>,
  node: &str,
) -> std::result::Result<Option<()>, String> {
  if node == s.state().node.as_str() {
    return match no_programs(s) {
      None => Ok(Some(())),
      Some(why) => Err(why.replace("there", "here")),
    };
  }
  match reach(s, node).await {
    Reach::Reached {
      runs: Some(Ok(())), ..
    } => Ok(Some(())),
    Reach::Reached {
      runs: Some(Err(why)),
      ..
    } => Err(why),
    _ => Ok(None),
  }
}

/// This node's answer to `Question::Runs`: `yes`, or `no` and why; then
/// what it is (`platform: linux x86_64`) and its default model
/// (`default: <model>`, when it knows it within a moment). A node from
/// before 2026-11-09 answers the first lines alone.
pub(crate) async fn answer_runs(s: &Shared) -> Reply {
  let mut lines = match no_programs(s) {
    None => vec!["yes".to_owned()],
    Some(why) => vec!["no".to_owned(), why],
  };
  lines.push(format!("platform: {}", platform()));
  if let Ok(m) = tokio::time::timeout(
    Duration::from_secs(2),
    crate::modelset::default_model(s, None),
  )
  .await
  {
    lines.push(format!("default: {m}"));
  }
  Reply::Lines(lines)
}

/// What this node runs on: `linux x86_64`.
fn platform() -> String {
  format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Whether a peer was reached, and what it said of itself
/// (`Question::Runs`).
pub(crate) enum Reach {
  /// Not reached within `CONNECT` (offline, asleep, no address).
  Unreached,
  Reached {
    /// Whether it runs programs, or why not; `None`: it did not say (a
    /// node from before the question).
    runs: Option<std::result::Result<(), String>>,
    /// `linux x86_64`, from a node that says it.
    platform: Option<String>,
    /// Its default model, from a node that says it.
    default: Option<String>,
    /// Its answer could not be read as one: it runs a build whose
    /// protocol this one does not speak (or the other way round).
    other_build: bool,
  },
}

/// Asks peer `node` whether it runs programs, which also tells whether it
/// is reached now.
pub(crate) async fn reach(s: &Arc<Shared>, node: &str) -> Reach {
  reached(ask(s, node, &Question::Runs).await)
}

/// What a peer's answer to `Question::Runs` says (`reach`).
fn reached(asked: std::result::Result<Reply, Unasked>) -> Reach {
  let (lines, other_build) = match asked {
    Err(Unasked::Unreachable) => return Reach::Unreached,
    Ok(Reply::Lines(l)) => (l, false),
    Ok(Reply::Error(_)) | Err(Unasked::Unknown) => (Vec::new(), false),
    Err(Unasked::Failed(e)) => (Vec::new(), e.starts_with(UNREADABLE)),
    // An answer to this question is `Lines`: another variant is one this
    // build numbers differently.
    Ok(_) => (Vec::new(), true),
  };
  let said = |key: &str| {
    lines
      .iter()
      .find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_owned()))
  };
  let runs = match lines.first().map(String::as_str) {
    Some("yes") => Some(Ok(())),
    Some("no") => Some(Err(
      lines
        .get(1)
        .filter(|l| !l.starts_with("platform:"))
        .cloned()
        .unwrap_or_else(|| "it runs no programs".into()),
    )),
    _ => None,
  };
  Reach::Reached {
    runs,
    platform: said("platform:"),
    default: said("default:"),
    other_build,
  }
}

/// How long the other nodes have, together, to say whether they could
/// run a spawn whose node was not reached: the spawn's result waits on it.
const ALTERNATIVES: Duration = Duration::from_secs(5);

/// What a spawn asks of the node that runs it, for `alternatives`.
pub(crate) struct Wanted<'a> {
  /// A thread with tools: only a node that runs programs takes it.
  pub tools: bool,
  /// The place asked for (`~/devel/otter`).
  pub place: Option<&'a str>,
  /// The model asked for, `provider/model` (an alias: none).
  pub model: Option<&'a str>,
}

/// The nodes other than `except` reached now that could run a spawn
/// (design §2.7, "A spawn to a node not reached"), asked all at once,
/// `ALTERNATIVES` at most: each as `name (what it is, whether it holds
/// the place, the model it runs)`, this node among them; those that hold
/// the place first. Nodes that run no programs are left out of a spawn
/// with tools, and so is a node that lists the model's provider without
/// the model.
pub(crate) async fn alternatives(s: &Arc<Shared>, except: &str, want: &Wanted<'_>) -> Vec<String> {
  let state = s.state().clone();
  let me = state.node.to_string();
  let peers = tokio::task::spawn_blocking(move || state.known_peers())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();
  let mut names: Vec<String> = peers
    .into_iter()
    .map(|p| p.name)
    .filter(|n| n != except && *n != me)
    .collect();
  if me != except {
    names.insert(0, me.clone());
  }
  let asks = names.into_iter().map(|name| async move {
    tokio::time::timeout(ALTERNATIVES, alternative(s, &name, want))
      .await
      .ok()
      .flatten()
  });
  let mut found: Vec<(bool, String)> = futures_util::future::join_all(asks)
    .await
    .into_iter()
    .flatten()
    .collect();
  // Stable: this node first among equals, then the order peers are known.
  found.sort_by_key(|(holds, _)| !holds);
  found.into_iter().map(|(_, said)| said).collect()
}

/// One node for `alternatives`: whether it holds the place (or there is
/// none to hold), and what is said of it; `None` when it is not reached
/// or could not run it.
async fn alternative(s: &Arc<Shared>, node: &str, want: &Wanted<'_>) -> Option<(bool, String)> {
  let here = node == s.state().node.as_str();
  let (runs, platform, default) = if here {
    let runs = match no_programs(s) {
      None => Some(Ok(())),
      Some(why) => Some(Err(why)),
    };
    let default = tokio::time::timeout(
      Duration::from_secs(2),
      crate::modelset::default_model(s, None),
    )
    .await
    .ok();
    (runs, Some(platform()), default)
  } else {
    match reach(s, node).await {
      Reach::Unreached => return None,
      Reach::Reached {
        runs,
        platform,
        default,
        ..
      } => (runs, platform, default),
    }
  };
  if want.tools && matches!(runs, Some(Err(_))) {
    return None;
  }
  let mut said = Vec::new();
  if here {
    said.push("this node".to_owned());
  }
  said.extend(platform);
  let holds = match want.place {
    None => true,
    Some(p) => {
      let found = if here {
        crate::weave::resolve_place(s, p).map(|r| Some(r.found().label))
      } else {
        match ask(s, node, &Question::Place(p.to_owned())).await {
          Ok(Reply::Lines(l)) => Ok(
            l.first()
              .and_then(|j| serde_json::from_str::<crate::weave::Found>(j).ok())
              .map(|f| f.label),
          ),
          Ok(Reply::Error(e)) => Err(Error::Config(e)),
          _ => Ok(None),
        }
      };
      match found {
        Ok(Some(label)) => {
          said.push(format!("holds {label}"));
          true
        }
        Ok(None) => {
          said.push(format!("{p} unchecked"));
          false
        }
        Err(_) => {
          said.push(format!("no {p} there"));
          false
        }
      }
    }
  };
  match want.model {
    Some(m) => match crate::node::lists_model(s, node, m).await {
      Some(true) => said.push(format!("runs {m}")),
      Some(false) => return None,
      None => said.push(format!("{m} unchecked")),
    },
    None => said.extend(default.map(|d| format!("runs {d}"))),
  }
  Some((holds, format!("{node} ({})", said.join(", "))))
}

/// How long each peer may take to say whether it runs programs, for
/// `runners`: the list waits for all of them, and a client's menu for the
/// list.
const RUNNERS_WITHIN: Duration = Duration::from_secs(3);

/// What `runners` says of a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Runs {
  Yes,
  No,
  /// Not reached to ask (offline), or a node from before the question.
  Unknown,
  /// Reached, but its answer could not be read: a build that speaks
  /// another protocol (`Reach::Reached::other_build`). Not offline: the
  /// fix is updating one of the two.
  OtherBuild,
}

impl Runs {
  /// As `Request::Runners` says it: `yes`, `no`, `unknown`, `other-build`.
  pub(crate) fn word(self) -> &'static str {
    match self {
      Runs::Yes => "yes",
      Runs::No => "no",
      Runs::Unknown => "unknown",
      Runs::OtherBuild => "other-build",
    }
  }

  fn of(reach: Reach) -> Runs {
    match reach {
      Reach::Reached {
        runs: Some(Ok(())), ..
      } => Runs::Yes,
      Reach::Reached {
        runs: Some(Err(_)), ..
      } => Runs::No,
      Reach::Reached {
        other_build: true, ..
      } => Runs::OtherBuild,
      _ => Runs::Unknown,
    }
  }
}

/// The user's other nodes, and whether each runs programs, asked of all at
/// once: where a thread can be set to run. Each is given `RUNNERS_WITHIN`:
/// one that has not answered by then (a kept connection gone dead, a node
/// asleep) is offered as offline instead of holding the whole list, so the
/// others are offered too.
pub(crate) async fn runners(s: &Arc<Shared>) -> Vec<(String, Runs)> {
  let state = s.state().clone();
  let peers = tokio::task::spawn_blocking(move || state.known_peers())
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();
  let asks = peers.into_iter().map(|p| async move {
    let asked = async { Runs::of(reach(s, &p.name).await) };
    let runs = runs_within(asked, RUNNERS_WITHIN).await;
    (p.name, runs)
  });
  futures_util::future::join_all(asks).await
}

/// What a peer says, as `runners` lists it: `Unknown` when it did not
/// answer within `within`.
async fn runs_within(asked: impl std::future::Future<Output = Runs>, within: Duration) -> Runs {
  tokio::time::timeout(within, asked)
    .await
    .unwrap_or(Runs::Unknown)
}

/// What a new chat for another node starts with (`Request::Chat`'s
/// `node`): its first message, the client's tool and skill changes, and
/// the memory chosen before it.
pub(crate) struct NewThread<'a> {
  pub(crate) text: &'a str,
  pub(crate) folder: Option<&'a chat::FolderAt>,
  pub(crate) tools: &'a [crate::toolset::Entry],
  pub(crate) skills: &'a [crate::toolset::Entry],
  pub(crate) memory: Option<bool>,
}

/// A new chat whose first turn runs on `node`, another of the user's nodes
/// that runs programs (design §2.7, "A new chat on another node"): the
/// thread is written here with no step (as a fork is), in `place`, set to
/// run on `node` (`runs-on`), so its first message goes there as any chat
/// into a thread continued there does (`chat`). Refused before anything
/// is written when `node` is not one of the user's nodes or says it runs
/// no programs; one not reached is taken at its word, as `RunOn` takes it.
pub(crate) async fn new_thread_on(
  s: &Arc<Shared>,
  node: &str,
  root: &stitch_store::RecordRoot,
  place: &crate::place::Place,
  new: NewThread<'_>,
) -> Result<RecordId> {
  crate::weave::known(s, node)?;
  if let Err(why) = runs_programs(s, node).await {
    return Err(Error::Config(format!(
      "a new thread cannot run on {node}: {why}. Pick a node with shells, or this device."
    )));
  }
  let state = s.state();
  let id = RecordId::random(jiff::Timestamp::now(), &state.node);
  crate::toolset::write_changes(root, &state.index, &state.node, &id, new.tools, new.skills)?;
  match new.memory.filter(|_| s.memory.available()) {
    Some(on) => {
      s.memory.remember(state, root, &id, on)?;
    }
    None => s.place_memory(place, root, &id)?,
  }
  // This node's prompt with no context of its own: the node that runs the
  // turns adds its instructions and skills with the first message (the
  // thread's context changed from what its prompt says), and says where
  // each turn runs (`where_note`), as for a thread continued there.
  let system = s.system_prompt(false, "");
  let mut draft = stitch_records::Draft::new(
    stitch_records::kinds::THREAD,
    stitch_records::Author::user(state.node.clone()),
  )?
  .body(format!("# {}\n", chat::title_of(new.text)));
  draft = chat::with_system(&state.blobs, draft, &system)?;
  if let Some((n, path)) = new.folder {
    let mut f = toml::Table::new();
    f.insert("node".into(), n.as_str().into());
    f.insert("path".into(), path.as_str().into());
    draft = draft.field("folder", toml::Value::Table(f))?;
  }
  let record = draft.into_record(id.clone(), jiff::Timestamp::now());
  root.publish_record(&record)?;
  state.index.lock().expect("index lock").refresh(root)?;
  crate::meta::set(
    s,
    &id,
    crate::weave::RUNS_ON,
    toml::Value::String(node.to_owned()),
    stitch_records::Author::user(state.node.clone()),
  )?;
  Ok(id)
}

/// Stops the loop running on `thread` here (`Question::Interrupt`).
pub(crate) fn answer_interrupt(s: &Arc<Shared>, thread: &str) -> Reply {
  let steer = s
    .detached
    .steers
    .lock()
    .expect("steers lock")
    .get(thread)
    .cloned();
  match steer {
    Some(st) => {
      st.interrupt();
      Reply::Ok
    }
    None => Reply::Error(format!("no turn runs on {thread} here")),
  }
}

/// What the user typed: the message, its files (in the blob store here),
/// and the model the client picked (`None`: the thread's there).
#[derive(Clone, Copy)]
pub(crate) struct Typed<'a> {
  pub(crate) text: &'a str,
  pub(crate) attachments: &'a [stitch_records::Attachment],
  pub(crate) model: Option<&'a str>,
  /// Said in a voice call: the reply is read aloud here.
  pub(crate) spoken: bool,
  /// Threads and efforts attached (design §2.27): resolved there.
  pub(crate) scopes: &'a [crate::api::ScopeTarget],
  /// A room's messages relayed to a participant that runs there.
  pub(crate) room_turn: Option<&'a stitch_records::Part>,
}

/// A chat on `thread`, which runs on `node` (another): passed on there
/// and followed until its turn ends (module docs). Its `Reply::ChatDone`
/// says `stop = "queued"` when the message waits for that node (not
/// reached, or not taken yet), `elsewhere` when this chat stopped
/// following a turn that goes on there.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn chat(
  s: &Arc<Shared>,
  thread: &RecordId,
  root: &stitch_store::RecordRoot,
  node: &str,
  typed: Typed<'_>,
  tx: &mpsc::UnboundedSender<Reply>,
  steer: &chat::Steer,
) -> Result<Reply> {
  let text = typed.text;
  if text.trim().is_empty() && typed.attachments.is_empty() && typed.scopes.is_empty() {
    return Err(Error::Config("message is empty".into()));
  }
  // The user writing in it takes it over, as a chat here would (§2.7).
  if let Err(e) = crate::weave::lift_budget(s, thread) {
    eprintln!("thread {thread}: its budget was not lifted: {e}");
  }
  let _ = tx.send(Reply::Thread(thread.to_string()));
  // Where its feed stands there, before the message goes.
  let start = follow(s, node, thread, None, None).await;
  let assignment = crate::weave::forward(
    s,
    root,
    node,
    thread,
    text,
    None,
    false,
    true,
    Some(crate::weave::AsChat {
      attachments: typed.attachments,
      model: typed.model,
      spoken: typed.spoken,
      room_turn: typed.room_turn,
    }),
    None,
    typed.scopes,
  )?;
  let mut after = match start {
    Ok(f) => f.next,
    Err(Unasked::Unreachable) => {
      let _ = tx.send(Reply::Note(format!(
        "[{node} is not reachable now: this thread runs there, so your message waits for it \
         (an assignment, synced to it once it is back), and runs there then. Its reply shows \
         here once synced. To go on here instead, bring the thread back (Continue here, \
         `/run-on here`).]"
      )));
      return done(s, thread, node, "queued", false).await;
    }
    Err(Unasked::Unknown) => {
      let _ = tx.send(Reply::Note(format!(
        "[Sent to {node}, which runs this thread; it is of an older version and cannot stream \
         its turn here: its reply shows once synced.]"
      )));
      return done(s, thread, node, "elsewhere", false).await;
    }
    Err(Unasked::Failed(e)) => {
      eprintln!("thread {thread}: following it on {node}: {e}");
      0
    }
  };
  let started = tokio::time::Instant::now();
  let mut failures = 0;
  let mut stopping = false;
  loop {
    let arrival = steer.arrival();
    tokio::pin!(arrival);
    arrival.as_mut().enable();
    // What the client typed meanwhile goes to that turn too.
    for i in steer.take() {
      if i.text.trim().is_empty() && i.attachments.is_empty() {
        continue;
      }
      let sent = crate::weave::forward(
        s,
        root,
        node,
        thread,
        &i.text,
        None,
        false,
        true,
        Some(crate::weave::AsChat {
          attachments: &i.attachments,
          model: typed.model,
          spoken: false,
          room_turn: None,
        }),
        None,
        i.scope.as_ref().map_or(&[][..], |sc| sc.targets.as_slice()),
      );
      match sent {
        Ok(_) => {
          if let Some(id) = i.id {
            let _ = tx.send(Reply::Woven(vec![id]));
          }
        }
        Err(e) => eprintln!("thread {thread}: a message for {node} not passed on: {e}"),
      }
    }
    let asked = tokio::select! {
      f = follow(s, node, thread, Some(after), Some(&assignment)) => f,
      () = steer.stopped(), if !stopping => {
        stopping = true;
        if steer.suspended() {
          // The app going to sleep: the turn goes on there, unfollowed.
          return done(s, thread, node, "elsewhere", false).await;
        }
        let _ = ask(s, node, &Question::Interrupt(thread.to_string())).await;
        continue;
      }
      () = steer.left() => return done(s, thread, node, "elsewhere", false).await,
      _ = &mut arrival => continue,
    };
    let f = match asked {
      Ok(f) => {
        failures = 0;
        f
      }
      Err(Unasked::Failed(e)) if failures + 1 < TRIES => {
        failures += 1;
        eprintln!("thread {thread}: following it on {node}: {e}");
        tokio::time::sleep(Duration::from_secs(1)).await;
        continue;
      }
      Err(Unasked::Unreachable) if failures + 1 < TRIES => {
        failures += 1;
        continue;
      }
      Err(_) => {
        let _ = tx.send(Reply::Note(format!(
          "[Lost touch with {node}: its turn goes on there, and shows here once synced.]"
        )));
        return done(s, thread, node, "elsewhere", false).await;
      }
    };
    if f.gap {
      let _ = tx.send(Reply::Note(format!(
        "[Some of {node}'s turn is not shown here live: the transcript has it once synced.]"
      )));
    }
    for e in f.events {
      let r = match e {
        LiveEvent::Text(t) => Reply::Text(t),
        LiveEvent::Thinking(t) => Reply::Thinking(t),
        LiveEvent::ToolCall(c) => Reply::ToolCall(c),
        LiveEvent::ToolResult { ok, output } => Reply::ToolResult { ok, output },
        LiveEvent::Note(n) => Reply::Note(n),
        LiveEvent::Retry(r) => Reply::Retry(r),
        // Its client switches to the new thread; the turn there is the
        // one this chat follows still (its assignment's).
        LiveEvent::Compacted(c) => {
          let _ = tx.send(Reply::Compacted(c.clone()));
          Reply::Thread(c.to)
        }
        LiveEvent::Started | LiveEvent::Ended => continue,
      };
      let _ = tx.send(r);
    }
    after = f.next;
    if f.done {
      return done(
        s,
        thread,
        node,
        if stopping { "interrupted" } else { "end_turn" },
        true,
      )
      .await;
    }
    if !f.taken && started.elapsed() > UNTAKEN {
      let _ = tx.send(Reply::Note(format!(
        "[{node} has not taken your message yet: it waits there (an assignment) and runs once \
         {node} has it. Its reply shows here once synced.]"
      )));
      return done(s, thread, node, "queued", false).await;
    }
  }
}

/// The chat's end: the thread synced from `node` first when its turn
/// ended there (`synced`), so its transcript here has it.
async fn done(
  s: &Arc<Shared>,
  thread: &RecordId,
  node: &str,
  stop: &str,
  synced: bool,
) -> Result<Reply> {
  let (root, _) = s.thread_place(thread)?;
  if synced {
    crate::weave::reach(s, node, root.repo()).await;
    let _ = s.state().index.lock().expect("index lock").refresh(&root);
  }
  let state = s.state();
  let ctx = ChatContext {
    node: &state.node,
    root: &root,
    index: &state.index,
    blobs: &state.blobs,
  };
  let segment = chat::line_head(&ctx, thread)?
    .map(|h| h.to_string())
    .unwrap_or_default();
  let model = state
    .index
    .lock()
    .expect("index lock")
    .record(thread)?
    .and_then(|r| {
      r.fields
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    })
    .unwrap_or_default();
  let thread_usd = crate::detached::spent_by_chain(s, thread, None).and_then(|(_, u)| u);
  Ok(Reply::ChatDone {
    thread: thread.to_string(),
    segment,
    model,
    stop: stop.into(),
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    usd: None,
    forked: false,
    thread_usd,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A peer that never answers is offered as offline, in time; the others
  /// as they said.
  #[tokio::test]
  async fn a_peer_that_does_not_answer_is_offline_in_time() {
    let within = Duration::from_millis(50);
    let started = std::time::Instant::now();
    let hung = runs_within(std::future::pending(), within).await;
    assert_eq!(hung, Runs::Unknown);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(runs_within(async { Runs::Yes }, within).await, Runs::Yes);
    assert_eq!(runs_within(async { Runs::No }, within).await, Runs::No);
  }

  /// A peer whose answer this build reads as another reply, or cannot
  /// decode at all, runs another build: said so, not offered as offline.
  #[test]
  fn an_answer_of_another_build_is_said_so() {
    let runs = |asked| Runs::of(reached(asked));
    let lines = |l: &[&str]| Ok(Reply::Lines(l.iter().map(|s| s.to_string()).collect()));
    assert_eq!(runs(lines(&["yes", "platform: linux x86_64"])), Runs::Yes);
    assert_eq!(runs(lines(&["no", "its shell is off"])), Runs::No);
    assert_eq!(runs(Err(Unasked::Unreachable)), Runs::Unknown);
    assert_eq!(runs(Err(Unasked::Unknown)), Runs::Unknown);
    assert_eq!(runs(Err(Unasked::Failed("reset".into()))), Runs::Unknown);
    // `Lines` numbered as another build numbers it.
    assert_eq!(runs(Ok(Reply::Rebind("yes".into()))), Runs::OtherBuild);
    // Bytes that are no reply at all.
    let garbled = postcard::from_bytes::<Reply>(&[250, 1])
      .err()
      .map(|e| Unasked::Failed(format!("{UNREADABLE}: {e}")))
      .unwrap();
    assert_eq!(runs(Err(garbled)), Runs::OtherBuild);
  }
}
