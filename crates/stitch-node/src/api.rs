//! The local client protocol (design §2.1): the same binary on both ends,
//! postcard frames (a big-endian `u32` length, then the message) over the
//! node's Unix socket. A connection opens with `Hello` both ways and the
//! versions must match; then each request gets its replies, a chat streaming
//! `Text` and `Thinking` before `ChatDone`.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Error, Result};

const MAX_FRAME: usize = 64 << 20;

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
  Hello {
    version: String,
  },
  Chat {
    /// A thread ID, `last` (the latest of the place `cwd` is in, else the
    /// latest global one), or `None` for a new thread.
    thread: Option<String>,
    text: String,
    /// Files on this machine, read by the node, or images in its blob store (`blake3:…`).
    attachments: Vec<String>,
    /// `provider/model`; `None` for the configured default.
    model: Option<String>,
    /// The repository a new thread lives in; `None`: from `cwd` (see
    /// `place`).
    repo: Option<String>,
    /// Where the client runs: a new thread's place, and where shell
    /// sessions start when it is inside the thread's place.
    cwd: Option<String>,
    /// A new thread belongs to no folder.
    #[serde(default)]
    global: bool,
    /// The client's environment: shell sessions this chat starts use it
    /// (its PATH, SSH agent, credential helpers) instead of the node's.
    #[serde(default)]
    env: Vec<(String, String)>,
    /// Changes to the thread's tool and skill sets (`+x`, `-x`, design
    /// §2.14), written before this turn: a new thread's, from its client.
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    /// The place a new thread belongs to, by its key (`global`,
    /// `project:<repo>`, `folder:<folder>`), for a client with no
    /// directory there (the phone); it wins over `repo`, `cwd` and
    /// `global`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    place: Option<String>,
    /// A new thread starts in memory (`Some(true)`) or out of it, as
    /// `/memory` chose before its first message; `None`: as its place
    /// says. Ignored for an existing thread.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    memory: Option<bool>,
    /// Who this turn is for, when a client serves other people (the
    /// Discord bridge: `discord:<user id>`): its model calls carry it as
    /// `x-stitch-end-user` (and the thread as `x-stitch-thread`), for a
    /// key-holding proxy to account by (design §2.21).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    end_user: Option<String>,
    /// The node a new thread's turns run on (design §2.7, "A new chat on
    /// another node"): written here, set to run there, its first message
    /// passed on as a chat into it. `None`, empty or this node: here.
    /// Ignored for an existing thread (`RunOn` moves one).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    node: Option<String>,
    /// Said in a voice call (the app's, design §2.26): the model is told,
    /// with the message, that its reply will be read aloud.
    #[serde(default)]
    spoken: bool,
    /// Threads and efforts the user attached to the message (design §2.27,
    /// docs/rooms.md §3): the node resolves them and writes their briefing
    /// with it (`Part::Scope`).
    #[serde(default)]
    scopes: Vec<ScopeTarget>,
  },
  /// Shell commands run without asking (`on`), or ask again; `None` only
  /// says which. `Mode` says more (`auto`).
  Yolo {
    on: Option<bool>,
  },
  /// The approval mode (design §2.28): `ask` (the default: sandboxed where
  /// the system can, else asking), `auto` (yolo with the hazard guard) or
  /// `yolo`; `None` only says which. From a thread's shell, a change asks
  /// the user first. `Reply::Lines`. `cwd`: the client's directory, whose
  /// place may set its own mode (`approval` in its settings): the lines
  /// then say the mode there first, and the node's.
  Mode {
    mode: Option<String>,
    cwd: Option<String>,
  },
  /// Syncs every held repository with one peer, or with all.
  Sync {
    peer: Option<String>,
  },
  /// Fetches blobs this node lacks (`blake3:<hex>`) from whichever peer
  /// holds them (a phone's, before they are needed); the reply is
  /// `Fetched` with how many arrived.
  FetchBlobs {
    hashes: Vec<String>,
  },
  /// Syncs a folder place's files now (design §2.16), `key` as `Places`
  /// knows it (`~/notes`); the reply is `FolderPass`.
  FolderSync {
    key: String,
  },
  /// Clones a project here, or fetches its clone (a phone's, design
  /// §2.16); the reply is a line saying what it did.
  ProjectFetch {
    repo: String,
  },
  /// What to ask the user before a project's first clone on this device
  /// (with its size on GitHub): one line, or none when it is cloned or
  /// answered already (`again`: a "no" asked again).
  ProjectCloneQuestion {
    repo: String,
    again: bool,
  },
  /// The user's answer to cloning a project on this device: remembered,
  /// and a yes clones it now; the reply is a line saying what it did.
  ProjectClone {
    repo: String,
    allow: bool,
  },
  /// Moves a phone's clone to another branch of its origin.
  ProjectSwitch {
    repo: String,
    branch: String,
  },
  /// Puts a phone's clone's files back as its branch has them: `path`'s,
  /// or every change.
  ProjectDiscard {
    repo: String,
    path: Option<String>,
  },
  /// Commits a phone's clone's changes and pushes them through GitHub's
  /// API: to its branch, or to `new_branch` (with a pull request into its
  /// branch when asked).
  ProjectCommit {
    repo: String,
    message: String,
    new_branch: Option<String>,
    #[serde(default)]
    pull_request: bool,
  },
  /// A phone's registration for push notifications (design §5, milestone
  /// 4): its device token, `env` `sandbox` or `production`, and its key;
  /// written as its `device` record when new, and synced.
  RegisterDevice {
    token: String,
    env: String,
    /// The public half of the phone's X25519 key, base64: pushes are
    /// sealed for it.
    key: String,
  },
  /// Sends a test push to the user's phones; a line per phone.
  ApnsTest,
  /// A notice to the user's phones (`stitch notify`), from a thread
  /// (opening it opens that chat) or none; a line per phone.
  Notify {
    title: Option<String>,
    body: String,
    thread: Option<String>,
  },
  /// Harness issues and fixes (`stitch issue`, docs/self-improve.md
  /// §2.2): `input` as JSON, `{"action": "list" | "show" | "report" |
  /// "set" | "fix" | "schedule", …}`; `thread`, the thread whose shell
  /// asks (it files, or a fixer reports to it). `Reply::Lines`.
  Issue {
    input: String,
    thread: Option<String>,
  },
  /// Opens a pairing invitation; the reply carries the ticket.
  Invite {
    ttl_secs: u64,
  },
  Status,
  /// Restart the node: the node itself starts its successor (or, under a
  /// supervisor, exits for it to), so a restart asked from a sandboxed
  /// shell never runs the new node inside that sandbox. `now` stops what
  /// runs at once (detached threads resume); otherwise once idle.
  /// `successor: false` only stops (once idle): a service takes over.
  Restart {
    now: bool,
    successor: bool,
  },
  /// Memory's state on this node, and whether `thread` is in memory;
  /// without a thread but with `cwd` (or `global`), whether a new thread
  /// there would be (its place's `memory = false` keeps it out).
  Memory {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    thread: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    cwd: Option<String>,
    #[serde(default)]
    global: bool,
  },
  /// The models the configured providers offer (cached for minutes), of
  /// this node or of the peer `node`.
  Models {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    node: Option<String>,
  },
  /// The effective chat model, resolved exactly as a turn: explicit pick,
  /// thread choice, then the place's runnable default. Answers `Lines`
  /// with one provider/model; does not list providers or spend a call.
  EffectiveModel {
    thread: Option<String>,
    place: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
  },
  /// Memory's drift check: decisions by novelty band, strengths.
  MemoryAudit,
  /// Every unasked note's memories with their measures and outcome, as
  /// tab-separated lines (header first): what recalibrating takes.
  MemoryNotes,
  /// Takes a thread out of memory (`on: false`) or brings it back. The
  /// thread as for `Chat` (`last`: the latest of the place `cwd` is in).
  Remember {
    thread: String,
    on: bool,
    cwd: Option<String>,
  },
  Stop,
  /// Starts a detached thread toward `goal` (design §2.3).
  Detach {
    goal: String,
    model: Option<String>,
    repo: Option<String>,
    cwd: Option<String>,
    /// The thread its report goes to.
    parent: Option<String>,
    budget_usd: Option<f64>,
    budget_tokens: Option<u64>,
    budget_minutes: Option<u64>,
    /// Model calls each of its turns takes at most (the node's
    /// `[detached] max_steps` when `None`).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    max_steps: Option<u64>,
    /// It belongs to no folder.
    #[serde(default)]
    global: bool,
    /// The node to run it (delegation, design §2.7); this one when `None`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    node: Option<String>,
    /// The conversation it starts from: `none` (`None` too), `this`
    /// (`parent`'s so far), or a thread's ID.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    context: Option<String>,
    /// It runs without tools (a question to a model).
    #[serde(default)]
    no_tools: bool,
    /// Changes to its tool and skill sets (design §2.14).
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    /// Its report is kept for the user and wakes no thread (`--no-wake`,
    /// design §2.7).
    #[serde(default)]
    keep: bool,
    /// Memory remembers it (`--memory`); by default it is out of memory
    /// from its start, though it recalls (design §2.9).
    #[serde(default)]
    memory: bool,
    /// Read-only (`--read-only`): what the rules do not allow is refused,
    /// never asked.
    #[serde(default)]
    read_only: bool,
    /// Threads and efforts attached to its goal (`--scope`, design
    /// §2.27), as the `spawn` tool's `scopes`.
    #[serde(default)]
    scopes: Vec<ScopeTarget>,
  },
  /// A thread's tool and skill sets (design §2.14): the changes given are
  /// written first. Without a thread, the sets a new one in the place of
  /// `cwd` would start with, the changes applied but not written.
  Toolset {
    thread: Option<String>,
    cwd: Option<String>,
    /// A new thread that belongs to no folder (`thread` is `None`).
    #[serde(default)]
    global: bool,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
  },
  /// Continues a thread with a message, on the node that owns it; its
  /// reply is reported to `from` when given.
  Send {
    thread: String,
    text: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    from: Option<String>,
    /// Its reply is kept for the user and wakes no thread.
    #[serde(default)]
    keep: bool,
    /// Woven into the turn running on the thread, between its steps, when
    /// one runs (its last reply goes to `from`); else it starts a turn.
    #[serde(default)]
    interject: bool,
    /// Threads and efforts attached to the message (`Part::Scope`, as for
    /// `Chat`).
    #[serde(default)]
    scopes: Vec<ScopeTarget>,
  },
  /// What attaching scopes to a message would tell the model (design
  /// §2.27): the briefing `Chat` and `Send` would write with it, and with
  /// `query`, what `recall` with that scope finds there by keyword (every
  /// thread, memory on or off). `Reply::Lines`: the briefing, then the
  /// matches.
  Scope {
    scopes: Vec<ScopeTarget>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    query: Option<String>,
  },
  /// Answers an approval question (from the attention list).
  Answer {
    question: String,
    allow: bool,
    reason: Option<String>,
    /// Allowed, and what the question grants from then on in its place.
    #[serde(default)]
    always: bool,
  },
  /// Forks a thread (design §2.9): a new thread continuing it from a
  /// message, read from it; `summarize`: a summary stands for what is not
  /// kept (all of it, or what comes before the `last` messages).
  Fork {
    thread: String,
    /// Up to this message (`m12`); the latest by default.
    upto: Option<String>,
    last: Option<usize>,
    no_attachments: bool,
    no_tools: bool,
    summarize: bool,
  },
  /// Names a thread (a `set` of its title).
  Rename {
    thread: String,
    title: String,
  },
  /// Pins a thread (first in lists), or unpins it.
  Pin {
    thread: String,
    on: bool,
  },
  /// The user pressed button `item` of action set `set` (its
  /// `offer_actions` call's ID) in `thread` (design §2.30, step 2): the
  /// node that offered it looks the call up in its own records and runs
  /// it, after a turn running on the thread. `Reply::Lines`: one line,
  /// JSON `{ok, output}`; an error when the set is not live.
  Press {
    thread: String,
    set: String,
    item: u32,
  },
  /// The user has read a conversation up to `upto` (an instant, RFC 3339:
  /// a row's `last_step` they saw; none: its latest activity now), a `set`
  /// of `read` on its newest thread, written only when it moves the cursor
  /// forward (design §2.25); `Ok`. A row's `unread` is activity after it.
  Read {
    thread: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    upto: Option<String>,
  },
  /// What a conversation touched (design §2.25, step 2), read from the
  /// tool calls of its sealed segments by rule: paths read and written,
  /// URLs read, commits written, threads spawned and told; every thread
  /// of its compaction chain together. `Reply::Lines`: the one line, JSON
  /// `{thread, chain, kinds: {path: […], url: […], commit: […], thread: […]}}`,
  /// each the latest first (`artefacts.rs`).
  Artefacts {
    thread: String,
  },
  /// Every conversation that touched a path, URL (its fragment ignored),
  /// commit (its short hash, as git printed it) or thread, in time order,
  /// chains folded to their newest thread, deleted threads left out.
  /// `kind` is `path`, `url`, `commit` or `thread`. `Reply::Lines`: the one
  /// line, JSON `{kind, value, conversations: […]}` (`artefacts.rs`).
  Touched {
    kind: String,
    value: String,
  },
  /// A conversation's digest (design §2.25, step 2): the summary the quick
  /// model keeps of it, its newest thread's (else the latest of its earlier
  /// threads'), each item with the thread and message to open, and whether
  /// it covers all the thread has done. `Reply::Lines`: the one line, JSON
  /// `{thread, digest: {record, thread, headline, items: [{kind, text, at,
  /// thread, msg}], model, usd, upto, at, fresh} | null}` (`digests.rs`).
  /// With `update`, one update is run first, even inside the ten minutes a
  /// thread is digested at most once in (the day's cap holds, and the node
  /// must be the one that runs the thread), and the line gains `update:
  /// {outcome, usd, model, windows, more, dropped, day_usd, cap_usd}`.
  Digest {
    thread: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    update: Option<bool>,
  },
  /// How digests stand on this node (design §2.25): how many conversations
  /// have a fresh digest, a stale one, none; what this node's sweep would
  /// take; the day's cost against the cap; whether the sweep runs and what
  /// the last one did. With `sweep`, one sweep runs now (held to the cap,
  /// the floor for stale threads and the node's share of the conversations)
  /// and its report is in `sweep`. `Reply::Lines`: the one line, JSON
  /// (`digests.rs`).
  DigestStatus {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    sweep: Option<bool>,
  },
  /// The user has read every listed conversation (of one place, by the
  /// name a row's `place` says, when given) up to its latest activity now:
  /// a floor: one synced record ("everything up to the latest listed
  /// activity is read", for all or for that place), none when nothing is
  /// unread; `Reply::Lines` with how many conversations it read.
  ReadAll {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    place: Option<String>,
  },
  /// The user opened a thread (a client shows it): kept as the time they
  /// saw it when it moved since (loose ends, design §2.10); `Ok`.
  Opened {
    thread: String,
  },
  /// Loose ends (design §2.10): with `check`, a pass first (the quick
  /// model on the conversations not judged at their latest step); with
  /// `dismiss`, that thread's hidden until it moves; the reply is the
  /// list, as lines.
  LooseEnds {
    check: bool,
    dismiss: Option<String>,
  },
  /// Loose ends' auto-handling (design §2.10): its mode (`off`,
  /// `dry-run`, `on`) and the user's rules, set when given (the user's
  /// alone: never a thread's commands); `Reply::Lines`, the first the
  /// setting as JSON (`mode`, `synced`, `here`, `rules`).
  LooseEndsAuto {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    mode: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    rules: Option<String>,
  },
  /// Switches a thread between read-only (`Some(true)`) and writable
  /// (`Some(false)`), the user's call only, or only says which it is
  /// (`None`); the reply is one line, `read-only` or `writable`.
  ReadOnly {
    thread: String,
    on: Option<bool>,
  },
  /// A thread's transcript as a Markdown file framed as quoted material,
  /// to attach to another conversation; the reply is its path.
  Quote {
    thread: String,
  },
  /// Searches every thread's messages, by their words and (where memory
  /// runs) their meaning; `here`: only the threads of the place `cwd` is in.
  Search {
    query: String,
    here: bool,
    cwd: Option<String>,
    limit: usize,
  },
  /// Splits the checkout `cwd` is in off into a project of its own.
  Split {
    cwd: String,
  },
  /// Has the project of the origin of the checkout `cwd` is in follow it,
  /// its repository re-created from a new first commit (`project`: its name
  /// or ID, when several could be).
  Rebind {
    cwd: String,
    project: Option<String>,
  },
  /// Deletes threads for good (design §2.5, "Deleting a thread"): those
  /// named and those `spawned_by` spawned, each with its compaction chain;
  /// the reply lists them, `<id>\t<title>\t<role>`, the role `this` (named
  /// or spawned), `earlier` or `later` (a part of its conversation,
  /// compacted into it or from it); an older node sends no role.
  /// `dry_run` only lists them.
  /// `from_thread` is the thread whose shell sends it (`STITCH_THREAD`):
  /// such a request is refused, deleting being the user's.
  DeleteThreads {
    threads: Vec<String>,
    spawned_by: Option<String>,
    dry_run: bool,
    from_thread: Option<String>,
  },
  /// Rewinds a thread (design §2.15, "Rewinding a thread"): its
  /// conversation goes on from message `upto` (from its start, `None`),
  /// what came after it set aside, stored; `Reply::Ok`.
  Rewind {
    thread: String,
    upto: Option<String>,
  },
  /// Stops a running detached thread.
  Cancel {
    thread: String,
  },
  /// On a chat's connection while it runs: a message for it, woven in
  /// between its steps (`Reply::Woven` says when); one it never takes
  /// stays the client's to send.
  Interject {
    id: u64,
    text: String,
    /// Files on this machine, read by the node, or images in its blob store (`blake3:…`).
    attachments: Vec<String>,
  },
  /// On a chat's connection: takes back a message not woven yet.
  Retract {
    id: u64,
  },
  /// On a chat's connection: stops it; it ends with `interrupted`.
  Interrupt,
  /// Attaches this connection to a shell session (`Job::id`,
  /// `<thread>/<session>`): `ShellOutput` streams what it shows, from its
  /// screen as it is; `ShellInput` types into it, as the user (the agent is
  /// told). Closing the connection detaches.
  ShellAttach {
    id: String,
  },
  ShellInput(Vec<u8>),
  /// What runs in the background: commands in shell sessions here, and
  /// detached threads on every node.
  Jobs,
  /// Stops a job by its `Job::id`: Ctrl+C to a shell command (its session
  /// closed when stopped again), a watch cancelled, a detached thread
  /// cancelled.
  StopJob {
    id: String,
  },
  /// The user's answer to `Reply::Approve`.
  Approval {
    id: u64,
    allow: bool,
    /// Why, when declining: given to the model.
    reason: Option<String>,
    /// Allowed, and the approval's `grant` from then on.
    #[serde(default)]
    always: bool,
  },
  /// The pinned apps (design §2.10); the reply is `Apps`.
  Apps,
  /// Where a browser on the node's machine opens an app: `what` is a
  /// pinned app (its name or ID), a thread (the latest app it showed) or
  /// a version (`blake3:…`); the reply is `App`, its address with the
  /// node's token.
  AppOpen {
    what: String,
  },
  /// Pins an app: `what` is a version (`blake3:…`) or a thread (the
  /// latest version shown in it), as a new pinned app or as the newer
  /// version of the pinned app of that thread and name; the reply is
  /// `Apps` with it alone.
  AppPin {
    what: String,
    name: Option<String>,
    /// An SF Symbol's name, for the phone.
    icon: Option<String>,
    /// It stays on that version; else it follows its thread, moving to
    /// each newer version of its name the thread shows. Unsaid: fixed
    /// when `what` is a version, following when it is a thread.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    fixed: Option<bool>,
  },
  /// Unpins an app, by its name or ID.
  AppUnpin {
    app: String,
  },
  /// The user lets an app read the node's live state (`stitch.live`:
  /// threads, jobs, approvals), or no longer: a pinned app by its name, ID
  /// or host; an app in its conversation by its version's host, or by its
  /// name with `thread`. `Reply::Lines` says what was done.
  AppLive {
    app: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    thread: Option<String>,
    allow: bool,
  },
  /// The user grants an app what its manifest (`stitch-app.json`) asks
  /// for its own threads, or denies it (design §2.22): a pinned app by
  /// its name, ID or host, an app in its conversation by its version's
  /// host or by its name with `thread`. `Reply::Lines` lists what was
  /// granted.
  AppThreads {
    app: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    thread: Option<String>,
    allow: bool,
    /// The hash of the needs shown, to grant them; none: shown only,
    /// nothing written (a denial needs none).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    needs: Option<String>,
  },
  /// The user grants an app's containers (design §2.23) what its latest
  /// version declares: each grant by the hash shown (`build`, `run`,
  /// `shell`); none given, only shown; `deny` takes them all back.
  /// `Reply::Lines` says what.
  AppContainer {
    app: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    thread: Option<String>,
    #[serde(default)]
    deny: bool,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    build: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    run: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    shell: Option<String>,
  },
  /// Apps' container storage on this node (images, caches), or one app's
  /// removed (`remove`: its name). `Reply::Lines`.
  AppStorage {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    remove: Option<String>,
  },
  /// Pins the built-in dashboard (or moves its pin to this node's
  /// version), allowed to read live state; `Apps` with it alone.
  Dashboard,
  /// The built-in web client's address (`stitch client`): found by its builtin
  /// key, whatever the pin is named, and pinned when it is not yet.
  /// `Reply::App`; its `url` is `None` where this node serves no apps.
  Client,
  /// Continues a thread whose last turn never ended (`chat::unfinished`):
  /// a chat with no message, its step sent again from what is written.
  /// Streams and ends as `Chat` does.
  Resume {
    thread: String,
    /// `provider/model`; the node's default when `None`.
    model: Option<String>,
  },
  /// On a chat's connection: stops it as `Interrupt` does, because the app
  /// hosting the node is being suspended; recorded so (`why =
  /// "suspended"`), and it ends with stop `suspended`, to be resumed.
  Suspend,
  /// The user's own instructions (`crate::instructions`) for a place: the
  /// one named by its key (`place`, as `ThreadInfo` gives it), else the
  /// one `cwd` is in, or every thread's (`global`); `set` writes them
  /// (empty: none). `Reply::Instructions`.
  Instructions {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    place: Option<String>,
    cwd: Option<String>,
    global: bool,
    set: Option<String>,
  },
  /// Makes this connection a browser's link (design §2.18): `stitch
  /// browser-host` relaying a Chrome profile's extension. `hello` is the
  /// extension's hello (JSON); then both sides send `BrowserMsg` frames,
  /// the extension's JSON messages as they are.
  BrowserLink {
    hello: String,
  },
  BrowserMsg(String),
  /// Runs a browser tool (`browser_*`) as a thread would, outside any
  /// model turn (`stitch browser call`, and tests): `Reply::ToolDone`.
  /// `thread` names the session's owner (default: `cli`).
  BrowserCall {
    thread: Option<String>,
    tool: String,
    input: String,
    /// Where uploads come from (the caller's directory).
    cwd: Option<String>,
  },
  /// The browser profiles that have connected (`stitch browser
  /// profiles`); `disable` turns one off first. `Reply::Lines`.
  BrowserProfiles {
    disable: Option<String>,
  },
  /// Acts on schedules as the user (design §2.3, `stitch schedule`):
  /// `input` is the `schedule` tool's input as JSON (`action` and the
  /// rest); `cwd` places a new one. `Reply::Lines`.
  Schedule {
    input: String,
    cwd: Option<String>,
  },
  /// Secrets and variables (design §2.19, `stitch secret`, `stitch var`).
  /// A scope of `here` is the place `cwd` is in.
  Secret {
    op: SecretOp,
    cwd: Option<String>,
  },
  /// SSH hosts, host keys and this device's key (design §2.20, `stitch
  /// ssh`).
  Ssh {
    op: SshOp,
  },
  /// Has a thread run on `node` (design §2.7, "Continue on another
  /// node"): what is typed into it on any node goes there, and streams
  /// back. `None` (or `here`): back to running where it is typed
  /// ("Continue here"). The user's call. `Reply::Lines`, what it did.
  RunOn {
    thread: String,
    node: Option<String>,
  },
  /// Where a thread runs, when the user had it run on a node:
  /// `Reply::Lines`, that node's name, or none.
  RunsOn {
    thread: String,
  },
  /// The user's other nodes, each asked whether it runs programs (a
  /// thread can be set to run there, `RunOn`): `Reply::Lines`, one
  /// `<node>\t<yes|no|unknown|other-build>` each (`unknown`: not reached;
  /// `other-build`: reached, its answer unreadable to this build).
  Runners,
  /// The user's models, synced (design §2.7, `stitch model`): `input` is
  /// `modelset::Input` as JSON. `Reply::Lines`.
  ModelSettings {
    input: String,
  },
  /// Decisions (design §2.29, `stitch decide`): `input` as JSON, `{"action":
  /// "profiles" | "set" | "try" | "log", …}` (`set`: `profile`,
  /// `thresholds` as `["0.6", "key=0.6"]`, `fallback`, `model`; `try`:
  /// `profile`, `input`, `questions`; `log`: `profile`, `n`).
  /// `Reply::Lines`.
  Decide {
    input: String,
  },
  /// An app version's files, for `stitch app export`: `what` is a pinned
  /// app (name or ID), a thread (the latest app it showed) or a version
  /// (`blake3:…`), its files fetched here when missing; the reply is
  /// `AppFiles`, each file's path and blob, read from this node's blobs.
  AppExport {
    what: String,
  },
  /// The step a thread's loop runs the calls of now, not written yet (a
  /// step is written whole once its calls return): its calls, the one
  /// running and since when (`chat::StepInFlight`), for clients following
  /// a turn they did not start. Read here, else asked of the node that
  /// runs the thread. `Reply::Lines`: the step as JSON, or none.
  InFlight {
    thread: String,
  },
  /// Every thread this node knows of (conversations too, any node's, the
  /// latest activity first), as `stitch.live.threads` has them (`apps::live::threads`; `status` is one of `working`,
  /// `blocked`, `idle`, `ended`, `cancelled`, `since` an RFC 3339 instant).
  /// `Reply::Lines`: the one line, JSON `{node, at, threads: […], more}`;
  /// a thread waiting on an approval has `question: {id, command}`, `id`
  /// answered by `Answer`. `thread`: that thread's row alone, whatever
  /// the limit (none when unknown or deleted), for a client opening a
  /// thread older than its list. A compaction chain is one row (design
  /// §2.15, "One conversation"): its newest thread, the chain's `title`
  /// and `pinned`, and `earlier`, the threads it was compacted from,
  /// oldest first; `thread` naming one of those gives the newest's row.
  Threads {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    status: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    since: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    limit: Option<u32>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    thread: Option<String>,
    /// Only this place's threads (its name as a row's `place` says it),
    /// the limit applied after the filter; the reply's `places` still
    /// counts every place.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    place: Option<String>,
    /// The whole forest, as `stitch.live.threads` has it with
    /// `conversations` (the Atlas map, design §2.25): every conversation and
    /// thread, rows without `in_flight` and `app`, `limit` up to 5000.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    forest: Option<bool>,
  },
  /// A thread's transcript over its lineage, from the event `after` on
  /// (`apps::threads::transcript`). `Reply::Lines`: the one line, JSON
  /// `{thread, entries: […], next, forked_from, current}`; at most 500
  /// entries, `next` the `after` to read the rest from. Over a compaction
  /// chain up to `thread` (an earlier one reads as it was), each
  /// compaction a `{n, kind: "compacted", …}` divider entry; `current` is
  /// the chain's newest thread, where the conversation goes on.
  ///
  /// `tail`: a window instead of pages: the last `tail` entries of those
  /// from `after` up to the event `before` (excluded), cut between events
  /// (an event's entries and the divider before it stay together), with
  /// `start` the `n` of its first entry (the `before` that reads what
  /// precedes it) and `earlier` whether anything does. A long conversation
  /// opens at its end, and reads back a window at a time.
  Transcript {
    thread: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    after: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    before: Option<u64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    tail: Option<u64>,
  },
  /// The rest of `stitch.live` (`apps::live`), as the dashboard reads it:
  /// `what` is `attention` (the questions waiting on the user, with
  /// `Answer`'s IDs), `jobs`, `schedules` (`query` `all=1`: the ended ones
  /// too) or `loose-ends` (`crate::loose::live`). `Reply::Lines`: the one
  /// line, JSON. Loose ends are checked and dismissed with `LooseEnds`.
  /// Any change to these moves the socket's `changed` push.
  Live {
    what: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    query: Option<String>,
  },
  /// Makes a room (design §2.27, docs/rooms.md §4): a thread with
  /// `room = true` whose participants are threads, fresh models spawned
  /// under it (read-only, or without tools) or existing threads invited.
  /// `from_thread`: "Pull in model…", the room is pulled out of that 1:1
  /// conversation, which joins first, untouched, and the new participants
  /// are briefed on it. `mode` `sequential` (the default) or `parallel`
  /// (council); `hop_cap` the `@handoff`s a user message allows (2).
  /// `Reply::Lines`: the room as JSON, as `RoomInfo`.
  RoomCreate {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    title: Option<String>,
    /// The place's key (as `Chat.place`); `None`: `from_thread`'s, else
    /// global.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    place: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    from_thread: Option<String>,
    participants: Vec<RoomParticipantSpec>,
    /// Threads and efforts the room is opened on: its first message
    /// briefs the participants on them.
    #[serde(default)]
    scopes: Vec<ScopeTarget>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    mode: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    hop_cap: Option<u32>,
    /// Dollars the room's participants may spend on its turns, together.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    budget_usd: Option<f64>,
  },
  /// A room: `Reply::Lines`, the one line, JSON `{room, title,
  /// promoted_from, budget_usd, mode, hop_cap, participants: [{slug, name,
  /// model, thread, muted}], running}`.
  RoomInfo {
    thread: String,
  },
  /// Changes a room: participants added (as `RoomCreate`'s), removed,
  /// muted or unmuted by their handles, its mode and hop cap. The reply is
  /// `RoomInfo`'s.
  RoomUpdate {
    thread: String,
    #[serde(default)]
    add: Vec<RoomParticipantSpec>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    mute: Vec<String>,
    #[serde(default)]
    unmute: Vec<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    mode: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    hop_cap: Option<u32>,
  },
  /// An event from an outside program (docs/events.md §1): `topic` a
  /// slash path (`ci/escalate/retry`), `payload` JSON text (at
  /// most 64 KB). It wakes the threads and schedules subscribed to it.
  /// With `ask`, the publisher wants an answer by its deadline: with
  /// `wait`, the reply comes when it is answered or expired, else at once
  /// (`pending`; `EventStatus` follows it). `Reply::Event`. Programs not
  /// built with this `stitch` speak the stable events protocol instead
  /// (`Hello` with `events/1`, `stitch-client`).
  Event {
    topic: String,
    payload: String,
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    ask: Option<EventAsk>,
    wait: bool,
  },
  /// An event's state now, or once it changes within `wait_ms` (an ask
  /// answered or expired). `Reply::Event`.
  EventStatus {
    id: String,
    wait_ms: u64,
  },
  /// Events and subscriptions (`stitch event ls|subscribe|unsubscribe`):
  /// `input` as JSON, `{"action": "ls" | "subscribe" | "unsubscribe", …}`,
  /// answered as the user. `Reply::Lines`.
  Events {
    input: String,
    cwd: Option<String>,
  },
  /// Shares a thread as an archive (design §2.31): its conversation (its
  /// compaction chain), or with `effort` the spawn tree it is in, at
  /// `level` (`messages`, `calls`, `full`; empty: `calls`), redacted. With
  /// `preview`, nothing is written; otherwise the bundle is written into
  /// the node's downloads, and when the leak scan found something only if
  /// `confirm` is the payload's hash a preview gave. `Reply::Lines`: one
  /// JSON line, `share::Preview` (`path` set once written).
  Share {
    thread: String,
    effort: bool,
    level: String,
    preview: bool,
    #[cfg_attr(feature = "ts", ts(optional = nullable))]
    confirm: Option<String>,
  },
  /// Imports a shared thread's archive (design §2.31), its bytes:
  /// verified whole; with `preview`, only said (nothing written), else
  /// its blobs and new threads written, foreign and tainted.
  /// `Reply::Lines`: one JSON line, `share::Imported`.
  Import {
    bundle: Vec<u8>,
    preview: bool,
  },
}

