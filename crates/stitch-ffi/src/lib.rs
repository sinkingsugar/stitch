//! The node for the iOS and visionOS app (design §5, milestone 4): it runs
//! in the app's process, and Swift reaches it through UniFFI. Requests go
//! to it on an in-process connection, as the CLI's go on the socket; reads
//! (threads, transcripts) come from the store and index, as the TUI's do.
//!
//! Calls that wait (a chat, pairing, sync) are `async` in Swift; their work
//! runs on the node's own runtime, so no Swift thread is held meanwhile.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use stitch_node::api::{Reply, Request, SecretOp, SshOp};
use stitch_node::chat::ChatContext;
use stitch_node::client::Client;
use stitch_node::config::Config;
use stitch_node::keys::{self, Store};
use stitch_node::place::{self, Place};
use stitch_node::{NodeHandle, setup};
use stitch_providers::oauth::Service;
use stitch_records::{BlobHash, NodeName, RecordId};
use stitch_store::{BlobStore, Index, RecordRoot, Role};
use tokio::sync::mpsc;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum StitchError {
  #[error("{message}")]
  Failed { message: String },
}

impl StitchError {
  fn of(e: impl std::fmt::Display) -> StitchError {
    StitchError::Failed {
      message: e.to_string(),
    }
  }
}

type Result<T, E = StitchError> = std::result::Result<T, E>;

/// A thread, as the list shows it.
#[derive(uniffi::Record)]
pub struct ThreadInfo {
  pub id: String,
  pub title: String,
  /// Its last activity, in seconds since the epoch.
  pub at: i64,
  pub pinned: bool,
  /// A detached thread's state (`working`, `blocked`, …).
  pub state: Option<String>,
  /// The project or folder it belongs to; `None`: global.
  pub place: Option<String>,
  /// Its place as `chat` takes it (`folder:~/notes`); `None`: global.
  pub place_key: Option<String>,
  /// Read-only: its shells write only temporary directories and only
  /// read-only commands run (`set_read_only` switches it).
  pub read_only: bool,
  /// The thread above it (`meta::parent`: who spawned it, else whom it
  /// reports to): the list hangs it under that one.
  pub parent: Option<String>,
  /// A spawned thread's goal, its first line: what it is for.
  pub goal: Option<String>,
  /// The threads it was compacted from, oldest first (design §2.15, "One
  /// conversation"): the row stands for them too.
  #[uniffi(default = [])]
  pub earlier: Vec<String>,
  /// A room (design §2.27, docs/rooms.md §4): a conversation with several
  /// participants, each a thread of its own.
  #[uniffi(default = false)]
  pub room: bool,
  /// A room's mode: `sequential`, or `parallel` (council).
  #[uniffi(default = None)]
  pub room_mode: Option<String>,
  /// A room's participants, in roster order.
  #[uniffi(default = [])]
  pub participants: Vec<ParticipantInfo>,
}

/// A room's participant (design §2.27): its `@handle`, how it is shown,
/// its own thread (the newest of its chain), the model it runs on.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ParticipantInfo {
  pub slug: String,
  pub name: String,
  pub thread: String,
  pub model: Option<String>,
  pub muted: bool,
}

/// A room as `RoomInfo` says it: its policy, its participants, and whether
/// a round runs in it now.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct RoomInfo {
  pub room: String,
  pub title: Option<String>,
  /// The 1:1 conversation it was pulled out of ("Pull in model…").
  pub promoted_from: Option<String>,
  pub budget_usd: Option<f64>,
  /// `sequential` or `parallel`.
  pub mode: String,
  /// The `@handoff`s one user message allows.
  pub hop_cap: u32,
  pub participants: Vec<ParticipantInfo>,
  pub running: bool,
}

/// A participant to add to a room: a model (`provider/model`, `default`,
/// `quick`), read-only unless `read_only` is false and without tools when
/// `tools` is false; or an existing thread, joined as it is.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ParticipantSpec {
  #[uniffi(default = None)]
  pub model: Option<String>,
  #[uniffi(default = None)]
  pub thread: Option<String>,
  #[uniffi(default = None)]
  pub name: Option<String>,
  #[uniffi(default = None)]
  pub slug: Option<String>,
  #[uniffi(default = None)]
  pub node: Option<String>,
  #[uniffi(default = true)]
  pub tools: bool,
  #[uniffi(default = true)]
  pub read_only: bool,
}

impl ParticipantSpec {
  fn spec(self) -> stitch_node::api::RoomParticipantSpec {
    stitch_node::api::RoomParticipantSpec {
      model: self.model,
      thread: self.thread,
      name: self.name,
      slug: self.slug,
      node: self.node,
      tools: self.tools,
      read_only: self.read_only,
    }
  }
}

/// A thread or an effort attached to a message (design §2.27, docs/rooms.md
/// §3): `kind` `effort` (a conversation and every thread spawned under it)
/// or `thread` (the conversation alone).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ScopeRef {
  pub kind: String,
  pub id: String,
}

fn scope_targets(scopes: Vec<ScopeRef>) -> Result<Vec<stitch_node::api::ScopeTarget>> {
  scopes
    .into_iter()
    .map(|s| {
      let kind = match s.kind.as_str() {
        "effort" => stitch_records::ScopeKind::Effort,
        "thread" => stitch_records::ScopeKind::Thread,
        k => return Err(StitchError::of(format!("not a scope kind: {k}"))),
      };
      Ok(stitch_node::api::ScopeTarget { kind, id: s.id })
    })
    .collect()
}

/// A scope of a message, as its chip shows it (`Part::Scope`): what it
/// points at, its title, how many threads it covers (an effort's).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ScopeInfo {
  /// `effort` or `thread`.
  pub kind: String,
  /// The conversation it opens.
  pub id: String,
  pub title: Option<String>,
  pub threads: u32,
}

/// Who said a room's reply (design §2.27): the participant's handle and
/// name, its own thread (tapped, it opens), the model it ran on.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SpeakerInfo {
  pub slug: String,
  pub name: String,
  pub thread: Option<String>,
  pub model: Option<String>,
}

fn participants_of(index: &Index, room: &RecordId) -> Vec<ParticipantInfo> {
  stitch_node::rooms::roster(index, room)
    .into_iter()
    .map(|m| {
      let t = index.compaction_newest(&m.thread);
      ParticipantInfo {
        slug: m.slug,
        name: m.name,
        model: stitch_node::meta::thread_model(index, &t),
        thread: t.to_string(),
        muted: m.muted,
      }
    })
    .collect()
}

/// `RoomInfo`'s line read.
fn room_of(line: &str) -> Result<RoomInfo> {
  let v: serde_json::Value = serde_json::from_str(line).map_err(StitchError::of)?;
  let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
  Ok(RoomInfo {
    room: s("room").unwrap_or_default(),
    title: s("title"),
    promoted_from: s("promoted_from"),
    budget_usd: v.get("budget_usd").and_then(|b| b.as_f64()),
    mode: s("mode").unwrap_or_else(|| "sequential".into()),
    hop_cap: v.get("hop_cap").and_then(|h| h.as_u64()).unwrap_or(2) as u32,
    running: v.get("running").and_then(|r| r.as_bool()).unwrap_or(false),
    participants: v
      .get("participants")
      .and_then(|p| p.as_array())
      .map(|ps| {
        ps.iter()
          .map(|p| {
            let s = |k: &str| p.get(k).and_then(|x| x.as_str()).map(str::to_owned);
            ParticipantInfo {
              slug: s("slug").unwrap_or_default(),
              name: s("name").unwrap_or_default(),
              thread: s("thread").unwrap_or_default(),
              model: s("model"),
              muted: p.get("muted").and_then(|m| m.as_bool()).unwrap_or(false),
            }
          })
          .collect()
      })
      .unwrap_or_default(),
  })
}

/// A message `Stitch::search` found.
#[derive(uniffi::Record)]
pub struct SearchHitInfo {
  pub thread: String,
  pub title: String,
  /// The message (`m12`).
  pub msg: String,
  pub at: String,
  /// `user`, or the model that wrote it.
  pub sender: String,
  /// Around what matched.
  pub snippet: String,
  /// What found it: `words`, `meaning`, or `both`.
  pub found_by: String,
}

/// A notice one of the phone's own threads sent (`Stitch::take_notices`).
#[derive(uniffi::Record)]
pub struct NoticeInfo {
  pub title: String,
  /// The thread's title.
  pub subtitle: Option<String>,
  pub body: String,
  /// The thread it is from: tapping it opens that chat.
  pub thread: Option<String>,
}

/// A secret as Settings lists it (design §2.19): never its value.
#[derive(uniffi::Record)]
pub struct SecretEntry {
  pub name: String,
  /// `global`, `project:<repo>`, `folder:<key>`.
  pub scope: String,
  /// The scope as lists name it.
  pub scope_label: String,
  /// Kept on this device only, never synced.
  pub device_only: bool,
  /// This device holds its value.
  pub here: bool,
  /// The machines that hold it.
  pub nodes: Vec<String>,
  /// Where it is used (`provider openai`, `env OPENAI_API_KEY (stitch)`).
  pub used: Vec<String>,
  /// When it was set (RFC 3339), and on which machine.
  pub updated: String,
  pub from: String,
  /// Machines removed since they held it: rotating it is suggested.
  pub rotate: Vec<String>,
}

/// A variable as Settings lists it.
#[derive(uniffi::Record)]
pub struct VariableEntry {
  pub name: String,
  pub scope: String,
  pub scope_label: String,
  pub value: String,
  pub updated: String,
  pub from: String,
}

/// An SSH host as Settings lists it (design §2.20).
#[derive(uniffi::Record)]
pub struct SshHostEntry {
  pub name: String,
  pub address: String,
  /// 0: 22.
  pub port: u16,
  pub user: String,
  pub jump: String,
  /// `auto`, `agent`, `device`, `secret:<name>`, `key:<path>`.
  pub auth: String,
  /// `relaxed`, `ask`, `allow`.
  pub approval: String,
  pub tmux: bool,
  /// From `~/.ssh/config` of the node that listed it (not a record).
  pub from_config: bool,
}

/// A host key trusted (on first use).
#[derive(uniffi::Record)]
pub struct SshHostKeyEntry {
  pub host: String,
  pub name: String,
  pub fingerprint: String,
  pub trusted: String,
  pub node: String,
}

/// This device's SSH key.
#[derive(uniffi::Record)]
pub struct SshDeviceKey {
  /// As `authorized_keys` takes it.
  pub line: String,
  pub fingerprint: String,
  /// `secure-enclave` or `keychain`.
  pub kind: String,
}

/// A request to authorize a key on a host, and how it went.
#[derive(uniffi::Record)]
pub struct SshAuthorizationEntry {
  pub host: String,
  pub key: String,
  pub asked: String,
  /// `pending`, `done`, `failed`.
  pub state: String,
  pub said: Vec<String>,
}

/// One of the user's other nodes, and whether it runs programs
/// (`Stitch::runners`).
#[derive(uniffi::Record)]
pub struct NodeRuns {
  pub name: String,
  /// `None`: not reached to ask (offline), or `other_build`.
  pub runs: Option<bool>,
  /// Reached, but it runs a build whose answers this one cannot read: to
  /// be shown as such ("other build"), not as offline; updating one of the
  /// two fixes it.
  #[uniffi(default = false)]
  pub other_build: bool,
}

/// Your instructions for a place (`Stitch::instructions`).
#[derive(uniffi::Record)]
pub struct InstructionsInfo {
  /// The place as lists name it (`global`, a project, a folder).
  pub place: String,
  pub text: Option<String>,
}

/// A thread's last turn that never ended (`Stitch::unfinished`).
#[derive(uniffi::Record)]
pub struct UnfinishedTurn {
  /// `suspended` (this app stopped it as iOS was suspending it) or `cut
  /// off` (it stopped mid-turn: the app was killed, or the loop failed).
  pub why: String,
  /// The node whose loop has the turn, when not this app's: another node,
  /// which may be running it now, or a detached loop at work. Resume
  /// refuses it; a fork of the thread resumes here instead.
  pub elsewhere: Option<String>,
}

/// A tool call of a step whose calls run now (`Stitch::in_flight`).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct CallInFlightInfo {
  pub name: String,
  /// What runs, in a few words: `shell_run in `deploy` on devbox-1`.
  pub headline: String,
  /// Its input as one line or a few, as a turn's own calls are shown
  /// (`[deploy] $ cargo build --release`): the command whole.
  pub summary: String,
  /// Its input whole, as JSON.
  pub input: String,
  /// When it began running, in milliseconds since the epoch (the running
  /// call's only).
  pub since_ms: Option<i64>,
  /// Its result's outcome, once it had one.
  pub ok: Option<bool>,
}

/// The step a thread's loop runs the calls of now, not written yet: what
/// the chat shows of a turn it did not start while its calls run.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct StepInFlightInfo {
  pub calls: Vec<CallInFlightInfo>,
  /// The call running, an index into `calls`.
  pub running: Option<u32>,
}

/// A branch of a project's origin.
#[derive(uniffi::Record)]
pub struct BranchInfo {
  pub name: String,
  /// The clone here is on it.
  pub current: bool,
}

/// A file a project's clone here changed from its branch.
#[derive(uniffi::Record)]
pub struct ChangedFile {
  pub path: String,
  /// `added`, `modified`, `deleted`.
  pub status: String,
}

/// A question waiting on the user (the attention list): an approval when
/// `choices` say so.
#[derive(uniffi::Record)]
pub struct Question {
  pub id: String,
  /// What it asks: the command or call, for an approval.
  pub text: String,
  /// The thread that asks, and its title.
  pub thread: Option<String>,
  pub thread_title: Option<String>,
  /// The node that asked.
  pub node: String,
  /// When, in seconds since the epoch.
  pub at: i64,
  /// What answering "always" allows from then on in the thread's place;
  /// empty: no such answer.
  pub grant: Vec<String>,
  /// What to read before answering (a landing's facts and diff, written
  /// by the node), shown monospaced: such a question is the user's alone.
  pub detail: Option<String>,
  /// A file with it (the whole diff, when `detail` cuts it): open it with
  /// `attachment(hash, name)`.
  pub detail_file: Option<QuestionFile>,
}