/// An event's question (docs/events.md §3): answered by a thread's
/// `event_reply` within `deadline_ms`, checked against `schema` (JSON
/// Schema, as JSON text) when given.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventAsk {
  pub deadline_ms: u64,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub schema: Option<String>,
}

/// What became of an event: `published` (no ask), `pending` (an ask not
/// answered yet), `answered` (with `answer`, JSON text, and `by`, the
/// thread), `expired` (its deadline passed), `dropped` (an ask no
/// subscription took: every filter said nothing, or a limit held it), or
/// `unheard` (an ask nothing subscribes to). `matched`: the
/// subscriptions its topic matched.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventState {
  pub id: String,
  pub state: String,
  pub matched: u32,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub answer: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub by: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub note: Option<String>,
}

/// A room's participant to add (design §2.27): a model (`provider/model`,
/// `default`, `quick`; `@node` after it, or `node`, runs it there), or an
/// existing thread (`thread`, joined as it is). A model's thread is
/// read-only unless `read_only` is false, and has no tools when `tools` is
/// false. `slug` is its `@handle` (from the model's family when `None`),
/// `name` how it is shown.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomParticipantSpec {
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub model: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub thread: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub name: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub slug: Option<String>,
  #[serde(default)]
  #[cfg_attr(feature = "ts", ts(optional = nullable))]
  pub node: Option<String>,
  #[serde(default = "on")]
  pub tools: bool,
  #[serde(default = "on")]
  pub read_only: bool,
}

fn on() -> bool {
  true
}

/// A thread or effort attached to a message (design §2.27): `effort` is a
/// conversation with every thread spawned under it, `thread` the
/// conversation alone (its compaction chain). Written `effort:<id>`,
/// `thread:<id>` or a bare `<id>` (a thread) where text is taken
/// (`scopes::parse_target`).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeTarget {
  #[cfg_attr(feature = "ts", ts(type = "\"effort\" | \"thread\""))]
  pub kind: stitch_records::ScopeKind,
  pub id: String,
}

/// What `Request::Ssh` asks.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SshOp {
  /// `Reply::SshHosts`: the hosts (records, and `~/.ssh/config` here).
  Hosts,
  /// Adds a host record, or replaces the one of that name.
  HostSet(SshHost),
  /// Removes a host record.
  HostRemove { name: String },
  /// `Reply::SshHostKeys`: the host keys trusted.
  HostKeys,
  /// Forgets the keys trusted for `host` (`[address]:port`, or a host's
  /// name).
  HostKeyRemove { host: String },
  /// `Reply::SshKey`: this device's public key (made the first time).
  Key,
  /// Adds a device's public key (`key`, an `authorized_keys` line; else
  /// the key `node` published; else this node's) to `host`'s
  /// `authorized_keys`: here when this node reaches it, else asked of the
  /// nodes that do (`Reply::Lines` says which).
  Authorize {
    host: String,
    key: Option<String>,
    node: Option<String>,
  },
  /// `Reply::SshAuthorizations`: requests to authorize keys, and how
  /// they went.
  Authorizations,
}