/// A file a question carries.
#[derive(uniffi::Record)]
pub struct QuestionFile {
  pub hash: String,
  pub name: String,
}

/// A file of a folder on this device.
#[derive(uniffi::Record)]
pub struct FolderFile {
  /// Relative to the folder, `/`-separated.
  pub path: String,
  pub size: u64,
  /// Carried to the other machines (any file up to the size cap, 25 MB by
  /// default); larger ones stay here.
  pub synced: bool,
}

/// What a folder sync did.
#[derive(uniffi::Record)]
pub struct FolderPass {
  pub published: bool,
  pub applied: u32,
  pub conflicts: Vec<String>,
  pub skipped: Option<String>,
}

/// A change of a folder.
#[derive(uniffi::Record)]
pub struct FolderChange {
  pub id: String,
  /// Seconds since the epoch.
  pub at: i64,
  pub node: String,
  pub thread: Option<String>,
  pub changed: Vec<String>,
  pub merge: bool,
}

/// A place a new thread can belong to: a project, or a folder.
#[derive(uniffi::Record)]
pub struct PlaceInfo {
  /// What `chat` takes: `project:<repo>`, `folder:<folder>`.
  pub key: String,
  pub label: String,
}

/// One entry of a transcript: who (`you`, a model, `tool`, `output`,
/// `error`, …) and what.
#[derive(uniffi::Record)]
pub struct Entry {
  pub who: String,
  pub text: String,
  /// A message's ID (`m12`), what `fork` takes as `upto`; `None` for tool
  /// calls and results.
  pub id: Option<String>,
  /// What the message carries (images, files); `text` does not name them.
  pub attachments: Vec<AttachmentInfo>,
  /// A message's text alone, as written: `text` says what else it
  /// carries first (`[instructions updated]`), this does not. What a
  /// message sent from here is matched by until the transcript has it.
  #[uniffi(default = "")]
  pub typed: String,
  /// A `compacted` entry's compaction (design §2.15, "One
  /// conversation"): the divider between the thread it was compacted from
  /// and the one it went on in.
  #[uniffi(default = None)]
  pub compaction: Option<CompactionInfo>,
  /// The user said this message in a voice call (design §2.26): the chat
  /// marks it.
  #[uniffi(default = false)]
  pub spoken: bool,
  /// Threads and efforts attached to the message (`Part::Scope`, design
  /// §2.27): chips, their briefing not in `text`.
  #[uniffi(default = [])]
  pub scopes: Vec<ScopeInfo>,
  /// A room's reply: which participant said it.
  #[uniffi(default = None)]
  pub speaker: Option<SpeakerInfo>,
  /// A `suggest_replies` call's preset answers (design §2.30), on its
  /// `tool` entry; empty for any other. Buttons at the end of its turn,
  /// live only for the last entry that has them, while no `you` entry
  /// comes after it and no turn runs; pressing one sends its `text` as a
  /// message. Older ones are inert.
  #[uniffi(default = [])]
  pub replies: Vec<SuggestedReply>,
  /// An accepted `offer_actions` call's buttons (design §2.30, step 2),
  /// on its `tool` entry; empty for any other. The last entry that has
  /// them is live, whatever comes after it; pressing one is `press(thread,
  /// set, index)`, with `set` this entry's `call`.
  #[uniffi(default = [])]
  pub actions: Vec<ActionInfo>,
  /// A tool call's ID, on its `tool` entry (an action set's, which a
  /// press names).
  #[uniffi(default = None)]
  pub call: Option<String>,
  /// A `press` entry: which button of which set was pressed, and how its
  /// call ended.
  #[uniffi(default = None)]
  pub press: Option<PressInfo>,
}

/// An action button (design §2.30, step 2): its label, the call it runs
/// as one line, and whether its press also wakes the agent.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ActionInfo {
  pub label: String,
  pub tool: String,
  pub call: String,
  pub wake: bool,
}

/// A press of an action button, as the transcript keeps it.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PressInfo {
  pub set: String,
  pub item: u32,
  pub label: String,
  pub ok: bool,
  pub output: String,
}

/// What a press returned: whether its call succeeded, and its output.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Pressed {
  pub ok: bool,
  pub output: String,
}

/// Something sharing's leak scan found (design §2.31): its kind (`home
/// path`, `private address`, `email`, `hostname`, `node name`, `user
/// name`), the text, how often, and where first (`t1#m4`, `title`).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ShareFinding {
  pub kind: String,
  pub text: String,
  pub count: u32,
  pub at: String,
}

/// What sharing a thread holds (`share_preview`), or wrote
/// (`share_export`: `path`, the file in the node's downloads, named
/// `file_name`). `hash` is what `share_export` takes as `confirm` when
/// there are findings, which the app shows the user first.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SharePreview {
  pub title: String,
  pub kind: String,
  pub level: String,
  pub threads: u32,
  pub messages: u32,
  pub calls: u32,
  pub attachments: u32,
  pub apps: u32,
  pub left_out: Vec<String>,
  pub bytes: u64,
  pub hash: String,
  pub key: String,
  pub redactions: u32,
  pub findings: Vec<ShareFinding>,
  pub file_name: String,
  pub path: Option<String>,
}

impl From<stitch_node::share::Preview> for SharePreview {
  fn from(p: stitch_node::share::Preview) -> Self {
    SharePreview {
      title: p.title,
      kind: p.kind,
      level: p.level.as_str().to_owned(),
      threads: p.threads as u32,
      messages: p.messages as u32,
      calls: p.calls as u32,
      attachments: p.attachments as u32,
      apps: p.apps as u32,
      left_out: p.left_out,
      bytes: p.bytes as u64,
      hash: p.hash,
      key: p.key,
      redactions: p.redactions as u32,
      findings: p
        .findings
        .into_iter()
        .map(|f| ShareFinding {
          kind: f.kind,
          text: f.text,
          count: f.count as u32,
          at: f.at,
        })
        .collect(),
      file_name: p.file_name,
      path: p.path,
    }
  }
}

/// What a shared file would import (`import_preview`: `written` false, no
/// threads) or made (`import_thread`): the thread to open, every thread
/// made, the file's parts (`t1 <title>`), and who signed it (`signer`,
/// hex; `mine`: this node's own key; none: unsigned). `already`: the same
/// file was imported before, and those are its threads.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ImportedThread {
  pub thread: String,
  pub threads: Vec<String>,
  pub parts: Vec<String>,
  pub title: String,
  pub hash: String,
  pub signer: Option<String>,
  pub mine: bool,
  pub level: String,
  pub kind: String,
  pub messages: u32,
  pub calls: u32,
  pub attachments: u32,
  pub apps: u32,
  pub already: bool,
  pub written: bool,
}

/// A preset answer (design §2.30): the button's `label`, the message its
/// press sends (`text`, the label unless the agent gave another).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SuggestedReply {
  pub label: String,
  pub text: String,
}

/// A compaction: a thread outgrew its model's window and went on in a new
/// one, its summary standing for what came before the messages kept
/// (`Reply::Compacted`, `LiveEvent::Compacted`, the lineage's divider).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct CompactionInfo {
  /// The thread compacted.
  pub from: String,
  /// The thread it goes on in.
  pub to: String,
  /// The model whose window it outgrew (`provider/model`); empty for a
  /// compaction older than these figures.
  pub model: String,
  /// That window, in tokens (0: not known).
  pub window: u64,
  /// What the last step sent, in tokens; `None` when not known.
  pub tokens: Option<u64>,
  /// The messages kept verbatim after the summary.
  pub kept: u64,
  pub summary: String,
  /// The divider's line: "compacted here (162k of 200k window): summary +
  /// last 10 messages" (`Compaction::said`).
  pub said: String,
}

impl CompactionInfo {
  fn of(c: &stitch_node::api::Compaction) -> CompactionInfo {
    let mut said = c.said();
    if c.window == 0 {
      // Written before the figures were kept: no window to say.
      said = said.replacen(" (past the 0k window)", "", 1);
    }
    CompactionInfo {
      from: c.from.clone(),
      to: c.to.clone(),
      model: c.model.clone(),
      window: c.window,
      tokens: c.tokens,
      kept: c.kept,
      summary: c.summary.clone(),
      said,
    }
  }
}

/// An attachment of a message: `attachment` gives it as a file.
#[derive(uniffi::Record)]
pub struct AttachmentInfo {
  /// `blake3:<hex>`
  pub hash: String,
  pub name: Option<String>,
  pub media_type: String,
  pub bytes: u64,
  /// Whether this node holds it (else it is fetched when asked for).
  pub here: bool,
}

/// The media type of an app's attachment (design §2.10): it is shown as
/// the app, served through `app_serve` at `app_host(hash)`.
#[uniffi::export]
pub fn app_media_type() -> String {
  stitch_node::apps::MEDIA.into()
}

/// The URL scheme apps are served under (`stitch-app://<host>/`).
#[uniffi::export]
pub fn app_scheme() -> String {
  stitch_node::apps::serve::SCHEME.into()
}

/// The built-in dashboard on sample data (the app's fixtures, no node).
#[uniffi::export]
pub fn app_fixture(path: String) -> AppResponse {
  let r = stitch_node::apps::live::fixture(&path);
  AppResponse {
    status: r.status,
    headers: r
      .headers
      .into_iter()
      .map(|(name, value)| AppHeader { name, value })
      .collect(),
    body: r.body,
  }
}

/// What an app's origin answered.
#[derive(uniffi::Record)]
pub struct AppResponse {
  pub status: u16,
  pub headers: Vec<AppHeader>,
  pub body: Vec<u8>,
}

#[derive(uniffi::Record)]
pub struct AppHeader {
  pub name: String,
  pub value: String,
}

/// A pinned app.
#[derive(uniffi::Record)]
pub struct PinnedApp {
  pub id: String,
  pub name: String,
  /// An SF Symbol's name.
  pub icon: Option<String>,
  /// Where it is served (`stitch-app://<host>/`): the same from version
  /// to version.
  pub host: String,
  /// Its version: its manifest's hash.
  pub version: String,
  /// The thread it was pinned from.
  pub thread: Option<String>,
  /// When it was last pinned, in seconds since the epoch.
  pub at: i64,
  /// The built-in app it is (`dashboard`).
  pub builtin: Option<String>,
  /// It may read the node's live state (true), may not (false), or was
  /// never asked.
  pub live: Option<bool>,
}

/// An app asking to read the node's live state, as the consent sheet
/// names it.
#[derive(uniffi::Record)]
pub struct AppLive {
  pub name: String,
  /// The user's answer so far: none, never asked.
  pub consent: Option<bool>,
}

impl PinnedApp {
  fn of(p: stitch_node::apps::Pinned) -> PinnedApp {
    PinnedApp {
      id: p.id.to_string(),
      name: p.name,
      icon: p.icon,
      host: p.host,
      version: p.manifest.to_string(),
      thread: p.thread.map(|t| t.to_string()),
      at: p.at.as_second(),
      builtin: p.builtin.map(|(b, _)| b),
      live: p.live,
    }
  }
}

/// How a chat ended.
#[derive(uniffi::Record)]
pub struct ChatOutcome {
  pub thread: String,
  pub model: String,
  /// `end_turn`, `max_tokens`, `interrupted`, `refusal: …`
  pub stop: String,
  pub usd: Option<f64>,
  /// What the thread cost so far (API keys only).
  pub thread_usd: Option<f64>,
}

#[derive(uniffi::Record)]
pub struct NodeStatus {
  pub node: String,
  pub endpoint: String,
  pub repos: Vec<String>,
  pub peers: Vec<String>,
}

#[derive(uniffi::Record)]
pub struct Models {
  /// `provider/model` IDs.
  pub models: Vec<String>,
  /// Why a provider listed none.
  pub problems: Vec<String>,
  pub default_model: String,
}

/// Memory on this device (design §2.9).
#[derive(uniffi::Record)]
pub struct MemoryInfo {
  /// The text model is downloaded.
  pub installed: bool,
  /// The vision projector too: images are remembered.
  pub images: bool,
  /// Memory runs (installed, and not turned off).
  pub running: bool,
  /// Facts folded, from every node.
  pub facts: u64,
  /// Memories in the graph.
  pub episodes: u64,
}

#[derive(uniffi::Record)]
pub struct Joined {
  pub inviter: String,
  /// The other nodes trusted with it.
  pub fleet: Vec<String>,
  /// The home workspace's first sync, or `None` when the inviter holds
  /// none.
  pub home: Option<String>,
}

/// What a chat streams as it runs, called on the node's threads.
#[uniffi::export(with_foreign)]
pub trait ChatListener: Send + Sync {
  /// The thread, as soon as it is known (a new one's ID).
  fn thread(&self, id: String);
  fn text(&self, delta: String);
  fn thinking(&self, delta: String);
  /// A tool call, as one line.
  fn tool_call(&self, line: String);
  fn tool_result(&self, ok: bool, output: String);
  /// A note given to the model (a report that came, a job that ended).
  fn note(&self, text: String);
  /// The thread was compacted (design §2.15, "One conversation"): the
  /// conversation goes on in `compaction.to`, which `thread` names next.
  /// The chat shows the divider and switches to it.
  fn compacted(&self, compaction: CompactionInfo);
  /// The model call was cut off by the network and is sent again: what
  /// it wrote since the last tool call is dropped.
  fn retry(&self, reason: String);
  /// Messages sent with `queue_message` (by their IDs), now given to the
  /// model between its steps.
  fn woven(&self, ids: Vec<u64>);
}

/// Shows GitHub's device code (`github_sign_in`): the user enters it at
/// the address given.
#[uniffi::export(with_foreign)]
pub trait GitHubPrompt: Send + Sync {
  fn show(&self, code: String, url: String);
}

/// Shows a subscription's sign-in page (`sign_in`): in the app, so the
/// page's redirect reaches the node listening on this device.
#[uniffi::export(with_foreign)]
pub trait SignInBrowser: Send + Sync {
  fn show(&self, url: String);
}

/// How a download goes (`install_memory`), called on the node's threads.
#[uniffi::export(with_foreign)]
pub trait InstallProgress: Send + Sync {
  /// `total` is 0 when unknown.
  fn progress(&self, file: String, got: u64, total: u64);
}