/// An SSH host, as a record holds it (and as `~/.ssh/config` gives one).
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHost {
  pub name: String,
  /// Where it is (the name itself when empty).
  pub address: String,
  /// 0: 22.
  pub port: u16,
  /// Empty: the config's, else this user's.
  pub user: String,
  /// Another host's name to go through.
  pub jump: String,
  /// `auto` (empty: the agent and key files, then this device's key),
  /// `agent`, `device`, `secret:<name>` (a password), `key:<path>`.
  pub auth: String,
  /// `relaxed` (empty), `ask` or `allow` (`rules::HostApproval`).
  pub approval: String,
  /// Sessions run in tmux where it has it (default).
  pub no_tmux: bool,
  /// `record`, or `ssh-config` (read from `~/.ssh/config`, not a record).
  pub from: String,
}

/// A trusted host key.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHostKey {
  /// `address`, or `[address]:port`.
  pub host: String,
  /// The host's name when it was first trusted.
  pub name: String,
  /// `SHA256:…`.
  pub fingerprint: String,
  pub key: String,
  /// When and by which node.
  pub trusted: String,
  pub node: String,
}

/// A request to authorize a key on a host, and how it went.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshAuthorization {
  pub id: String,
  pub host: String,
  /// The key's comment, or its fingerprint.
  pub key: String,
  pub asked: String,
  /// `pending`, `done` or `failed`.
  pub state: String,
  /// Who did it (or why each node could not).
  pub said: Vec<String>,
}

/// A compaction, as clients are told of it (`LiveEvent::Compacted`,
/// `Reply::Compacted`) and as the lineage transcript marks it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
  /// The thread compacted.
  pub from: String,
  /// The thread it continues in.
  pub to: String,
  /// The model whose window it outgrew (`provider/model`).
  pub model: String,
  /// That model's context window, in tokens.
  pub window: u64,
  /// What its last step sent, in tokens; none when not known (a prompt
  /// the provider refused as too long).
  pub tokens: Option<u64>,
  /// The messages kept verbatim after the summary.
  pub kept: u64,
  /// The summary standing for what came before them.
  pub summary: String,
}

impl Compaction {
  /// The divider line clients show (`── compacted here (…) ──` without
  /// the rules): "compacted here (162k of 200k window): summary + last 10
  /// messages".
  pub fn said(&self) -> String {
    let figures = match self.tokens {
      Some(t) => format!("{}k of {}k window", t / 1000, self.window / 1000),
      None => format!("past the {}k window", self.window / 1000),
    };
    let rest = match self.kept {
      0 => "a summary".to_owned(),
      1 => "summary + the last message".to_owned(),
      n => format!("summary + last {n} messages"),
    };
    format!("compacted here ({figures}): {rest}")
  }
}

/// A pinned app, as `Reply::Apps` lists it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
  pub id: String,
  pub name: String,
  pub icon: Option<String>,
  /// Its version: its manifest's hash.
  pub version: String,
  pub thread: Option<String>,
  /// Where a browser on the node's machine opens it (with the node's
  /// token); none where the node serves no browser.
  pub url: Option<String>,
  /// It stays on its version; else it follows its thread.
  #[serde(default)]
  pub fixed: bool,
  /// It may read the node's live state (`Some(true)`), may not
  /// (`Some(false)`), or was never asked.
  #[serde(default)]
  pub live: Option<bool>,
  /// The built-in app it is (`dashboard`).
  #[serde(default)]
  pub builtin: Option<String>,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Reply {
  /// Fixed: first, and these fields in this order, so a client of any
  /// version reads it (`busy` and `restarting` came later; a client also
  /// reads the older form, see `client`).
  Hello {
    version: String,
    node: String,
    /// What runs on the node, when it runs a different version (see
    /// `restarting`).
    busy: Vec<String>,
    /// The client is newer and nothing runs: the node stops after this,
    /// for the client to start its own. A busy node stops once idle.
    restarting: bool,
  },
  /// The thread a chat runs in, as soon as it is known.
  Thread(String),
  Text(String),
  Thinking(String),
  /// Memory's unprompted afterthought, after the answer (before
  /// `ChatDone`).
  Afterthought(String),
  /// A tool call, as one line (`$ command`).
  ToolCall(String),
  /// Its result: whether it worked, and the output (clipped).
  ToolResult {
    ok: bool,
    output: String,
  },
  /// A tool showed an app: its name, and the address a browser on the
  /// node's machine opens it at, the node's token in it (for this client
  /// only: never stored), when the node serves apps to a browser. Also the
  /// answer to `AppOpen`.
  App {
    name: String,
    url: Option<String>,
  },
  /// A command needs the user's approval; answer with `Request::Approval`.
  /// It is a `question` record too (`question`, on the attention list):
  /// answered there first, `Answered` takes this back.
  Approve {
    id: u64,
    session: String,
    command: String,
    question: String,
    /// What answering "always" allows from then on where the thread
    /// works (command prefixes); empty: only yes or no.
    grant: Vec<String>,
  },
  /// An approval asked with `Approve` needs no answer here any more: it
  /// was answered elsewhere, or its loop stopped.
  Answered {
    id: u64,
  },
  /// Messages sent with `Request::Interject` (by ID), now given to the
  /// model.
  Woven(Vec<u64>),
  /// A note from the node (a background job ended, a report came), now
  /// given to the model.
  Note(String),
  /// The chat's thread was compacted (design §2.15): the turn goes on in
  /// `to` (a `Thread` naming it follows). Clients show the divider
  /// (`Compaction::said`) and switch to `to`.
  Compacted(Compaction),
  /// What a folder sync did.
  FolderPass {
    /// This node's changes were published.
    published: bool,
    /// Files written or removed here from other nodes' changes.
    applied: usize,
    /// Conflict copies made.
    conflicts: Vec<String>,
    /// Why nothing was done, when it was not.
    skipped: Option<String>,
  },
  /// Blobs fetched from peers: before the chat could run, or as
  /// `FetchBlobs` asked.
  Fetched(usize),
  /// `true`: another turn runs on the thread (a report woke it, another
  /// client sent to it), and this chat waits for it to end; `false`: it
  /// ended, this chat's turn begins.
  Waiting(bool),
  Jobs(Vec<Job>),
  /// What an attached session shows (terminal bytes).
  ShellOutput(Vec<u8>),
  /// The attached session ended.
  ShellEnded,
  ChatDone {
    thread: String,
    segment: String,
    model: String,
    /// `end_turn`, `max_tokens`, `refusal: <category>`...
    stop: String,
    /// The prompt not read from or written to the cache: the whole
    /// prompt is the sum of the three (`stitch_providers::Usage`).
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    usd: Option<f64>,
    forked: bool,
    /// What the thread cost so far, all its turns (API keys only).
    thread_usd: Option<f64>,
  },
  Status {
    node: String,
    endpoint: String,
    repos: Vec<String>,
    peers: Vec<String>,
    /// The node's build (`stitch_node::build`).
    build: String,
    /// A restart it waits to make (for a new build, a newer client, a
    /// `Restart` request), with what it waits for.
    pending: Option<String>,
  },
  /// One line per peer and repository.
  Synced(Vec<String>),
  Ticket(String),
  /// A detached thread started.
  Detached(String),
  /// A fork written: its thread.
  Forked(String),
  /// What a search found, best first.
  Hits(Vec<crate::memory::SearchHit>),
  /// Memory: whether it runs here, facts folded, episodes in the graph,
  /// and whether the thread asked about is in memory.
  Memory {
    available: bool,
    facts: u64,
    episodes: u64,
    thread_on: Option<bool>,
  },
  /// `provider/model` IDs, and why a provider gave none; `default` is the
  /// model the node uses when none is named.
  Models {
    models: Vec<String>,
    problems: Vec<String>,
    default: String,
  },
  /// The project a checkout was split off into.
  Split(String),
  /// What `Rebind` did: the project that follows the checkout now. (Put
  /// above `Lines` by mistake on 2026-11-22, which split the fleet's
  /// builds; it stays here now that every build has it.)
  Rebind(String),
  /// Lines to show as they are.
  Lines(Vec<String>),
  Ok,
  Error(String),
  // Peers of other versions decode what is above (postcard: by index):
  // new replies go at the end of the enum, never above this line
  // (`the_replies_peers_of_other_builds_decode_keep_their_indices`).
  /// A thread's tool and skill sets: each item and its state.
  Toolset {
    /// The thread, or `None` for a new one.
    thread: Option<String>,
    tools: Vec<SetItem>,
    skills: Vec<SetItem>,
  },
  /// A tool call run for another node (design §2.14): what it returned,
  /// images as (media type, bytes).
  ToolDone {
    ok: bool,
    output: String,
    images: Vec<(String, Vec<u8>)>,
  },
  /// Pinned apps, and where a browser on the node's machine lists them.
  Apps {
    apps: Vec<AppInfo>,
    launcher: Option<String>,
  },
  /// A chat's end: interrupted while it waited for another turn on the
  /// thread (`Waiting`), so its message was never sent.
  Unsent,
  /// The model call was cut off by the network and is sent again: the
  /// text and thinking streamed since the last step are dropped.
  Retry(String),
  /// A place's instructions as they are now (after a `set`, as written).
  Instructions {
    /// The place, as lists name it (`global`, a project, a folder).
    place: String,
    text: Option<String>,
  },
  /// A message to a browser link's extension (JSON).
  BrowserMsg(String),
  /// The secrets this node knows of (design §2.19), by scope then name.
  Secrets(Vec<SecretInfo>),
  /// The variables, by scope then name.
  Vars(Vec<VarInfo>),
  /// A secret's value, asked for to be shown (`SecretGet`).
  Secret(String),
  /// SSH hosts (design §2.20), by name.
  SshHosts(Vec<SshHost>),
  SshHostKeys(Vec<SshHostKey>),
  /// This device's public key: its `authorized_keys` line, fingerprint,
  /// and where its private half is (`secure-enclave`, `keychain`).
  SshKey {
    line: String,
    fingerprint: String,
    kind: String,
  },
  SshAuthorizations(Vec<SshAuthorization>),
  /// A chat failed after its message was written to the thread (with
  /// those woven in, and why it failed): the `Error` that ends the chat
  /// follows. The client does not give the message back to be sent again;
  /// the turn can be resumed, or the thread goes on with the next message.
  Kept,
  /// An app version's files (`AppExport`): its name and version, and each
  /// file's path in it with its blob (`blake3:…`), all here.
  AppFiles {
    name: String,
    version: String,
    files: Vec<(String, String)>,
  },
  /// What became of an event (`Event`, `EventStatus`).
  Event(EventState),
}

/// A secret as `Reply::Secrets` lists it: never its value.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretInfo {
  pub name: String,
  /// `global`, `project:<repo>`, `folder:<key>`.
  pub scope: String,
  /// The scope as lists name it (a project's or folder's name).
  pub scope_label: String,
  /// Kept on this node only, never synced.
  pub device_only: bool,
  /// This node holds its value.
  pub here: bool,
  /// The nodes that hold it, as its latest record was sealed for them
  /// (this node alone when it is this device's only).
  pub nodes: Vec<String>,
  /// Where it is used: `env OPENAI_API_KEY (global)`, `mcp github`,
  /// `provider openai`.
  pub used: Vec<String>,
  /// When it was last set (RFC 3339), and on which node.
  pub updated: String,
  pub from: String,
  /// Nodes removed since they were given it: rotating it is suggested.
  pub rotate: Vec<String>,
}

/// A variable as `Reply::Vars` lists it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarInfo {
  pub name: String,
  pub scope: String,
  pub scope_label: String,
  pub value: String,
  pub updated: String,
  pub from: String,
}