/// The node's name when `data_dir` holds one set up; `None` before the
/// first start.
#[uniffi::export]
pub fn configured_name(data_dir: String) -> Option<String> {
  Config::load(Path::new(&data_dir)).ok()?.node
}

/// Where edge talk's app group container is (`group.fragcolor.edge-talk`,
/// as `FileManager.containerURL` resolves it), given before `Stitch.open`:
/// the `edge-talk` skill reads edge talk's database there, read-only.
#[uniffi::export]
pub fn set_edge_talk_container(path: String) {
  stitch_node::otter::edge_talk::set_container(PathBuf::from(path));
}

/// A page the app loaded for `browse_web` or `web_eval` (`WebPages`).
#[derive(uniffi::Record)]
pub struct WebPage {
  /// The app's handle of the page (0 for a file or a failure).
  pub page: u64,
  /// Where it is: after redirects, where a script moved it on.
  pub url: String,
  /// The main document's HTTP status (0 when not known).
  pub status: i64,
  /// The main document's media type, lowercase.
  pub mime: String,
  /// Not a web page (a PDF, an image, a download): nothing kept, the node
  /// fetches `url` itself.
  pub file: bool,
  /// Why it did not load.
  pub error: Option<String>,
}

/// What a script run in a page gave (`WebPages::call`).
#[derive(uniffi::Record)]
pub struct WebCall {
  /// The string the script returned (`None` for anything else).
  pub value: Option<String>,
  /// Why it failed: the engine's message.
  pub error: Option<String>,
  /// The line of the body the error is at (0 when not known).
  pub line: i64,
  /// The page navigated while it ran.
  pub navigated: bool,
  /// It did not finish within its time.
  pub timed_out: bool,
}

/// A PNG of a page, or why there is none.
#[derive(uniffi::Record)]
pub struct WebShot {
  pub png: Vec<u8>,
  pub error: Option<String>,
}

/// A region of a page's viewport, in CSS pixels.
#[derive(uniffi::Record)]
pub struct WebArea {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

/// The app's browser, lent to the node for `browse_web` and `web_eval`
/// (design §2.3): WebKit pages, offscreen, each fresh (no cookies or
/// storage, nothing of the user's), downloads refused, media silent. The
/// node runs its own scripts in them; the calls come on the node's
/// threads, never the main one, and block until answered.
#[uniffi::export(with_foreign)]
pub trait WebPages: Send + Sync {
  /// Loads `url` in a fresh page, within `timeout_ms`.
  fn load(&self, url: String, timeout_ms: u64) -> WebPage;
  /// Where the page is now.
  fn state(&self, page: u64) -> WebPage;
  /// Runs `body` as an async function body in the page's own world.
  fn call(&self, page: u64, body: String, timeout_ms: u64) -> WebCall;
  /// A PNG of the viewport, or of `area` of it.
  fn screenshot(&self, page: u64, area: Option<WebArea>) -> WebShot;
  /// Disposes of the page, without waiting.
  fn close(&self, page: u64);
}

/// The app's pages as the node's `PageHost`.
struct LentPages(Arc<dyn WebPages>);

fn ms(d: std::time::Duration) -> u64 {
  d.as_millis().try_into().unwrap_or(u64::MAX)
}

impl LentPages {
  fn page(p: WebPage) -> std::result::Result<stitch_node::web::HostPage, String> {
    match p.error {
      Some(e) => Err(e),
      None => Ok(stitch_node::web::HostPage {
        page: p.page,
        url: p.url,
        status: p.status,
        mime: p.mime,
        file: p.file,
      }),
    }
  }
}

impl stitch_node::web::PageHost for LentPages {
  fn open(
    &self,
    url: &str,
    timeout: std::time::Duration,
  ) -> std::result::Result<stitch_node::web::HostPage, String> {
    Self::page(self.0.load(url.to_owned(), ms(timeout)))
  }

  fn state(&self, page: u64) -> std::result::Result<stitch_node::web::HostPage, String> {
    Self::page(self.0.state(page))
  }

  fn call(
    &self,
    page: u64,
    body: &str,
    timeout: std::time::Duration,
  ) -> std::result::Result<Option<String>, stitch_node::web::CallError> {
    use stitch_node::web::CallError;
    let c = self.0.call(page, body.to_owned(), ms(timeout));
    if c.navigated {
      Err(CallError::Navigated)
    } else if c.timed_out {
      Err(CallError::TimedOut)
    } else if let Some(message) = c.error {
      Err(CallError::Script {
        message,
        line: c.line,
      })
    } else {
      Ok(c.value)
    }
  }

  fn screenshot(
    &self,
    page: u64,
    area: Option<stitch_node::web::Area>,
  ) -> std::result::Result<Vec<u8>, String> {
    let shot = self.0.screenshot(
      page,
      area.map(|a| WebArea {
        x: a.x,
        y: a.y,
        width: a.width,
        height: a.height,
      }),
    );
    match shot.error {
      Some(e) => Err(e),
      None if shot.png.is_empty() => Err("the page gave no screenshot".into()),
      None => Ok(shot.png),
    }
  }