/// What `Request::Secret` asks.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SecretOp {
  /// Sets a secret's value (`None`: keeps it, to change `device_only`
  /// alone); `device_only` `None` keeps what it was (synced for a new
  /// one); `file`: in `credentials.toml` rather than the keychain.
  Set {
    name: String,
    scope: String,
    value: Option<String>,
    device_only: Option<bool>,
    file: bool,
  },
  /// Its value: `Reply::Secret`.
  Get {
    name: String,
    scope: String,
  },
  /// `Reply::Secrets`.
  List,
  /// Removed here and, unless this device's only, on every node.
  Remove {
    name: String,
    scope: String,
  },
  /// A variable set (`Reply::Ok`), removed, listed (`Reply::Vars`).
  VarSet {
    name: String,
    scope: String,
    value: String,
  },
  VarRemove {
    name: String,
    scope: String,
  },
  VarList,
}

pub async fn write<T: Serialize, W: AsyncWrite + Unpin>(w: &mut W, message: &T) -> Result<()> {
  let bytes = postcard::to_stdvec(message).map_err(|e| Error::Protocol(e.to_string()))?;
  w.write_all(&(bytes.len() as u32).to_be_bytes())
    .await
    .map_err(Error::io("node socket"))?;
  w.write_all(&bytes)
    .await
    .map_err(Error::io("node socket"))?;
  w.flush().await.map_err(Error::io("node socket"))
}

/// Reads one message; `None` when the other side closed cleanly.
pub async fn read<T: serde::de::DeserializeOwned, R: AsyncRead + Unpin>(
  r: &mut R,
) -> Result<Option<T>> {
  match read_frame(r).await? {
    Some(buf) => postcard::from_bytes(&buf)
      .map(Some)
      .map_err(|e| Error::Protocol(e.to_string())),
    None => Ok(None),
  }
}

/// Reads one frame, undecoded.
pub(crate) async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
  let mut len = [0u8; 4];
  match r.read_exact(&mut len).await {
    Ok(_) => {}
    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
    Err(e) => return Err(Error::io("node socket")(e)),
  }
  let len = u32::from_be_bytes(len) as usize;
  if len > MAX_FRAME {
    return Err(Error::Protocol(format!("frame of {len} bytes")));
  }
  // Grown as it comes, not allocated whole from a length a peer said.
  let mut buf = Vec::with_capacity(len.min(1 << 20));
  (&mut *r)
    .take(len as u64)
    .read_to_end(&mut buf)
    .await
    .map_err(Error::io("node socket"))?;
  if buf.len() != len {
    return Err(Error::io("node socket")(std::io::Error::from(
      std::io::ErrorKind::UnexpectedEof,
    )));
  }
  Ok(Some(buf))
}

/// A version's place in time: its crate version, then its protocol
/// number (`0.1.0/local-14`). `None` when it reads otherwise.
fn version_key(v: &str) -> Option<(Vec<u64>, u64)> {
  let (crate_version, protocol) = v.split_once("/local-")?;
  let parts = crate_version
    .split('.')
    .map(|p| p.parse().ok())
    .collect::<Option<Vec<u64>>>()?;
  Some((parts, protocol.parse().ok()?))
}

/// A step's tokens as clients show them: the whole prompt, the part read
/// from the cache and the part written to it, then the output.
pub fn tokens_text(input: u64, cache_read: u64, cache_write: u64, output: u64) -> String {
  let prompt = input + cache_read + cache_write;
  let written = if cache_write > 0 {
    format!(", {cache_write} to cache")
  } else {
    String::new()
  };
  format!("{prompt} in ({cache_read} cached{written}), {output} out")
}

/// Whether version `a` is newer than `b` (both readable).
pub fn newer(a: &str, b: &str) -> bool {
  matches!((version_key(a), version_key(b)), (Some(a), Some(b)) if a > b)
}

/// Something running in the background.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
  /// `shell` (a command running), `watch` (a job on a file, or on a
  /// session with nothing running) or `thread`.
  pub kind: String,
  /// What `StopJob` takes: `<thread>/<session>`, `job:<n>` or a thread's
  /// ID.
  pub id: String,
  /// The thread it works for, and its title.
  pub thread: String,
  pub title: String,
  /// Where it runs.
  pub node: String,
  /// The command, or the thread's goal.
  pub what: String,
  /// `running`, `waiting` (on an approval), or what a shell job tells.
  pub state: String,
  /// When it started (RFC 3339).
  pub since: String,
}

/// An item of a tool or skill set, as `/tools` and `stitch thread tools`
/// show it.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetItem {
  /// What toggling it writes (`web`, `github`, `github__get_*`, `release`).
  pub name: String,
  /// `group`, `mcp`, `pattern` or `skill`.
  pub kind: String,
  pub on: bool,
  /// It can run for this thread here: on this node, or shared by `node`.
  pub available: bool,
  /// The node it runs on, when not this one.
  pub node: Option<String>,
  /// A short description (a group's tools, a skill's description).
  pub note: String,
}

#[cfg(test)]
mod tests {
  use super::*;

  /// `api`'s code without its comments, blank lines and indentation: what
  /// changes the protocol (and what doc edits and formatting leave alone).
  fn api_shape() -> String {
    let src = include_str!("api.rs");
    let code = &src[..src.find("\n#[cfg(test)]").unwrap()];
    let mut h = blake3::Hasher::new();
    for line in code.lines().map(str::trim) {
      if !line.is_empty() && !line.starts_with("//") {
        h.update(line.as_bytes());
        h.update(b"\n");
      }
    }
    h.finalize().to_hex()[..16].to_string()
  }

  #[test]
  fn the_protocol_number_moves_with_the_api() {
    assert_eq!(
      api_shape(),
      crate::API_SHAPE,
      "the local protocol changed shape: bump `local-N` in VERSION (lib.rs) \
       and set API_SHAPE to the new hash, or clients of the same version \
       misread each other"
    );
  }

  /// The web client the node serves says the node's version in its Hello:
  /// a bundle of another version reloads once, then stops at the mismatch
  /// (every browser of the fleet did, after a bump without `just client`).
  #[test]
  fn the_bundled_web_client_speaks_this_version() {
    let quoted = format!("\"{}\"", crate::VERSION);
    let source = include_str!("../../../apps/client/src/version.ts");
    let bundle = include_str!("../../../apps/client/dist/app.js");
    assert!(
      source.contains(&quoted) && bundle.contains(&quoted),
      "the web client is not of {}: run `just client` and commit \
       apps/client/src/version.ts and apps/client/dist",
      crate::VERSION
    );
  }

  #[test]
  fn tokens_show_the_whole_prompt_and_its_cached_part() {
    assert_eq!(
      tokens_text(8_211, 40_102, 0, 6_120),
      "48313 in (40102 cached), 6120 out"
    );
    assert_eq!(
      tokens_text(10, 100, 50, 5),
      "160 in (100 cached, 50 to cache), 5 out"
    );
  }
}

#[cfg(test)]
mod wire_tests {
  use super::*;
  use serde_json::{Value, json};

  fn s(x: &str) -> String {
    x.to_string()
  }
  fn some(x: &str) -> Option<String> {
    Some(s(x))
  }
  fn strs() -> Vec<String> {
    vec![s("a"), s("b")]
  }

  fn requests() -> Vec<Request> {
    use Request::*;
    vec![
      Threads {
        status: some("idle"),
        since: some("2026-11-02T10:00:00Z"),
        limit: Some(3),
        thread: some("t"),
        place: some("stitch"),
        forest: Some(true),
      },
      Transcript {
        thread: s("t"),
        after: Some(4),
        before: Some(9),
        tail: Some(300),
      },
      Live {
        what: s("schedules"),
        query: some("all=1"),
      },
      Hello {
        version: s("0.1.0/local-1"),
      },
      Chat {
        thread: some("last"),
        text: s("hi"),
        attachments: strs(),
        model: some("p/m"),
        repo: some("r"),
        cwd: some("/x"),
        global: true,
        env: vec![(s("PATH"), s("/bin"))],
        tools: strs(),
        skills: strs(),
        place: some("global"),
        memory: Some(true),
        end_user: some("discord:1"),
        node: None,
        spoken: false,
        scopes: vec![
          ScopeTarget {
            kind: stitch_records::ScopeKind::Effort,
            id: s("t"),
          },
          ScopeTarget {
            kind: stitch_records::ScopeKind::Thread,
            id: s("u"),
          },
        ],
      },
      Yolo { on: Some(true) },
      Sync { peer: some("p") },
      FetchBlobs { hashes: strs() },
      FolderSync { key: s("~/notes") },
      ProjectFetch { repo: s("r") },
      ProjectCloneQuestion {
        repo: s("r"),
        again: true,
      },
      ProjectClone {
        repo: s("r"),
        allow: true,
      },
      ProjectSwitch {
        repo: s("r"),
        branch: s("b"),
      },
      ProjectDiscard {
        repo: s("r"),
        path: some("p"),
      },
      ProjectCommit {
        repo: s("r"),
        message: s("m"),
        new_branch: some("b"),
        pull_request: true,
      },
      RegisterDevice {
        token: s("t"),
        env: s("sandbox"),
        key: s("k"),
      },
      ApnsTest,
      Notify {
        title: some("t"),
        body: s("b"),
        thread: some("th"),
      },
      Issue {
        input: s("{}"),
        thread: some("th"),
      },
      Invite { ttl_secs: 60 },
      Status,
      Restart {
        now: true,
        successor: false,
      },
      Memory {
        thread: some("t"),
        cwd: some("/"),
        global: true,
      },
      Models { node: some("n") },
      EffectiveModel {
        thread: some("t"),
        place: some("p"),
        cwd: some("/"),
        model: some("m"),
      },
      MemoryAudit,
      MemoryNotes,
      Remember {
        thread: s("t"),
        on: true,
        cwd: some("/"),
      },
      Stop,
      Detach {
        goal: s("g"),
        model: some("m"),
        repo: some("r"),
        cwd: some("/"),
        parent: some("p"),
        budget_usd: Some(1.5),
        budget_tokens: Some(10),
        budget_minutes: Some(5),
        max_steps: Some(7),
        global: true,
        node: some("n"),
        context: some("this"),
        no_tools: true,
        tools: strs(),
        skills: strs(),
        keep: true,
        memory: true,
        read_only: true,
        scopes: Vec::new(),
      },
      Toolset {
        thread: some("t"),
        cwd: some("/"),
        global: true,
        tools: strs(),
        skills: strs(),
      },
      Send {
        thread: s("t"),
        text: s("x"),
        from: some("f"),
        keep: true,
        interject: true,
        scopes: vec![ScopeTarget {
          kind: stitch_records::ScopeKind::Thread,
          id: s("u"),
        }],
      },
      Scope {
        scopes: vec![ScopeTarget {
          kind: stitch_records::ScopeKind::Effort,
          id: s("t"),
        }],
        query: some("q"),
      },
      Answer {
        question: s("q"),
        allow: true,
        reason: some("r"),
        always: true,
      },
      Fork {
        thread: s("t"),
        upto: some("m1"),
        last: Some(3),
        no_attachments: true,
        no_tools: true,
        summarize: true,
      },
      Rename {
        thread: s("t"),
        title: s("T"),
      },
      Pin {
        thread: s("t"),
        on: true,
      },
      Press {
        thread: s("t"),
        set: s("c1"),
        item: 2,
      },
      Read {
        thread: s("t"),
        upto: some("2026-11-11T09:00:00Z"),
      },
      Artefacts { thread: s("t") },
      Digest {
        thread: s("t"),
        update: Some(true),
      },
      DigestStatus { sweep: Some(true) },
      Touched {
        kind: s("path"),
        value: s("src/a.rs"),
      },
      ReadAll {
        place: some("stitch"),
      },
      Opened { thread: s("t") },
      LooseEnds {
        check: true,
        dismiss: some("t"),
      },
      LooseEndsAuto {
        mode: some("dry-run"),
        rules: some("r"),
      },
      ReadOnly {
        thread: s("t"),
        on: Some(true),
      },
      Quote { thread: s("t") },
      Search {
        query: s("q"),
        here: true,
        cwd: some("/"),
        limit: 10,
      },
      Split { cwd: s("/") },
      Rebind {
        cwd: s("/"),
        project: some("p"),
      },
      DeleteThreads {
        threads: strs(),
        spawned_by: some("t"),
        dry_run: true,
        from_thread: some("t"),
      },
      Rewind {
        thread: s("t"),
        upto: some("m1"),
      },
      Cancel { thread: s("t") },
      Interject {
        id: 1,
        text: s("x"),
        attachments: strs(),
      },
      Retract { id: 1 },
      Interrupt,
      ShellAttach { id: s("t/s") },
      ShellInput(vec![1, 2, 3]),
      Jobs,
      StopJob { id: s("job:1") },
      Approval {
        id: 1,
        allow: true,
        reason: some("r"),
        always: true,
      },
      Apps,
      AppOpen { what: s("w") },
      AppPin {
        what: s("w"),
        name: some("n"),
        icon: some("i"),
        fixed: Some(true),
      },
      AppUnpin { app: s("a") },
      AppLive {
        app: s("a"),
        thread: some("t"),
        allow: true,
      },
      AppThreads {
        app: s("a"),
        thread: some("t"),
        allow: true,
        needs: some("h"),
      },
      AppContainer {
        app: s("a"),
        thread: some("t"),
        deny: true,
        build: some("b"),
        run: some("r"),
        shell: some("s"),
      },
      AppStorage { remove: some("a") },
      Dashboard,
      Client,
      Resume {
        thread: s("t"),
        model: some("m"),
      },
      Suspend,
      Instructions {
        place: some("global"),
        cwd: some("/"),
        global: true,
        set: some("x"),
      },
      BrowserLink { hello: s("{}") },
      BrowserMsg(s("{}")),
      BrowserCall {
        thread: some("t"),
        tool: s("browser_x"),
        input: s("{}"),
        cwd: some("/"),
      },
      BrowserProfiles { disable: some("p") },
      Schedule {
        input: s("{}"),
        cwd: some("/"),
      },
      Secret {
        op: SecretOp::Set {
          name: s("n"),
          scope: s("global"),
          value: some("v"),
          device_only: Some(true),
          file: true,
        },
        cwd: some("/"),
      },
      Ssh { op: SshOp::Hosts },
      RunOn {
        thread: s("t"),
        node: some("n"),
      },
      RunsOn { thread: s("t") },
      Runners,
      ModelSettings { input: s("{}") },
      Mode {
        mode: None,
        cwd: some("/"),
      },
      Decide { input: s("{}") },
      AppExport { what: s("w") },
      InFlight { thread: s("t") },
      RoomCreate {
        title: some("Review"),
        place: some("global"),
        from_thread: some("t"),
        participants: vec![RoomParticipantSpec {
          model: some("p/m@n"),
          thread: None,
          name: some("M"),
          slug: some("m"),
          node: some("n"),
          tools: true,
          read_only: true,
        }],
        scopes: vec![ScopeTarget {
          kind: stitch_records::ScopeKind::Thread,
          id: s("u"),
        }],
        mode: some("parallel"),
        hop_cap: Some(1),
        budget_usd: Some(2.5),
      },
      RoomInfo { thread: s("t") },
      RoomUpdate {
        thread: s("t"),
        add: vec![RoomParticipantSpec {
          model: None,
          thread: some("u"),
          name: None,
          slug: None,
          node: None,
          tools: false,
          read_only: false,
        }],
        remove: strs(),
        mute: strs(),
        unmute: strs(),
        mode: some("sequential"),
        hop_cap: Some(0),
      },
      Event {
        topic: s("a/b"),
        payload: s("{}"),
        ask: Some(EventAsk {
          deadline_ms: 20_000,
          schema: some("{}"),
        }),
        wait: true,
      },
      EventStatus {
        id: s("e"),
        wait_ms: 0,
      },
      Events {
        input: s("{}"),
        cwd: some("/"),
      },
      Share {
        thread: s("t"),
        effort: true,
        level: s("calls"),
        preview: true,
        confirm: some("blake3:00"),
      },
      Import {
        bundle: vec![1, 2, 3],
        preview: true,
      },
    ]
  }

  fn host() -> SshHost {
    SshHost {
      name: s("n"),
      address: s("a"),
      port: 22,
      user: s("u"),
      jump: s("j"),
      auth: s("auto"),
      approval: s("ask"),
      no_tmux: true,
      from: s("record"),
    }
  }

  fn replies() -> Vec<Reply> {
    use Reply::*;
    vec![
      Hello {
        version: s("v"),
        node: s("n"),
        busy: strs(),
        restarting: true,
      },
      Thread(s("t")),
      Text(s("x")),
      Thinking(s("x")),
      Afterthought(s("x")),
      ToolCall(s("$ ls")),
      ToolResult {
        ok: true,
        output: s("o"),
      },
      App {
        name: s("n"),
        url: some("http://x"),
      },
      Approve {
        id: 1,
        session: s("s"),
        command: s("c"),
        question: s("q"),
        grant: strs(),
      },
      Answered { id: 1 },
      Woven(vec![1, 2]),
      Note(s("n")),
      Compacted(Compaction {
        from: s("a"),
        to: s("b"),
        model: s("m"),
        window: 200_000,
        tokens: None,
        kept: 10,
        summary: s("s"),
      }),
      FolderPass {
        published: true,
        applied: 2,
        conflicts: strs(),
        skipped: some("why"),
      },
      Fetched(3),
      Waiting(true),
      Jobs(vec![Job {
        kind: s("shell"),
        id: s("t/s"),
        thread: s("t"),
        title: s("T"),
        node: s("n"),
        what: s("w"),
        state: s("running"),
        since: s("2026-11-02T10:00:00Z"),
      }]),
      ShellOutput(vec![27, 91]),
      ShellEnded,
      ChatDone {
        thread: s("t"),
        segment: s("s"),
        model: s("m"),
        stop: s("end_turn"),
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_write_tokens: 4,
        usd: Some(0.5),
        forked: true,
        thread_usd: Some(1.5),
      },
      Status {
        node: s("n"),
        endpoint: s("e"),
        repos: strs(),
        peers: strs(),
        build: s("b"),
        pending: some("p"),
      },
      Synced(strs()),
      Ticket(s("t")),
      Detached(s("t")),
      Forked(s("t")),
      Hits(vec![crate::memory::SearchHit {
        thread: s("t"),
        title: s("T"),
        msg: s("m1"),
        at: s("now"),
        sender: s("user"),
        snippet: s("..."),
        found_by: s("both"),
      }]),
      Memory {
        available: true,
        facts: 1,
        episodes: 2,
        thread_on: Some(true),
      },
      Models {
        models: strs(),
        problems: strs(),
        default: s("p/m"),
      },
      Split(s("p")),
      Rebind(s("p")),
      Lines(strs()),
      Ok,
      Error(s("e")),
      Toolset {
        thread: some("t"),
        tools: vec![SetItem {
          name: s("web"),
          kind: s("group"),
          on: true,
          available: true,
          node: some("n"),
          note: s("note"),
        }],
        skills: vec![],
      },
      ToolDone {
        ok: true,
        output: s("o"),
        images: vec![(s("image/png"), vec![1, 2])],
      },
      Apps {
        apps: vec![AppInfo {
          id: s("i"),
          name: s("n"),
          icon: some("i"),
          version: s("blake3:00"),
          thread: some("t"),
          url: some("u"),
          fixed: true,
          live: Some(false),
          builtin: some("dashboard"),
        }],
        launcher: some("l"),
      },
      Unsent,
      Retry(s("r")),
      Instructions {
        place: s("global"),
        text: some("x"),
      },
      BrowserMsg(s("{}")),
      Secrets(vec![SecretInfo {
        name: s("n"),
        scope: s("global"),
        scope_label: s("global"),
        device_only: true,
        here: true,
        nodes: strs(),
        used: strs(),
        updated: s("u"),
        from: s("f"),
        rotate: strs(),
      }]),
      Vars(vec![VarInfo {
        name: s("n"),
        scope: s("global"),
        scope_label: s("global"),
        value: s("v"),
        updated: s("u"),
        from: s("f"),
      }]),
      Secret(s("v")),
      SshHosts(vec![host()]),
      SshHostKeys(vec![SshHostKey {
        host: s("h"),
        name: s("n"),
        fingerprint: s("SHA256:x"),
        key: s("k"),
        trusted: s("t"),
        node: s("n"),
      }]),
      SshKey {
        line: s("l"),
        fingerprint: s("f"),
        kind: s("keychain"),
      },
      SshAuthorizations(vec![SshAuthorization {
        id: s("i"),
        host: s("h"),
        key: s("k"),
        asked: s("a"),
        state: s("done"),
        said: strs(),
      }]),
      Kept,
      AppFiles {
        name: s("n"),
        version: s("v"),
        files: vec![(s("p"), s("blake3:00"))],
      },
      Event(EventState {
        id: s("e"),
        state: s("answered"),
        matched: 1,
        answer: some("2"),
        by: some("t"),
        note: None,
      }),
    ]
  }