  fn close(&self, page: u64) {
    self.0.close(page);
  }
}

/// Lends the node the app's browser (`WebPages`), before `Stitch.open`:
/// `browse_web` and `web_eval` render pages in it.
#[uniffi::export]
pub fn set_web_pages(pages: Arc<dyn WebPages>) {
  stitch_node::web::set_host(Arc::new(LentPages(pages)));
}

/// What an operation on the device's personal data gave
/// (`PersonalData::call`): its JSON answer, or why there is none (a
/// permission denied, an item not found).
#[derive(uniffi::Record)]
pub struct PersonalReply {
  pub json: Option<String>,
  pub error: Option<String>,
}

/// The device's calendar, reminders and maps (EventKit, MapKit), lent to
/// the node for the `calendar` and `maps` tools (design §2.3). The node
/// checks the calls and asks the user what needs asking; the app runs one
/// operation (`stitch_node::personal::Host` lists them, with their JSON),
/// asking the system's permission on first use. Called on the node's
/// threads, never the main one; blocks until answered.
#[uniffi::export(with_foreign)]
pub trait PersonalData: Send + Sync {
  fn call(&self, op: String, input: String) -> PersonalReply;
}

struct LentPersonal(Arc<dyn PersonalData>);

impl stitch_node::personal::Host for LentPersonal {
  fn call(
    &self,
    op: &str,
    input: &serde_json::Value,
  ) -> std::result::Result<serde_json::Value, String> {
    let r = self.0.call(op.to_owned(), input.to_string());
    match (r.error, r.json) {
      (Some(e), _) => Err(e),
      (None, Some(j)) => serde_json::from_str(&j).map_err(|e| format!("the app's answer: {e}")),
      (None, None) => Err("the app gave no answer".into()),
    }
  }
}

/// Lends the node the device's calendar, reminders and maps
/// (`PersonalData`), before `Stitch.open`.
#[uniffi::export]
pub fn set_personal_data(data: Arc<dyn PersonalData>) {
  stitch_node::personal::set_host(Arc::new(LentPersonal(data)));
}

/// A chat running for the app: the thread it runs on once known, and
/// where requests for it (`Interrupt`, `Suspend`) go.
struct RunningChat {
  thread: Arc<Mutex<Option<String>>>,
  tx: mpsc::UnboundedSender<Request>,
}

/// The node, running in this process.
#[derive(uniffi::Object)]
pub struct Stitch {
  rt: tokio::runtime::Runtime,
  data: PathBuf,
  name: NodeName,
  node: tokio::sync::Mutex<Option<NodeHandle>>,
  /// For reads; the node has its own.
  index: Mutex<Index>,
  /// What `revision` last saw.
  seen: Mutex<Seen>,
  blobs: BlobStore,
  /// Where `interrupt` reaches the chats running.
  chats: Mutex<Vec<RunningChat>>,
  /// Stops the sign-in waiting for its redirect (`cancel_sign_in`).
  signing_in: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
  /// Stops the GitHub sign-in waiting for its code (`cancel_github_sign_in`).
  github_signing_in: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[uniffi::export]
impl Stitch {
  /// Opens the node of `data_dir`, setting it up first as `name` when it
  /// has none (a home workspace of its own until it joins; blobs fetched
  /// when needed), and starts it.
  #[uniffi::constructor]
  pub fn open(data_dir: String, name: String) -> Result<Arc<Stitch>> {
    // The build, as the app's build says it (tools/build-label.sh: the commits).
    stitch_node::set_build(
      option_env!("STITCH_BUILD").unwrap_or(concat!("app ", env!("CARGO_PKG_VERSION"))),
    );
    let data = PathBuf::from(data_dir);
    let config = Config::load(&data).map_err(StitchError::of)?;
    let name = match config.node {
      Some(n) => n.parse().map_err(StitchError::of)?,
      None => {
        let name: NodeName = name.parse().map_err(StitchError::of)?;
        setup::setup(&data, &name, Some(&setup::default_home(&data)), false, true)
          .map_err(StitchError::of)?;
        name
      }
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .thread_name("stitch")
      .build()
      .map_err(StitchError::of)?;
    let node = rt
      .block_on(stitch_node::start(&data))
      .map_err(StitchError::of)?;
    Ok(Arc::new(Stitch {
      index: Mutex::new(Index::open(&data.join("index.sqlite")).map_err(StitchError::of)?),
      seen: Mutex::default(),
      blobs: BlobStore::open(&data.join("blobs")).map_err(StitchError::of)?,
      rt,
      data,
      name,
      node: tokio::sync::Mutex::new(Some(node)),
      chats: Mutex::default(),
      signing_in: Mutex::default(),
      github_signing_in: Mutex::default(),
    }))
  }

  pub fn node_name(&self) -> String {
    self.name.to_string()
  }

  /// A number that changes when what the app shows may have: another
  /// connection (the node writing a turn's step, a sync bringing records)
  /// committed to the index (`PRAGMA data_version`), or files came into a
  /// root that nothing indexed (scanned every 30 s at most). Cheap: the
  /// app asks every few seconds, and reads the threads, the questions and
  /// the open transcript again only when it changed.
  pub async fn revision(self: Arc<Self>) -> u64 {
    self.blocking(|s| s.revision_now()).await.unwrap_or(0)
  }

  /// Every thread, the pinned first, then the latest.
  pub async fn threads(self: Arc<Self>) -> Result<Vec<ThreadInfo>> {
    self.blocking(|s| s.threads_now()).await?
  }

  /// The places a new thread can go to: those of the threads, the most
  /// recently active first, then every other project.
  pub async fn places(self: Arc<Self>) -> Result<Vec<PlaceInfo>> {
    self.blocking(|s| s.places_now()).await?
  }

  /// A thread's transcript, as it reads now.
  pub async fn transcript(self: Arc<Self>, thread: String) -> Result<Vec<Entry>> {
    self.blocking(move |s| s.transcript_now(thread)).await?
  }

  /// Fetches the attachments of a thread this node lacks, from whichever
  /// peer holds them (when it is opened, so they are here to show, and
  /// before a turn needs them); how many arrived.
  pub async fn fetch_attachments(&self, thread: String) -> Result<u32> {
    let mut hashes = Vec::new();
    for e in self.shown(&thread)? {
      for a in e.attachments {
        if !self.blobs.contains(&a.hash) {
          hashes.push(a.hash.to_string());
        }
      }
    }
    if hashes.is_empty() {
      return Ok(0);
    }
    self.fetch(hashes).await
  }

  /// An attachment as a file to show or share, named `name`: fetched first
  /// when this node lacks it. The file is a link to the stored blob, in
  /// the app's temporary directory.
  pub async fn attachment(&self, hash: String, name: String) -> Result<String> {
    let h: BlobHash = hash.parse().map_err(StitchError::of)?;
    if !self.blobs.contains(&h) {
      self.fetch(vec![hash]).await?;
      if !self.blobs.contains(&h) {
        return Err(StitchError::of(
          "no node that holds this attachment could be reached",
        ));
      }
    }
    // One directory per blob, so two attachments of one name never meet.
    let dir = std::env::temp_dir().join("attachments").join(h.hex());
    let name = Path::new(&name)
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .filter(|n| !n.is_empty())
      .unwrap_or_else(|| "attachment".into());
    let file = dir.join(name);
    if !file.is_file() {
      std::fs::create_dir_all(&dir).map_err(StitchError::of)?;
      let blob = self.blobs.path(&h);
      if std::fs::hard_link(&blob, &file).is_err() {
        std::fs::copy(&blob, &file).map_err(StitchError::of)?;
      }
      // A link shares the stored blob: nothing that opens it may change it.
      let mut perms = std::fs::metadata(&file)
        .map_err(StitchError::of)?
        .permissions();
      perms.set_readonly(true);
      std::fs::set_permissions(&file, perms).map_err(StitchError::of)?;
    }
    Ok(file.display().to_string())
  }

  /// Sends `text` (with files in this app's container) to a thread, or to
  /// a new one (`thread`: `None`) in `place` (`PlaceInfo::key`; `None`:
  /// global), with `model` (`provider/model`) or the node's default; what
  /// it streams goes to `listener`. A new thread's turns run on `node`
  /// (another of the user's nodes that runs programs, design §2.7, "A new
  /// chat on another node"); `None`: here. `spoken`: said in a voice call
  /// (design §2.26), its reply to be read aloud. `scopes`: threads and
  /// efforts attached (design §2.27), briefed to the model with it. To a
  /// room, its participants answer (`Reply::Note` names each before its
  /// reply).
  #[allow(clippy::too_many_arguments)]
  pub async fn chat(
    &self,
    thread: Option<String>,
    place: Option<String>,
    node: Option<String>,
    text: String,
    attachments: Vec<String>,
    model: Option<String>,
    spoken: bool,
    scopes: Vec<ScopeRef>,
    listener: Arc<dyn ChatListener>,
  ) -> Result<ChatOutcome> {
    let req = Request::Chat {
      thread: thread.clone(),
      text,
      attachments,
      model,
      repo: None,
      cwd: None,
      global: true,
      env: Vec::new(),
      tools: Vec::new(),
      skills: Vec::new(),
      place,
      memory: None,
      end_user: None,
      node,
      spoken,
      scopes: scope_targets(scopes)?,
    };
    self.run_chat(req, thread, listener).await
  }

  /// Continues a thread whose last turn never ended (`unfinished`): its
  /// step is sent again from what is written, and streams to `listener`
  /// as a chat does.
  pub async fn resume(
    &self,
    thread: String,
    model: Option<String>,
    listener: Arc<dyn ChatListener>,
  ) -> Result<ChatOutcome> {
    let req = Request::Resume {
      thread: thread.clone(),
      model,
    };
    self.run_chat(req, Some(thread), listener).await
  }

  /// Sends the user's message to the turn running on `thread`, on the
  /// node that runs it (`send` with `interject`): woven into that loop
  /// between its steps, or starting a turn there when none runs. For a
  /// thread whose turn another node has (`UnfinishedTurn::elsewhere`),
  /// where a chat here is refused. Returns what became of it, as said.
  pub async fn interject(
    &self,
    thread: String,
    text: String,
    scopes: Vec<ScopeRef>,
  ) -> Result<String> {
    let req = Request::Send {
      thread,
      text,
      from: None,
      keep: false,
      interject: true,
      scopes: scope_targets(scopes)?,
    };
    match self.request(req).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Makes a room (design §2.27, docs/rooms.md §4) in `place` (`None`:
  /// `from_thread`'s, else global) with `participants`; `from_thread`:
  /// "Pull in model…", the room pulled out of that 1:1 conversation, which
  /// joins first as it is. `mode` `sequential` (default) or `parallel`;
  /// `hop_cap` the `@handoff`s one message allows (2).
  #[allow(clippy::too_many_arguments)]
  pub async fn room_create(
    &self,
    title: Option<String>,
    place: Option<String>,
    from_thread: Option<String>,
    participants: Vec<ParticipantSpec>,
    scopes: Vec<ScopeRef>,
    mode: Option<String>,
    hop_cap: Option<u32>,
    budget_usd: Option<f64>,
  ) -> Result<RoomInfo> {
    let req = Request::RoomCreate {
      title,
      place,
      from_thread,
      participants: participants
        .into_iter()
        .map(ParticipantSpec::spec)
        .collect(),
      scopes: scope_targets(scopes)?,
      mode,
      hop_cap,
      budget_usd,
    };
    self.room_request(req).await
  }

  /// A room: its policy and participants.
  pub async fn room_info(&self, thread: String) -> Result<RoomInfo> {
    self.room_request(Request::RoomInfo { thread }).await
  }

  /// Changes a room: participants added, removed, muted or unmuted (by
  /// their handles), its mode (`sequential`, `parallel`) and hop cap.
  #[allow(clippy::too_many_arguments)]
  pub async fn room_update(
    &self,
    thread: String,
    add: Vec<ParticipantSpec>,
    remove: Vec<String>,
    mute: Vec<String>,
    unmute: Vec<String>,
    mode: Option<String>,
    hop_cap: Option<u32>,
  ) -> Result<RoomInfo> {
    let req = Request::RoomUpdate {
      thread,
      add: add.into_iter().map(ParticipantSpec::spec).collect(),
      remove,
      mute,
      unmute,
      mode,
      hop_cap,
    };
    self.room_request(req).await
  }

  /// Whether a thread's last turn never ended, why, and whose it is
  /// (`UnfinishedTurn`); `None` when it ended. A chat running here now
  /// looks unfinished too: ask only of a thread with none.
  pub async fn unfinished(self: Arc<Self>, thread: String) -> Result<Option<UnfinishedTurn>> {
    self.blocking(move |s| s.unfinished_now(thread)).await?
  }

  /// The notices this phone's own threads sent (`notify`) since last
  /// asked, for the app to show as local notifications: a phone's node
  /// cannot push to itself.
  pub async fn take_notices(&self) -> Vec<NoticeInfo> {
    match self.node.lock().await.as_ref() {
      Some(n) => n
        .take_notices()
        .into_iter()
        .map(|n| NoticeInfo {
          title: n.title,
          subtitle: n.subtitle,
          body: n.body,
          thread: n.thread,
        })
        .collect(),
      None => Vec::new(),
    }
  }

  /// What runs on the node that the app being suspended would cut short
  /// (chats, detached loops at work); empty when idle.
  pub async fn busy(&self) -> Vec<String> {
    let busy = match self.node.lock().await.as_ref() {
      Some(n) => n.busy(),
      None => return Vec::new(),
    };
    self.rt.spawn(busy).await.unwrap_or_default()
  }

  /// Offers a message to the chat running on `thread` (the TUI's queue,
  /// `Request::Interject`): the loop weaves it in after its tool calls and
  /// `ChatListener::woven` tells its `id` then. Not woven when the turn
  /// ends, the caller sends it as a turn of its own. `false`: no chat of
  /// this app runs on the thread.
  pub fn queue_message(&self, thread: String, id: u64, text: String) -> bool {
    self.to_chat(
      &thread,
      Request::Interject {
        id,
        text,
        attachments: Vec::new(),
      },
    )
  }

  /// Takes a message back that `queue_message` offered (`Request::Retract`),
  /// to be edited: it is not woven after this. One already woven comes
  /// back through `woven` all the same.
  pub fn retract_message(&self, thread: String, id: u64) -> bool {
    self.to_chat(&thread, Request::Retract { id })
  }

  /// Stops the chat running on `thread`: it ends `interrupted`.
  pub fn stop_chat(&self, thread: String) {
    for c in self.chats.lock().expect("chats lock").iter() {
      if c.thread.lock().expect("chat lock").as_deref() == Some(thread.as_str()) {
        let _ = c.tx.send(Request::Interrupt);
      }
    }
  }

  /// The app is being suspended: every chat running stops as it is (what
  /// the model was writing dropped, a tool call cut), recorded as
  /// `suspended` so it is resumed later. Waits until they ended, or
  /// `wait_ms` passed; returns the threads they ran on.
  pub fn suspend_chats(&self, wait_ms: u32) -> Vec<String> {
    let threads: Vec<String> = {
      let chats = self.chats.lock().expect("chats lock");
      chats
        .iter()
        .filter(|c| c.tx.send(Request::Suspend).is_ok())
        .filter_map(|c| c.thread.lock().expect("chat lock").clone())
        .collect()
    };
    let until = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms.into());
    while std::time::Instant::now() < until
      && self
        .chats
        .lock()
        .expect("chats lock")
        .iter()
        .any(|c| !c.tx.is_closed())
    {
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    threads
  }
}

impl Stitch {
  /// A room request, its reply (`RoomInfo`'s line) read.
  async fn room_request(&self, req: Request) -> Result<RoomInfo> {
    match self.request(req).await? {
      Reply::Lines(lines) => room_of(lines.first().map(String::as_str).unwrap_or("{}")),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Sends a request to the chat running on `thread`, if one does.
  fn to_chat(&self, thread: &str, req: Request) -> bool {
    let mut sent = false;
    for c in self.chats.lock().expect("chats lock").iter() {
      if c.thread.lock().expect("chat lock").as_deref() == Some(thread) {
        sent |= c.tx.send(req.clone()).is_ok();
      }
    }
    sent
  }

  /// Runs a chat or a resumed turn on a connection of its own, streaming
  /// to `listener`; `interrupt`, `stop_chat` and `suspend_chats` reach it.
  async fn run_chat(
    &self,
    req: Request,
    thread: Option<String>,
    listener: Arc<dyn ChatListener>,
  ) -> Result<ChatOutcome> {
    let mut client = self.client().await?;
    let (tx, rx) = mpsc::unbounded_channel();
    let on = Arc::new(Mutex::new(thread));
    self.chats.lock().expect("chats lock").push(RunningChat {
      thread: on.clone(),
      tx,
    });
    let done = self
      .rt
      .spawn(async move {
        client
          .chat_with(&req, Some(rx), |r| {
            match r {
              Reply::Thread(id) => {
                *on.lock().expect("chat lock") = Some(id.clone());
                listener.thread(id.clone())
              }
              Reply::Retry(why) => listener.retry(why.clone()),
              Reply::Text(t) => listener.text(t.clone()),
              Reply::Thinking(t) => listener.thinking(t.clone()),
              Reply::Afterthought(t) => listener.text(format!("\n\n{t}")),
              Reply::ToolCall(c) => listener.tool_call(c.clone()),
              Reply::ToolResult { ok, output } => listener.tool_result(*ok, output.clone()),
              Reply::Note(n) => listener.note(n.clone()),
              // The divider; the `Thread` that follows names the thread
              // the turn goes on in (design §2.15).
              Reply::Compacted(c) => listener.compacted(CompactionInfo::of(c)),
              Reply::Woven(ids) => listener.woven(ids.clone()),
              _ => {}
            }
            // No approvals asked here: a question waits on the attention
            // list, for any client.
            None
          })
          .await
      })
      .await
      .map_err(StitchError::of)?;
    self
      .chats
      .lock()
      .expect("chats lock")
      .retain(|c| !c.tx.is_closed());
    match done.map_err(StitchError::of)? {
      Reply::ChatDone {
        thread,
        model,
        stop,
        usd,
        thread_usd,
        ..
      } => Ok(ChatOutcome {
        thread,
        model,
        stop,
        usd,
        thread_usd,
      }),
      Reply::Unsent => Err(StitchError::of(
        "interrupted before it was sent: another turn ran on the thread",
      )),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }
}

#[uniffi::export]
impl Stitch {
  /// Forks a thread (design §2.15): a new thread continuing it, read from
  /// it, up to a message (`upto`, an `Entry::id`; the latest by default),
  /// all of it or its `last` messages, with or without its attachments and
  /// tool calls; `summarize`: a summary stands for what is not kept (all of
  /// it, or what comes before the `last` messages). Returns the new
  /// thread, whose first message is the next one sent to it.
  pub async fn fork(
    &self,
    thread: String,
    upto: Option<String>,
    last: Option<u32>,
    no_attachments: bool,
    no_tools: bool,
    summarize: bool,
  ) -> Result<String> {
    let req = Request::Fork {
      thread,
      upto,
      last: last.map(|n| n as usize),
      no_attachments,
      no_tools,
      summarize,
    };
    match self.request(req).await? {
      Reply::Forked(id) => Ok(id),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Names a thread, on every node.
  pub async fn rename(&self, thread: String, title: String) -> Result<()> {
    self.done(Request::Rename { thread, title }).await
  }

  /// The user sees a thread (its chat is open and shows it): kept, when it
  /// moved since they last did, so loose ends tell it read (design §2.10).
  pub async fn opened(&self, thread: String) -> Result<()> {
    self.done(Request::Opened { thread }).await
  }

  /// Pins a thread (first in lists), or unpins it, on every node.
  pub async fn pin(&self, thread: String, on: bool) -> Result<()> {
    self.done(Request::Pin { thread, on }).await
  }

  /// Presses button `item` of action set `set` (its entry's `call`) in
  /// `thread` (design §2.30, step 2): the node that offered it runs the
  /// call, after a turn running on the thread; no model call unless the
  /// button wakes the agent.
  pub async fn press(&self, thread: String, set: String, item: u32) -> Result<Pressed> {
    match self.request(Request::Press { thread, set, item }).await? {
      Reply::Lines(l) if !l.is_empty() => {
        let v: serde_json::Value =
          serde_json::from_str(&l[0]).map_err(|e| StitchError::of(e.to_string()))?;
        Ok(Pressed {
          ok: v["ok"].as_bool().unwrap_or(false),
          output: v["output"].as_str().unwrap_or_default().to_owned(),
        })
      }
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// What sharing `thread` as a file would hold (design §2.31): its
  /// conversation, or with `effort` the spawn tree it is in, at `level`
  /// (`messages`, `calls`, `full`; empty: `calls`), redacted, with what
  /// the leak scan found. Nothing is written.
  pub async fn share_preview(
    &self,
    thread: String,
    effort: bool,
    level: String,
  ) -> Result<SharePreview> {
    self.share(thread, effort, level, true, None).await
  }

  /// Shares `thread` as a file in the node's downloads (`path` in the
  /// result, to hand to the share sheet). With findings, written only when
  /// `confirm` is the `hash` of a preview the user saw.
  pub async fn share_export(
    &self,
    thread: String,
    effort: bool,
    level: String,
    confirm: Option<String>,
  ) -> Result<SharePreview> {
    self.share(thread, effort, level, false, confirm).await
  }

  /// What a shared `.stitch` file would import (design §2.31), verified
  /// whole, nothing written: its title, parts, counts and signer, or the
  /// threads it made when it was imported before (`already`).
  pub async fn import_preview(&self, path: String) -> Result<ImportedThread> {
    self.import(path, true).await
  }

  /// Imports a shared `.stitch` file (AirDrop, Messages, Mail: the file
  /// the app was opened with), after `import_preview`: its blobs and new
  /// threads written, foreign history with no tools until the user lifts
  /// it.
  pub async fn import_thread(&self, path: String) -> Result<ImportedThread> {
    self.import(path, false).await
  }

  /// Rewinds a thread to message `upto` (its start: `None`), design §2.15:
  /// it goes on from there, on every node, what came after set aside
  /// (stored, out of its transcript, context and search). Edit & Resend
  /// and Retry rewind to the message before the user's.
  pub async fn rewind(&self, thread: String, upto: Option<String>) -> Result<()> {
    self.done(Request::Rewind { thread, upto }).await
  }

  /// The threads deleting `threads` would delete (each with its compaction
  /// chain), as `<id>\t<title>\t<role>` (`this`, `earlier`, `later`: a
  /// part of its conversation); deleted unless `dry_run` (design §2.5,
  /// "Deleting a thread"): for good, on every node, never the threads they
  /// spawned or their forks. The app asks the user first.
  pub async fn delete_threads(&self, threads: Vec<String>, dry_run: bool) -> Result<Vec<String>> {
    let req = Request::DeleteThreads {
      threads,
      spawned_by: None,
      dry_run,
      from_thread: None,
    };
    match self.request(req).await? {
      Reply::Lines(lines) => Ok(lines),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Has a thread run on `node` (design §2.7, "Continue on another
  /// node"): what is typed into it here goes there, and its turns stream
  /// back to `chat` as they run; `None`: back here ("Continue here").
  /// Returns what was done, as said.
  pub async fn run_on(&self, thread: String, node: Option<String>) -> Result<String> {
    match self.request(Request::RunOn { thread, node }).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The step whose calls run now on a thread, when a turn this app did
  /// not start runs one (a report's, a detached thread's, another
  /// node's): here, or asked of the node that runs the thread
  /// (`Request::InFlight`). `None` when none runs, or it could not say.
  pub async fn in_flight(&self, thread: String) -> Option<StepInFlightInfo> {
    let Ok(Reply::Lines(lines)) = self.request(Request::InFlight { thread }).await else {
      return None;
    };
    let step = stitch_node::chat::StepInFlight::from_json(lines.first()?)?;
    Some(StepInFlightInfo {
      running: step.running.map(|i| i as u32),
      calls: step
        .calls
        .into_iter()
        .map(|c| CallInFlightInfo {
          headline: c.headline(),
          input: c.input.to_string(),
          since_ms: c.since.map(|t| t.as_millisecond()),
          name: c.name,
          summary: c.summary,
          ok: c.ok,
        })
        .collect(),
    })
  }

  /// The user's other nodes, each asked whether it runs programs (shells),
  /// for "Continue on…": only those that do, or could not be asked, are
  /// offered (`run_on` refuses one that says it runs none).
  pub async fn runners(&self) -> Vec<NodeRuns> {
    match self.request(Request::Runners).await {
      Ok(Reply::Lines(lines)) => lines
        .iter()
        .filter_map(|l| l.split_once('\t'))
        .map(|(name, runs)| NodeRuns {
          name: name.to_owned(),
          runs: match runs {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
          },
          other_build: runs == "other-build",
        })
        .collect(),
      _ => Vec::new(),
    }
  }

  /// The thread a conversation goes on in now (design §2.15, "One
  /// conversation"): `thread`'s compactions followed to the newest;
  /// `thread` itself when it was never compacted, or is not an ID. What
  /// the app opens for an ID it was given (a link, a notification, a
  /// report, what it kept).
  pub fn newest(&self, thread: String) -> String {
    let Ok(id) = thread.parse::<RecordId>() else {
      return thread;
    };
    let index = self.index.lock().expect("index lock");
    index.compaction_newest(&id).to_string()
  }

  /// The node a thread was set to run on (`run_on`), when it is one;
  /// `None`: it runs where it is typed into.
  pub fn runs_on(&self, thread: String) -> Result<Option<String>> {
    let id: RecordId = thread.parse().map_err(StitchError::of)?;
    let index = self.index.lock().expect("index lock");
    Ok(stitch_node::meta::runs_on(&index, &id))
  }

  /// The node that owns a thread nobody chose a node for, when it is
  /// another than this one: where what is typed here runs by default
  /// (design §2.7, "Continue on another node"), when it runs programs.
  /// `None`: it is pinned (`runs_on`), or this node's own.
  pub fn owner_elsewhere(&self, thread: String) -> Result<Option<String>> {
    let id: RecordId = thread.parse().map_err(StitchError::of)?;
    let index = self.index.lock().expect("index lock");
    Ok(stitch_node::meta::owner_elsewhere(
      &index,
      &id,
      &self.node_name(),
    ))
  }

  /// Whether this device can run programs (shells): a phone cannot, so it
  /// offers no "Run here".
  pub fn runs_programs_here(&self) -> bool {
    stitch_node::meta::can_run_programs()
  }

  /// Whether a thread is read-only (the user's last switch, else as it
  /// was created).
  pub fn thread_read_only(&self, thread: String) -> Result<bool> {
    let id: RecordId = thread.parse().map_err(StitchError::of)?;
    let index = self.index.lock().expect("index lock");
    Ok(stitch_node::meta::read_only(&index, &id))
  }

  /// Switches a thread to read-only (`on`), or writable, on every node:
  /// the user's call (agents cannot). Read-only takes effect at its next
  /// tool call, writable at its next turn. Returns whether it is
  /// read-only now.
  pub async fn set_read_only(&self, thread: String, on: bool) -> Result<bool> {
    match self
      .request(Request::ReadOnly {
        thread,
        on: Some(on),
      })
      .await?
    {
      Reply::Lines(lines) => Ok(lines.first().is_some_and(|l| l == "read-only")),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Your own instructions for a place (`place_key`: a thread's
  /// `place_key`; `None`: every thread's), synced to every node and told
  /// to its threads before the place's files: (the place as named, the
  /// text, when there is one).
  pub async fn instructions(&self, place_key: Option<String>) -> Result<InstructionsInfo> {
    self.instructions_request(place_key, None).await
  }

  /// Writes your instructions for a place (empty: none), as `instructions`.
  pub async fn set_instructions(
    &self,
    place_key: Option<String>,
    text: String,
  ) -> Result<InstructionsInfo> {
    self.instructions_request(place_key, Some(text)).await
  }

  /// A thread's transcript as a Markdown file framed as quoted material,
  /// to attach to another chat; its path.
  pub async fn quote(&self, thread: String) -> Result<String> {
    self.lines(Request::Quote { thread }).await
  }

  /// Searches every thread's messages, by their words and (where memory
  /// runs) their meaning, best first: `stitch search`.
  pub async fn search(&self, query: String, limit: u32) -> Result<Vec<SearchHitInfo>> {
    let req = Request::Search {
      query,
      here: false,
      cwd: None,
      limit: limit as usize,
    };
    match self.request(req).await? {
      Reply::Hits(hits) => Ok(
        hits
          .into_iter()
          .map(|h| SearchHitInfo {
            thread: h.thread,
            title: h.title,
            msg: h.msg,
            at: h.at,
            sender: h.sender,
            snippet: h.snippet,
            found_by: h.found_by,
          })
          .collect(),
      ),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Stops the chats running: each ends `interrupted`.
  pub fn interrupt(&self) {
    for c in self.chats.lock().expect("chats lock").iter() {
      let _ = c.tx.send(Request::Interrupt);
    }
  }

  /// Joins the user's other nodes by a ticket (`stitch invite` on one of
  /// them, scanned from its QR code): the node stops meanwhile.
  pub async fn join(&self, ticket: String) -> Result<Joined> {
    let ticket: stitch_net::Ticket = ticket.trim().parse().map_err(StitchError::of)?;
    let (data, name) = (self.data.clone(), self.name.clone());
    let mut node = self.node.lock().await;
    let running = node.take();
    // Every home workspace is the repository `home`: what this one holds
    // from before joining is kept, and syncs with the fleet's.
    let (joined, started) = self
      .rt
      .spawn(async move {
        let stopped = match running {
          Some(n) => n.stop().await,
          None => Ok(()),
        };
        let joined = match stopped {
          Ok(()) => setup::join(&data, &name, &ticket, &setup::default_home(&data)).await,
          Err(e) => Err(e),
        };
        // Started again whether it joined or not.
        (joined, stitch_node::start(&data).await)
      })
      .await
      .map_err(StitchError::of)?;
    *node = Some(started.map_err(StitchError::of)?);
    drop(node);
    let joined = joined.map_err(StitchError::of)?;
    let mut index = self.index.lock().expect("index lock");
    refresh_roots(&mut index);
    Ok(Joined {
      inviter: joined.inviter.name.to_string(),
      fleet: joined.fleet.iter().map(|p| p.name.to_string()).collect(),
      home: joined.home.map(|r| {
        format!(
          "{} records in, {} out; {} blobs in",
          r.received.len(),
          r.sent,
          r.blobs_received
        )
      }),
    })
  }

  /// Syncs with one peer now, by name (the node a pushed question came
  /// from: it is reachable, and others that are not hold nothing up).
  pub async fn sync_with(&self, peer: String) -> Result<Vec<String>> {
    match self.request(Request::Sync { peer: Some(peer) }).await? {
      Reply::Synced(lines) => Ok(lines),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Syncs with every peer now: a line per peer and repository.
  pub async fn sync(&self) -> Result<Vec<String>> {
    match self.request(Request::Sync { peer: None }).await? {
      Reply::Synced(lines) => Ok(lines),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  pub async fn status(&self) -> Result<NodeStatus> {
    match self.request(Request::Status).await? {
      Reply::Status {
        node,
        endpoint,
        repos,
        peers,
        ..
      } => Ok(NodeStatus {
        node,
        endpoint,
        repos,
        peers,
      }),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The next turn's effective model, including a thread's deliberate
  /// choice and the place's synced default, without pinning that default.
  pub async fn effective_model(
    &self,
    thread: Option<String>,
    place: Option<String>,
    model: Option<String>,
  ) -> Result<String> {
    match self
      .request(Request::EffectiveModel {
        thread,
        place,
        cwd: None,
        model,
      })
      .await?
    {
      Reply::Lines(lines) => lines
        .into_iter()
        .next()
        .ok_or_else(|| StitchError::of("no model")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The models the providers with a key offer.
  pub async fn models(&self) -> Result<Models> {
    match self.request(Request::Models { node: None }).await? {
      Reply::Models {
        models,
        problems,
        default,
      } => Ok(Models {
        models,
        problems,
        default_model: default,
      }),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The model used when a chat names none (`provider/model`); it takes
  /// effect when the node starts again.
  pub fn set_default_model(&self, model: String) -> Result<()> {
    let mut c = Config::load(&self.data).map_err(StitchError::of)?;
    c.default_model = Some(model);
    c.save(&self.data).map_err(StitchError::of)
  }

  /// Stores a provider's API key in the keychain (`anthropic`, `openai`,
  /// `grok`, `brave`, …).
  pub fn set_key(&self, provider: String, key: String) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
      return Err(StitchError::of("an empty key"));
    }
    keys::set(&self.data, &provider, key, Store::Keychain)
      .map(|_| ())
      .map_err(StitchError::of)
  }

  pub fn has_key(&self, provider: String) -> bool {
    keys::get(&self.data, &provider, &provider).is_some()
  }

  pub fn remove_key(&self, provider: String) -> Result<()> {
    keys::remove(&self.data, &provider).map_err(StitchError::of)
  }

  /// The secrets this device knows of (design §2.19): never their values.
  pub async fn secrets(&self) -> Result<Vec<SecretEntry>> {
    match self.secret_op(SecretOp::List).await? {
      Reply::Secrets(list) => Ok(
        list
          .into_iter()
          .map(|s| SecretEntry {
            name: s.name,
            scope: s.scope,
            scope_label: s.scope_label,
            device_only: s.device_only,
            here: s.here,
            nodes: s.nodes,
            used: s.used,
            updated: s.updated,
            from: s.from,
            rotate: s.rotate,
          })
          .collect(),
      ),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Sets a secret (`value` `None`: keeps it, to change `device_only`
  /// alone; `device_only` `None`: as it was, synced for a new one). What
  /// the node says it did.
  pub async fn set_secret(
    &self,
    name: String,
    scope: String,
    value: Option<String>,
    device_only: Option<bool>,
  ) -> Result<String> {
    if value.as_deref().is_some_and(|v| v.trim().is_empty()) {
      return Err(StitchError::of("an empty secret"));
    }
    match self
      .secret_op(SecretOp::Set {
        name,
        scope,
        value,
        device_only,
        file: false,
      })
      .await?
    {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// A secret's value, to show it: the app asks for the owner's
  /// authentication first.
  pub async fn reveal_secret(&self, name: String, scope: String) -> Result<String> {
    match self.secret_op(SecretOp::Get { name, scope }).await? {
      Reply::Secret(value) => Ok(value),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Removes a secret here and, unless this device's only, everywhere.
  pub async fn remove_secret(&self, name: String, scope: String) -> Result<String> {
    match self.secret_op(SecretOp::Remove { name, scope }).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The variables.
  pub async fn variables(&self) -> Result<Vec<VariableEntry>> {
    match self.secret_op(SecretOp::VarList).await? {
      Reply::Vars(list) => Ok(
        list
          .into_iter()
          .map(|v| VariableEntry {
            name: v.name,
            scope: v.scope,
            scope_label: v.scope_label,
            value: v.value,
            updated: v.updated,
            from: v.from,
          })
          .collect(),
      ),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  pub async fn set_variable(&self, name: String, scope: String, value: String) -> Result<()> {
    self
      .secret_op(SecretOp::VarSet { name, scope, value })
      .await
      .map(|_| ())
  }

  pub async fn remove_variable(&self, name: String, scope: String) -> Result<()> {
    self
      .secret_op(SecretOp::VarRemove { name, scope })
      .await
      .map(|_| ())
  }

  /// The SSH hosts (design §2.20).
  pub async fn ssh_hosts(&self) -> Result<Vec<SshHostEntry>> {
    match self.ssh_op(SshOp::Hosts).await? {
      Reply::SshHosts(list) => Ok(
        list
          .into_iter()
          .map(|h| SshHostEntry {
            from_config: h.from == "ssh-config",
            name: h.name,
            address: h.address,
            port: h.port,
            user: h.user,
            jump: h.jump,
            auth: if h.auth.is_empty() {
              "auto".into()
            } else {
              h.auth
            },
            approval: if h.approval.is_empty() {
              "relaxed".into()
            } else {
              h.approval
            },
            tmux: !h.no_tmux,
          })
          .collect(),
      ),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Adds a host, or replaces the one of that name.
  pub async fn set_ssh_host(&self, host: SshHostEntry) -> Result<()> {
    self
      .ssh_op(SshOp::HostSet(stitch_node::api::SshHost {
        name: host.name.trim().to_owned(),
        address: host.address.trim().to_owned(),
        port: host.port,
        user: host.user.trim().to_owned(),
        jump: host.jump.trim().to_owned(),
        auth: host.auth,
        approval: host.approval,
        no_tmux: !host.tmux,
        from: String::new(),
      }))
      .await
      .map(|_| ())
  }

  pub async fn remove_ssh_host(&self, name: String) -> Result<()> {
    self.ssh_op(SshOp::HostRemove { name }).await.map(|_| ())
  }

  pub async fn ssh_host_keys(&self) -> Result<Vec<SshHostKeyEntry>> {
    match self.ssh_op(SshOp::HostKeys).await? {
      Reply::SshHostKeys(list) => Ok(
        list
          .into_iter()
          .map(|k| SshHostKeyEntry {
            host: k.host,
            name: k.name,
            fingerprint: k.fingerprint,
            trusted: k.trusted,
            node: k.node,
          })
          .collect(),
      ),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Forgets a host's keys: the next connection trusts what it finds.
  pub async fn forget_ssh_host_key(&self, host: String) -> Result<String> {
    match self.ssh_op(SshOp::HostKeyRemove { host }).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// This device's SSH key (made the first time: in the Secure Enclave on
  /// an iPhone).
  pub async fn ssh_device_key(&self) -> Result<SshDeviceKey> {
    match self.ssh_op(SshOp::Key).await? {
      Reply::SshKey {
        line,
        fingerprint,
        kind,
      } => Ok(SshDeviceKey {
        line,
        fingerprint,
        kind,
      }),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Adds this device's key to `host`'s `authorized_keys`: here when this
  /// device gets in, else asked of the nodes that reach it. What happened.
  pub async fn authorize_on_ssh_host(&self, host: String) -> Result<String> {
    match self
      .ssh_op(SshOp::Authorize {
        host,
        key: None,
        node: None,
      })
      .await?
    {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  pub async fn ssh_authorizations(&self) -> Result<Vec<SshAuthorizationEntry>> {
    match self.ssh_op(SshOp::Authorizations).await? {
      Reply::SshAuthorizations(list) => Ok(
        list
          .into_iter()
          .map(|a| SshAuthorizationEntry {
            host: a.host,
            key: a.key,
            asked: a.asked,
            state: a.state,
            said: a.said,
          })
          .collect(),
      ),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Signs in to a subscription (`anthropic`, `openai`, `grok`), as
  /// `stitch auth login` does: the node listens for the sign-in page's
  /// redirect on this device, and `browser` shows the page. The tokens go
  /// to the keychain.
  pub async fn sign_in(&self, service: String, browser: Arc<dyn SignInBrowser>) -> Result<()> {
    let service = Service::parse(&service).map_err(StitchError::of)?;
    let data = self.data.clone();
    // Ready before the page shows, so closing it at once still cancels.
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    *self.signing_in.lock().expect("sign-in lock") = Some(cancel);
    let done = self
      .rt
      .spawn(async move {
        let login = stitch_node::auth::login(&data, service, Store::Keychain, |url| {
          browser.show(url.to_owned())
        });
        tokio::select! {
          r = login => r.map(|_| ()).map_err(StitchError::of),
          // The listener and the sign-in's lock go with the login.
          _ = cancelled => Err(StitchError::of("sign-in cancelled")),
        }
      })
      .await;
    self.signing_in.lock().expect("sign-in lock").take();
    done.map_err(StitchError::of)?
  }

  /// Stops a sign-in waiting for its redirect (its page was closed).
  pub fn cancel_sign_in(&self) {
    if let Some(cancel) = self.signing_in.lock().expect("sign-in lock").take() {
      let _ = cancel.send(());
    }
  }

  /// Whether a subscription is signed in: `signed out`, or `signed in (…)`.
  pub async fn sign_in_status(&self, service: String) -> Result<String> {
    let service = Service::parse(&service).map_err(StitchError::of)?;
    let data = self.data.clone();
    self
      .rt
      .spawn(async move { stitch_node::auth::status(&data, service).await })
      .await
      .map_err(StitchError::of)?
      .map_err(StitchError::of)
  }

  pub async fn sign_out(&self, service: String) -> Result<()> {
    let service = Service::parse(&service).map_err(StitchError::of)?;
    let data = self.data.clone();
    self
      .rt
      .spawn(async move { stitch_node::auth::logout(&data, service).await })
      .await
      .map_err(StitchError::of)?
      .map_err(StitchError::of)
  }

  /// Signs in to GitHub with the device flow (design §2.8): `prompt` shows
  /// the code to enter, and the call returns the account's login once it
  /// was. The token goes to the keychain.
  pub async fn github_sign_in(&self, prompt: Arc<dyn GitHubPrompt>) -> Result<String> {
    let data = self.data.clone();
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    *self.github_signing_in.lock().expect("sign-in lock") = Some(cancel);
    let done = self
      .rt
      .spawn(async move {
        let login = stitch_node::github::login(&data, Store::Keychain, |p| {
          prompt.show(p.user_code.clone(), p.verification_uri.clone())
        });
        tokio::select! {
          r = login => r.map(|a| a.login).map_err(StitchError::of),
          _ = cancelled => Err(StitchError::of("sign-in cancelled")),
        }
      })
      .await;
    self.github_signing_in.lock().expect("sign-in lock").take();
    done.map_err(StitchError::of)?
  }

  /// Stops a GitHub sign-in waiting for its code to be entered.
  pub fn cancel_github_sign_in(&self) {
    if let Some(cancel) = self.github_signing_in.lock().expect("sign-in lock").take() {
      let _ = cancel.send(());
    }
  }

  /// The GitHub account signed in here, if any.
  pub fn github_account(&self) -> Option<String> {
    stitch_node::github::account(&self.data).map(|a| a.login)
  }

  pub fn github_sign_out(&self) -> Result<()> {
    stitch_node::github::logout(&self.data).map_err(StitchError::of)
  }

  /// Clones a project on this device, or fetches its clone (design
  /// §2.16): read-only, from its `origin`, with the GitHub sign-in for
  /// GitHub; a line saying what it did.
  pub async fn project_fetch(&self, repo: String) -> Result<String> {
    match self.request(Request::ProjectFetch { repo }).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// What to ask before a project's first clone on this device ("Clone
  /// github.com/o/r to this device (123 MB on GitHub)"), or none: cloned
  /// already, or answered (`again`: a "no" is asked again).
  pub async fn project_clone_question(&self, repo: String, again: bool) -> Result<Option<String>> {
    match self
      .request(Request::ProjectCloneQuestion { repo, again })
      .await?
    {
      Reply::Lines(lines) => Ok(lines.into_iter().next()),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The user's answer to cloning a project on this device: remembered;
  /// a yes clones it now. A line saying what it did.
  pub async fn project_clone(&self, repo: String, allow: bool) -> Result<String> {
    self.lines(Request::ProjectClone { repo, allow }).await
  }

  /// A project's files on this device: its clone's (`project_fetch`),
  /// read-only; `synced` is false for all, as none is edited here. Only
  /// listed, not read: a repository can be large.
  pub fn project_files(&self, repo: String) -> Result<Vec<FolderFile>> {
    let dir = self.clone_dir(&repo)?;
    let mut out = Vec::new();
    let mut walk = walkdir::WalkDir::new(&dir).min_depth(1).into_iter();
    while let Some(Ok(e)) = walk.next() {
      if e.file_name() == ".git" {
        if e.file_type().is_dir() {
          walk.skip_current_dir();
        }
        continue;
      }
      if !e.file_type().is_file() {
        continue;
      }
      let Some(rel) = e
        .path()
        .strip_prefix(&dir)
        .ok()
        .and_then(|r| r.to_str())
        .map(str::to_owned)
      else {
        continue;
      };
      out.push(FolderFile {
        path: rel,
        size: e.metadata().map(|m| m.len()).unwrap_or(0),
        synced: false,
      });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
  }

  /// A file of a project's clone, as text.
  pub fn project_read(&self, repo: String, path: String) -> Result<String> {
    let dir = self.clone_dir(&repo)?;
    let rel = Path::new(&path);
    if rel.is_absolute()
      || rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(n) if n != ".git"))
    {
      return Err(StitchError::of(format!(
        "not a path in the project: {path}"
      )));
    }
    let bytes = std::fs::read(dir.join(rel)).map_err(StitchError::of)?;
    if bytes.len() > 1024 * 1024 {
      return Err(StitchError::of("larger than 1 MB: not shown here"));
    }
    String::from_utf8(bytes).map_err(|_| StitchError::of("not text: not shown here"))
  }

  /// The branches of a project's origin, the clone's first.
  pub fn project_branches(&self, repo: String) -> Result<Vec<BranchInfo>> {
    let repo: stitch_records::RepoId = repo.parse().map_err(StitchError::of)?;
    Ok(
      stitch_node::clones::branches(&self.data, &repo)
        .map_err(StitchError::of)?
        .into_iter()
        .map(|b| BranchInfo {
          name: b.name,
          current: b.current,
        })
        .collect(),
    )
  }

  /// Moves a project's clone to another branch (fetched first).
  pub async fn project_switch(&self, repo: String, branch: String) -> Result<String> {
    self.lines(Request::ProjectSwitch { repo, branch }).await
  }

  /// What a project's clone changed from its branch.
  pub fn project_changes(&self, repo: String) -> Result<Vec<ChangedFile>> {
    let repo: stitch_records::RepoId = repo.parse().map_err(StitchError::of)?;
    Ok(
      stitch_node::clones::changes(&self.data, &repo)
        .map_err(StitchError::of)?
        .into_iter()
        .map(|c| ChangedFile {
          path: c.path,
          status: c.status.into(),
        })
        .collect(),
    )
  }

  /// A project's clone's changes as a diff, `path`'s only when given.
  pub fn project_diff(&self, repo: String, path: Option<String>) -> Result<String> {
    let repo: stitch_records::RepoId = repo.parse().map_err(StitchError::of)?;
    stitch_node::clones::diff(&self.data, &repo, path.as_deref()).map_err(StitchError::of)
  }

  /// Puts a project's clone's files back as its branch has them.
  pub async fn project_discard(&self, repo: String, path: Option<String>) -> Result<String> {
    self.lines(Request::ProjectDiscard { repo, path }).await
  }

  /// Writes a file of a project's clone.
  pub fn project_write(&self, repo: String, path: String, text: String) -> Result<()> {
    let dir = self.clone_dir(&repo)?;
    let rel = Path::new(&path);
    if rel.is_absolute()
      || rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(n) if n != ".git"))
    {
      return Err(StitchError::of(format!(
        "not a path in the project: {path}"
      )));
    }
    let file = dir.join(rel);
    if let Some(p) = file.parent() {
      std::fs::create_dir_all(p).map_err(StitchError::of)?;
    }
    std::fs::write(&file, text).map_err(StitchError::of)
  }

  /// Commits a project's clone's changes and pushes them through GitHub:
  /// to its branch, or to `new_branch` (a pull request into its branch
  /// when asked). A line saying what it did.
  pub async fn project_commit(
    &self,
    repo: String,
    message: String,
    new_branch: Option<String>,
    pull_request: bool,
  ) -> Result<String> {
    self
      .lines(Request::ProjectCommit {
        repo,
        message,
        new_branch,
        pull_request,
      })
      .await
  }

  /// This device's push token (APNs), `env` `sandbox` for a development
  /// build, and the public half of its push key (X25519, base64; the
  /// private half stays in its keychain): recorded so the user's nodes push
  /// questions to it, sealed for it.
  pub async fn register_push(&self, token: String, env: String, key: String) -> Result<()> {
    match self
      .request(Request::RegisterDevice { token, env, key })
      .await?
    {
      Reply::Ok => Ok(()),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// What an app's origin answers (design §2.10): the web view's URL
  /// scheme handler (`stitch-app://<host><path>`) asks with each request.
  /// Never fails: an error is a response too.
  pub async fn app_serve(
    &self,
    host: String,
    path: String,
    method: String,
    body: Vec<u8>,
  ) -> AppResponse {
    let serve = match self.node.lock().await.as_ref() {
      Some(n) => n.serve_app(stitch_node::apps::serve::Request {
        host,
        path,
        method,
        body,
      }),
      None => {
        return AppResponse {
          status: 503,
          headers: Vec::new(),
          body: b"the node is stopped".to_vec(),
        };
      }
    };
    match self.rt.spawn(serve).await {
      Ok(r) => AppResponse {
        status: r.status,
        headers: r
          .headers
          .into_iter()
          .map(|(name, value)| AppHeader { name, value })
          .collect(),
        body: r.body,
      },
      Err(e) => AppResponse {
        status: 500,
        headers: Vec::new(),
        body: e.to_string().into_bytes(),
      },
    }
  }

  /// The host an app's version is served at (`hash`: its attachment's).
  pub fn app_host(&self, hash: String) -> Result<String> {
    let h: BlobHash = hash.parse().map_err(StitchError::of)?;
    Ok(stitch_node::apps::version_host(&h))
  }

  /// The pinned apps, in the order they were first pinned.
  pub async fn pinned_apps(self: Arc<Self>) -> Result<Vec<PinnedApp>> {
    self.blocking(|s| s.pinned_apps_now()).await?
  }

  /// Pins the version `hash` (an app's attachment), or the latest version
  /// a thread showed (`hash`: its ID): a new pinned app, or the newer
  /// version of the one pinned from the same thread with that name. It
  /// follows its thread: each newer version of its name moves it.
  pub async fn pin_app(
    &self,
    hash: String,
    name: Option<String>,
    icon: Option<String>,
  ) -> Result<PinnedApp> {
    match self
      .request(Request::AppPin {
        what: hash,
        name,
        icon,
        // Pinned from its conversation: it follows the newer versions
        // that conversation shows.
        fixed: Some(false),
      })
      .await?
    {
      Reply::Apps { apps, .. } => {
        let id = apps
          .into_iter()
          .next()
          .ok_or_else(|| StitchError::of("nothing was pinned"))?
          .id;
        self
          .pinned_apps_now()?
          .into_iter()
          .find(|p| p.id == id)
          .ok_or_else(|| StitchError::of("the pin is not read back"))
      }
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Unpins an app (its ID); its versions stay in their threads.
  pub async fn unpin_app(&self, id: String) -> Result<()> {
    match self.request(Request::AppUnpin { app: id }).await? {
      Reply::Ok => Ok(()),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// The app at `host` (`stitch-app://<host>/`), as the sheet asking to
  /// let it read live state names it, and the user's answer so far.
  pub fn app_live(&self, host: String) -> Result<AppLive> {
    let index = self.index.lock().expect("index lock");
    let (name, consent) = stitch_node::apps::live::asking(&index, &self.blobs, &host)
      .ok_or_else(|| StitchError::of("no such app here"))?;
    Ok(AppLive { name, consent })
  }

  /// The user's answer to an app asking to read the node's live state
  /// (threads, jobs, approvals): kept on its pin, or in its conversation.
  pub async fn app_allow_live(&self, host: String, allow: bool) -> Result<String> {
    match self
      .request(Request::AppLive {
        app: host,
        thread: None,
        allow,
      })
      .await?
    {
      Reply::Lines(l) => Ok(l.join("\n")),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Pins the built-in dashboard (or moves it to this version), allowed to
  /// read live state: the user asked for it.
  pub async fn pin_dashboard(&self) -> Result<PinnedApp> {
    match self.request(Request::Dashboard).await? {
      Reply::Apps { apps, .. } => {
        let id = apps
          .into_iter()
          .next()
          .ok_or_else(|| StitchError::of("nothing was pinned"))?
          .id;
        self
          .pinned_apps_now()?
          .into_iter()
          .find(|p| p.id == id)
          .ok_or_else(|| StitchError::of("the pin is not read back"))
      }
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Of these questions, those answered (or withdrawn) as far as this node
  /// knows: their notifications go. One it does not hold yet is not among
  /// them (a push can come before the sync that brings its question).
  pub async fn answered(self: Arc<Self>, questions: Vec<String>) -> Vec<String> {
    self
      .blocking(move |s| s.answered_now(questions))
      .await
      .unwrap_or_default()
  }

  /// The questions waiting on the user, from any node: approvals first,
  /// the oldest first.
  pub async fn questions(self: Arc<Self>) -> Result<Vec<Question>> {
    self.blocking(|s| s.questions_now()).await?
  }

  /// Answers a question: allowed (and from then on, with `always`), or
  /// declined with a reason. The answer reaches the node that asked at
  /// once.
  pub async fn answer(
    &self,
    question: String,
    allow: bool,
    always: bool,
    reason: Option<String>,
  ) -> Result<()> {
    match self
      .request(Request::Answer {
        question,
        allow,
        reason: reason.filter(|r| !r.trim().is_empty()),
        always,
      })
      .await?
    {
      Reply::Ok => Ok(()),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// A folder's files on this device (its copy, on a phone): every path,
  /// the carried ones (`synced`) editable.
  pub fn folder_files(&self, key: String) -> Result<Vec<FolderFile>> {
    let Some(dir) = self.folder_dir(&key) else {
      return Ok(Vec::new());
    };
    let ignore = stitch_node::folders::Ignore::of(&dir);
    let mut out = Vec::new();
    let mut walk = walkdir::WalkDir::new(&dir).min_depth(1).into_iter();
    while let Some(Ok(e)) = walk.next() {
      let Some(rel) = e
        .path()
        .strip_prefix(&dir)
        .ok()
        .and_then(|r| r.to_str())
        .map(str::to_owned)
      else {
        continue;
      };
      if ignore.ignores(&rel)
        || (e.file_type().is_dir() && stitch_node::folders::is_checkout(e.path()))
      {
        if e.file_type().is_dir() {
          walk.skip_current_dir();
        }
        continue;
      }
      if !e.file_type().is_file() {
        continue;
      }
      let size = e.metadata().map(|m| m.len()).unwrap_or(0);
      let synced = size <= stitch_node::folders::Limits::default().file;
      out.push(FolderFile {
        path: rel,
        size,
        synced,
      });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
  }

  /// A file of a folder, as text.
  pub fn folder_read(&self, key: String, path: String) -> Result<String> {
    let file = self.folder_file(&key, &path)?;
    let bytes = std::fs::read(&file).map_err(StitchError::of)?;
    String::from_utf8(bytes).map_err(|_| StitchError::of("not text: not shown here"))
  }

  /// Writes a file of a folder, then syncs the folder.
  pub async fn folder_write(&self, key: String, path: String, text: String) -> Result<FolderPass> {
    let file = self.folder_file(&key, &path)?;
    if let Some(parent) = file.parent() {
      std::fs::create_dir_all(parent).map_err(StitchError::of)?;
    }
    std::fs::write(&file, text).map_err(StitchError::of)?;
    self.folder_sync(key).await
  }

  /// Syncs a folder now: what changed here published, what other machines
  /// published merged in.
  pub async fn folder_sync(&self, key: String) -> Result<FolderPass> {
    match self.request(Request::FolderSync { key }).await? {
      Reply::FolderPass {
        published,
        applied,
        conflicts,
        skipped,
      } => Ok(FolderPass {
        published,
        applied: applied as u32,
        conflicts,
        skipped,
      }),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// A folder's changes, or one file's, the latest first.
  pub fn folder_log(&self, key: String, path: Option<String>) -> Result<Vec<FolderChange>> {
    let index = self.index.lock().expect("index lock");
    Ok(
      stitch_node::folders::log(&index, &key, path.as_deref())
        .map_err(StitchError::of)?
        .into_iter()
        .map(|c| FolderChange {
          id: c.id.to_string(),
          at: c.at.as_second(),
          node: c.node,
          thread: c.thread,
          changed: c.changed,
          merge: c.merge,
        })
        .collect(),
    )
  }

  /// What a change did, as a unified diff (`to`: the latest by default;
  /// `from`: what it follows by default), `path` only when given.
  pub fn folder_diff(
    &self,
    key: String,
    from: Option<String>,
    to: Option<String>,
    path: Option<String>,
  ) -> Result<String> {
    let parse = |s: Option<String>| -> Result<Option<RecordId>> {
      s.map(|s| s.parse().map_err(StitchError::of)).transpose()
    };
    let (from, to) = (parse(from)?, parse(to)?);
    let index = self.index.lock().expect("index lock");
    stitch_node::folders::diff(
      &index,
      &self.blobs,
      &key,
      from.as_ref(),
      to.as_ref(),
      path.as_deref(),
    )
    .map_err(StitchError::of)
  }

  /// Memory on this device.
  pub async fn memory(&self) -> Result<MemoryInfo> {
    let (installed, images) = stitch_node::memory::installed(&self.data);
    match self
      .request(Request::Memory {
        thread: None,
        cwd: None,
        global: false,
      })
      .await?
    {
      Reply::Memory {
        available,
        facts,
        episodes,
        ..
      } => Ok(MemoryInfo {
        installed,
        images,
        running: available,
        facts,
        episodes,
      }),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Downloads memory's model (EmbeddingGemma 2, about 310 MB; with
  /// `images`, its vision projector, 555 MB more), then starts the node
  /// again with memory on, as `stitch memory install` does.
  pub async fn install_memory(
    &self,
    images: bool,
    progress: Arc<dyn InstallProgress>,
  ) -> Result<()> {
    let data = self.data.clone();
    self
      .rt
      .spawn(async move {
        stitch_node::memory::install(&data, images, |name, got, total| {
          progress.progress(name.to_owned(), got, total)
        })
        .await
      })
      .await
      .map_err(StitchError::of)?
      .map_err(StitchError::of)?;
    self.restart().await
  }

  /// Removes memory's model (`images`: only the vision projector), then
  /// starts the node again without it. What it remembered stays, synced.
  pub async fn remove_memory(&self, images: bool) -> Result<()> {
    stitch_node::memory::uninstall(&self.data, images).map_err(StitchError::of)?;
    self.restart().await
  }

  /// Whether a thread is in memory; `None` where memory does not run.
  pub async fn thread_memory(&self, thread: String) -> Result<Option<bool>> {
    match self
      .request(Request::Memory {
        thread: Some(thread),
        cwd: None,
        global: false,
      })
      .await?
    {
      Reply::Memory {
        available,
        thread_on,
        ..
      } => Ok(thread_on.filter(|_| available)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Takes a thread out of memory, or brings it back (caught up in the
  /// background), on every node.
  pub async fn remember(&self, thread: String, on: bool) -> Result<()> {
    match self
      .request(Request::Remember {
        thread,
        on,
        cwd: None,
      })
      .await?
    {
      Reply::Error(e) => Err(StitchError::of(e)),
      _ => Ok(()),
    }
  }

  /// The app is in front (`true`) or in the background: memory's encoder
  /// is unloaded while in the background, where iOS refuses the GPU.
  /// Back in front, iroh is told the network may have changed (its relay
  /// connection and paths may have died while the app was suspended), a
  /// second at most, so a sync after this finds them renewed.
  pub async fn set_foreground(&self, on: bool) {
    let change = match self.node.lock().await.as_ref() {
      Some(n) => {
        n.set_foreground(on);
        on.then(|| n.network_change())
      }
      None => None,
    };
    if let Some(change) = change {
      let _ = self
        .rt
        .spawn(async move {
          // Made on the node's runtime: its timer, not the caller's.
          tokio::time::timeout(std::time::Duration::from_secs(1), change).await
        })
        .await;
    }
  }

  /// Stops the node (the app is going away).
  pub async fn stop(&self) -> Result<()> {
    if let Some(n) = self.node.lock().await.take() {
      self
        .rt
        .spawn(n.stop())
        .await
        .map_err(StitchError::of)?
        .map_err(StitchError::of)?;
    }
    Ok(())
  }
}

impl Stitch {
  /// Runs `read` on a thread of the runtime's blocking pool: reads of the
  /// store and index take milliseconds to tens of them, which Swift must
  /// not wait for on its main thread (the app hitched every 3 s, typing
  /// and scrolling, while it read the questions there).
  async fn blocking<T: Send + 'static>(
    self: &Arc<Self>,
    read: impl FnOnce(&Stitch) -> T + Send + 'static,
  ) -> Result<T> {
    let me = Arc::clone(self);
    self
      .rt
      .spawn_blocking(move || read(&me))
      .await
      .map_err(StitchError::of)
  }

  fn revision_now(&self) -> u64 {
    let mut index = self.index.lock().expect("index lock");
    let mut seen = self.seen.lock().expect("seen lock");
    let version = index.data_version().ok();
    if version != seen.version {
      seen.version = version;
      seen.revision += 1;
    }
    if seen.scanned.is_none_or(|t| t.elapsed() >= Seen::SCAN_EVERY) {
      seen.scanned = Some(std::time::Instant::now());
      if refresh_roots(&mut index) {
        seen.revision += 1;
      }
    }
    seen.revision
  }

  fn threads_now(&self) -> Result<Vec<ThreadInfo>> {
    let index = self.index.lock().expect("index lock");
    let roots = roots(&index);
    let home = roots
      .iter()
      .find(|r| r.config().role == Role::Home)
      .map(|r| r.repo().clone());
    let places = place::Places::load(&index);
    // A compaction chain is one row, its newest thread, with the chain's
    // title and pin (design §2.15, "One conversation").
    let threads = index.listed_threads().map_err(StitchError::of)?;
    Ok(
      place::listed(threads, home.as_ref(), &Place::Global, true, &places)
        .into_iter()
        .map(|(r, at, p)| ThreadInfo {
          title: index
            .lineage_title(&r.id)
            .unwrap_or_else(|| "(untitled)".into()),
          at: at.as_second(),
          pinned: index.lineage_pinned(&r.id),
          state: stitch_node::chat::detached_state(&index, &r).ok().flatten(),
          place: (p != Place::Global).then(|| p.label(self.name.as_str(), &places)),
          place_key: (p != Place::Global).then(|| p.key()),
          read_only: stitch_node::meta::read_only(&index, &r.id),
          // Spawned from a thread compacted since: under the conversation
          // it goes on in, which the list has.
          parent: stitch_node::meta::parent(&index, &r.id)
            .map(|p| index.compaction_newest(&p).to_string()),
          earlier: {
            let mut chain = index.compaction_lineage(&r.id);
            chain.retain(|t| t != &r.id);
            chain.into_iter().map(|t| t.to_string()).collect()
          },
          goal: r
            .fields
            .get("goal")
            .and_then(|v| v.as_str())
            .and_then(|g| g.trim().lines().next())
            .map(|g| g.chars().take(200).collect()),
          room: r.fields.get("room") == Some(&serde_json::Value::Bool(true)),
          room_mode: (r.fields.get("room") == Some(&serde_json::Value::Bool(true)))
            .then(|| stitch_node::rooms::policy(&index, &r.id).mode),
          participants: if r.fields.get("room") == Some(&serde_json::Value::Bool(true)) {
            participants_of(&index, &r.id)
          } else {
            Vec::new()
          },
          id: r.id.to_string(),
        })
        .collect(),
    )
  }

  fn places_now(&self) -> Result<Vec<PlaceInfo>> {
    let index = self.index.lock().expect("index lock");
    let roots = roots(&index);
    let home = roots
      .iter()
      .find(|r| r.config().role == Role::Home)
      .map(|r| r.repo().clone());
    let places = place::Places::load(&index);
    let threads = index.threads().map_err(StitchError::of)?;
    let mut seen = std::collections::HashSet::new();
    let used = threads
      .iter()
      .map(|(r, _)| Place::of_thread(r, home.as_ref(), &places));
    let projects = places.listed().map(|p| Place::Project(p.repo.clone()));
    Ok(
      used
        .chain(projects)
        .filter(|p| *p != Place::Global && seen.insert(p.key()))
        .map(|p| PlaceInfo {
          key: p.key(),
          label: p.label(self.name.as_str(), &places),
        })
        .collect(),
    )
  }

  fn transcript_now(&self, thread: String) -> Result<Vec<Entry>> {
    let shown = self.shown(&thread)?;
    // A room's roster: its replies' speakers (design §2.27).
    let roster = thread
      .parse::<RecordId>()
      .ok()
      .map(|id| {
        let index = self.index.lock().expect("index lock");
        if stitch_node::rooms::is_room(&index, &id) {
          participants_of(&index, &id)
        } else {
          Vec::new()
        }
      })
      .unwrap_or_default();
    // Each divider, its compaction: the chain's, in order.
    let mut compactions = if shown.iter().any(|e| e.who == "compacted") {
      self.compactions(&thread).into_iter()
    } else {
      Vec::new().into_iter()
    };
    Ok(
      shown
        .into_iter()
        .map(|e| Entry {
          speaker: speaker_of(&e.who, &roster),
          scopes: e
            .scopes
            .iter()
            .map(|s| ScopeInfo {
              kind: s.kind.as_str().into(),
              id: s.id.to_string(),
              title: s.title.clone(),
              threads: s.threads as u32,
            })
            .collect(),
          compaction: (e.who == "compacted").then(|| compactions.next()).flatten(),
          replies: e
            .replies
            .iter()
            .map(|r| SuggestedReply {
              label: r.label.clone(),
              text: r.text.clone(),
            })
            .collect(),
          actions: e
            .actions
            .iter()
            .map(|a| ActionInfo {
              label: a.label.clone(),
              tool: a.tool.clone(),
              call: a.call(),
              wake: a.wake,
            })
            .collect(),
          call: e.call.clone(),
          press: e.press.as_ref().map(|p| PressInfo {
            set: p.set.clone(),
            item: p.item as u32,
            label: p.label.clone(),
            ok: p.ok,
            output: p.output.clone(),
          }),
          who: e.who,
          text: e.said,
          typed: e.typed,
          spoken: e.spoken,
          id: e.id,
          attachments: e
            .attachments
            .into_iter()
            .map(|a| AttachmentInfo {
              here: self.blobs.contains(&a.hash),
              hash: a.hash.to_string(),
              name: a.name,
              media_type: a.media_type,
              bytes: a.bytes,
            })
            .collect(),
        })
        .collect(),
    )
  }

  fn unfinished_now(&self, thread: String) -> Result<Option<UnfinishedTurn>> {
    self.reading(&thread, |ctx, id| {
      stitch_node::chat::unfinished(ctx, id)
        .map(|u| {
          u.map(|u| UnfinishedTurn {
            why: u.why.to_owned(),
            elsewhere: u.elsewhere,
          })
        })
        .map_err(StitchError::of)
    })
  }

  fn pinned_apps_now(&self) -> Result<Vec<PinnedApp>> {
    // Pins other devices made: synced since, indexed by the node.
    let index = self.index.lock().expect("index lock");
    Ok(
      stitch_node::apps::pinned(&index)
        .map_err(StitchError::of)?
        .into_iter()
        .map(PinnedApp::of)
        .collect(),
    )
  }

  fn answered_now(&self, questions: Vec<String>) -> Vec<String> {
    let index = self.index.lock().expect("index lock");
    questions
      .into_iter()
      .filter(|q| {
        let Ok(id) = q.parse::<RecordId>() else {
          return false;
        };
        matches!(index.record(&id), Ok(Some(_)))
          && index
            .question_answered(&stitch_records::Ref::to(id))
            .unwrap_or(false)
      })
      .collect()
  }

  fn questions_now(&self) -> Result<Vec<Question>> {
    let index = self.index.lock().expect("index lock");
    let mut out = Vec::new();
    // The questions alone: `attention` also works out every finding and
    // task, which the app never shows.
    for (id, title) in index.open_questions().map_err(StitchError::of)? {
      let Some(r) = index.record(&id).map_err(StitchError::of)? else {
        continue;
      };
      let thread = index
        .edges_from(&id)
        .ok()
        .and_then(|e| e.into_iter().find(|e| e.rel == "thread"))
        .map(|e| e.dst.id);
      let text = r
        .fields
        .get("command")
        .and_then(|c| c.as_str())
        .map(str::to_owned)
        .or(title)
        .unwrap_or_else(|| "(a question)".into());
      out.push(Question {
        thread_title: thread.as_ref().and_then(|t| index.title(t).ok().flatten()),
        thread: thread.map(|t| t.to_string()),
        text,
        node: r.node.clone(),
        at: r.created.as_second(),
        grant: r
          .fields
          .get("grant")
          .and_then(|g| g.as_array())
          .map(|g| {
            g.iter()
              .filter_map(|x| x.as_str().map(str::to_owned))
              .collect()
          })
          .unwrap_or_default(),
        detail: r
          .fields
          .get("detail")
          .and_then(|d| d.as_str())
          .map(str::to_owned),
        detail_file: r
          .fields
          .get("detail_file")
          .and_then(|h| h.as_str())
          .map(|hash| QuestionFile {
            hash: hash.to_owned(),
            name: r
              .fields
              .get("detail_file_name")
              .and_then(|n| n.as_str())
              .unwrap_or("diff.txt")
              .to_owned(),
          }),
        id: id.to_string(),
      });
    }
    out.sort_by_key(|q| q.at);
    Ok(out)
  }
}

impl Stitch {
  /// Stops the node and starts it again (what it runs at start changed),
  /// as the CLI's next command does after `stitch memory install`.
  async fn restart(&self) -> Result<()> {
    let mut node = self.node.lock().await;
    if let Some(n) = node.take() {
      self
        .rt
        .spawn(n.stop())
        .await
        .map_err(StitchError::of)?
        .map_err(StitchError::of)?;
    }
    let data = self.data.clone();
    *node = Some(
      self
        .rt
        .spawn(async move { stitch_node::start(&data).await })
        .await
        .map_err(StitchError::of)?
        .map_err(StitchError::of)?,
    );
    Ok(())
  }

  /// A folder's directory on this device: the folder, or its copy here.
  fn folder_dir(&self, key: &str) -> Option<PathBuf> {
    let index = self.index.lock().expect("index lock");
    let places = place::Places::load(&index);
    places
      .folders
      .dir(key, self.name.as_str())
      .or_else(|| Some(stitch_node::folders::copy_dir(&self.data, key)))
      .filter(|d| d.is_dir())
  }

  /// A project's clone here, when there is one.
  fn clone_dir(&self, repo: &str) -> Result<PathBuf> {
    let repo: stitch_records::RepoId = repo.parse().map_err(StitchError::of)?;
    let dir = stitch_node::clones::dir(&self.data, &repo);
    if dir.join(".git").is_dir() {
      Ok(dir)
    } else {
      Err(StitchError::of(
        "the project's code is not on this device yet: it is fetched when its chat opens",
      ))
    }
  }

  /// A file of a folder, its path checked to stay inside it.
  fn folder_file(&self, key: &str, path: &str) -> Result<PathBuf> {
    let dir = self
      .folder_dir(key)
      .ok_or_else(|| StitchError::of(format!("{key} is not on this device yet")))?;
    let rel = Path::new(path);
    if path.is_empty()
      || rel.is_absolute()
      || rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(n) if n != ".git"))
    {
      return Err(StitchError::of(format!("not a path in the folder: {path}")));
    }
    Ok(dir.join(rel))
  }

  /// A thread's transcript entries, attachments with them: over its
  /// compaction chain up to it (design §2.15, "One conversation"), each
  /// compaction a `compacted` entry.
  fn shown(&self, thread: &str) -> Result<Vec<stitch_node::chat::Shown>> {
    self.reading(thread, |ctx, id| {
      stitch_node::chat::lineage_shown(ctx, id, None).map_err(StitchError::of)
    })
  }

  /// The compactions of `thread`'s chain up to it, oldest first: what
  /// its transcript's `compacted` entries stand for, in their order.
  fn compactions(&self, thread: &str) -> Vec<CompactionInfo> {
    let Ok(id) = thread.parse::<RecordId>() else {
      return Vec::new();
    };
    let index = self.index.lock().expect("index lock");
    let mut chain = index.compaction_lineage(&id);
    if let Some(at) = chain.iter().position(|t| t == &id) {
      chain.truncate(at + 1);
    }
    chain
      .iter()
      .skip(1)
      .map(|t| {
        stitch_node::fork::compaction_of(&index, t)
          .map(|c| CompactionInfo::of(&c))
          .unwrap_or_else(|| CompactionInfo {
            from: String::new(),
            to: t.to_string(),
            model: String::new(),
            window: 0,
            tokens: None,
            kept: 0,
            summary: String::new(),
            said: "compacted here".into(),
          })
      })
      .collect()
  }

  /// `read` on a thread's records, from this node's store and index.
  fn reading<T>(
    &self,
    thread: &str,
    read: impl FnOnce(&ChatContext<'_>, &RecordId) -> Result<T>,
  ) -> Result<T> {
    let id: RecordId = thread.parse().map_err(StitchError::of)?;
    let root = {
      let index = self.index.lock().expect("index lock");
      let repo = index
        .record(&id)
        .map_err(StitchError::of)?
        .ok_or_else(|| StitchError::of(format!("no thread {id}")))?
        .repo;
      roots(&index)
        .into_iter()
        .find(|r| r.repo() == &repo)
        .ok_or_else(|| StitchError::of(format!("the repository of {id} is not here")))?
    };
    let ctx = ChatContext {
      node: &self.name,
      root: &root,
      index: &self.index,
      blobs: &self.blobs,
    };
    read(&ctx, &id)
  }

  /// A request whose reply is lines, joined.
  async fn lines(&self, req: Request) -> Result<String> {
    match self.request(req).await? {
      Reply::Lines(lines) => Ok(lines.join("\n")),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  async fn instructions_request(
    &self,
    place: Option<String>,
    set: Option<String>,
  ) -> Result<InstructionsInfo> {
    let global = place.is_none();
    match self
      .request(Request::Instructions {
        place,
        cwd: None,
        global,
        set,
      })
      .await?
    {
      Reply::Instructions { place, text } => Ok(InstructionsInfo { place, text }),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// SSH hosts, keys and authorizing (`Request::Ssh`).
  async fn ssh_op(&self, op: SshOp) -> Result<Reply> {
    match self.request(Request::Ssh { op }).await? {
      Reply::Error(e) => Err(StitchError::of(e)),
      r => Ok(r),
    }
  }

  /// Secrets and variables (`Request::Secret`).
  async fn secret_op(&self, op: SecretOp) -> Result<Reply> {
    match self.request(Request::Secret { op, cwd: None }).await? {
      Reply::Error(e) => Err(StitchError::of(e)),
      r => Ok(r),
    }
  }

  /// A request whose reply is `Ok`.
  async fn import(&self, path: String, preview: bool) -> Result<ImportedThread> {
    let bundle = std::fs::read(&path).map_err(|e| StitchError::of(format!("{path}: {e}")))?;
    match self.request(Request::Import { bundle, preview }).await? {
      Reply::Lines(l) if !l.is_empty() => {
        let i: stitch_node::share::Imported =
          serde_json::from_str(&l[0]).map_err(|e| StitchError::of(e.to_string()))?;
        Ok(ImportedThread {
          thread: i.thread,
          threads: i.threads,
          parts: i.parts,
          title: i.title,
          hash: i.hash,
          signer: i.signer,
          mine: i.mine,
          level: i.level.as_str().to_owned(),
          kind: i.kind,
          messages: i.messages as u32,
          calls: i.calls as u32,
          attachments: i.attachments as u32,
          apps: i.apps as u32,
          already: i.already,
          written: i.written,
        })
      }
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  async fn share(
    &self,
    thread: String,
    effort: bool,
    level: String,
    preview: bool,
    confirm: Option<String>,
  ) -> Result<SharePreview> {
    let req = Request::Share {
      thread,
      effort,
      level,
      preview,
      confirm,
    };
    match self.request(req).await? {
      Reply::Lines(l) if !l.is_empty() => {
        let p: stitch_node::share::Preview =
          serde_json::from_str(&l[0]).map_err(|e| StitchError::of(e.to_string()))?;
        Ok(p.into())
      }
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  async fn done(&self, req: Request) -> Result<()> {
    match self.request(req).await? {
      Reply::Ok => Ok(()),
      Reply::Error(e) => Err(StitchError::of(e)),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  /// Fetches blobs from peers (`FetchBlobs`); how many arrived.
  async fn fetch(&self, hashes: Vec<String>) -> Result<u32> {
    match self.request(Request::FetchBlobs { hashes }).await? {
      Reply::Fetched(n) => Ok(n as u32),
      other => Err(StitchError::of(format!("unexpected reply: {other:?}"))),
    }
  }

  async fn client(&self) -> Result<Client> {
    let connect = self
      .node
      .lock()
      .await
      .as_ref()
      .ok_or_else(|| StitchError::of("the node is stopped"))?
      .client();
    self
      .rt
      .spawn(connect)
      .await
      .map_err(StitchError::of)?
      .map_err(StitchError::of)
  }

  async fn request(&self, req: Request) -> Result<Reply> {
    let mut client = self.client().await?;
    self
      .rt
      .spawn(async move { client.request(&req).await })
      .await
      .map_err(StitchError::of)?
      .map_err(StitchError::of)
  }
}

/// Who said a room's reply: its entry's `who` is `<slug> · <model>`
/// (`chat::transcript_shown`), the slug one of the roster's. A slug the
/// roster no longer has (removed since) still names the speaker, with no
/// thread to open.
fn speaker_of(who: &str, roster: &[ParticipantInfo]) -> Option<SpeakerInfo> {
  if roster.is_empty() {
    return None;
  }
  let (slug, model) = match who.split_once(" · ") {
    Some((s, m)) => (s, Some(m.to_owned())),
    None => (who, None),
  };
  if slug.is_empty()
    || slug.contains(' ')
    || ["you", "tool", "output", "error", "note"].contains(&slug)
  {
    return None;
  }
  let known = roster.iter().find(|p| p.slug == slug);
  if known.is_none() && model.is_none() {
    return None;
  }
  Some(SpeakerInfo {
    slug: slug.to_owned(),
    name: known.map_or_else(|| slug.to_owned(), |p| p.name.clone()),
    thread: known.map(|p| p.thread.clone()),
    model: model.or_else(|| known.and_then(|p| p.model.clone())),
  })
}

/// What `Stitch::revision` last saw.
#[derive(Default)]
struct Seen {
  /// The index's `data_version`.
  version: Option<i64>,
  revision: u64,
  /// When the roots were last scanned for files nothing indexed.
  scanned: Option<std::time::Instant>,
}

impl Seen {
  /// The node indexes what it writes and what syncs bring; a scan only
  /// catches what came in otherwise. It lists every file of every root
  /// (about 11,000 on a desktop's), so not at every read: before, every
  /// read did, on the app's main thread.
  const SCAN_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
}

/// Every record root the index knows, as it is (not scanned: the node
/// indexes what it writes and syncs, and `revision` scans now and then).
fn roots(index: &Index) -> Vec<RecordRoot> {
  index
    .roots()
    .unwrap_or_default()
    .into_values()
    .filter_map(|dir| RecordRoot::open(Path::new(&dir)).ok())
    .collect()
}

/// Every record root, scanned for files the index lacks; whether any came
/// or went.
fn refresh_roots(index: &mut Index) -> bool {
  let mut changed = false;
  for r in roots(index) {
    if let Ok(report) = index.refresh(&r) {
      changed |= report.records + report.segments + report.removed > 0;
    }
  }
  changed
}