  /// Serialized and read back as JSON (the web client's wire) and as
  /// postcard (the socket's): the same value each time.
  fn round_trip<T: Serialize + serde::de::DeserializeOwned>(v: &T) -> Value {
    let json = serde_json::to_value(v).unwrap();
    let back: T = serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
    let text = serde_json::to_string(v).unwrap();
    let back: T = serde_json::from_str(&text).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
    let bytes = postcard::to_stdvec(v).unwrap();
    let back: T = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
    json
  }

  /// The variant a value serializes as: a string (unit) or the one key.
  fn variant(json: &Value) -> String {
    match json {
      Value::String(s) => s.clone(),
      Value::Object(o) if o.len() == 1 => o.keys().next().unwrap().clone(),
      other => panic!("not an externally tagged enum: {other}"),
    }
  }

  /// The variants an enum declares in this file's source.
  fn declared(name: &str) -> Vec<String> {
    let src = include_str!("api.rs");
    let src = &src[..src.find("\n#[cfg(test)]").unwrap()];
    let start = src.find(&format!("pub enum {name} {{")).unwrap();
    let body = &src[start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    body
      .lines()
      .filter(|l| l.starts_with("  ") && !l.starts_with("   ") && !l.starts_with("  //"))
      .filter(|l| !l.starts_with("  #") && !l.starts_with("  }"))
      .map(|l| {
        l.trim()
          .split(|c: char| !c.is_alphanumeric())
          .next()
          .unwrap()
          .to_string()
      })
      .filter(|v| !v.is_empty())
      .collect()
  }

  #[test]
  fn every_request_variant_round_trips_as_json_and_postcard() {
    let mut seen: Vec<String> = requests().iter().map(|r| variant(&round_trip(r))).collect();
    seen.sort();
    let mut want = declared("Request");
    want.sort();
    assert_eq!(
      seen, want,
      "a Request variant has no sample (or a sample is doubled)"
    );
  }

  #[test]
  fn every_reply_variant_round_trips_as_json_and_postcard() {
    let mut seen: Vec<String> = replies().iter().map(|r| variant(&round_trip(r))).collect();
    seen.sort();
    let mut want = declared("Reply");
    want.sort();
    assert_eq!(
      seen, want,
      "a Reply variant has no sample (or a sample is doubled)"
    );
  }

  /// Peers of other builds answer each other's questions with replies
  /// (postcard: a variant is its index), so every reply keeps its place
  /// and new ones go at the end: `Rebind` inserted above `Lines` made
  /// every peer of another build read `Lines` as another variant (every
  /// node offered as offline, 2026-11-23).
  #[test]
  fn the_replies_peers_of_other_builds_decode_keep_their_indices() {
    let pinned = [
      "Hello",
      "Thread",
      "Text",
      "Thinking",
      "Afterthought",
      "ToolCall",
      "ToolResult",
      "App",
      "Approve",
      "Answered",
      "Woven",
      "Note",
      "Compacted",
      "FolderPass",
      "Fetched",
      "Waiting",
      "Jobs",
      "ShellOutput",
      "ShellEnded",
      "ChatDone",
      "Status",
      "Synced",
      "Ticket",
      "Detached",
      "Forked",
      "Hits",
      "Memory",
      "Models",
      "Split",
      "Rebind",
      "Lines",
      "Ok",
      "Error",
      "Toolset",
      "ToolDone",
      "Apps",
      "Unsent",
      "Retry",
      "Instructions",
      "BrowserMsg",
      "Secrets",
      "Vars",
      "Secret",
      "SshHosts",
      "SshHostKeys",
      "SshKey",
      "SshAuthorizations",
      "Kept",
      "AppFiles",
      "Event",
    ];
    let declared = declared("Reply");
    assert_eq!(
      &declared[..pinned.len()],
      &pinned[..],
      "a Reply variant was inserted, removed or moved: new ones go at the end"
    );
    let tag = |r: &Reply| postcard::to_stdvec(r).unwrap()[0];
    assert_eq!(tag(&Reply::Lines(strs())), 30);
    assert_eq!(tag(&Reply::Error(s("e"))), 32);
  }

  #[test]
  fn the_sub_operations_and_streamed_types_round_trip() {
    let ops = [
      SecretOp::Set {
        name: s("n"),
        scope: s("global"),
        value: None,
        device_only: None,
        file: false,
      },
      SecretOp::Get {
        name: s("n"),
        scope: s("global"),
      },
      SecretOp::List,
      SecretOp::Remove {
        name: s("n"),
        scope: s("global"),
      },
      SecretOp::VarSet {
        name: s("n"),
        scope: s("global"),
        value: s("v"),
      },
      SecretOp::VarRemove {
        name: s("n"),
        scope: s("global"),
      },
      SecretOp::VarList,
    ];
    for o in &ops {
      round_trip(o);
    }
    let ssh = [
      SshOp::Hosts,
      SshOp::HostSet(host()),
      SshOp::HostRemove { name: s("n") },
      SshOp::HostKeys,
      SshOp::HostKeyRemove { host: s("h") },
      SshOp::Key,
      SshOp::Authorize {
        host: s("h"),
        key: some("k"),
        node: None,
      },
      SshOp::Authorizations,
    ];
    for o in &ssh {
      round_trip(o);
    }

    // What follows travels as JSON inside `Reply::Lines`.
    use crate::live::{Followed, LiveEvent};
    let events = vec![
      LiveEvent::Started,
      LiveEvent::Text(s("t")),
      LiveEvent::Thinking(s("t")),
      LiveEvent::ToolCall(s("$ ls")),
      LiveEvent::ToolResult {
        ok: true,
        output: s("o"),
      },
      LiveEvent::Note(s("n")),
      LiveEvent::Retry(s("r")),
      LiveEvent::Ended,
      LiveEvent::Compacted(Compaction {
        from: s("a"),
        to: s("b"),
        model: s("m"),
        window: 200_000,
        tokens: Some(1),
        kept: 0,
        summary: s("s"),
      }),
    ];
    for e in &events {
      round_trip(e);
    }
    round_trip(&Followed {
      next: 9,
      events,
      gap: true,
      taken: true,
      done: false,
      running: true,
    });
    use crate::chat::{CallInFlight, StepInFlight};
    let step = StepInFlight {
      calls: vec![CallInFlight {
        id: s("c"),
        name: s("shell_run"),
        input: json!({"command": "ls"}),
        summary: s("$ ls"),
        since: Some("2026-11-02T10:00:00Z".parse().unwrap()),
        ok: Some(true),
      }],
      running: Some(0),
    };
    let json = serde_json::to_string(&step).unwrap();
    assert_eq!(StepInFlight::from_json(&json), Some(step));
  }

  /// Postcard is not self-describing: a field skipped when empty shifts
  /// everything after it.
  #[test]
  fn no_protocol_type_skips_fields_when_serializing() {
    for (file, src) in [
      ("api.rs", include_str!("api.rs")),
      ("memory.rs (SearchHit)", include_str!("memory.rs")),
      ("live.rs", include_str!("live.rs")),
    ] {
      let code = match src.find("\n#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
      };
      let hit = if file.starts_with("memory") {
        let at = code.find("pub struct SearchHit").unwrap();
        code[at..].find("\n}\n").map(|e| &code[at..at + e]).unwrap()
      } else {
        code
      };
      assert!(!hit.contains("skip_serializing_if"), "{file} skips fields");
    }
    let chat = include_str!("chat.rs");
    let at = chat.find("pub struct CallInFlight").unwrap();
    let end = chat[at..].find("impl StepInFlight").unwrap();
    assert!(!chat[at..at + end].contains("skip_serializing_if"));
  }
}
